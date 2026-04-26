use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AddressError {
    #[error("invalid base56 alphabet")]
    InvalidAlphabet,
    #[error("invalid visible prefix")]
    InvalidPrefix,
    #[error("invalid checksum")]
    InvalidChecksum,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConsensusError {
    #[error("invalid subsidy schedule")]
    InvalidSubsidySchedule,
    #[error("invalid proof-of-work target")]
    InvalidProofOfWorkTarget,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TransactionError {
    #[error("transaction has no inputs")]
    NoInputs,
    #[error("transaction has no outputs")]
    NoOutputs,
    #[error("duplicate transaction input")]
    DuplicateInput,
    #[error("zero-value output")]
    ZeroValueOutput,
    #[error("transaction too large")]
    TooLarge,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BlockError {
    #[error("block has no transactions")]
    EmptyBlock,
    #[error("block too large")]
    TooLarge,
    #[error("block merkle root mismatch")]
    MerkleRootMismatch,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CoreError {
    #[error(transparent)]
    Address(#[from] AddressError),
    #[error(transparent)]
    Consensus(#[from] ConsensusError),
    #[error(transparent)]
    Transaction(#[from] TransactionError),
    #[error(transparent)]
    Block(#[from] BlockError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_error_wraps_specific_protocol_errors() {
        let err = CoreError::from(AddressError::InvalidChecksum);
        assert!(matches!(
            err,
            CoreError::Address(AddressError::InvalidChecksum)
        ));
    }
}
