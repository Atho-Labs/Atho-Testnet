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
    RequestTestnetFaucet {
        destination_address: String,
        amount_atoms: u64,
        requester_id: String,
    },
    ExecuteCommand(CommandInvocation),
}
