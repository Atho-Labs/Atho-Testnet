// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Canonical Atho error descriptors and conversion helpers.
//!
//! This crate gives the rest of the workspace one place to describe user-facing
//! and operator-facing failures with stable codes and structured metadata.

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
    MINE_REWARD_ADDRESS_REQUIRED, NET_BLOCK_NETWORK_MISMATCH, NET_GENESIS_MISMATCH,
    NET_INVALID_MAGIC, NET_INVALID_NETWORK_SELECTION, NET_RULESET_MISMATCH,
    NET_UNSUPPORTED_NETWORK, P2P_BANNED_PEER, P2P_HANDSHAKE_INCOMPLETE, P2P_INBOUND_LIMIT,
    P2P_INVALID_COMPACT_BLOCK, P2P_INVALID_HEADERS_SEQUENCE, P2P_IO_FAILURE, P2P_MALFORMED_PAYLOAD,
    P2P_MESSAGE_TOO_SHORT, P2P_OUTBOUND_LIMIT, P2P_PAYLOAD_TOO_LARGE, P2P_PEER_ALREADY_CONNECTED,
    P2P_PEER_BOOK_FULL, P2P_TOO_MANY_HEADERS, P2P_TOO_MANY_INVENTORY, P2P_TOO_MANY_LOCATORS,
    P2P_TOO_MANY_PEER_ADDRESSES, P2P_UNEXPECTED_PAYLOAD, P2P_UNKNOWN_COMMAND, P2P_UNKNOWN_PEER,
    P2P_UNSUPPORTED_PROTOCOL, P2P_USER_AGENT_TOO_LONG, REGISTRY, RPC_EMPTY_RESPONSE, RPC_INTERNAL,
    RPC_INVALID_REQUEST, RPC_MESSAGE_TOO_LARGE, RPC_METHOD_NOT_FOUND, RPC_QT_UNEXPECTED,
    RPC_SERIALIZATION, RPC_TRANSPORT_IO, RPC_VALIDATION, SIG_BACKEND_UNAVAILABLE,
    SIG_CRYPTO_OPERATION_FAILED, SIG_INVALID_KEY_LENGTH, SIG_INVALID_WITNESS,
    SIG_WITNESS_INPUT_REF_MISMATCH, TX_DUPLICATE_INPUT, TX_FEE_BELOW_MINIMUM, TX_FEE_MISMATCH,
    TX_INPUT_OWNERSHIP_MISMATCH, TX_INSUFFICIENT_CONFIRMATIONS, TX_INVALID_POW_NONCE,
    TX_INVALID_VERSION, TX_LEGACY_LOCK_FORMAT_REJECTED, TX_MISSING_UTXO, TX_NO_INPUTS,
    TX_NO_OUTPUTS, TX_TOO_LARGE, TX_TOO_MANY_OUTPUTS, TX_WRONG_POW_BITS, TX_ZERO_VALUE_OUTPUT,
    UTXO_DUPLICATE, UTXO_MISSING, WALLET_INVALID_ENTROPY_LENGTH, WALLET_INVALID_HEADER,
    WALLET_INVALID_MNEMONIC_CHECKSUM, WALLET_INVALID_MNEMONIC_WORD,
    WALLET_INVALID_MNEMONIC_WORD_COUNT, WALLET_INVALID_PASSWORD, WALLET_IO,
    WALLET_RANDOMNESS_FAILURE, WALLET_SERIALIZATION, WALLET_UNSUPPORTED_ENCRYPTION_MODE,
    WALLET_UNSUPPORTED_VERSION,
};

/// Stable registry-backed Atho error code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AthoErrorCode(&'static str);

impl AthoErrorCode {
    /// Creates a new static error code wrapper.
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    /// Returns the raw string form used in logs and APIs.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for AthoErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// High-level bucket used to group related errors.
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
    /// Returns the short category token embedded in Atho error codes.
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

/// Severity level attached to a registry descriptor.
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
    /// Returns the lowercase string form used in logs and JSON payloads.
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

/// Immutable metadata entry for a single canonical Atho error code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AthoErrorDescriptor {
    /// Stable registry code such as `ATHO-TX-001`.
    pub code: AthoErrorCode,
    /// Broad area of the system the error belongs to.
    pub category: AthoErrorCategory,
    /// Relative urgency of the failure.
    pub severity: AthoSeverity,
    /// Short title suitable for logs or UI headings.
    pub title: &'static str,
    /// Human-readable explanation of what went wrong.
    pub explanation: &'static str,
    /// Typical root cause to help operators triage the issue.
    pub common_cause: &'static str,
    /// Recommended next step for a user or operator.
    pub suggested_fix: &'static str,
    /// Whether the descriptor is suitable to show directly to end users.
    pub user_facing: bool,
    /// Whether the failure can indicate a consensus-invalid state transition.
    pub consensus_critical: bool,
}

/// Sanitized runtime details attached to a concrete error instance.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AthoErrorContext {
    /// Logical module reporting the failure.
    pub source_module: &'static str,
    /// Optional subject such as a txid, path, or peer label.
    pub subject: Option<String>,
    /// Optional network context for multi-network tools.
    pub network: Option<String>,
    /// Optional chain height related to the failure.
    pub height: Option<u64>,
    /// Safe diagnostic details that may be shown in logs or UI.
    pub safe_details: Option<String>,
}

/// Structured runtime error instance paired with a registry descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AthoError {
    descriptor: &'static AthoErrorDescriptor,
    /// User-focused summary message for the specific occurrence.
    pub message: String,
    /// Optional lower-level details intended for logs and diagnostics.
    pub technical: Option<String>,
    /// Sanitized context fields attached to this error instance.
    pub context: AthoErrorContext,
}

impl AthoError {
    /// Creates a new runtime error from a descriptor and source module.
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

    /// Attaches implementation-specific diagnostics that should stay out of UX copy.
    pub fn with_technical(mut self, technical: impl Into<String>) -> Self {
        self.technical = Some(technical.into());
        self
    }

    /// Attaches extra safe details suitable for logs and UI surfaces.
    pub fn with_safe_details(mut self, details: impl Into<String>) -> Self {
        self.context.safe_details = Some(details.into());
        self
    }

    /// Attaches the primary subject of the failure, such as a txid or file path.
    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.context.subject = Some(subject.into());
        self
    }

    /// Attaches network context for multi-network callers.
    pub fn with_network(mut self, network: impl Into<String>) -> Self {
        self.context.network = Some(network.into());
        self
    }

    /// Attaches a relevant chain height to the error.
    pub fn with_height(mut self, height: u64) -> Self {
        self.context.height = Some(height);
        self
    }

    /// Returns the static descriptor describing this error family.
    pub fn descriptor(&self) -> &'static AthoErrorDescriptor {
        self.descriptor
    }

    /// Returns the stable Atho error code.
    pub fn code(&self) -> AthoErrorCode {
        self.descriptor.code
    }

    /// Returns the short descriptor title.
    pub fn title(&self) -> &'static str {
        self.descriptor.title
    }

    /// Returns the descriptor severity.
    pub fn severity(&self) -> AthoSeverity {
        self.descriptor.severity
    }

    /// Returns the module that reported the error.
    pub fn source_module(&self) -> &'static str {
        self.context.source_module
    }

    /// Formats the error as a structured one-line log entry.
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

/// Trait for typed errors that can map themselves into the Atho registry model.
pub trait AthoErrorMeta: fmt::Display {
    /// Returns the canonical descriptor for the error variant.
    fn descriptor(&self) -> &'static AthoErrorDescriptor;
    /// Returns the logical module originating this error.
    fn source_module(&self) -> &'static str;

    /// Returns a user-oriented message for the specific error instance.
    fn user_message(&self) -> String {
        self.descriptor().explanation.to_string()
    }

    /// Returns technical diagnostics appropriate for logs.
    fn technical_details(&self) -> Option<String> {
        Some(self.to_string())
    }

    /// Returns additional sanitized details suitable for user-visible surfaces.
    fn safe_details(&self) -> Option<String> {
        None
    }

    /// Converts the typed error into the shared runtime error shape.
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
