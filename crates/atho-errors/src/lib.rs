mod registry;

use serde::{Deserialize, Serialize};
use std::fmt;

pub use registry::{
    registry_descriptor, render_json_registry, render_markdown_registry, ADDR_INVALID_ALPHABET,
    ADDR_INVALID_CHECKSUM, ADDR_INVALID_PREFIX, BLK_BLOCK_TOO_LARGE, BLK_COINBASE_REWARD_MISMATCH,
    BLK_DUPLICATE_TRANSACTION_ID, BLK_EMPTY_BLOCK, BLK_INVALID_COINBASE, BLK_INVALID_HEIGHT,
    BLK_INVALID_TIMESTAMP, BLK_INVALID_VERSION, BLK_MERKLE_ROOT_MISMATCH, BLK_MULTIPLE_COINBASE,
    BLK_PARENT_HASH_MISMATCH, BLK_POW_INVALID, BLK_TARGET_OUT_OF_BOUNDS, BLK_WITNESS_ROOT_MISMATCH,
    CONS_INVALID_POW_TARGET, CONS_INVALID_SUBSIDY_SCHEDULE, DB_BRANCH_NOT_PREFERRED,
    DB_CORRUPT_DATA, DB_CROSS_NETWORK_REPLAY, DB_EMPTY_BRANCH, DB_FORK_POINT_UNAVAILABLE,
    DB_INCOMPLETE_BLOCK_HISTORY, DB_INVALID_BRANCH_SEQUENCE, DB_IO, DB_LEGACY_STORAGE_LAYOUT,
    DB_LMDB, DB_NO_BLOCK_TO_DISCONNECT, DB_PATH_UNAVAILABLE, DB_PERSISTED_GENESIS_MISMATCH,
    DB_PERSISTED_TIP_MISMATCH, DB_ROLLBACK_FAILURE, DB_SCHEMA_VERSION_MISMATCH,
    HASH_CHECKSUM_MISMATCH, LAUNCH_INVALID_PEER_ADDRESS, LAUNCH_P2P_BIND_FAILED,
    LAUNCH_PUBLIC_RPC_DENIED, LAUNCH_RPC_BIND_FAILED, MEM_DUST_OUTPUT, MEM_MEMPOOL_CONFLICT,
    MINE_BACKEND_FAILURE, MINE_CANCELLED, MINE_GPU_BATCH_TOO_LARGE, MINE_GPU_BUFFER_ALLOC_FAILED,
    MINE_GPU_BUFFER_IO_FAILED, MINE_GPU_CONTEXT_CREATE_FAILED, MINE_GPU_FEATURE_DISABLED,
    MINE_GPU_INVALID_ARGUMENT, MINE_GPU_KERNEL_BUILD_FAILED, MINE_GPU_KERNEL_CREATE_FAILED,
    MINE_GPU_KERNEL_EXEC_FAILED, MINE_GPU_KERNEL_LOAD_FAILED, MINE_GPU_KERNEL_MISSING,
    MINE_GPU_NONCE_OVERFLOW, MINE_GPU_NOT_FOUND, MINE_GPU_PROBE_FAILED,
    MINE_GPU_QUEUE_CREATE_FAILED, MINE_GPU_SOLUTION_MISMATCH, MINE_GPU_UNKNOWN,
    NET_BLOCK_NETWORK_MISMATCH, NET_GENESIS_MISMATCH, NET_INVALID_MAGIC,
    NET_INVALID_NETWORK_SELECTION, NET_RULESET_MISMATCH, NET_UNSUPPORTED_NETWORK, P2P_BANNED_PEER,
    P2P_HANDSHAKE_INCOMPLETE, P2P_INBOUND_LIMIT, P2P_INVALID_COMPACT_BLOCK,
    P2P_INVALID_HEADERS_SEQUENCE, P2P_IO_FAILURE, P2P_MALFORMED_PAYLOAD, P2P_MESSAGE_TOO_SHORT,
    P2P_OUTBOUND_LIMIT, P2P_PAYLOAD_TOO_LARGE, P2P_PEER_ALREADY_CONNECTED, P2P_PEER_BOOK_FULL,
    P2P_TOO_MANY_HEADERS, P2P_TOO_MANY_INVENTORY, P2P_TOO_MANY_LOCATORS,
    P2P_TOO_MANY_PEER_ADDRESSES, P2P_UNEXPECTED_PAYLOAD, P2P_UNKNOWN_COMMAND, P2P_UNKNOWN_PEER,
    P2P_UNSUPPORTED_PROTOCOL, P2P_USER_AGENT_TOO_LONG, REGISTRY, RPC_EMPTY_RESPONSE, RPC_INTERNAL,
    RPC_INVALID_REQUEST, RPC_MESSAGE_TOO_LARGE, RPC_METHOD_NOT_FOUND, RPC_QT_UNEXPECTED,
    RPC_SERIALIZATION, RPC_TRANSPORT_IO, RPC_VALIDATION, SIG_BACKEND_UNAVAILABLE,
    SIG_CRYPTO_OPERATION_FAILED, SIG_INVALID_KEY_LENGTH, SIG_INVALID_WITNESS,
    SIG_WITNESS_INPUT_REF_MISMATCH, TX_DUPLICATE_INPUT, TX_FEE_BELOW_MINIMUM, TX_FEE_MISMATCH,
    TX_INPUT_OWNERSHIP_MISMATCH, TX_INSUFFICIENT_CONFIRMATIONS, TX_INVALID_POW_NONCE,
    TX_INVALID_VERSION, TX_MISSING_UTXO, TX_NO_INPUTS, TX_NO_OUTPUTS, TX_TOO_LARGE,
    TX_TOO_MANY_OUTPUTS, TX_WRONG_POW_BITS, TX_ZERO_VALUE_OUTPUT, UTXO_DUPLICATE, UTXO_MISSING,
    WALLET_INVALID_ENTROPY_LENGTH, WALLET_INVALID_HEADER, WALLET_INVALID_MNEMONIC_CHECKSUM,
    WALLET_INVALID_MNEMONIC_WORD, WALLET_INVALID_MNEMONIC_WORD_COUNT, WALLET_INVALID_PASSWORD,
    WALLET_IO, WALLET_RANDOMNESS_FAILURE, WALLET_SERIALIZATION, WALLET_UNSUPPORTED_ENCRYPTION_MODE,
    WALLET_UNSUPPORTED_VERSION,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AthoErrorCode(&'static str);

impl AthoErrorCode {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for AthoErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AthoErrorCategory {
    Config,
    Network,
    Consensus,
    Block,
    Transaction,
    Utxo,
    Mempool,
    Signature,
    Hash,
    Address,
    Rpc,
    P2p,
    Database,
    Serialization,
    Mining,
    Reorg,
    Wallet,
    Launcher,
    Test,
}

impl AthoErrorCategory {
    pub const fn short_code(self) -> &'static str {
        match self {
            Self::Config => "CFG",
            Self::Network => "NET",
            Self::Consensus => "CONS",
            Self::Block => "BLK",
            Self::Transaction => "TX",
            Self::Utxo => "UTXO",
            Self::Mempool => "MEM",
            Self::Signature => "SIG",
            Self::Hash => "HASH",
            Self::Address => "ADDR",
            Self::Rpc => "RPC",
            Self::P2p => "P2P",
            Self::Database => "DB",
            Self::Serialization => "SER",
            Self::Mining => "MINE",
            Self::Reorg => "REORG",
            Self::Wallet => "WALLET",
            Self::Launcher => "LAUNCH",
            Self::Test => "TEST",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AthoSeverity {
    Info,
    Warning,
    Error,
    Critical,
    Fatal,
}

impl AthoSeverity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Critical => "critical",
            Self::Fatal => "fatal",
        }
    }
}

impl fmt::Display for AthoSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AthoErrorDescriptor {
    pub code: AthoErrorCode,
    pub category: AthoErrorCategory,
    pub severity: AthoSeverity,
    pub title: &'static str,
    pub explanation: &'static str,
    pub common_cause: &'static str,
    pub suggested_fix: &'static str,
    pub user_facing: bool,
    pub consensus_critical: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AthoErrorContext {
    pub source_module: &'static str,
    pub subject: Option<String>,
    pub network: Option<String>,
    pub height: Option<u64>,
    pub safe_details: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AthoError {
    descriptor: &'static AthoErrorDescriptor,
    pub message: String,
    pub technical: Option<String>,
    pub context: AthoErrorContext,
}

impl AthoError {
    pub fn new(
        descriptor: &'static AthoErrorDescriptor,
        source_module: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            descriptor,
            message: message.into(),
            technical: None,
            context: AthoErrorContext {
                source_module,
                ..AthoErrorContext::default()
            },
        }
    }

    pub fn with_technical(mut self, technical: impl Into<String>) -> Self {
        self.technical = Some(technical.into());
        self
    }

    pub fn with_safe_details(mut self, details: impl Into<String>) -> Self {
        self.context.safe_details = Some(details.into());
        self
    }

    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.context.subject = Some(subject.into());
        self
    }

    pub fn with_network(mut self, network: impl Into<String>) -> Self {
        self.context.network = Some(network.into());
        self
    }

    pub fn with_height(mut self, height: u64) -> Self {
        self.context.height = Some(height);
        self
    }

    pub fn descriptor(&self) -> &'static AthoErrorDescriptor {
        self.descriptor
    }

    pub fn code(&self) -> AthoErrorCode {
        self.descriptor.code
    }

    pub fn title(&self) -> &'static str {
        self.descriptor.title
    }

    pub fn severity(&self) -> AthoSeverity {
        self.descriptor.severity
    }

    pub fn source_module(&self) -> &'static str {
        self.context.source_module
    }

    pub fn log_line(&self) -> String {
        let mut line = format!(
            "[{}] [{}] [{}] {}",
            self.severity().as_str().to_uppercase(),
            self.code(),
            self.source_module(),
            self.title()
        );
        if let Some(subject) = &self.context.subject {
            line.push_str(&format!(" | subject={subject}"));
        }
        if let Some(network) = &self.context.network {
            line.push_str(&format!(" | network={network}"));
        }
        if let Some(height) = self.context.height {
            line.push_str(&format!(" | height={height}"));
        }
        if let Some(details) = &self.context.safe_details {
            line.push_str(&format!(" | details={details}"));
        }
        if let Some(technical) = &self.technical {
            line.push_str(&format!(" | technical={technical}"));
        }
        line
    }
}

impl fmt::Display for AthoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.code(), self.title(), self.message)?;
        if let Some(details) = &self.context.safe_details {
            write!(f, " ({details})")?;
        }
        Ok(())
    }
}

pub trait AthoErrorMeta: fmt::Display {
    fn descriptor(&self) -> &'static AthoErrorDescriptor;
    fn source_module(&self) -> &'static str;

    fn user_message(&self) -> String {
        self.descriptor().explanation.to_string()
    }

    fn technical_details(&self) -> Option<String> {
        Some(self.to_string())
    }

    fn safe_details(&self) -> Option<String> {
        None
    }

    fn to_atho_error(&self) -> AthoError {
        let mut error =
            AthoError::new(self.descriptor(), self.source_module(), self.user_message());
        if let Some(technical) = self.technical_details() {
            error.technical = Some(technical);
        }
        if let Some(details) = self.safe_details() {
            error.context.safe_details = Some(details);
        }
        error
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn registry_has_unique_codes() {
        let mut seen = BTreeSet::new();
        for descriptor in REGISTRY {
            assert!(
                seen.insert(descriptor.code.as_str()),
                "duplicate error code {}",
                descriptor.code
            );
        }
    }

    #[test]
    fn registry_entries_have_descriptions() {
        for descriptor in REGISTRY {
            assert!(
                !descriptor.title.is_empty(),
                "missing title for {}",
                descriptor.code
            );
            assert!(
                !descriptor.explanation.is_empty(),
                "missing explanation for {}",
                descriptor.code
            );
            assert!(
                !descriptor.common_cause.is_empty(),
                "missing common cause for {}",
                descriptor.code
            );
            assert!(
                !descriptor.suggested_fix.is_empty(),
                "missing suggested fix for {}",
                descriptor.code
            );
        }
    }

    #[test]
    fn log_line_includes_code_and_module() {
        let error = AthoError::new(&RPC_INVALID_REQUEST, "rpc", "The RPC request is invalid.")
            .with_safe_details("missing request field");
        let line = error.log_line();
        assert!(line.contains("[ATHO-RPC-002]"));
        assert!(line.contains("[rpc]"));
        assert!(line.contains("missing request field"));
    }

    #[test]
    fn registry_json_document_matches_generated_output() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let json = fs::read_to_string(root.join("error_codes.json")).expect("error code json");
        assert_eq!(json, render_json_registry().expect("render json"));
    }
}
