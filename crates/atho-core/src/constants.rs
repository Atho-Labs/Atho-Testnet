// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Consensus and policy constants shared across Atho crates.
//!
//! These values define monetary units, relay policy defaults, witness sizing,
//! and network-independent protocol limits.
//!
//! CONSENSUS: Supply, block, and witness constants here feed directly into block
//! and transaction validation. Changes require explicit protocol coordination.
use crate::network::Network;
use hex_literal::hex;

// Display precision only. Consensus and backend accounting use integer atoms.
pub const DECIMALS: usize = 12;
pub const ATOMS_PER_ATHO: u64 = 1_000_000_000_000;
pub const HALVING_INTERVAL_BLOCKS: u64 = 1_260_000;
pub const COINBASE_MATURITY_BLOCKS: u64 = 100;
pub const STANDARD_TX_CONFIRMATIONS: u64 = 6;
pub const MIN_RELAY_FEE_RATE_ATOMS_PER_VBYTE: u64 = 1;
pub const MIN_TX_FEE_PER_VBYTE_ATOMS: u64 = MIN_RELAY_FEE_RATE_ATOMS_PER_VBYTE;
pub const MIN_TX_FEE_ATOMS: u64 = 500;
pub const MIN_OUTPUT_AMOUNT_ATOMS: u64 = 1_000;
pub const DUST_RELAY_VALUE_ATOMS: u64 = MIN_OUTPUT_AMOUNT_ATOMS;
pub const MAX_STANDARD_INPUTS: usize = 1_024;
pub const MAX_STANDARD_OUTPUTS: usize = 64;
pub const TX_POW_MIN_BITS: u8 = 16;
pub const TX_POW_MAX_BITS: u8 = 28;
pub const TX_POW_NONCE_BYTES: usize = 8;
pub const TX_POW_BITS_BYTES: usize = 1;
pub const TX_POW_DOMAIN: &[u8] = b"ATHO_TX_POW_V1";
pub const TX_SIGN_DOMAIN: &[u8] = b"ATHO_TX_SIGN_V1";
pub const BLOCK_TIME_SECONDS: u64 = 100;
pub const TARGET_BLOCK_TIME_SECONDS: u64 = BLOCK_TIME_SECONDS;
pub const BLOCKS_PER_YEAR: u64 = 315_360;
pub const INITIAL_BLOCK_REWARD_ATOMS: u64 = 5_000_000_000_000;
pub const TAIL_REWARD_ATOMS: u64 = 625_000_000_000;
pub const GENESIS_COINBASE_ATOMS: u64 = INITIAL_BLOCK_REWARD_ATOMS;
pub const POW_RETARGET_INTERVAL_BLOCKS: u64 = 1;
pub const POW_AVERAGING_WINDOW_BLOCKS: u64 = 17;
pub const POW_MEDIAN_WINDOW_BLOCKS: u64 = 11;
pub const POW_DAMPING_FACTOR: u64 = 4;
pub const POW_MAX_ADJUST_UP_PERCENT: u64 = 16;
pub const POW_MAX_ADJUST_DOWN_PERCENT: u64 = 32;
pub const SHA3_384_HASH_BITS: usize = 384;
pub const SHA3_384_HASH_HEX_CHARS: usize = 96;
pub const WITNESS_SCALE_FACTOR: usize = 4;
// Falcon-512 is the active signature profile for Atho transaction witnesses.
pub const FALCON_512_LOGN: u32 = 9;
pub const FALCON_512_PUBLIC_KEY_BYTES: usize = 897;
pub const FALCON_512_SECRET_KEY_BYTES: usize = 1_281;
pub const FALCON_512_SIGNATURE_BYTES: usize = 666;
pub const MIN_TRANSACTION_FULL_BYTES_WITH_ONE_INPUT_AND_ONE_OUTPUT: usize = 88;
pub const TX_WITNESS_FIXED_BYTES: usize =
    4 + FALCON_512_SIGNATURE_BYTES + 4 + FALCON_512_PUBLIC_KEY_BYTES + 4;
pub const MAX_WITNESS_INPUT_REFS: usize = (MAX_TRANSACTION_RAW_BYTES
    - MIN_TRANSACTION_FULL_BYTES_WITH_ONE_INPUT_AND_ONE_OUTPUT
    - TX_WITNESS_FIXED_BYTES)
    / 18;
pub const MAX_BLOCK_VBYTES: usize = 3_000_000;
pub const MAX_BLOCK_RAW_BYTES: usize = 12_000_000;
pub const MAX_BLOCK_SIZE_BYTES: usize = MAX_BLOCK_VBYTES;
pub const MAX_BLOCK_WEIGHT: usize = MAX_BLOCK_VBYTES * WITNESS_SCALE_FACTOR;
pub const MAX_BLOCK_SERIALIZED_BYTES: usize = MAX_BLOCK_RAW_BYTES;
pub const MAX_TRANSACTION_RAW_BYTES: usize = 250_000;
pub const MAX_TRANSACTION_VBYTES: usize = 250_000;
pub const MAX_TRANSACTION_SIZE_BYTES: usize = MAX_TRANSACTION_VBYTES;
pub const ADDRESS_DIGEST_BYTES: usize = 32;
pub const ADDRESS_CHECKSUM_BYTES: usize = 4;
pub const ADDRESS_CHECKSUM_BASE56_CHARS: usize = 6;
pub const STANDARD_TRANSACTION_ALLOCATION_BPS: u16 = 9_500;
pub const BLOCK_FILE_ROTATE_BYTES: u64 = 128 * 1024 * 1024;
pub const BLOCK_FILE_RECORD_OVERHEAD_BYTES: u64 = 8;
pub const PRUNE_DEPTH_BLOCKS: u64 = 100_000;
pub const MAX_REORG_DEPTH_BLOCKS: u64 = PRUNE_DEPTH_BLOCKS;
pub const FINALIZATION_DEPTH_BLOCKS: u64 = MAX_REORG_DEPTH_BLOCKS;
pub const WITNESS_SIGNATURE_REFERENCE_BYTES: usize = 16;
pub const INPUT_REFERENCE_BYTES: usize = WITNESS_SIGNATURE_REFERENCE_BYTES;

pub const ADDRESS_ROLE_DOMAIN: &str = "ATHO_ADDR_V1";

pub const FOUNDERS_HASH_SHA3_384: [u8; 48] =
    hex!("3582d92eff98ba5e575ae721ced2b32bf7bea4628971d339dc189fde10716a40b91a67f8eb300894eaebb921754cdd86");
pub const FOUNDERS_HASH_SHA3_512: [u8; 64] =
    hex!("c93086ce58c4a64ef15672b00836eca94d14ea3810a584360fb11e11f065d5949aa1307f9315bce31316c820e6f58f0dace98f48111e6538a3d7a231e58e51bf");

pub const BASE56_ALPHABET: &str = "23456789ABCDEFGHJKMNPQRSTUVWXYZabcdefghjkmnpqrstuvwxyz";

pub const fn atoms_per_atho_for_network(_network: Network) -> u64 {
    ATOMS_PER_ATHO
}

pub const fn decimals_for_network(_network: Network) -> usize {
    DECIMALS
}
