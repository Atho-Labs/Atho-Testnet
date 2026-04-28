use crate::config::network_params;
use atho_core::network::Network;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BanState {
    pub score: u32,
    pub banned_until_unix: Option<u64>,
    pub last_updated_unix: u64,
}

#[derive(Debug, Clone)]
pub struct BanList {
    network: Network,
    entries: BTreeMap<String, BanState>,
}

impl BanList {
    pub fn new(network: Network) -> Self {
        Self {
            network,
            entries: BTreeMap::new(),
        }
    }

    pub fn is_banned(&self, peer: &str, now_unix: u64) -> bool {
        self.entries
            .get(peer)
            .and_then(|entry| entry.banned_until_unix)
            .is_some_and(|banned_until| banned_until > now_unix)
    }

    pub fn score(&self, peer: &str) -> u32 {
        self.entries.get(peer).map(|entry| entry.score).unwrap_or(0)
    }

    pub fn record(&mut self, peer: impl Into<String>, points: u32) -> bool {
        let now = unix_timestamp();
        self.record_at(peer, points, now)
    }

    pub fn record_at(&mut self, peer: impl Into<String>, points: u32, now_unix: u64) -> bool {
        let threshold = network_params(self.network).limits.ban_score_threshold;
        let decay_interval = network_params(self.network)
            .limits
            .peer_decay_interval_secs
            .max(1);
        let peer = peer.into();
        let entry = self.entries.entry(peer).or_insert(BanState {
            score: 0,
            banned_until_unix: None,
            last_updated_unix: now_unix,
        });
        let elapsed_intervals = now_unix
            .saturating_sub(entry.last_updated_unix)
            .saturating_div(decay_interval);
        if elapsed_intervals > 0 {
            entry.score = entry.score.saturating_sub(elapsed_intervals as u32);
        }
        entry.score = entry.score.saturating_add(points);
        entry.last_updated_unix = now_unix;
        if entry.score >= threshold {
            entry.banned_until_unix = Some(now_unix.saturating_add(decay_interval * 10));
            return true;
        }
        false
    }

    pub fn decay(&mut self, now_unix: u64) {
        let decay_interval = network_params(self.network)
            .limits
            .peer_decay_interval_secs
            .max(1);
        self.entries.retain(|_, entry| {
            let elapsed_intervals = now_unix
                .saturating_sub(entry.last_updated_unix)
                .saturating_div(decay_interval);
            if elapsed_intervals > 0 {
                entry.score = entry.score.saturating_sub(elapsed_intervals as u32);
                entry.last_updated_unix = now_unix;
            }
            match entry.banned_until_unix {
                Some(banned_until) if banned_until <= now_unix && entry.score == 0 => false,
                Some(banned_until) if banned_until <= now_unix => {
                    entry.banned_until_unix = None;
                    true
                }
                _ => true,
            }
        });
    }
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ban_threshold_is_enforced() {
        let mut banlist = BanList::new(Network::Mainnet);
        assert!(!banlist.record_at("1.1.1.1:56000", 40, 100));
        assert!(!banlist.is_banned("1.1.1.1:56000", 100));
        assert!(banlist.record_at("1.1.1.1:56000", 60, 100));
        assert!(banlist.is_banned("1.1.1.1:56000", 100));
    }

    #[test]
    fn scores_decay_over_time() {
        let mut banlist = BanList::new(Network::Mainnet);
        banlist.record_at("1.1.1.1:56000", 10, 100);
        banlist.decay(170);
        assert!(banlist.score("1.1.1.1:56000") < 10);
    }
}
