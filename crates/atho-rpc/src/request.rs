use atho_core::block::Block;
use atho_core::transaction::Transaction;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RpcRequest {
    GetBlockCount,
    GetNetwork,
    GetBlockTemplate,
    SubmitBlock(Block),
    SubmitTransaction {
        transaction: Transaction,
        fee_atoms: u64,
    },
    ListUtxos,
    GetMempoolInfo,
}
