// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Core Atho protocol types and consensus-facing helpers.
//!
//! This crate defines the canonical block, transaction, address, network, and
//! consensus primitives shared by every higher-level Atho component.
//!
//! CONSENSUS: Any serialization, hashing, or validation helper in this crate
//! must remain deterministic across all nodes for the same input data.
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
    use std::fs;
    use std::path::Path;

    #[test]
    fn carries_forward_protocol_basics() {
        assert_eq!(DECIMALS, 12);
        assert_eq!(ATOMS_PER_ATHO, 1_000_000_000_000);
        assert_eq!(INITIAL_BLOCK_REWARD_ATOMS, 6_250_000_000_000);
        assert_eq!(TAIL_REWARD_ATOMS, 781_250_000_000);
        assert_eq!(HALVING_INTERVAL_BLOCKS, 1_680_000);
        assert_eq!(COINBASE_MATURITY_BLOCKS, 150);
        assert_eq!(STANDARD_TX_CONFIRMATIONS, 7);
        assert_eq!(MIN_TX_FEE_ATOMS, 500);
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
        assert_eq!(GENESIS_COINBASE_ATOMS, 6_250_000_000_000);
    }

    #[test]
    fn backend_amount_accounting_stays_integer_only() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        let files = [
            "crates/atho-core/src/constants.rs",
            "crates/atho-core/src/transaction.rs",
            "crates/atho-core/src/genesis.rs",
            "crates/atho-core/src/consensus/subsidy.rs",
            "crates/atho-core/src/consensus/tx_policy.rs",
            "crates/atho-storage/src/validation.rs",
            "crates/atho-storage/src/chainstate.rs",
            "crates/atho-storage/src/utxo.rs",
            "crates/atho-storage/src/db.rs",
            "crates/atho-storage/src/block_files.rs",
            "crates/atho-wallet/src/wallet.rs",
            "crates/atho-node/src/node.rs",
            "crates/atho-node/src/mempool.rs",
            "crates/atho-node/src/mining.rs",
            "crates/atho-node/src/wallet_history.rs",
            "crates/atho-node/src/sync.rs",
            "crates/atho-rpc/src/request.rs",
            "crates/atho-rpc/src/response.rs",
        ];
        let forbidden = [
            "parse::<f32>",
            "parse::<f64>",
            " as f32",
            " as f64",
            "Decimal",
            ".round(",
            ".floor(",
            ".ceil(",
        ];

        for relative in files {
            let path = repo_root.join(relative);
            let contents = fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
            assert_no_forbidden_amount_math(&path, &contents, &forbidden);
        }
    }

    fn assert_no_forbidden_amount_math(path: &Path, contents: &str, forbidden: &[&str]) {
        for (index, line) in contents.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            for needle in forbidden {
                assert!(
                    !line.contains(needle),
                    "backend amount/accounting source {}:{} contains forbidden float-based pattern `{}`: {}",
                    path.display(),
                    index + 1,
                    needle,
                    line.trim()
                );
            }
        }
    }
}
