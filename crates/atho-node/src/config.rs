//! Node configuration defaults and runtime selection helpers.
use atho_core::network::Network;

const DEFAULT_API_BIND: &str = "127.0.0.1";
const DEFAULT_API_PORT: u16 = 8080;
const DEFAULT_API_MAX_RESPONSE_BYTES: usize = 1_048_576;
const DEFAULT_ALLOWED_ORIGINS: &[&str] = &["https://atho.io", "https://www.atho.io"];
const DEFAULT_RATE_LIMIT_RPM: u32 = 60;
const DEFAULT_HEAVY_RATE_LIMIT_RPM: u32 = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeConfig {
    pub network: Network,
    pub api: ApiConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiConfig {
    pub enabled: bool,
    pub bind: String,
    pub port: u16,
    pub public_read_only: bool,
    pub admin_enabled: bool,
    pub wallet_enabled: bool,
    pub mining_enabled: bool,
    pub max_response_bytes: usize,
    pub cors: CorsConfig,
    pub rate_limit: RateLimitConfig,
    pub explorer: ExplorerConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorsConfig {
    pub allowed_origins: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitConfig {
    pub enabled: bool,
    pub requests_per_minute: u32,
    pub heavy_requests_per_minute: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplorerConfig {
    pub index_enabled: bool,
    pub snapshot_enabled: bool,
    pub network: Network,
}

impl NodeConfig {
    pub fn new(network: Network) -> Self {
        Self {
            network,
            api: ApiConfig::for_network(network),
        }
    }

    pub fn from_env(network: Network) -> Self {
        let mut config = Self::new(network);
        config.api.enabled = env_bool("ATHO_API_ENABLED", config.api.enabled);
        config.api.bind = std::env::var("ATHO_API_BIND").unwrap_or(config.api.bind);
        config.api.port = env_u16("ATHO_API_PORT", config.api.port);
        config.api.public_read_only =
            env_bool("ATHO_API_PUBLIC_READ_ONLY", config.api.public_read_only);
        config.api.admin_enabled = env_bool("ATHO_API_ADMIN_ENABLED", config.api.admin_enabled);
        config.api.wallet_enabled = env_bool("ATHO_API_WALLET_ENABLED", config.api.wallet_enabled);
        config.api.mining_enabled = env_bool("ATHO_API_MINING_ENABLED", config.api.mining_enabled);
        config.api.max_response_bytes =
            env_usize("ATHO_API_MAX_RESPONSE_BYTES", config.api.max_response_bytes);
        config.api.cors.allowed_origins =
            env_csv("ATHO_API_ALLOWED_ORIGINS", &config.api.cors.allowed_origins);
        config.api.rate_limit.enabled =
            env_bool("ATHO_API_RATE_LIMIT_ENABLED", config.api.rate_limit.enabled);
        config.api.rate_limit.requests_per_minute = env_u32(
            "ATHO_API_RATE_LIMIT_RPM",
            config.api.rate_limit.requests_per_minute,
        );
        config.api.rate_limit.heavy_requests_per_minute = env_u32(
            "ATHO_API_HEAVY_RATE_LIMIT_RPM",
            config.api.rate_limit.heavy_requests_per_minute,
        );
        config.api.explorer.index_enabled = env_bool(
            "ATHO_EXPLORER_INDEX_ENABLED",
            config.api.explorer.index_enabled,
        );
        config.api.explorer.snapshot_enabled = env_bool(
            "ATHO_EXPLORER_SNAPSHOT_ENABLED",
            config.api.explorer.snapshot_enabled,
        );
        config.api.explorer.network = network;
        config
    }
}

impl ApiConfig {
    pub fn for_network(network: Network) -> Self {
        Self {
            enabled: true,
            bind: DEFAULT_API_BIND.to_string(),
            port: DEFAULT_API_PORT,
            public_read_only: true,
            admin_enabled: false,
            wallet_enabled: false,
            mining_enabled: false,
            max_response_bytes: DEFAULT_API_MAX_RESPONSE_BYTES,
            cors: CorsConfig {
                allowed_origins: DEFAULT_ALLOWED_ORIGINS
                    .iter()
                    .map(|origin| (*origin).to_string())
                    .collect(),
            },
            rate_limit: RateLimitConfig {
                enabled: true,
                requests_per_minute: DEFAULT_RATE_LIMIT_RPM,
                heavy_requests_per_minute: DEFAULT_HEAVY_RATE_LIMIT_RPM,
            },
            explorer: ExplorerConfig {
                index_enabled: true,
                snapshot_enabled: true,
                network,
            },
        }
    }

    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.bind, self.port)
    }
}

fn env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => default,
    }
}

fn env_u16(key: &str, default: u16) -> u16 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<u16>().ok())
        .unwrap_or(default)
}

fn env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(default)
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(default)
}

fn env_csv(key: &str, default: &[String]) -> Vec<String> {
    match std::env::var(key) {
        Ok(value) => {
            let values = value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            if values.is_empty() {
                default.to_vec()
            } else {
                values
            }
        }
        Err(_) => default.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_public_read_only_api_profile() {
        let config = NodeConfig::new(Network::Testnet);
        assert_eq!(config.network, Network::Testnet);
        assert!(config.api.enabled);
        assert_eq!(config.api.bind, "127.0.0.1");
        assert_eq!(config.api.port, 8080);
        assert!(config.api.public_read_only);
        assert!(!config.api.admin_enabled);
        assert!(!config.api.wallet_enabled);
        assert!(!config.api.mining_enabled);
        assert_eq!(config.api.max_response_bytes, 1_048_576);
        assert_eq!(
            config.api.cors.allowed_origins,
            vec![
                "https://atho.io".to_string(),
                "https://www.atho.io".to_string()
            ]
        );
        assert_eq!(config.api.rate_limit.requests_per_minute, 60);
        assert_eq!(config.api.rate_limit.heavy_requests_per_minute, 20);
        assert_eq!(config.api.bind_address(), "127.0.0.1:8080");
        assert!(config.api.explorer.index_enabled);
        assert!(config.api.explorer.snapshot_enabled);
    }

    #[test]
    fn env_overrides_api_defaults() {
        std::env::set_var("ATHO_API_ENABLED", "false");
        std::env::set_var("ATHO_API_BIND", "0.0.0.0");
        std::env::set_var("ATHO_API_PORT", "18080");
        std::env::set_var(
            "ATHO_API_ALLOWED_ORIGINS",
            "https://example.com,https://atho.io",
        );
        std::env::set_var("ATHO_API_RATE_LIMIT_RPM", "42");
        std::env::set_var("ATHO_API_HEAVY_RATE_LIMIT_RPM", "7");
        std::env::set_var("ATHO_API_MAX_RESPONSE_BYTES", "2048");
        std::env::set_var("ATHO_EXPLORER_SNAPSHOT_ENABLED", "false");
        let config = NodeConfig::from_env(Network::Regnet);
        std::env::remove_var("ATHO_API_ENABLED");
        std::env::remove_var("ATHO_API_BIND");
        std::env::remove_var("ATHO_API_PORT");
        std::env::remove_var("ATHO_API_ALLOWED_ORIGINS");
        std::env::remove_var("ATHO_API_RATE_LIMIT_RPM");
        std::env::remove_var("ATHO_API_HEAVY_RATE_LIMIT_RPM");
        std::env::remove_var("ATHO_API_MAX_RESPONSE_BYTES");
        std::env::remove_var("ATHO_EXPLORER_SNAPSHOT_ENABLED");

        assert!(!config.api.enabled);
        assert_eq!(config.api.bind, "0.0.0.0");
        assert_eq!(config.api.port, 18080);
        assert_eq!(
            config.api.cors.allowed_origins,
            vec![
                "https://example.com".to_string(),
                "https://atho.io".to_string()
            ]
        );
        assert_eq!(config.api.rate_limit.requests_per_minute, 42);
        assert_eq!(config.api.rate_limit.heavy_requests_per_minute, 7);
        assert_eq!(config.api.max_response_bytes, 2048);
        assert!(!config.api.explorer.snapshot_enabled);
    }
}
