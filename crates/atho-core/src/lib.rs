#![forbid(unsafe_code)]

pub mod address;
pub mod block;
pub mod consensus;
pub mod constants;
pub mod crypto;
pub mod error;
pub mod genesis;
pub mod network;
pub mod transaction;

#[cfg(test)]
mod tests {
    use super::constants::*;

    #[test]
    fn carries_forward_protocol_basics() {
        assert_eq!(MAX_SUPPLY_ATHO, 168_000_000);
        assert_eq!(HALVING_INTERVAL_BLOCKS, 1_680_000);
        assert_eq!(MIN_TX_FEE_ATOMS, 1);
        assert_eq!(BLOCK_TIME_SECONDS, 75);
        assert_eq!(MAX_BLOCK_SIZE_BYTES, 2_000_000);
        assert_eq!(MAX_BLOCK_WEIGHT, 8_000_000);
        assert_eq!(ADDRESS_CHECKSUM_BYTES, 4);
        assert_eq!(GENESIS_COINBASE_ATOMS, 5_000_000_000);
    }
}
