#![forbid(unsafe_code)]

pub mod address;
pub mod block;
pub mod consensus;
pub mod constants;
pub mod crypto;
pub mod encoding;
pub mod error;
pub mod genesis;
pub mod network;
pub mod transaction;

#[cfg(test)]
mod tests {
    use super::constants::*;

    #[test]
    fn carries_forward_protocol_basics() {
        assert_eq!(ATOMS_PER_ATHO, 100_000_000);
        assert_eq!(MAX_SUPPLY_ATHO, 168_000_000);
        assert_eq!(MAX_SUPPLY_ATOMS, 16_800_000_000_000_000);
        assert_eq!(HALVING_INTERVAL_BLOCKS, 1_680_000);
        assert_eq!(COINBASE_MATURITY_BLOCKS, 150);
        assert_eq!(STANDARD_TX_CONFIRMATIONS, 7);
        assert_eq!(MIN_TX_FEE_ATOMS, 1);
        assert_eq!(BLOCK_TIME_SECONDS, 75);
        assert_eq!(MAX_BLOCK_VBYTES, 3_000_000);
        assert_eq!(MAX_BLOCK_RAW_BYTES, 12_000_000);
        assert_eq!(MAX_BLOCK_SIZE_BYTES, 3_000_000);
        assert_eq!(MAX_BLOCK_WEIGHT, 12_000_000);
        assert_eq!(MAX_BLOCK_SERIALIZED_BYTES, 12_000_000);
        assert_eq!(FALCON_512_LOGN, 9);
        assert_eq!(FALCON_512_PUBLIC_KEY_BYTES, 897);
        assert_eq!(FALCON_512_SECRET_KEY_BYTES, 1_281);
        assert_eq!(FALCON_512_SIGNATURE_BYTES, 666);
        assert_eq!(MIN_TRANSACTION_FULL_BYTES_WITH_ONE_INPUT_AND_ONE_OUTPUT, 88);
        assert_eq!(TX_WITNESS_FIXED_BYTES, 1_575);
        assert_eq!(MAX_WITNESS_INPUT_REFS, 13_796);
        assert_eq!(ADDRESS_CHECKSUM_BYTES, 4);
        assert_eq!(GENESIS_COINBASE_ATOMS, 5_000_000_000);
    }
}
