// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Structured RPC errors and safe user-facing serialization.
use atho_errors::{
    AthoError, AthoErrorDescriptor, AthoSeverity, RPC_INTERNAL, RPC_INVALID_REQUEST,
    RPC_METHOD_NOT_FOUND, RPC_VALIDATION,
};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: String,
    pub title: String,
    pub message: String,
    pub severity: AthoSeverity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

impl RpcError {
    pub fn method_not_found() -> Self {
        Self::from_descriptor(&RPC_METHOD_NOT_FOUND, None)
    }

    pub fn invalid_request(details: impl Into<String>) -> Self {
        Self::from_descriptor(&RPC_INVALID_REQUEST, Some(details.into()))
    }

    pub fn internal() -> Self {
        Self::from_descriptor(&RPC_INTERNAL, None)
    }

    pub fn validation(details: impl Into<String>) -> Self {
        Self::from_descriptor(&RPC_VALIDATION, Some(details.into()))
    }

    pub fn from_atho_error(error: AthoError) -> Self {
        let severity = error.severity();
        Self {
            code: error.code().as_str().to_string(),
            title: error.title().to_string(),
            message: error.message,
            severity,
            details: error.context.safe_details.or(error.technical),
        }
    }

    fn from_descriptor(descriptor: &'static AthoErrorDescriptor, details: Option<String>) -> Self {
        Self {
            code: descriptor.code.as_str().to_string(),
            title: descriptor.title.to_string(),
            message: descriptor.explanation.to_string(),
            severity: descriptor.severity,
            details,
        }
    }
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.code, self.title, self.message)?;
        if let Some(details) = &self.details {
            write!(f, " ({details})")?;
        }
        Ok(())
    }
}

impl std::error::Error for RpcError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_error_serializes_structured_fields() {
        let error = RpcError::invalid_request("missing network parameter");
        let json = serde_json::to_value(&error).expect("serialize rpc error");
        assert_eq!(json["code"], "ATHO-RPC-002");
        assert_eq!(json["title"], "Invalid RPC Request");
        assert_eq!(
            json["message"],
            "The RPC request was malformed or missing required data."
        );
        assert_eq!(json["severity"], "error");
        assert_eq!(json["details"], "missing network parameter");
    }
}
