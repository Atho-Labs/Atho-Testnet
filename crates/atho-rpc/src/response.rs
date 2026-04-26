use crate::error::RpcError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RpcResponse {
    BlockCount(u64),
    Network(String),
    Error(RpcError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_error_response_is_stable() {
        assert_eq!(RpcResponse::Error(RpcError::InvalidRequest), RpcResponse::Error(RpcError::InvalidRequest));
    }
}
