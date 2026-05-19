// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Block template construction for solo mining.
//!
//! This module assembles candidate blocks from the current mempool and tip
//! state. It keeps the node authoritative: miners search nonce space, but the
//! node defines the canonical template contents and validates solved blocks.
use crate::dev;
use crate::error::NodeError;
use crate::node::Node;
use crate::validation::finalize_witness_commit_refs;
use atho_core::address::decode_base56_address;
use atho_core::block::{merkle_root, witness_root, Block, BlockHeader};
use atho_core::consensus::rules;
use atho_core::consensus::{pow, subsidy};
use atho_core::constants::{MAX_BLOCK_RAW_BYTES, MAX_BLOCK_VBYTES, MAX_BLOCK_WEIGHT};
use atho_core::transaction::{Transaction, TxOutput};
use atho_storage::validation::ValidationError;
use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

/// Builds a candidate block from the current tip and mempool contents.
pub(crate) fn build_candidate_block(node: &Node) -> Result<Block, NodeError> {
    let height = node.height().saturating_add(1);
    let (validated_entries, _, skipped_entries) =
        node.mempool
            .validated_entries_for_mining(node.network(), height, |txid, output_index| {
                node.utxo_entry(*txid, output_index)
            });
    if skipped_entries > 0 {
        let _ = dev::append_log(
            "miner",
            &format!(
                "candidate block skipped {} stale or invalid mempool entr{} at height={}",
                skipped_entries,
                if skipped_entries == 1 { "y" } else { "ies" },
                height
            ),
        );
    }
    let active_rules = rules::rules_at_height(height);
    let subsidy_atoms =
        subsidy::block_subsidy_atoms_for_network(node.network(), node.height().saturating_add(1));
    let (reward_address, reward_script) = configured_reward_target(node)?;
    let previous_block_hash = node.tip_hash();
    let timestamp = candidate_block_timestamp(node.blocks());
    let difficulty_target_or_bits = node.difficulty_target_for_next_block_at(timestamp);
    let header_template = BlockHeader {
        version: active_rules.block_version,
        network_id: node.network(),
        height,
        previous_block_hash,
        merkle_root: [0; 48],
        witness_root: [0; 48],
        founders_hash_sha3_384: BlockHeader::consensus_founders_hash_sha3_384(),
        founders_hash_sha3_512: BlockHeader::consensus_founders_hash_sha3_512(),
        timestamp,
        difficulty_target_or_bits,
        nonce: 0,
    };
    let coinbase_template = Transaction {
        version: active_rules.transaction_version,
        inputs: vec![],
        outputs: vec![TxOutput {
            value_atoms: subsidy_atoms,
            locking_script: reward_script.clone(),
        }],
        lock_time: u32::try_from(height).unwrap_or(u32::MAX),
        witness: vec![],
        tx_pow_nonce: 0,
        tx_pow_bits: 0,
    };
    let header_size_bytes = header_template.canonical_size_bytes();
    let coinbase_base_bytes = coinbase_template.base_size_bytes();
    let coinbase_full_bytes = coinbase_template.full_size_bytes();
    let mut block_base_bytes = header_size_bytes + 4 + 4 + coinbase_base_bytes;
    let mut block_full_bytes = header_size_bytes + 4 + 4 + coinbase_full_bytes;
    let mut selected_transactions = Vec::new();
    let mut selected_fee_atoms = 0u64;
    let mut block_spent_set = BTreeSet::new();

    for entry in validated_entries {
        let fee_atoms = entry.fee_atoms;
        let base_size_bytes = entry.base_size_bytes();
        let raw_size_bytes = entry.raw_size_bytes();
        let tx = entry.transaction;
        let mut inserted_inputs = Vec::with_capacity(tx.inputs.len());
        let mut conflicted = false;
        for input in &tx.inputs {
            let key = (input.previous_txid, input.output_index);
            if !block_spent_set.insert(key) {
                conflicted = true;
                break;
            }
            inserted_inputs.push(key);
        }
        if conflicted {
            for key in inserted_inputs {
                let _ = block_spent_set.remove(&key);
            }
            continue;
        }

        let next_base_bytes = block_base_bytes + 4 + base_size_bytes;
        let next_full_bytes = block_full_bytes + 4 + raw_size_bytes;
        let next_weight = next_base_bytes
            .saturating_mul(3)
            .saturating_add(next_full_bytes);
        let next_vbytes = (next_weight.saturating_add(3)) / 4;
        // PERFORMANCE: Skip oversize candidates incrementally so block assembly
        // does not have to rebuild the block from scratch when one transaction
        // would exceed raw-size, vsize, or weight limits.
        if next_full_bytes > MAX_BLOCK_RAW_BYTES
            || next_vbytes > MAX_BLOCK_VBYTES
            || next_weight > MAX_BLOCK_WEIGHT
        {
            for key in inserted_inputs {
                let _ = block_spent_set.remove(&key);
            }
            continue;
        }

        block_base_bytes = next_base_bytes;
        block_full_bytes = next_full_bytes;
        selected_fee_atoms = selected_fee_atoms
            .checked_add(fee_atoms)
            .ok_or(NodeError::Validation(ValidationError::FeeMismatch))?;
        selected_transactions.push(tx);
    }

    let coinbase = Transaction {
        version: active_rules.transaction_version,
        inputs: vec![],
        outputs: vec![TxOutput {
            value_atoms: subsidy_atoms.checked_add(selected_fee_atoms).ok_or(
                NodeError::Validation(ValidationError::CoinbaseRewardMismatch),
            )?,
            locking_script: reward_script,
        }],
        lock_time: u32::try_from(height).unwrap_or(u32::MAX),
        witness: vec![],
        tx_pow_nonce: 0,
        tx_pow_bits: 0,
    };
    let mut transactions = Vec::with_capacity(selected_transactions.len().saturating_add(1));
    transactions.push(coinbase);
    transactions.extend(selected_transactions);
    let witness_root = witness_root(&transactions);
    transactions = transactions
        .into_iter()
        .map(|tx| finalize_witness_commit_refs(&tx, witness_root))
        .collect();
    let header = BlockHeader {
        version: active_rules.block_version,
        network_id: node.network(),
        height,
        previous_block_hash,
        merkle_root: merkle_root(&transactions),
        witness_root,
        founders_hash_sha3_384: BlockHeader::consensus_founders_hash_sha3_384(),
        founders_hash_sha3_512: BlockHeader::consensus_founders_hash_sha3_512(),
        timestamp,
        difficulty_target_or_bits,
        nonce: 0,
    };
    let mut block = Block::new(header, transactions);
    block.fees_total_atoms = selected_fee_atoms;
    block.fees_miner_atoms = selected_fee_atoms;
    let _ = dev::append_log(
        "miner",
        &format!(
            "assembled candidate block prev={} reward={} txs={} network={}",
            hex::encode(previous_block_hash),
            reward_address,
            block.transactions.len(),
            node.config.network.id()
        ),
    );
    Ok(block)
}

/// Returns a timestamp that respects median-time-past constraints.
fn candidate_block_timestamp(previous_blocks: &[Block]) -> u64 {
    let now = current_unix_timestamp_seconds();
    pow::minimum_next_block_timestamp(previous_blocks).map_or(now, |minimum| now.max(minimum))
}

fn current_unix_timestamp_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn configured_reward_target(node: &Node) -> Result<(String, Vec<u8>), NodeError> {
    let configured = node.config.mining_reward_address.trim();
    if configured.is_empty() {
        return Err(NodeError::MiningRewardAddressRequired(format!(
            "configure ATHO_MINING_REWARD_ADDRESS or atho.conf miningrewardaddress for {}",
            node.network().id()
        )));
    }
    let (payment_digest, decoded_network) = decode_base56_address(configured).map_err(|_| {
        NodeError::MiningRewardAddressRequired(format!(
            "configured miningrewardaddress is not a valid Atho address for {}",
            node.network().id()
        ))
    })?;
    if decoded_network != node.network() {
        return Err(NodeError::MiningRewardAddressRequired(format!(
            "configured miningrewardaddress targets {} but node is on {}",
            decoded_network.id(),
            node.network().id()
        )));
    }
    Ok((configured.to_string(), payment_digest.to_vec()))
}
