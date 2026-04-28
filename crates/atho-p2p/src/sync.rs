use crate::config::network_params;
use crate::protocol::{GetHeadersMessage, Hash48, ProtocolError};
use atho_core::block::{Block, BlockHeader};
use atho_core::network::Network;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncState {
    pub best_height: u64,
    pub headers_synced: bool,
    pub best_tip: Option<Hash48>,
    pub inflight_headers_peer: Option<String>,
    pub locator_hashes: Vec<Hash48>,
}

impl SyncState {
    pub fn prime(&mut self, blocks: &[Block]) {
        self.best_height = blocks.last().map(|block| block.header.height).unwrap_or(0);
        self.best_tip = blocks
            .last()
            .map(|block| Hash48::from(block.header.block_hash()));
        self.locator_hashes = block_locator(blocks);
        self.headers_synced = false;
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
        if headers.len() > network_params(network).limits.max_headers_per_message {
            return Err(ProtocolError::TooManyHeaders);
        }
        for window in headers.windows(2) {
            let [left, right] = window else {
                continue;
            };
            if left.network_id != network
                || right.network_id != network
                || right.previous_block_hash != left.block_hash()
                || right.height != left.height.saturating_add(1)
            {
                return Err(ProtocolError::InvalidHeadersSequence);
            }
        }
        if let Some(first) = headers.first() {
            self.best_height = headers
                .last()
                .map(|header| header.height)
                .unwrap_or(self.best_height);
            self.best_tip = headers
                .last()
                .map(|header| Hash48::from(header.block_hash()));
            self.headers_synced =
                headers.len() < network_params(network).limits.max_headers_per_message;
            self.locator_hashes
                .insert(0, Hash48::from(first.block_hash()));
            self.locator_hashes.truncate(32);
            self.inflight_headers_peer = None;
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
            self.inflight_headers_peer = None;
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use atho_core::block::Block;

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
        let first = BlockHeader {
            version: 1,
            network_id: Network::Mainnet,
            height: 1,
            previous_block_hash: [0; 48],
            merkle_root: [1; 48],
            witness_root: [2; 48],
            timestamp: 1_700_000_001,
            difficulty_target_or_bits: [3; 48],
            nonce: 1,
        };
        let mut second = first.clone();
        second.height = 2;
        second.previous_block_hash = [9; 48];
        assert_eq!(
            state.accept_headers(Network::Mainnet, &[first, second]),
            Err(ProtocolError::InvalidHeadersSequence)
        );
    }
}
