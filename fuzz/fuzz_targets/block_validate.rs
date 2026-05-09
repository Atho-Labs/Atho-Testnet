#![no_main]

mod common;

use atho_node::validation::validate_block_with_context;
use libfuzzer_sys::fuzz_target;
use atho_storage::utxo::UtxoSet;

fuzz_target!(|data: &[u8]| {
    let fixture = common::validation_fixture();
    let mut block = fixture.block.clone();
    let selector = data.first().copied().unwrap_or(0) % 6;

    match selector {
        0 => {}
        1 => {
            block.header.merkle_root[0] ^= 1;
        }
        2 => {
            block.header.witness_root[0] ^= 1;
        }
        3 => {
            if let Some(tx) = block.transactions.get_mut(1) {
                tx.outputs[0].value_atoms = tx.outputs[0].value_atoms.wrapping_add(1);
            }
        }
        4 => {
            if block.transactions.len() > 2 {
                block.transactions[2] = block.transactions[1].clone();
                block.header.merkle_root = atho_core::block::merkle_root(&block.transactions);
                block.header.witness_root = atho_core::block::witness_root(&block.transactions);
            }
        }
        _ => {
            block.header.previous_block_hash[0] ^= 1;
        }
    }

    let mut utxos = UtxoSet::new(fixture.network);
    for utxo in fixture.utxos.iter().cloned() {
        utxos.insert(utxo).expect("fixture utxo insert");
    }
    let _ = validate_block_with_context(
        &block,
        block.header.height,
        fixture.network,
        fixture.tip_hash,
        atho_core::consensus::pow::initial_target_for_network(fixture.network),
        &[],
        utxos,
    );
});
