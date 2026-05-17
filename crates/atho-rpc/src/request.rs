// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! RPC request types accepted by the Atho node service.
use crate::command::CommandInvocation;
use atho_core::block::Block;
use atho_core::transaction::Transaction;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletHistoryAddress {
    #[serde(with = "serde_big_array::BigArray")]
    pub payment_digest: [u8; 32],
    pub address: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RpcRequest {
    Authenticated {
        username: String,
        password: String,
        request: Box<RpcRequest>,
    },
    GetBlockCount,
    GetNetwork,
    GetNodeStatus,
    GetBlockTemplate,
    SubmitBlock(Block),
    SubmitTransaction {
        transaction: Transaction,
        fee_atoms: u64,
    },
    ListUtxos,
    GetWalletActivity {
        addresses: Vec<WalletHistoryAddress>,
    },
    GetMempoolInfo,
    GetMempoolSpentInputs,
    ExecuteCommand(CommandInvocation),
}

impl RpcRequest {
    pub fn authenticated(
        username: impl Into<String>,
        password: impl Into<String>,
        request: RpcRequest,
    ) -> Self {
        Self::Authenticated {
            username: username.into(),
            password: password.into(),
            request: Box::new(request),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_faucet_rpc_method_is_not_deserializable() {
        assert!(serde_json::from_str::<RpcRequest>(r#""RequestTestnetFaucet""#).is_err());
        assert!(serde_json::from_str::<RpcRequest>(
            r#"{"RequestTestnetFaucet":{"address":"T6AD","amount_atoms":1000}}"#
        )
        .is_err());
    }
}
