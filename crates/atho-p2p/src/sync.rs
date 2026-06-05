// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! P2P synchronization state and request planning.
use crate::config::network_params;
use crate::protocol::{GetHeadersMessage, Hash48, ProtocolError};
use atho_core::block::{Block, BlockHeader};
use atho_core::consensus::pow;
use atho_core::network::Network;
use std::cmp::Ordering;
use std::collections::BTreeMap;

const MAX_OUTSTANDING_HEADER_LOCATORS_PER_PEER: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncState {
    pub best_height: u64,
    pub headers_synced: bool,
    pub best_tip: Option<Hash48>,
    pub best_chainwork: Option<Hash48>,
    pub inflight_headers_peer: Option<String>,
    pub locator_hashes: Vec<Hash48>,
    pub(crate) requested_locator_hashes: Vec<Hash48>,
    pub(crate) requested_locator_hashes_by_peer: BTreeMap<String, Vec<Vec<Hash48>>>,
}

impl SyncState {
    pub fn prime(&mut self, blocks: &[Block]) {
        let best_height = blocks.last().map(|block| block.header.height).unwrap_or(0);
        let best_tip = blocks
            .last()
            .map(|block| Hash48::from(block.header.block_hash()));
        let best_chainwork = Some(chainwork_hash(blocks));
        self.prime_with_locator_and_chainwork(
            best_height,
            best_tip,
            best_chainwork,
            block_locator(blocks),
        );
    }

    pub fn prime_with_locator(
        &mut self,
        best_height: u64,
        best_tip: Option<Hash48>,
        locator_hashes: Vec<Hash48>,
    ) {
        self.prime_with_locator_and_chainwork(best_height, best_tip, None, locator_hashes);
    }

    pub fn prime_with_locator_and_chainwork(
        &mut self,
        best_height: u64,
        best_tip: Option<Hash48>,
        best_chainwork: Option<Hash48>,
        locator_hashes: Vec<Hash48>,
    ) {
        self.best_height = best_height;
        self.best_tip = best_tip;
        self.best_chainwork = known_chainwork(best_chainwork);
        self.locator_hashes = locator_hashes;
        self.clear_requested_header_locators();
        self.headers_synced = true;
        crate::audit::append_log(
            "p2p",
            &format!(
                "headers-first sync primed best_height={} locator_len={}",
                self.best_height,
                self.locator_hashes.len()
            ),
        );
    }

    pub fn request_headers(
        &mut self,
        peer: impl Into<String>,
        stop_hash: [u8; 48],
    ) -> GetHeadersMessage {
        let peer = peer.into();
        self.inflight_headers_peer = Some(peer.clone());
        self.requested_locator_hashes = self.locator_hashes.clone();
        let peer_locators = self
            .requested_locator_hashes_by_peer
            .entry(peer.clone())
            .or_default();
        peer_locators.push(self.locator_hashes.clone());
        while peer_locators.len() > MAX_OUTSTANDING_HEADER_LOCATORS_PER_PEER {
            peer_locators.remove(0);
        }
        crate::audit::append_log(
            "p2p",
            &format!(
                "requesting headers peer={} locator_len={}",
                peer,
                self.locator_hashes.len()
            ),
        );
        GetHeadersMessage {
            locator_hashes: self.locator_hashes.clone(),
            stop_hash: Hash48::from(stop_hash),
        }
    }

    pub fn accept_headers(
        &mut self,
        network: Network,
        headers: &[BlockHeader],
    ) -> Result<(), ProtocolError> {
        self.accept_headers_with_locator(network, headers, self.requested_locator_hashes.clone())?;
        self.clear_requested_header_locators();
        Ok(())
    }

    pub fn accept_headers_from_peer(
        &mut self,
        peer: &str,
        network: Network,
        headers: &[BlockHeader],
    ) -> Result<(), ProtocolError> {
        let requested_locator_hashes = self
            .take_requested_locator_for_headers(peer, headers)
            .unwrap_or_else(|| self.requested_locator_hashes.clone());
        self.accept_headers_with_locator(network, headers, requested_locator_hashes)?;
        if self.inflight_headers_peer.as_deref() == Some(peer) {
            self.inflight_headers_peer = None;
        }
        Ok(())
    }

    fn take_requested_locator_for_headers(
        &mut self,
        peer: &str,
        headers: &[BlockHeader],
    ) -> Option<Vec<Hash48>> {
        let mut locators = self.requested_locator_hashes_by_peer.remove(peer)?;
        let mut matched_pos = None;
        let locator = if let Some(first) = headers.first() {
            let previous_hash = Hash48::from(first.previous_block_hash);
            matched_pos = locators
                .iter()
                .position(|locator| locator.contains(&previous_hash));
            match matched_pos {
                Some(index) => locators.remove(index),
                None => locators.first().cloned().unwrap_or_default(),
            }
        } else if locators.is_empty() {
            Vec::new()
        } else {
            matched_pos = Some(0);
            locators.remove(0)
        };
        if !locators.is_empty() || matched_pos.is_none() {
            self.requested_locator_hashes_by_peer
                .insert(peer.to_string(), locators);
        }
        Some(locator)
    }

    fn accept_headers_with_locator(
        &mut self,
        network: Network,
        headers: &[BlockHeader],
        requested_locator_hashes: Vec<Hash48>,
    ) -> Result<(), ProtocolError> {
        if headers.len() > network_params(network).limits.max_headers_per_message {
            return Err(ProtocolError::TooManyHeaders);
        }
        if let Some(first) = headers.first() {
            if !requested_locator_hashes.is_empty()
                && !requested_locator_hashes.contains(&Hash48::from(first.previous_block_hash))
            {
                return Err(ProtocolError::InvalidHeadersSequence);
            }
        }
        for header in headers {
            if header.network_id != network
                || !pow::target_within_bounds(&header.difficulty_target_or_bits)
                || !pow::meets_target(&header.block_hash(), &header.difficulty_target_or_bits)
            {
                return Err(ProtocolError::InvalidHeadersSequence);
            }
        }
        for window in headers.windows(2) {
            let [left, right] = window else {
                continue;
            };
            if right.previous_block_hash != left.block_hash()
                || right.height != left.height.saturating_add(1)
            {
                return Err(ProtocolError::InvalidHeadersSequence);
            }
        }
        if let Some(last) = headers.last() {
            let prior_target = self.best_height;
            let prior_headers_synced = self.headers_synced;
            self.best_height = self.best_height.max(last.height);
            let reaches_prior_target = last.height >= prior_target;
            let should_advance_locator = reaches_prior_target || !prior_headers_synced;
            if should_advance_locator {
                self.best_tip = Some(Hash48::from(last.block_hash()));
                self.locator_hashes
                    .retain(|hash| *hash != Hash48::from(last.block_hash()));
                self.locator_hashes
                    .insert(0, Hash48::from(last.block_hash()));
                self.locator_hashes.truncate(32);
            }
            if reaches_prior_target {
                self.headers_synced =
                    headers.len() < network_params(network).limits.max_headers_per_message;
            } else if !prior_headers_synced {
                self.headers_synced = false;
            }
            crate::audit::append_log(
                "p2p",
                &format!(
                    "accepted headers batch={} best_height={} synced={}",
                    headers.len(),
                    self.best_height,
                    self.headers_synced
                ),
            );
        } else {
            self.headers_synced = true;
        }
        Ok(())
    }

    pub fn observe_tip(
        &mut self,
        observed_height: u64,
        observed_tip: Option<Hash48>,
        observed_chainwork: Option<Hash48>,
    ) -> bool {
        let current = ChainTipCandidate {
            height: self.best_height,
            tip: self.best_tip,
            chainwork: self.best_chainwork,
        };
        let observed = ChainTipCandidate {
            height: observed_height,
            tip: observed_tip,
            chainwork: known_chainwork(observed_chainwork),
        };
        if compare_tip_candidates(&observed, &current).is_gt() {
            self.best_height = observed.height;
            self.best_tip = observed.tip;
            self.best_chainwork = observed.chainwork;
            self.headers_synced = false;
            true
        } else {
            false
        }
    }

    pub fn note_local_tip(
        &mut self,
        local_height: u64,
        local_tip: Option<Hash48>,
        local_chainwork: Option<Hash48>,
    ) {
        let local = ChainTipCandidate {
            height: local_height,
            tip: local_tip,
            chainwork: known_chainwork(local_chainwork),
        };
        let target = ChainTipCandidate {
            height: self.best_height,
            tip: self.best_tip,
            chainwork: self.best_chainwork,
        };
        if compare_tip_candidates(&local, &target).is_ge() {
            self.best_height = local.height;
            self.best_tip = local.tip;
            self.best_chainwork = local.chainwork.or(self.best_chainwork);
            self.headers_synced = true;
            self.clear_requested_header_locators();
        } else {
            self.headers_synced = false;
        }
    }

    pub fn local_tip_satisfies_target(
        &self,
        local_height: u64,
        local_tip: Option<Hash48>,
        local_chainwork: Option<Hash48>,
    ) -> bool {
        let local = ChainTipCandidate {
            height: local_height,
            tip: local_tip,
            chainwork: known_chainwork(local_chainwork),
        };
        let target = ChainTipCandidate {
            height: self.best_height,
            tip: self.best_tip,
            chainwork: self.best_chainwork,
        };
        compare_tip_candidates(&local, &target).is_ge()
    }

    pub fn clear_requested_header_locators(&mut self) {
        self.inflight_headers_peer = None;
        self.requested_locator_hashes.clear();
        self.requested_locator_hashes_by_peer.clear();
    }
}

pub fn block_locator(blocks: &[Block]) -> Vec<Hash48> {
    if blocks.is_empty() {
        return Vec::new();
    }

    let mut hashes = Vec::new();
    let mut index = blocks.len() - 1;
    let mut step = 1usize;

    loop {
        hashes.push(Hash48::from(blocks[index].header.block_hash()));
        if index == 0 {
            break;
        }
        if hashes.len() >= 10 {
            step = step.saturating_mul(2);
        }
        index = index.saturating_sub(step);
        if index == 0 {
            hashes.push(Hash48::from(blocks[0].header.block_hash()));
            break;
        }
    }

    hashes
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChainTipCandidate {
    height: u64,
    tip: Option<Hash48>,
    chainwork: Option<Hash48>,
}

fn known_chainwork(chainwork: Option<Hash48>) -> Option<Hash48> {
    chainwork.filter(|work| *work != Hash48::ZERO)
}

fn compare_tip_candidates(candidate: &ChainTipCandidate, current: &ChainTipCandidate) -> Ordering {
    match (candidate.chainwork, current.chainwork) {
        (Some(candidate_work), Some(current_work)) if candidate_work != current_work => {
            return candidate_work.cmp(&current_work);
        }
        _ => {}
    }

    candidate
        .height
        .cmp(&current.height)
        .then_with(|| match (candidate.tip, current.tip) {
            (Some(candidate_tip), Some(current_tip)) if candidate_tip != current_tip => {
                current_tip.cmp(&candidate_tip)
            }
            (Some(_), None) => Ordering::Greater,
            (None, Some(_)) => Ordering::Less,
            _ => Ordering::Equal,
        })
}

fn chainwork_hash(blocks: &[Block]) -> Hash48 {
    let bytes = pow::accumulated_chain_work(blocks).to_bytes_be();
    let mut out = [0u8; 48];
    let out_len = out.len();
    if bytes.len() >= out_len {
        out.copy_from_slice(&bytes[bytes.len() - out_len..]);
    } else {
        let start = out_len - bytes.len();
        out[start..].copy_from_slice(&bytes);
    }
    Hash48::from(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use atho_core::block::Block;

    fn test_header(
        network: Network,
        height: u64,
        previous_block_hash: [u8; 48],
        nonce: u64,
    ) -> BlockHeader {
        BlockHeader {
            version: 1,
            network_id: network,
            height,
            previous_block_hash,
            merkle_root: [1; 48],
            witness_root: [2; 48],
            founders_hash_sha3_384: BlockHeader::consensus_founders_hash_sha3_384(),
            founders_hash_sha3_512: BlockHeader::consensus_founders_hash_sha3_512(),
            timestamp: 1_700_000_000 + height,
            difficulty_target_or_bits: pow::initial_target_for_network(network),
            nonce,
        }
    }

    fn solved_header(network: Network, height: u64, previous_block_hash: [u8; 48]) -> BlockHeader {
        let target = pow::initial_target_for_network(network);
        let mainnet_first = test_header(Network::Mainnet, 1, [0; 48], 40_706);
        let mainnet_second = test_header(Network::Mainnet, 2, mainnet_first.block_hash(), 46_121);
        let nonce = match (network, height, previous_block_hash) {
            (Network::Mainnet, 1, previous) if previous == [0; 48] => Some(40_706),
            (Network::Mainnet, 2, previous) if previous == mainnet_first.block_hash() => {
                Some(46_121)
            }
            (Network::Mainnet, 3, previous) if previous == mainnet_second.block_hash() => {
                Some(358_819)
            }
            (Network::Mainnet, 2, previous) if previous == [9; 48] => Some(24_728),
            (Network::Mainnet, 335, previous) if previous == [9; 48] => Some(8_046),
            (Network::Testnet, 1, previous) if previous == [0; 48] => Some(58_475),
            _ => None,
        };
        if let Some(nonce) = nonce {
            let header = test_header(network, height, previous_block_hash, nonce);
            if pow::meets_target(&header.block_hash(), &target) {
                return header;
            }
        }

        let mut header = test_header(network, height, previous_block_hash, 0);
        loop {
            if pow::meets_target(&header.block_hash(), &target) {
                return header;
            }
            header.nonce = header.nonce.checked_add(1).expect("header nonce space");
        }
    }

    fn unsolved_header(
        network: Network,
        height: u64,
        previous_block_hash: [u8; 48],
    ) -> BlockHeader {
        let target = pow::initial_target_for_network(network);
        let mut header = test_header(network, height, previous_block_hash, 0);
        while pow::meets_target(&header.block_hash(), &target) {
            header.nonce = header.nonce.checked_add(1).expect("header nonce space");
        }
        header
    }

    #[test]
    fn locator_prefers_recent_blocks_then_exponentially_steps_back() {
        let blocks: Vec<Block> = (0..20)
            .map(|height| Block {
                header: BlockHeader {
                    version: 1,
                    network_id: Network::Mainnet,
                    height,
                    previous_block_hash: [height as u8; 48],
                    merkle_root: [1; 48],
                    witness_root: [2; 48],
                    founders_hash_sha3_384: BlockHeader::consensus_founders_hash_sha3_384(),
                    founders_hash_sha3_512: BlockHeader::consensus_founders_hash_sha3_512(),
                    timestamp: 1_700_000_000 + height,
                    difficulty_target_or_bits: [3; 48],
                    nonce: height,
                },
                ..Block::default()
            })
            .collect();
        let locator = block_locator(&blocks);
        assert!(!locator.is_empty());
        assert_eq!(
            locator[0],
            Hash48::from(blocks.last().expect("tip").header.block_hash())
        );
    }

    #[test]
    fn headers_sequence_must_link() {
        let mut state = SyncState::default();
        let first = solved_header(Network::Mainnet, 1, [0; 48]);
        let second = solved_header(Network::Mainnet, 2, [9; 48]);
        assert_eq!(
            state.accept_headers(Network::Mainnet, &[first, second]),
            Err(ProtocolError::InvalidHeadersSequence)
        );
    }

    #[test]
    fn headers_must_match_network_even_for_single_header() {
        let mut state = SyncState::default();
        let header = solved_header(Network::Testnet, 1, [0; 48]);

        assert_eq!(
            state.accept_headers(Network::Mainnet, &[header]),
            Err(ProtocolError::InvalidHeadersSequence)
        );
    }

    #[test]
    fn headers_must_satisfy_committed_proof_of_work() {
        let mut state = SyncState {
            requested_locator_hashes: vec![Hash48::from([0; 48])],
            ..Default::default()
        };
        let header = unsolved_header(Network::Mainnet, 1, [0; 48]);

        assert_eq!(
            state.accept_headers(Network::Mainnet, &[header]),
            Err(ProtocolError::InvalidHeadersSequence)
        );
    }

    #[test]
    fn short_header_batch_does_not_shrink_a_higher_advertised_sync_target() {
        let mut state = SyncState {
            best_height: 128,
            headers_synced: false,
            ..SyncState::default()
        };
        state.requested_locator_hashes = vec![Hash48::from([0; 48])];
        let first = solved_header(Network::Mainnet, 1, [0; 48]);
        let second = solved_header(Network::Mainnet, 2, first.block_hash());
        let third = solved_header(Network::Mainnet, 3, second.block_hash());

        state
            .accept_headers(Network::Mainnet, &[first, second, third.clone()])
            .expect("accept headers");

        assert_eq!(state.best_height, 128);
        assert!(!state.headers_synced);
        assert_eq!(state.best_tip, Some(Hash48::from(third.block_hash())));
        assert_eq!(
            state.locator_hashes.first().copied(),
            Some(Hash48::from(third.block_hash()))
        );
    }

    #[test]
    fn stale_header_batch_does_not_unsync_a_higher_synced_target() {
        let synced_tip = Hash48::from([8; 48]);
        let mut state = SyncState {
            best_height: 128,
            best_tip: Some(synced_tip),
            locator_hashes: vec![synced_tip],
            headers_synced: true,
            ..SyncState::default()
        };
        state.requested_locator_hashes = vec![Hash48::from([0; 48])];
        let first = solved_header(Network::Mainnet, 1, [0; 48]);
        let second = solved_header(Network::Mainnet, 2, first.block_hash());
        let third = solved_header(Network::Mainnet, 3, second.block_hash());

        state
            .accept_headers(Network::Mainnet, &[first, second, third])
            .expect("accept stale peer headers");

        assert_eq!(state.best_height, 128);
        assert!(state.headers_synced);
        assert_eq!(state.best_tip, Some(synced_tip));
        assert_eq!(state.locator_hashes.first().copied(), Some(synced_tip));
    }

    #[test]
    fn headers_must_continue_from_requested_locator() {
        let mut state = SyncState {
            requested_locator_hashes: vec![Hash48::from([1; 48])],
            ..Default::default()
        };
        let header = solved_header(Network::Mainnet, 335, [9; 48]);

        assert_eq!(
            state.accept_headers(Network::Mainnet, &[header]),
            Err(ProtocolError::InvalidHeadersSequence)
        );
    }

    #[test]
    fn concurrent_header_responses_use_each_peers_locator() {
        let mut state = SyncState {
            locator_hashes: vec![Hash48::from([9; 48])],
            ..Default::default()
        };
        state.request_headers("first", [0; 48]);
        state.locator_hashes = vec![Hash48::from([1; 48])];
        state.request_headers("second", [0; 48]);

        let header = solved_header(Network::Mainnet, 335, [9; 48]);
        state
            .accept_headers_from_peer("first", Network::Mainnet, &[header])
            .expect("late first peer response should use first peer locator");
    }

    #[test]
    fn overlapping_header_responses_from_same_peer_keep_older_locator() {
        let mut state = SyncState {
            locator_hashes: vec![Hash48::from([9; 48])],
            ..Default::default()
        };
        state.request_headers("peer", [0; 48]);
        state.locator_hashes = vec![Hash48::from([1; 48])];
        state.request_headers("peer", [0; 48]);

        let header = solved_header(Network::Mainnet, 335, [9; 48]);
        state
            .accept_headers_from_peer("peer", Network::Mainnet, &[header])
            .expect("late response should match any outstanding locator for that peer");
    }
}
