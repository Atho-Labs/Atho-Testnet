#![no_main]
// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors


mod common;

use atho_node::validation::finalize_witness_commit_refs;
use atho_p2p::protocol::{reconstruct_compact_block, CompactBlockReconstruction};
use libfuzzer_sys::fuzz_target;
use std::collections::BTreeMap;

fuzz_target!(|data: &[u8]| {
    let fixture = common::validation_fixture();
    let mut message = fixture.compact_block.clone();
    let selector = data.first().copied().unwrap_or(0) % 7;

    match selector {
        0 => {}
        1 => {
            if let Some(short_id) = message.short_ids.first_mut() {
                *short_id ^= 1;
            }
        }
        2 => {
            message.tx_count = message.tx_count.saturating_add(1);
        }
        3 => {
            if let Some(prefilled) = message.prefilled_transactions.first_mut() {
                prefilled.index = prefilled.index.saturating_add(1);
            }
        }
        4 => {
            message.short_ids.push(0);
        }
        5 => {
            if message.prefilled_transactions.len() > 1 {
                message.prefilled_transactions[0].index = message.prefilled_transactions[1].index;
            }
        }
        _ => {
            message.fees_total_atoms = message.fees_total_atoms.wrapping_add(1);
        }
    }

    let mempool_by_short_id = fixture
        .transactions
        .iter()
        .cloned()
        .map(|tx| (atho_p2p::protocol::compact_short_id(tx.txid()), tx))
        .collect::<BTreeMap<_, _>>();
    let result = reconstruct_compact_block(
        &message,
        |short_id| mempool_by_short_id.get(&short_id).cloned(),
        &BTreeMap::new(),
    );

    if let Ok(CompactBlockReconstruction::Complete(block)) = result {
        let witness_root = block.header.witness_root;
        let finalized_transactions = block
            .transactions
            .iter()
            .map(|tx| finalize_witness_commit_refs(tx, witness_root))
            .collect::<Vec<_>>();
        let mut reconstructed_block = *block;
        reconstructed_block.transactions = finalized_transactions;
        let _ = reconstructed_block.merkle_root();
        let _ = reconstructed_block.compute_witness_root();
    }
});
