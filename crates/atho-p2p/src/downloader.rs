use crate::protocol::{Hash48, InventoryKind, InventoryVector};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadAssignment {
    pub peer: String,
    pub inventory: Vec<InventoryVector>,
}

#[derive(Debug, Default, Clone)]
pub struct BlockDownloadScheduler {
    ready_peers: BTreeSet<String>,
    pending: VecDeque<Hash48>,
    inflight_by_peer: BTreeMap<String, BTreeSet<Hash48>>,
    inflight_owner: BTreeMap<Hash48, String>,
    peer_hints: BTreeMap<Hash48, BTreeSet<String>>,
    completed: BTreeSet<Hash48>,
}

impl BlockDownloadScheduler {
    pub fn note_peer_ready(&mut self, peer: impl Into<String>) {
        let peer = peer.into();
        self.ready_peers.insert(peer.clone());
        self.inflight_by_peer.entry(peer).or_default();
    }

    pub fn note_peer_disconnected(&mut self, peer: &str) {
        self.ready_peers.remove(peer);
        if let Some(inflight) = self.inflight_by_peer.remove(peer) {
            for hash in inflight {
                self.inflight_owner.remove(&hash);
                if !self.pending.contains(&hash) && !self.completed.contains(&hash) {
                    self.pending.push_back(hash);
                }
                if let Some(hints) = self.peer_hints.get_mut(&hash) {
                    hints.remove(peer);
                }
            }
        }
    }

    pub fn note_headers<I>(&mut self, peer: &str, hashes: I)
    where
        I: IntoIterator<Item = [u8; 48]>,
    {
        for hash in hashes.into_iter().map(Hash48::from) {
            self.peer_hints
                .entry(hash)
                .or_default()
                .insert(peer.to_string());
            if self.completed.contains(&hash)
                || self.inflight_owner.contains_key(&hash)
                || self.pending.contains(&hash)
            {
                continue;
            }
            self.pending.push_back(hash);
        }
    }

    pub fn note_inventory(&mut self, peer: &str, hash: [u8; 48]) {
        self.peer_hints
            .entry(Hash48::from(hash))
            .or_default()
            .insert(peer.to_string());
    }

    pub fn note_block_received(&mut self, hash: [u8; 48]) {
        let hash = Hash48::from(hash);
        self.completed.insert(hash);
        self.pending.retain(|candidate| *candidate != hash);
        if let Some(owner) = self.inflight_owner.remove(&hash) {
            if let Some(inflight) = self.inflight_by_peer.get_mut(&owner) {
                inflight.remove(&hash);
            }
        }
    }

    pub fn note_not_found(&mut self, peer: &str, hashes: &[[u8; 48]]) {
        for hash in hashes.iter().copied().map(Hash48::from) {
            if let Some(owner) = self.inflight_owner.get(&hash) {
                if owner != peer {
                    continue;
                }
            }
            self.inflight_owner.remove(&hash);
            if let Some(inflight) = self.inflight_by_peer.get_mut(peer) {
                inflight.remove(&hash);
            }
            if let Some(hints) = self.peer_hints.get_mut(&hash) {
                hints.remove(peer);
            }
            if !self.completed.contains(&hash) && !self.pending.contains(&hash) {
                self.pending.push_back(hash);
            }
        }
    }

    pub fn assignments(
        &mut self,
        max_blocks_in_flight: usize,
        max_requests_per_peer: usize,
    ) -> Vec<DownloadAssignment> {
        if self.ready_peers.is_empty() || self.pending.is_empty() || max_blocks_in_flight == 0 {
            return Vec::new();
        }

        let mut total_inflight = self
            .inflight_by_peer
            .values()
            .map(BTreeSet::len)
            .sum::<usize>();
        if total_inflight >= max_blocks_in_flight {
            return Vec::new();
        }

        let mut staged: BTreeMap<String, Vec<InventoryVector>> = BTreeMap::new();
        let peers = self.ready_peers.iter().cloned().collect::<Vec<_>>();
        let mut cursor = 0usize;

        while total_inflight < max_blocks_in_flight {
            let Some(hash) = self.pending.pop_front() else {
                break;
            };
            // Prefer hinted peers when possible, but never let one peer monopolize the queue.
            // The scheduler balances by current in-flight load first, then uses hash hints and
            // round-robin rotation to keep propagation and catch-up work spread out.
            let Some(peer) =
                self.select_peer_for_hash(&peers, cursor, &hash, max_requests_per_peer)
            else {
                self.pending.push_front(hash);
                break;
            };
            cursor = peers
                .iter()
                .position(|candidate| candidate == &peer)
                .unwrap_or(cursor)
                .saturating_add(1);
            self.inflight_by_peer
                .entry(peer.clone())
                .or_default()
                .insert(hash);
            self.inflight_owner.insert(hash, peer.clone());
            staged.entry(peer).or_default().push(InventoryVector {
                kind: InventoryKind::Block,
                hash,
            });
            total_inflight = total_inflight.saturating_add(1);
        }

        staged
            .into_iter()
            .map(|(peer, inventory)| DownloadAssignment { peer, inventory })
            .collect()
    }

    fn select_peer_for_hash(
        &self,
        peers: &[String],
        start_index: usize,
        hash: &Hash48,
        max_requests_per_peer: usize,
    ) -> Option<String> {
        if peers.is_empty() {
            return None;
        }

        let hinted = self.peer_hints.get(hash);
        peers
            .iter()
            .enumerate()
            .filter_map(|(index, peer)| {
                let inflight = self
                    .inflight_by_peer
                    .get(peer)
                    .map(BTreeSet::len)
                    .unwrap_or(0);
                (inflight < max_requests_per_peer).then_some((
                    inflight,
                    hinted.is_none_or(|hints| hints.contains(peer)),
                    (index + peers.len() - (start_index % peers.len())) % peers.len(),
                    peer,
                ))
            })
            .min_by_key(|(inflight, hinted_peer, rotation, _)| {
                (*inflight, !*hinted_peer, *rotation)
            })
            .map(|(_, _, _, peer)| peer.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_spreads_requests_across_ready_peers() {
        let mut scheduler = BlockDownloadScheduler::default();
        scheduler.note_peer_ready("left");
        scheduler.note_peer_ready("right");
        scheduler.note_headers("left", [[1; 48], [2; 48], [3; 48], [4; 48]]);
        scheduler.note_headers("right", [[1; 48], [2; 48], [3; 48], [4; 48]]);

        let assignments = scheduler.assignments(4, 2);

        assert_eq!(assignments.len(), 2);
        assert_eq!(assignments[0].inventory.len(), 2);
        assert_eq!(assignments[1].inventory.len(), 2);
    }

    #[test]
    fn not_found_requeues_hash_on_other_peers() {
        let mut scheduler = BlockDownloadScheduler::default();
        scheduler.note_peer_ready("left");
        scheduler.note_peer_ready("right");
        scheduler.note_headers("left", [[7; 48]]);
        scheduler.note_headers("right", [[7; 48]]);

        let first = scheduler.assignments(1, 1);
        assert_eq!(first.len(), 1);
        scheduler.note_not_found(&first[0].peer, &[[7; 48]]);
        let retry = scheduler.assignments(1, 1);
        assert_eq!(retry.len(), 1);
        assert_ne!(retry[0].peer, first[0].peer);
    }

    #[test]
    fn hinted_peer_does_not_monopolize_when_another_ready_peer_is_idle() {
        let mut scheduler = BlockDownloadScheduler::default();
        scheduler.note_peer_ready("left");
        scheduler.note_peer_ready("right");
        scheduler.note_headers("left", [[1; 48], [2; 48], [3; 48], [4; 48]]);

        let assignments = scheduler.assignments(4, 4);
        assert_eq!(assignments.len(), 2);
        assert_eq!(
            assignments
                .iter()
                .map(|item| item.inventory.len())
                .sum::<usize>(),
            4
        );
    }
}
