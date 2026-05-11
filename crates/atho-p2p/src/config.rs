//! Network-specific P2P configuration defaults.
use atho_core::consensus::rules::PROTOCOL_VERSION;
use atho_core::network::Network;

pub const MIN_SUPPORTED_PROTOCOL_VERSION: u32 = 1;

pub const MAINNET_DNS_SEEDS: &[&str] = &[];
pub const TESTNET_DNS_SEEDS: &[&str] = &["testnet-node1.atho.io", "testnet-node2.atho.io"];
pub const REGNET_DNS_SEEDS: &[&str] = &[];
pub const PRUNETEST_DNS_SEEDS: &[&str] = &[];

pub const MAINNET_BOOTSTRAP_PEERS: &[&str] = &[];
pub const TESTNET_BOOTSTRAP_PEERS: &[&str] = &["162.222.206.163:9100", "74.208.219.116:9100"];
pub const REGNET_BOOTSTRAP_PEERS: &[&str] = &[];
pub const PRUNETEST_BOOTSTRAP_PEERS: &[&str] = &[];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P2pLimits {
    pub max_message_size: u32,
    pub max_addr_per_message: usize,
    pub max_inv_per_message: usize,
    pub max_headers_per_message: usize,
    pub max_blocks_in_flight: usize,
    pub max_requests_per_peer: usize,
    pub block_request_timeout_ms: u64,
    pub headers_request_timeout_ms: u64,
    pub block_download_lookahead: u64,
    pub max_fast_download_ahead: u64,
    pub block_request_batch_limit: usize,
    pub max_untrusted_block_cache: usize,
    pub max_untrusted_block_cache_bytes: usize,
    pub max_pending_validation_blocks: usize,
    pub enable_fast_body_download: bool,
    pub enable_background_validation: bool,
    pub enable_checkpoint_anchored_sync: bool,
    pub require_full_validation_before_mining: bool,
    pub sync_maintenance_interval_ms: u64,
    pub handshake_timeout_ms: u64,
    pub read_timeout_ms: u64,
    pub write_timeout_ms: u64,
    pub max_inbound_peers: usize,
    pub max_outbound_peers: usize,
    pub target_block_relay_peers: usize,
    pub target_sync_peers: usize,
    pub target_tx_relay_peers: usize,
    pub target_addr_relay_peers: usize,
    pub max_peers_per_ip: usize,
    pub max_peers_per_subnet: usize,
    pub max_known_peers: usize,
    pub max_addr_per_source: usize,
    pub max_addr_messages_per_window: u32,
    pub addr_rate_limit_window_secs: u64,
    pub getaddr_response_cooldown_secs: u64,
    pub max_peer_address_age_secs: u64,
    pub max_user_agent_bytes: usize,
    pub ban_score_threshold: u32,
    pub peer_decay_interval_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkParams {
    pub network: Network,
    pub magic: [u8; 4],
    pub default_port: u16,
    pub protocol_version: u32,
    pub min_supported_protocol_version: u32,
    pub dns_seeds: &'static [&'static str],
    pub limits: P2pLimits,
}

const DEFAULT_LIMITS: P2pLimits = P2pLimits {
    max_message_size: 8 * 1024 * 1024,
    max_addr_per_message: 1_000,
    max_inv_per_message: 50_000,
    max_headers_per_message: 2_000,
    max_blocks_in_flight: 1_024,
    max_requests_per_peer: 256,
    block_request_timeout_ms: 8_000,
    headers_request_timeout_ms: 20_000,
    block_download_lookahead: 160,
    max_fast_download_ahead: 4_096,
    block_request_batch_limit: 128,
    max_untrusted_block_cache: 4_096,
    max_untrusted_block_cache_bytes: 256 * 1024 * 1024,
    max_pending_validation_blocks: 4_096,
    enable_fast_body_download: true,
    enable_background_validation: false,
    enable_checkpoint_anchored_sync: true,
    require_full_validation_before_mining: true,
    sync_maintenance_interval_ms: 50,
    handshake_timeout_ms: 5_000,
    read_timeout_ms: 10_000,
    write_timeout_ms: 10_000,
    max_inbound_peers: 32,
    max_outbound_peers: 8,
    target_block_relay_peers: 4,
    target_sync_peers: 3,
    target_tx_relay_peers: 4,
    target_addr_relay_peers: 2,
    max_peers_per_ip: 8,
    max_peers_per_subnet: 16,
    max_known_peers: 4_096,
    max_addr_per_source: 128,
    max_addr_messages_per_window: 4,
    addr_rate_limit_window_secs: 60,
    getaddr_response_cooldown_secs: 60,
    max_peer_address_age_secs: 30 * 24 * 60 * 60,
    max_user_agent_bytes: 256,
    ban_score_threshold: 100,
    peer_decay_interval_secs: 60,
};

pub fn network_params(network: Network) -> NetworkParams {
    match network {
        Network::Mainnet => NetworkParams {
            network,
            magic: network.p2p_magic(),
            default_port: network.p2p_port(),
            protocol_version: PROTOCOL_VERSION,
            min_supported_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
            dns_seeds: MAINNET_DNS_SEEDS,
            limits: DEFAULT_LIMITS,
        },
        Network::Testnet => NetworkParams {
            network,
            magic: network.p2p_magic(),
            default_port: network.p2p_port(),
            protocol_version: PROTOCOL_VERSION,
            min_supported_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
            dns_seeds: TESTNET_DNS_SEEDS,
            limits: DEFAULT_LIMITS,
        },
        Network::Regnet => NetworkParams {
            network,
            magic: network.p2p_magic(),
            default_port: network.p2p_port(),
            protocol_version: PROTOCOL_VERSION,
            min_supported_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
            dns_seeds: REGNET_DNS_SEEDS,
            limits: DEFAULT_LIMITS,
        },
        Network::Prunetest => NetworkParams {
            network,
            magic: network.p2p_magic(),
            default_port: network.p2p_port(),
            protocol_version: PROTOCOL_VERSION,
            min_supported_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
            dns_seeds: PRUNETEST_DNS_SEEDS,
            limits: DEFAULT_LIMITS,
        },
    }
}

pub fn network_from_magic(magic: [u8; 4]) -> Option<Network> {
    Network::from_p2p_magic(magic)
}

/// Returns explicit peer addresses supplied through `ATHO_P2P_PEERS`.
///
/// This stays separate from network defaults so callers can treat an explicit
/// operator peer list as authoritative and only fall back to built-in bootstrap
/// peers when nothing was configured.
pub fn configured_peer_addresses_from_env() -> Vec<String> {
    std::env::var("ATHO_P2P_PEERS")
        .ok()
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Returns the built-in bootstrap peer list for a network.
///
/// Static bootstrap peers remain as a last-resort fallback if DNS seed
/// resolution is unavailable or an operator wants direct peer overrides.
pub fn default_bootstrap_peers(network: Network) -> &'static [&'static str] {
    match network {
        Network::Mainnet => MAINNET_BOOTSTRAP_PEERS,
        Network::Testnet => TESTNET_BOOTSTRAP_PEERS,
        Network::Regnet => REGNET_BOOTSTRAP_PEERS,
        Network::Prunetest => PRUNETEST_BOOTSTRAP_PEERS,
    }
}

/// Returns DNS seed targets formatted as `host:port` for outbound bootstrap.
pub fn dns_seed_targets(network: Network) -> Vec<String> {
    network_params(network)
        .dns_seeds
        .iter()
        .map(|host| format!("{host}:{}", network.p2p_port()))
        .collect()
}

/// Returns the operator-configured peer list or the network fallback peers.
///
/// Explicit peers supplied by `ATHO_P2P_PEERS` always win. When no override
/// exists, Atho tries DNS seeds first and then falls back to static peer
/// addresses.
pub fn configured_bootstrap_peers(network: Network) -> Vec<String> {
    let explicit = configured_peer_addresses_from_env();
    if !explicit.is_empty() {
        return explicit;
    }

    let mut peers = dns_seed_targets(network);
    peers.extend(
        default_bootstrap_peers(network)
            .iter()
            .map(|peer| (*peer).to_owned()),
    );
    peers
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .expect("env lock")
    }

    #[test]
    fn network_params_have_unique_magic_and_expected_dns_seeds() {
        let main = network_params(Network::Mainnet);
        let test = network_params(Network::Testnet);
        let reg = network_params(Network::Regnet);
        let prune = network_params(Network::Prunetest);
        assert_ne!(main.magic, test.magic);
        assert_ne!(main.magic, reg.magic);
        assert_ne!(main.magic, prune.magic);
        assert_ne!(test.magic, reg.magic);
        assert_ne!(test.magic, prune.magic);
        assert_ne!(reg.magic, prune.magic);
        assert_eq!(main.dns_seeds, MAINNET_DNS_SEEDS);
        assert_eq!(test.dns_seeds, TESTNET_DNS_SEEDS);
        assert!(reg.dns_seeds.is_empty());
        assert!(prune.dns_seeds.is_empty());
        assert_eq!(main.default_port, Network::Mainnet.p2p_port());
        assert_eq!(main.protocol_version, PROTOCOL_VERSION);
        assert_eq!(network_from_magic(prune.magic), Some(Network::Prunetest));
    }

    #[test]
    fn configured_bootstrap_peers_default_to_dns_seed_then_fallback_when_no_env_is_set() {
        let _lock = env_lock();
        std::env::remove_var("ATHO_P2P_PEERS");

        let peers = configured_bootstrap_peers(Network::Mainnet);
        assert!(peers.is_empty());
        assert_eq!(
            configured_bootstrap_peers(Network::Testnet),
            vec![
                String::from("testnet-node1.atho.io:9100"),
                String::from("testnet-node2.atho.io:9100"),
                String::from(TESTNET_BOOTSTRAP_PEERS[0]),
                String::from(TESTNET_BOOTSTRAP_PEERS[1]),
            ]
        );
        assert!(configured_bootstrap_peers(Network::Regnet).is_empty());
    }

    #[test]
    fn configured_bootstrap_peers_prefer_explicit_env_peers() {
        let _lock = env_lock();
        std::env::set_var("ATHO_P2P_PEERS", "1.1.1.1:56000, 2.2.2.2:56000");

        let peers = configured_bootstrap_peers(Network::Mainnet);
        assert_eq!(
            peers,
            vec![String::from("1.1.1.1:56000"), String::from("2.2.2.2:56000")]
        );

        std::env::remove_var("ATHO_P2P_PEERS");
    }

    #[test]
    fn dns_seed_targets_use_network_default_ports() {
        assert!(dns_seed_targets(Network::Mainnet).is_empty());
        assert_eq!(
            dns_seed_targets(Network::Testnet),
            vec![
                String::from("testnet-node1.atho.io:9100"),
                String::from("testnet-node2.atho.io:9100"),
            ]
        );
        assert!(dns_seed_targets(Network::Regnet).is_empty());
    }
}
