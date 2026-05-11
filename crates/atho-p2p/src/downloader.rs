//! Block and header download coordination primitives.
use crate::protocol::{Hash48, InventoryKind, InventoryVector};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant};

const PEER_TIMEOUT_PENALTY: u32 = 12;
const PEER_NOT_FOUND_PENALTY: u32 = 6;
const PEER_DISCONNECT_PENALTY: u32 = 18;
const PEER_SUCCESS_DECAY: u32 = 4;
const PEER_PENALTY_CAP: u32 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadAssignment {
    pub peer: String,
    pub inventory: Vec<InventoryVector>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleDownload {
    pub peer: String,
    pub hash: Hash48,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DownloadStats {
    pub ready_peers: usize,
    pub pending_blocks: usize,
    pub inflight_blocks: usize,
    pub completed_blocks: usize,
    pub penalized_peers: usize,
}

#[derive(Debug, Default, Clone)]
pub struct BlockDownloadScheduler {
    ready_peers: BTreeSet<String>,
    pending: VecDeque<Hash48>,
    inflight_by_peer: BTreeMap<String, VecDeque<Hash48>>,
    inflight_owner: BTreeMap<Hash48, String>,
    inflight_started: BTreeMap<Hash48, Instant>,
    peer_hints: BTreeMap<Hash48, BTreeSet<String>>,
    failed_peers: BTreeMap<Hash48, BTreeSet<String>>,
    completed: BTreeSet<Hash48>,
    peer_penalties: BTreeMap<String, u32>,
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
            if !inflight.is_empty() {
                self.penalize_peer(peer, PEER_DISCONNECT_PENALTY);
            }
            for hash in inflight {
                self.inflight_owner.remove(&hash);
                self.inflight_started.remove(&hash);
                if !self.pending.contains(&hash) && !self.completed.contains(&hash) {
                    self.pending.push_back(hash);
                }
                if let Some(hints) = self.peer_hints.get_mut(&hash) {
                    hints.remove(peer);
                }
                self.remove_empty_hints(hash);
                self.failed_peers
                    .entry(hash)
                    .or_default()
                    .insert(peer.to_string());
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

    pub fn queue_priority_block(&mut self, peer: Option<&str>, hash: [u8; 48]) -> bool {
        let hash = Hash48::from(hash);
        if let Some(peer) = peer {
            self.peer_hints
                .entry(hash)
                .or_default()
                .insert(peer.to_string());
        }
        if self.completed.contains(&hash)
            || self.inflight_owner.contains_key(&hash)
            || self.pending.contains(&hash)
        {
            return false;
        }
        self.pending.push_front(hash);
        true
    }

    pub fn reassign_priority_block(&mut self, peer: Option<&str>, hash: [u8; 48]) -> bool {
        let hash = Hash48::from(hash);
        if self.completed.contains(&hash) {
            return false;
        }
        if let Some(owner) = self.inflight_owner.get(&hash).cloned() {
            if peer.is_some_and(|preferred| preferred == owner) {
                return false;
            }
            self.inflight_owner.remove(&hash);
            self.inflight_started.remove(&hash);
            if let Some(inflight) = self.inflight_by_peer.get_mut(&owner) {
                inflight.retain(|candidate| *candidate != hash);
            }
        }
        if let Some(index) = self.pending.iter().position(|candidate| *candidate == hash) {
            self.pending.remove(index);
        }
        if let Some(peer) = peer {
            self.peer_hints
                .entry(hash)
                .or_default()
                .insert(peer.to_string());
        }
        self.pending.push_front(hash);
        true
    }

    pub fn note_block_received(&mut self, hash: [u8; 48]) {
        let hash = Hash48::from(hash);
        self.note_hash_completed(hash);
    }

    pub fn note_peer_delivery_progress(&mut self, peer: &str) {
        let Some(inflight) = self.inflight_by_peer.get(peer) else {
            return;
        };
        let now = Instant::now();
        for hash in inflight {
            if let Some(started) = self.inflight_started.get_mut(hash) {
                *started = now;
            }
        }
    }

    pub fn prune_completed_where(
        &mut self,
        mut is_completed: impl FnMut(&Hash48) -> bool,
    ) -> usize {
        let candidates = self
            .pending
            .iter()
            .chain(self.inflight_owner.keys())
            .copied()
            .collect::<BTreeSet<_>>();
        let mut pruned = 0usize;
        for hash in candidates {
            if is_completed(&hash) {
                self.note_hash_completed(hash);
                pruned = pruned.saturating_add(1);
            }
        }
        pruned
    }

    pub fn sort_pending_by_key<K: Ord>(&mut self, mut key: impl FnMut(&Hash48) -> K) {
        let mut pending = self.pending.drain(..).collect::<Vec<_>>();
        pending.sort_by_key(|hash| key(hash));
        self.pending = pending.into();
    }

    fn note_hash_completed(&mut self, hash: Hash48) {
        self.completed.insert(hash);
        self.failed_peers.remove(&hash);
        self.pending.retain(|candidate| *candidate != hash);
        if let Some(owner) = self.inflight_owner.remove(&hash) {
            self.decay_peer_penalty(&owner, PEER_SUCCESS_DECAY);
            if let Some(inflight) = self.inflight_by_peer.get_mut(&owner) {
                inflight.retain(|candidate| *candidate != hash);
            }
        }
        self.inflight_started.remove(&hash);
    }

    pub fn is_inflight(&self, hash: [u8; 48]) -> bool {
        self.inflight_owner.contains_key(&Hash48::from(hash))
    }

    pub fn inflight_peer(&self, hash: [u8; 48]) -> Option<&str> {
        self.inflight_owner
            .get(&Hash48::from(hash))
            .map(String::as_str)
    }

    #[doc(hidden)]
    pub fn backdate_inflight_for_peer(&mut self, peer: &str, age: Duration) {
        let Some(inflight) = self.inflight_by_peer.get(peer) else {
            return;
        };
        let started_at = Instant::now().checked_sub(age).unwrap_or_else(Instant::now);
        for hash in inflight {
            if let Some(started) = self.inflight_started.get_mut(hash) {
                *started = started_at;
            }
        }
    }

    pub fn note_not_found(&mut self, peer: &str, hashes: &[[u8; 48]]) {
        let mut penalize = false;
        for hash in hashes.iter().copied().map(Hash48::from) {
            if let Some(owner) = self.inflight_owner.get(&hash) {
                if owner != peer {
                    continue;
                }
            }
            penalize = true;
            self.inflight_owner.remove(&hash);
            self.inflight_started.remove(&hash);
            if let Some(inflight) = self.inflight_by_peer.get_mut(peer) {
                inflight.retain(|candidate| *candidate != hash);
            }
            if let Some(hints) = self.peer_hints.get_mut(&hash) {
                hints.remove(peer);
            }
            self.remove_empty_hints(hash);
            self.failed_peers
                .entry(hash)
                .or_default()
                .insert(peer.to_string());
            if !self.completed.contains(&hash) && !self.pending.contains(&hash) {
                self.pending.push_front(hash);
            }
        }
        if penalize {
            self.penalize_peer(peer, PEER_NOT_FOUND_PENALTY);
        }
    }

    pub fn assignments(
        &mut self,
        max_blocks_in_flight: usize,
        max_requests_per_peer: usize,
    ) -> Vec<DownloadAssignment> {
        self.assignments_limited(
            max_blocks_in_flight,
            max_requests_per_peer,
            max_requests_per_peer,
        )
    }

    pub fn assignments_limited(
        &mut self,
        max_blocks_in_flight: usize,
        max_requests_per_peer: usize,
        max_requests_per_batch: usize,
    ) -> Vec<DownloadAssignment> {
        if self.ready_peers.is_empty() || self.pending.is_empty() || max_blocks_in_flight == 0 {
            return Vec::new();
        }

        let mut total_inflight = self
            .inflight_by_peer
            .values()
            .map(VecDeque::len)
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
            let Some(peer) = self.select_peer_for_hash_limited(
                &peers,
                cursor,
                &hash,
                max_requests_per_peer,
                max_requests_per_batch,
                &staged,
            ) else {
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
                .push_back(hash);
            self.inflight_owner.insert(hash, peer.clone());
            self.inflight_started.insert(hash, Instant::now());
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

    fn select_peer_for_hash_limited(
        &self,
        peers: &[String],
        start_index: usize,
        hash: &Hash48,
        max_requests_per_peer: usize,
        max_requests_per_batch: usize,
        staged: &BTreeMap<String, Vec<InventoryVector>>,
    ) -> Option<String> {
        if peers.is_empty() {
            return None;
        }

        let hinted = self.peer_hints.get(hash).filter(|hints| !hints.is_empty());
        let failed = self.failed_peers.get(hash);
        self.select_peer_from_candidates(
            peers,
            start_index,
            max_requests_per_peer,
            max_requests_per_batch,
            staged,
            |peer| {
                hinted.is_none_or(|hints| hints.contains(peer))
                    && failed.is_none_or(|failed| !failed.contains(peer))
            },
        )
        .or_else(|| {
            self.select_peer_from_candidates(
                peers,
                start_index,
                max_requests_per_peer,
                max_requests_per_batch,
                staged,
                |peer| hinted.is_none_or(|hints| hints.contains(peer)),
            )
        })
    }

    fn select_peer_from_candidates(
        &self,
        peers: &[String],
        start_index: usize,
        max_requests_per_peer: usize,
        max_requests_per_batch: usize,
        staged: &BTreeMap<String, Vec<InventoryVector>>,
        mut allowed: impl FnMut(&String) -> bool,
    ) -> Option<String> {
        peers
            .iter()
            .enumerate()
            .filter(|(_, peer)| allowed(peer))
            .filter_map(|(index, peer)| {
                if staged
                    .get(peer)
                    .is_some_and(|inventory| inventory.len() >= max_requests_per_batch)
                {
                    return None;
                }
                let inflight = self
                    .inflight_by_peer
                    .get(peer)
                    .map(VecDeque::len)
                    .unwrap_or(0);
                let penalty = self.peer_penalty(peer);
                (inflight < max_requests_per_peer).then_some((
                    penalty,
                    inflight,
                    (index + peers.len() - (start_index % peers.len())) % peers.len(),
                    peer,
                ))
            })
            .min_by_key(|(penalty, inflight, rotation, _)| (*penalty, *inflight, *rotation))
            .map(|(_, _, _, peer)| peer.clone())
    }

    pub fn assignment_for_peer(
        &mut self,
        peer: &str,
        max_blocks_in_flight: usize,
        max_requests_per_peer: usize,
    ) -> Option<DownloadAssignment> {
        self.assignment_for_peer_limited(
            peer,
            max_blocks_in_flight,
            max_requests_per_peer,
            max_requests_per_peer,
        )
    }

    pub fn assignment_for_peer_limited(
        &mut self,
        peer: &str,
        max_blocks_in_flight: usize,
        max_requests_per_peer: usize,
        max_requests_per_batch: usize,
    ) -> Option<DownloadAssignment> {
        self.assignment_for_peer_filtered_limited(
            peer,
            max_blocks_in_flight,
            max_requests_per_peer,
            max_requests_per_batch,
            |_| true,
        )
    }

    pub fn assignment_for_peer_matching_limited(
        &mut self,
        peer: &str,
        max_blocks_in_flight: usize,
        max_requests_per_peer: usize,
        max_requests_per_batch: usize,
        allowed_hashes: &BTreeSet<Hash48>,
    ) -> Option<DownloadAssignment> {
        self.assignment_for_peer_filtered_limited(
            peer,
            max_blocks_in_flight,
            max_requests_per_peer,
            max_requests_per_batch,
            |hash| allowed_hashes.contains(hash),
        )
    }

    fn assignment_for_peer_filtered_limited(
        &mut self,
        peer: &str,
        max_blocks_in_flight: usize,
        max_requests_per_peer: usize,
        max_requests_per_batch: usize,
        mut allowed_hash: impl FnMut(&Hash48) -> bool,
    ) -> Option<DownloadAssignment> {
        if !self.ready_peers.contains(peer)
            || self.pending.is_empty()
            || max_blocks_in_flight == 0
            || max_requests_per_peer == 0
            || max_requests_per_batch == 0
        {
            return None;
        }

        let mut total_inflight = self
            .inflight_by_peer
            .values()
            .map(VecDeque::len)
            .sum::<usize>();
        if total_inflight >= max_blocks_in_flight {
            return None;
        }
        let peer_inflight = self
            .inflight_by_peer
            .get(peer)
            .map(VecDeque::len)
            .unwrap_or(0);
        let peer_capacity = max_requests_per_peer
            .saturating_sub(peer_inflight)
            .min(max_requests_per_batch);
        if peer_capacity == 0 {
            return None;
        }

        let mut inventory = Vec::new();
        let mut skipped = Vec::new();
        let scan_limit = self.pending.len();
        for _ in 0..scan_limit {
            if total_inflight >= max_blocks_in_flight || inventory.len() >= peer_capacity {
                break;
            }
            let Some(hash) = self.pending.pop_front() else {
                break;
            };
            if !allowed_hash(&hash) {
                skipped.push(hash);
                continue;
            }
            let hinted_to_peer = self
                .peer_hints
                .get(&hash)
                .filter(|hints| !hints.is_empty())
                .is_none_or(|hints| hints.contains(peer));
            let failed_for_peer = self
                .failed_peers
                .get(&hash)
                .is_some_and(|failed| failed.contains(peer));
            let all_ready_peers_failed = self.failed_peers.get(&hash).is_some_and(|failed| {
                !self.ready_peers.is_empty()
                    && self.ready_peers.iter().all(|ready| failed.contains(ready))
            });
            if !hinted_to_peer || (failed_for_peer && !all_ready_peers_failed) {
                skipped.push(hash);
                continue;
            }

            self.inflight_by_peer
                .entry(peer.to_string())
                .or_default()
                .push_back(hash);
            self.inflight_owner.insert(hash, peer.to_string());
            self.inflight_started.insert(hash, Instant::now());
            inventory.push(InventoryVector {
                kind: InventoryKind::Block,
                hash,
            });
            total_inflight = total_inflight.saturating_add(1);
        }
        for hash in skipped.into_iter().rev() {
            self.pending.push_front(hash);
        }

        (!inventory.is_empty()).then_some(DownloadAssignment {
            peer: peer.to_string(),
            inventory,
        })
    }

    pub fn requeue_stale_inflight(&mut self, timeout: Duration) -> usize {
        self.requeue_stale_inflight_details(timeout).len()
    }

    pub fn requeue_stale_inflight_details(&mut self, timeout: Duration) -> Vec<StaleDownload> {
        let now = Instant::now();
        let stale = self
            .inflight_started
            .iter()
            .filter_map(|(hash, started)| {
                (now.duration_since(*started) >= timeout).then_some(*hash)
            })
            .collect::<Vec<_>>();
        let mut requeued = Vec::new();
        for hash in stale {
            self.inflight_started.remove(&hash);
            if let Some(owner) = self.inflight_owner.remove(&hash) {
                if let Some(inflight) = self.inflight_by_peer.get_mut(&owner) {
                    inflight.retain(|candidate| *candidate != hash);
                }
                if let Some(hints) = self.peer_hints.get_mut(&hash) {
                    hints.remove(&owner);
                }
                self.remove_empty_hints(hash);
                self.failed_peers
                    .entry(hash)
                    .or_default()
                    .insert(owner.clone());
                self.penalize_peer(&owner, PEER_TIMEOUT_PENALTY);
                requeued.push(StaleDownload { peer: owner, hash });
            }
            if !self.completed.contains(&hash) && !self.pending.contains(&hash) {
                self.pending.push_front(hash);
            }
        }
        requeued
    }

    pub fn requeue_stale_inflight_for_peer(
        &mut self,
        peer: &str,
        timeout: Duration,
    ) -> Vec<StaleDownload> {
        let now = Instant::now();
        let Some(inflight) = self.inflight_by_peer.get(peer) else {
            return Vec::new();
        };
        let stale = inflight
            .iter()
            .filter(|hash| {
                self.inflight_started
                    .get(hash)
                    .is_some_and(|started| now.duration_since(*started) >= timeout)
            })
            .copied()
            .collect::<Vec<_>>();
        let mut requeued = Vec::new();
        for hash in stale {
            self.inflight_started.remove(&hash);
            self.inflight_owner.remove(&hash);
            if let Some(inflight) = self.inflight_by_peer.get_mut(peer) {
                inflight.retain(|candidate| *candidate != hash);
            }
            if let Some(hints) = self.peer_hints.get_mut(&hash) {
                hints.remove(peer);
            }
            self.remove_empty_hints(hash);
            self.failed_peers
                .entry(hash)
                .or_default()
                .insert(peer.to_string());
            self.penalize_peer(peer, PEER_TIMEOUT_PENALTY);
            if !self.completed.contains(&hash) && !self.pending.contains(&hash) {
                self.pending.push_front(hash);
            }
            requeued.push(StaleDownload {
                peer: peer.to_string(),
                hash,
            });
        }
        requeued
    }

    pub fn is_idle(&self) -> bool {
        self.pending.is_empty() && self.inflight_owner.is_empty()
    }

    pub fn total_inflight_len(&self) -> usize {
        self.inflight_owner.len()
    }

    pub fn peer_inflight_len(&self, peer: &str) -> usize {
        self.inflight_by_peer
            .get(peer)
            .map(VecDeque::len)
            .unwrap_or(0)
    }

    pub fn stats(&self) -> DownloadStats {
        DownloadStats {
            ready_peers: self.ready_peers.len(),
            pending_blocks: self.pending.len(),
            inflight_blocks: self.inflight_owner.len(),
            completed_blocks: self.completed.len(),
            penalized_peers: self
                .peer_penalties
                .values()
                .filter(|penalty| **penalty > 0)
                .count(),
        }
    }

    pub fn peer_penalty(&self, peer: &str) -> u32 {
        self.peer_penalties.get(peer).copied().unwrap_or(0)
    }

    fn penalize_peer(&mut self, peer: &str, points: u32) {
        let entry = self.peer_penalties.entry(peer.to_string()).or_default();
        *entry = entry.saturating_add(points).min(PEER_PENALTY_CAP);
    }

    fn decay_peer_penalty(&mut self, peer: &str, points: u32) {
        let Some(entry) = self.peer_penalties.get_mut(peer) else {
            return;
        };
        *entry = entry.saturating_sub(points);
        if *entry == 0 {
            self.peer_penalties.remove(peer);
        }
    }

    fn remove_empty_hints(&mut self, hash: Hash48) {
        if self.peer_hints.get(&hash).is_some_and(BTreeSet::is_empty) {
            self.peer_hints.remove(&hash);
        }
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
    fn hinted_hashes_stay_on_peers_that_advertised_them() {
        let mut scheduler = BlockDownloadScheduler::default();
        scheduler.note_peer_ready("left");
        scheduler.note_peer_ready("right");
        scheduler.note_headers("left", [[1; 48], [2; 48], [3; 48], [4; 48]]);

        let assignments = scheduler.assignments(4, 4);
        assert_eq!(assignments.len(), 1);
        assert_eq!(assignments[0].peer, "left");
        assert_eq!(
            assignments
                .iter()
                .map(|item| item.inventory.len())
                .sum::<usize>(),
            4
        );
    }

    #[test]
    fn stale_inflight_requests_are_requeued_for_retry() {
        let mut scheduler = BlockDownloadScheduler::default();
        scheduler.note_peer_ready("left");
        scheduler.note_headers("left", [[7; 48]]);

        let first = scheduler.assignments(1, 1);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].inventory[0].hash, Hash48::from([7; 48]));
        assert!(scheduler.is_inflight([7; 48]));
        assert!(scheduler.assignments(1, 1).is_empty());

        assert_eq!(scheduler.requeue_stale_inflight(Duration::ZERO), 1);
        assert!(!scheduler.is_inflight([7; 48]));
        let retry = scheduler
            .assignment_for_peer("left", 1, 1)
            .expect("retry assignment");
        assert_eq!(retry.peer, "left");
        assert_eq!(retry.inventory[0].hash, Hash48::from([7; 48]));
    }

    #[test]
    fn peer_delivery_progress_extends_remaining_batch_deadlines() {
        let mut scheduler = BlockDownloadScheduler::default();
        scheduler.note_peer_ready("left");
        scheduler.note_headers("left", [[1; 48], [2; 48]]);

        let first = scheduler.assignments(2, 2);
        assert_eq!(first.len(), 1);
        assert_eq!(scheduler.stats().inflight_blocks, 2);

        scheduler.backdate_inflight_for_peer("left", Duration::from_secs(30));
        scheduler.note_peer_delivery_progress("left");

        assert!(
            scheduler
                .requeue_stale_inflight_for_peer("left", Duration::from_secs(10))
                .is_empty(),
            "an actively serving peer should not have the tail of the same batch timed out"
        );
        assert_eq!(scheduler.stats().inflight_blocks, 2);
    }

    #[test]
    fn completed_prune_removes_pending_and_inflight_work() {
        let mut scheduler = BlockDownloadScheduler::default();
        scheduler.note_peer_ready("left");
        scheduler.note_headers("left", [[1; 48], [2; 48], [3; 48]]);

        let first = scheduler.assignments(2, 2);
        assert_eq!(first[0].inventory.len(), 2);
        assert_eq!(scheduler.stats().pending_blocks, 1);
        assert_eq!(scheduler.stats().inflight_blocks, 2);

        let pruned = scheduler.prune_completed_where(|hash| {
            *hash == Hash48::from([1; 48]) || *hash == Hash48::from([3; 48])
        });

        assert_eq!(pruned, 2);
        assert_eq!(scheduler.stats().pending_blocks, 0);
        assert_eq!(scheduler.stats().inflight_blocks, 1);
        assert!(scheduler.is_inflight([2; 48]));
        assert!(scheduler.assignments(3, 3).is_empty());
    }

    #[test]
    fn pending_downloads_can_be_reprioritized_by_chain_height() {
        let mut scheduler = BlockDownloadScheduler::default();
        scheduler.note_peer_ready("peer");
        scheduler.note_headers("peer", [[9; 48], [1; 48], [5; 48]]);

        scheduler.sort_pending_by_key(|hash| hash.into_inner()[0]);

        let assignment = scheduler
            .assignment_for_peer("peer", 3, 3)
            .expect("assignment");
        let requested = assignment
            .inventory
            .iter()
            .map(|item| item.hash.into_inner()[0])
            .collect::<Vec<_>>();
        assert_eq!(requested, vec![1, 5, 9]);
    }

    #[test]
    fn disconnect_requeues_inflight_blocks_in_original_chain_order() {
        let mut scheduler = BlockDownloadScheduler::default();
        scheduler.note_peer_ready("left");
        scheduler.note_headers("left", [[1; 48], [2; 48], [3; 48]]);

        let assignments = scheduler.assignments(3, 3);
        assert_eq!(assignments.len(), 1);
        assert_eq!(
            assignments[0]
                .inventory
                .iter()
                .map(|item| item.hash.into_inner())
                .collect::<Vec<_>>(),
            vec![[1; 48], [2; 48], [3; 48]]
        );

        scheduler.note_peer_disconnected("left");
        scheduler.note_peer_ready("right");
        scheduler.note_headers("right", [[1; 48], [2; 48], [3; 48]]);

        let retry = scheduler.assignments(3, 3);
        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].peer, "right");
        assert_eq!(
            retry[0]
                .inventory
                .iter()
                .map(|item| item.hash.into_inner())
                .collect::<Vec<_>>(),
            vec![[1; 48], [2; 48], [3; 48]]
        );
    }

    #[test]
    fn timed_out_only_hint_does_not_strand_pending_block() {
        let mut scheduler = BlockDownloadScheduler::default();
        scheduler.note_peer_ready("left");
        scheduler.note_peer_ready("right");
        scheduler.note_headers("left", [[7; 48]]);

        let first = scheduler
            .assignment_for_peer("left", 1, 1)
            .expect("left assignment");
        assert_eq!(first.inventory[0].hash, Hash48::from([7; 48]));

        let stale = scheduler.requeue_stale_inflight_for_peer("left", Duration::ZERO);
        assert_eq!(stale.len(), 1);
        scheduler.note_peer_disconnected("left");

        let retry = scheduler.assignments(1, 1);
        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].peer, "right");
        assert_eq!(retry[0].inventory[0].hash, Hash48::from([7; 48]));
    }

    #[test]
    fn priority_block_request_can_bootstrap_orphan_parent_download() {
        let mut scheduler = BlockDownloadScheduler::default();
        scheduler.note_peer_ready("left");
        scheduler.note_peer_ready("right");

        scheduler.queue_priority_block(Some("left"), [9; 48]);
        let first = scheduler.assignments(1, 1);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].peer, "left");

        scheduler.note_not_found("left", &[[9; 48]]);
        let retry = scheduler.assignments(1, 1);
        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].peer, "right");
        assert_eq!(retry[0].inventory[0].hash, Hash48::from([9; 48]));
    }

    #[test]
    fn priority_block_can_be_reassigned_from_backlogged_peer() {
        let mut scheduler = BlockDownloadScheduler::default();
        scheduler.note_peer_ready("slow");
        scheduler.note_peer_ready("fast");
        scheduler.note_headers("slow", [[9; 48]]);
        scheduler.note_headers("fast", [[9; 48]]);

        let slow = scheduler
            .assignment_for_peer("slow", 1, 1)
            .expect("slow first assignment");
        assert_eq!(slow.peer, "slow");
        assert_eq!(slow.inventory[0].hash, Hash48::from([9; 48]));
        assert_eq!(scheduler.peer_inflight_len("slow"), 1);

        assert!(scheduler.reassign_priority_block(Some("fast"), [9; 48]));
        assert_eq!(scheduler.peer_inflight_len("slow"), 0);
        let fast = scheduler
            .assignment_for_peer("fast", 1, 1)
            .expect("fast reassignment");
        assert_eq!(fast.peer, "fast");
        assert_eq!(fast.inventory[0].hash, Hash48::from([9; 48]));
    }

    #[test]
    fn filtered_peer_assignment_leaves_unrelated_future_blocks_pending() {
        let mut scheduler = BlockDownloadScheduler::default();
        scheduler.note_peer_ready("peer");
        scheduler.note_headers("peer", [[1; 48], [2; 48], [3; 48]]);
        let allowed = BTreeSet::from([Hash48::from([2; 48])]);

        let assignment = scheduler
            .assignment_for_peer_matching_limited("peer", 3, 3, 3, &allowed)
            .expect("allowed parent assignment");

        assert_eq!(assignment.inventory.len(), 1);
        assert_eq!(assignment.inventory[0].hash, Hash48::from([2; 48]));
        let future = scheduler
            .assignment_for_peer("peer", 3, 3)
            .expect("unrelated pending blocks remain available later");
        assert_eq!(
            future
                .inventory
                .iter()
                .map(|item| item.hash)
                .collect::<Vec<_>>(),
            vec![Hash48::from([1; 48]), Hash48::from([3; 48])]
        );
    }

    #[test]
    fn scheduler_prefers_lower_penalty_peer_after_timeout() {
        let mut scheduler = BlockDownloadScheduler::default();
        scheduler.note_peer_ready("slow");
        scheduler.note_peer_ready("fast");
        scheduler.note_headers("slow", [[1; 48], [2; 48]]);
        scheduler.note_headers("fast", [[1; 48], [2; 48]]);

        let first = scheduler
            .assignment_for_peer("slow", 1, 1)
            .expect("slow first assignment");
        assert_eq!(first.peer, "slow");
        scheduler.requeue_stale_inflight_for_peer("slow", Duration::ZERO);
        assert!(scheduler.peer_penalty("slow") > scheduler.peer_penalty("fast"));

        let retry = scheduler.assignments(1, 1);

        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].peer, "fast");
        assert_eq!(retry[0].inventory[0].hash, Hash48::from([1; 48]));
    }

    #[test]
    fn successful_response_decays_peer_penalty_and_updates_stats() {
        let mut scheduler = BlockDownloadScheduler::default();
        scheduler.note_peer_ready("left");
        scheduler.note_headers("left", [[3; 48]]);

        let first = scheduler.assignments(1, 1);
        assert_eq!(first.len(), 1);
        scheduler.requeue_stale_inflight_for_peer("left", Duration::ZERO);
        let penalized = scheduler.peer_penalty("left");
        assert!(penalized > 0);

        let retry = scheduler
            .assignment_for_peer("left", 1, 1)
            .expect("retry assignment");
        assert_eq!(retry.inventory[0].hash, Hash48::from([3; 48]));
        scheduler.note_block_received([3; 48]);

        assert!(scheduler.peer_penalty("left") < penalized);
        assert_eq!(scheduler.stats().pending_blocks, 0);
        assert_eq!(scheduler.stats().inflight_blocks, 0);
        assert_eq!(scheduler.stats().completed_blocks, 1);
    }
}
