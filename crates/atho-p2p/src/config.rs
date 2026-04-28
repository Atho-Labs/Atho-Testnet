use atho_core::consensus::rules::PROTOCOL_VERSION;
use atho_core::network::Network;

pub const MIN_SUPPORTED_PROTOCOL_VERSION: u32 = 1;

pub const MAINNET_DNS_SEEDS: &[&str] = &[];
pub const TESTNET_DNS_SEEDS: &[&str] = &[];
pub const REGNET_DNS_SEEDS: &[&str] = &[];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P2pLimits {
    pub max_message_size: u32,
    pub max_addr_per_message: usize,
    pub max_inv_per_message: usize,
    pub max_headers_per_message: usize,
    pub max_blocks_in_flight: usize,
    pub max_requests_per_peer: usize,
    pub handshake_timeout_ms: u64,
    pub read_timeout_ms: u64,
    pub write_timeout_ms: u64,
    pub max_inbound_peers: usize,
    pub max_outbound_peers: usize,
    pub max_peers_per_ip: usize,
    pub max_peers_per_subnet: usize,
    pub max_known_peers: usize,
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
    max_blocks_in_flight: 128,
    max_requests_per_peer: 256,
    handshake_timeout_ms: 5_000,
    read_timeout_ms: 10_000,
    write_timeout_ms: 10_000,
    max_inbound_peers: 32,
    max_outbound_peers: 8,
    max_peers_per_ip: 8,
    max_peers_per_subnet: 16,
    max_known_peers: 4_096,
    ban_score_threshold: 100,
    peer_decay_interval_secs: 60,
};

pub fn network_params(network: Network) -> NetworkParams {
    match network {
        Network::Mainnet => NetworkParams {
            network,
            magic: [0xa7, 0x54, 0x48, 0x01],
            default_port: network.p2p_port(),
            protocol_version: PROTOCOL_VERSION,
            min_supported_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
            dns_seeds: MAINNET_DNS_SEEDS,
            limits: DEFAULT_LIMITS,
        },
        Network::Testnet => NetworkParams {
            network,
            magic: [0xa7, 0x54, 0x48, 0x02],
            default_port: network.p2p_port(),
            protocol_version: PROTOCOL_VERSION,
            min_supported_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
            dns_seeds: TESTNET_DNS_SEEDS,
            limits: DEFAULT_LIMITS,
        },
        Network::Regnet => NetworkParams {
            network,
            magic: [0xa7, 0x54, 0x48, 0x03],
            default_port: network.p2p_port(),
            protocol_version: PROTOCOL_VERSION,
            min_supported_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
            dns_seeds: REGNET_DNS_SEEDS,
            limits: DEFAULT_LIMITS,
        },
    }
}

pub fn network_from_magic(magic: [u8; 4]) -> Option<Network> {
    [Network::Mainnet, Network::Testnet, Network::Regnet]
        .into_iter()
        .find(|network| network_params(*network).magic == magic)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_params_have_unique_magic_and_blank_dns_seeds() {
        let main = network_params(Network::Mainnet);
        let test = network_params(Network::Testnet);
        let reg = network_params(Network::Regnet);
        assert_ne!(main.magic, test.magic);
        assert_ne!(main.magic, reg.magic);
        assert_ne!(test.magic, reg.magic);
        assert!(main.dns_seeds.is_empty());
        assert!(test.dns_seeds.is_empty());
        assert!(reg.dns_seeds.is_empty());
        assert_eq!(main.default_port, Network::Mainnet.p2p_port());
        assert_eq!(main.protocol_version, PROTOCOL_VERSION);
    }
}
