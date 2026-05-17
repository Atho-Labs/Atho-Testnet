// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Node configuration defaults and runtime selection helpers.
use atho_core::network::Network;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

const DEFAULT_API_BIND: &str = "127.0.0.1";
const DEFAULT_API_PORT: u16 = 8080;
const DEFAULT_API_MAX_RESPONSE_BYTES: usize = 1_048_576;
const DEFAULT_ALLOWED_ORIGINS: &[&str] = &["https://atho.io", "https://www.atho.io"];
const DEFAULT_RATE_LIMIT_RPM: u32 = 180;
const DEFAULT_HEAVY_RATE_LIMIT_RPM: u32 = 90;
pub const ATHO_CONF_FILE_ENV: &str = "ATHO_CONF_FILE";
pub const ATHO_RPC_DEFAULT_USER: &str = "atho";
pub const ATHO_RPC_DEFAULT_PASSWORD: &str = "change-this-before-public-rpc";
pub const DEFAULT_MAX_MEMPOOL_TRANSACTIONS: usize = 50_000;
pub const DEFAULT_MAX_MEMPOOL_VBYTES: usize = 64 * 1024 * 1024;
pub const DEFAULT_PRUNE_TARGET_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const DEFAULT_DB_CACHE_BYTES: u64 = 512 * 1024 * 1024;
pub const DEFAULT_MAX_PEER_CONNECTIONS: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeConfig {
    pub network: Network,
    pub api: ApiConfig,
    pub rpc_auth: RpcAuthConfig,
    pub mempool: MempoolConfig,
    pub storage: StorageConfig,
    pub peers: PeerConfig,
    pub wallet: WalletRuntimeConfig,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcAuthConfig {
    pub enabled: bool,
    pub bind: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MempoolConfig {
    pub max_transactions: usize,
    pub max_vbytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageConfig {
    pub prune_target_bytes: u64,
    pub db_cache_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerConfig {
    pub max_connections: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalletRuntimeConfig {
    pub enabled: bool,
    pub require_encryption: bool,
}

impl NodeConfig {
    pub fn new(network: Network) -> Self {
        Self {
            network,
            api: ApiConfig::for_network(network),
            rpc_auth: RpcAuthConfig::for_network(network),
            mempool: MempoolConfig::default(),
            storage: StorageConfig::default(),
            peers: PeerConfig::default(),
            wallet: WalletRuntimeConfig::default(),
        }
    }

    pub fn from_env(network: Network) -> Self {
        let mut config = Self::new(network);
        if let Ok(file) = OperatorConfigFile::load_default() {
            config.apply_file_overrides(&file);
        }
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
        config.rpc_auth.enabled = env_bool("ATHO_RPC_AUTH_ENABLED", config.rpc_auth.enabled);
        config.rpc_auth.bind = std::env::var("ATHO_RPC_BIND").unwrap_or(config.rpc_auth.bind);
        config.rpc_auth.port = env_u16("ATHO_RPC_PORT", config.rpc_auth.port);
        config.rpc_auth.username =
            std::env::var("ATHO_RPC_USER").unwrap_or(config.rpc_auth.username);
        config.rpc_auth.password =
            std::env::var("ATHO_RPC_PASSWORD").unwrap_or(config.rpc_auth.password);
        config.mempool.max_transactions = env_usize(
            "ATHO_MAX_MEMPOOL_TRANSACTIONS",
            config.mempool.max_transactions,
        );
        config.mempool.max_vbytes = env_usize("ATHO_MAX_MEMPOOL_VBYTES", config.mempool.max_vbytes);
        config.storage.prune_target_bytes =
            env_u64("ATHO_PRUNE_TARGET_BYTES", config.storage.prune_target_bytes);
        config.storage.db_cache_bytes =
            env_u64("ATHO_DB_CACHE_BYTES", config.storage.db_cache_bytes);
        config.peers.max_connections =
            env_usize("ATHO_MAX_PEER_CONNECTIONS", config.peers.max_connections);
        config.wallet.enabled = env_bool("ATHO_WALLET_ENABLED", config.wallet.enabled);
        config.wallet.require_encryption = env_bool(
            "ATHO_WALLET_REQUIRE_ENCRYPTION",
            config.wallet.require_encryption,
        );
        config.clamp_user_tunable_bounds();
        config
    }

    pub fn network_from_sources(default: Network) -> Result<Network, String> {
        let file_network = OperatorConfigFile::load_default()
            .ok()
            .and_then(|file| file.get("network").and_then(Network::parse))
            .unwrap_or(default);
        match std::env::var("ATHO_NETWORK") {
            Ok(raw) => Network::parse(&raw).ok_or_else(|| format!("invalid network {raw}")),
            Err(_) => Ok(file_network),
        }
    }

    pub fn config_file_path() -> PathBuf {
        std::env::var_os(ATHO_CONF_FILE_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| atho_storage::path::sandbox_root().join("atho.conf"))
    }

    pub fn rpc_bind_address(&self) -> String {
        std::env::var("ATHO_RPC_ADDR")
            .unwrap_or_else(|_| format!("{}:{}", self.rpc_auth.bind, self.rpc_auth.port))
    }

    pub fn write_operator_config_file(&self) -> std::io::Result<()> {
        let path = Self::config_file_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, self.to_operator_config_text())
    }

    pub fn to_operator_config_text(&self) -> String {
        let prune_mib = bytes_to_mib_ceil(self.storage.prune_target_bytes);
        let db_cache_mib = bytes_to_mib_ceil(self.storage.db_cache_bytes);
        let max_mempool_mib = bytes_to_mib_ceil(self.mempool.max_vbytes as u64);
        format!(
            concat!(
                "# Atho node configuration\n",
                "# Edit manually or through the desktop client's Node Settings page.\n",
                "network={}\n",
                "rpcuser={}\n",
                "rpcpassword={}\n",
                "rpcauth={}\n",
                "rpcbind={}\n",
                "rpcport={}\n",
                "wallet={}\n",
                "walletrequireencryption={}\n",
                "maxmempool={}\n",
                "maxmempooltx={}\n",
                "prune={}\n",
                "dbcache={}\n",
                "maxconnections={}\n",
                "api={}\n",
                "apiwallet={}\n",
                "apimining={}\n"
            ),
            self.network.cli_arg(),
            self.rpc_auth.username,
            self.rpc_auth.password,
            bool_as_conf(self.rpc_auth.enabled),
            self.rpc_auth.bind,
            self.rpc_auth.port,
            bool_as_conf(self.wallet.enabled),
            bool_as_conf(self.wallet.require_encryption),
            max_mempool_mib,
            self.mempool.max_transactions,
            prune_mib,
            db_cache_mib,
            self.peers.max_connections,
            bool_as_conf(self.api.enabled),
            bool_as_conf(self.api.wallet_enabled),
            bool_as_conf(self.api.mining_enabled)
        )
    }

    pub fn apply_process_overrides(&self) {
        std::env::set_var(
            "ATHO_PRUNE_TARGET_BYTES",
            self.storage.prune_target_bytes.to_string(),
        );
        std::env::set_var(
            "ATHO_DB_CACHE_BYTES",
            self.storage.db_cache_bytes.to_string(),
        );
        std::env::set_var(
            "ATHO_MAX_PEER_CONNECTIONS",
            self.peers.max_connections.to_string(),
        );
    }

    fn apply_file_overrides(&mut self, file: &OperatorConfigFile) {
        if let Some(value) = file.get("rpcuser") {
            self.rpc_auth.username = value.to_string();
        }
        if let Some(value) = file.get("rpcpassword") {
            self.rpc_auth.password = value.to_string();
        }
        self.rpc_auth.enabled = file.bool("rpcauth").unwrap_or(self.rpc_auth.enabled);
        if let Some(value) = file.get("rpcbind") {
            self.rpc_auth.bind = value.to_string();
        }
        if let Some(value) = file.u16("rpcport") {
            self.rpc_auth.port = value;
        }
        if let Some(value) = file.bool("api") {
            self.api.enabled = value;
        }
        if let Some(value) = file.bool("apiwallet") {
            self.api.wallet_enabled = value;
        }
        if let Some(value) = file.bool("apimining") {
            self.api.mining_enabled = value;
        }
        if let Some(value) = file.bool("wallet") {
            self.wallet.enabled = value;
        }
        if let Some(value) = file.bool("walletrequireencryption") {
            self.wallet.require_encryption = value;
        }
        if let Some(value) = file.usize("maxmempooltx") {
            self.mempool.max_transactions = value;
        }
        if let Some(value) = file.u64("maxmempool") {
            self.mempool.max_vbytes = mib_to_bytes(value) as usize;
        }
        if let Some(value) = file.usize("maxmempoolvbytes") {
            self.mempool.max_vbytes = value;
        }
        if let Some(value) = file.u64("prune") {
            self.storage.prune_target_bytes = mib_to_bytes(value);
        }
        if let Some(value) = file.u64("prunebytes") {
            self.storage.prune_target_bytes = value;
        }
        if let Some(value) = file.u64("dbcache") {
            self.storage.db_cache_bytes = mib_to_bytes(value);
        }
        if let Some(value) = file.u64("dbcachebytes") {
            self.storage.db_cache_bytes = value;
        }
        if let Some(value) = file.usize("maxconnections") {
            self.peers.max_connections = value;
        }
    }

    fn clamp_user_tunable_bounds(&mut self) {
        self.mempool.max_transactions = self.mempool.max_transactions.clamp(1_000, 1_000_000);
        self.mempool.max_vbytes = self
            .mempool
            .max_vbytes
            .clamp(8 * 1024 * 1024, 1024 * 1024 * 1024);
        self.storage.prune_target_bytes = self
            .storage
            .prune_target_bytes
            .max(DEFAULT_PRUNE_TARGET_BYTES);
        self.storage.db_cache_bytes = self
            .storage
            .db_cache_bytes
            .clamp(64 * 1024 * 1024, 8 * 1024 * 1024 * 1024);
        self.peers.max_connections = self.peers.max_connections.clamp(8, 512);
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

impl Default for RpcAuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: DEFAULT_API_BIND.to_string(),
            port: 0,
            username: ATHO_RPC_DEFAULT_USER.to_string(),
            password: ATHO_RPC_DEFAULT_PASSWORD.to_string(),
        }
    }
}

impl RpcAuthConfig {
    pub fn for_network(network: Network) -> Self {
        Self {
            port: network.rpc_port(),
            ..Self::default()
        }
    }

    pub fn credentials_are_default(&self) -> bool {
        self.username == ATHO_RPC_DEFAULT_USER && self.password == ATHO_RPC_DEFAULT_PASSWORD
    }

    pub fn securely_configured_for_public_rpc(&self) -> bool {
        self.enabled
            && !self.username.trim().is_empty()
            && !self.password.trim().is_empty()
            && !self.credentials_are_default()
    }
}

impl Default for MempoolConfig {
    fn default() -> Self {
        Self {
            max_transactions: DEFAULT_MAX_MEMPOOL_TRANSACTIONS,
            max_vbytes: DEFAULT_MAX_MEMPOOL_VBYTES,
        }
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            prune_target_bytes: DEFAULT_PRUNE_TARGET_BYTES,
            db_cache_bytes: DEFAULT_DB_CACHE_BYTES,
        }
    }
}

impl Default for PeerConfig {
    fn default() -> Self {
        Self {
            max_connections: DEFAULT_MAX_PEER_CONNECTIONS,
        }
    }
}

impl Default for WalletRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            require_encryption: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct OperatorConfigFile {
    values: BTreeMap<String, String>,
}

impl OperatorConfigFile {
    fn load_default() -> std::io::Result<Self> {
        Self::load(NodeConfig::config_file_path())
    }

    fn load(path: PathBuf) -> std::io::Result<Self> {
        match fs::read_to_string(path) {
            Ok(contents) => Ok(Self::parse(&contents)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err),
        }
    }

    fn parse(contents: &str) -> Self {
        let mut values = BTreeMap::new();
        for line in contents.lines() {
            let line = line.split('#').next().unwrap_or_default().trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim().trim_matches('"').to_string();
            if !key.is_empty() {
                values.insert(key, value);
            }
        }
        Self { values }
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.values
            .get(&key.to_ascii_lowercase())
            .map(String::as_str)
    }

    fn bool(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(parse_bool)
    }

    fn u16(&self, key: &str) -> Option<u16> {
        self.get(key).and_then(|value| value.parse().ok())
    }

    fn u64(&self, key: &str) -> Option<u64> {
        self.get(key).and_then(|value| value.parse().ok())
    }

    fn usize(&self, key: &str) -> Option<usize> {
        self.get(key).and_then(|value| value.parse().ok())
    }
}

fn env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(value) => parse_bool(&value).unwrap_or(default),
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

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(default)
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn bool_as_conf(value: bool) -> &'static str {
    if value {
        "1"
    } else {
        "0"
    }
}

fn mib_to_bytes(mib: u64) -> u64 {
    mib.saturating_mul(1024).saturating_mul(1024)
}

fn bytes_to_mib_ceil(bytes: u64) -> u64 {
    bytes.saturating_add(1024 * 1024 - 1) / (1024 * 1024)
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
        assert_eq!(config.rpc_bind_address(), "127.0.0.1:9110");
        assert_eq!(
            config.api.cors.allowed_origins,
            vec![
                "https://atho.io".to_string(),
                "https://www.atho.io".to_string()
            ]
        );
        assert_eq!(config.api.rate_limit.requests_per_minute, 180);
        assert_eq!(config.api.rate_limit.heavy_requests_per_minute, 90);
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

    #[test]
    fn operator_config_file_overrides_node_tunables() {
        let path = std::env::temp_dir().join(format!(
            "atho-conf-test-{}-{}.conf",
            std::process::id(),
            "node-tunables"
        ));
        fs::write(
            &path,
            "\
rpcuser=operator
rpcpassword=secret
rpcauth=1
rpcbind=127.0.0.2
rpcport=18100
maxmempool=96
maxmempooltx=12345
prune=2048
dbcache=768
maxconnections=64
wallet=0
walletrequireencryption=1
",
        )
        .expect("write config");
        std::env::set_var(ATHO_CONF_FILE_ENV, &path);
        let config = NodeConfig::from_env(Network::Mainnet);
        std::env::remove_var(ATHO_CONF_FILE_ENV);
        let _ = fs::remove_file(&path);

        assert!(config.rpc_auth.enabled);
        assert_eq!(config.rpc_auth.username, "operator");
        assert_eq!(config.rpc_auth.password, "secret");
        assert_eq!(config.rpc_bind_address(), "127.0.0.2:18100");
        assert_eq!(config.mempool.max_vbytes, 96 * 1024 * 1024);
        assert_eq!(config.mempool.max_transactions, 12_345);
        assert_eq!(
            config.storage.prune_target_bytes,
            DEFAULT_PRUNE_TARGET_BYTES
        );
        assert_eq!(config.storage.db_cache_bytes, 768 * 1024 * 1024);
        assert_eq!(config.peers.max_connections, 64);
        assert!(!config.wallet.enabled);
        assert!(config.wallet.require_encryption);
    }
}
