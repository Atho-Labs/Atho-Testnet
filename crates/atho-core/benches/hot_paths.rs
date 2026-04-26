use atho_core::block::{merkle_root, Block, BlockHeader};
use atho_core::consensus::pow;
use atho_core::consensus::subsidy;
use atho_core::transaction::{Transaction, TxInput, TxOutput};
use atho_core::network::Network;
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn sample_transaction() -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![TxInput {
            previous_txid: [1; 48],
            output_index: 0,
            unlocking_script: vec![1, 2, 3, 4],
        }],
        outputs: vec![TxOutput {
            value_atoms: 500,
            locking_script: vec![5, 6, 7, 8],
        }],
        lock_time: 0,
        witness: vec![],
    }
}

fn sample_block() -> Block {
    let coinbase = Transaction {
        version: 1,
        inputs: vec![],
        outputs: vec![TxOutput {
            value_atoms: subsidy::block_subsidy_atho(0),
            locking_script: vec![0],
        }],
        lock_time: 0,
        witness: vec![],
    };
    let tx = sample_transaction();
    let transactions = vec![coinbase, tx.clone(), tx];
    let merkle = merkle_root(&transactions);
    Block::new(
        BlockHeader {
            version: 1,
            previous_block_hash: [2; 48],
            merkle_root: merkle,
            timestamp: 75,
            target: pow::DIFFICULTY_PROFILE.min_difficulty_target,
            nonce: 42,
        },
        transactions,
    )
}

fn bench_core_hot_paths(c: &mut Criterion) {
    let tx = sample_transaction();
    let block = sample_block();

    c.bench_function("transaction_canonical_bytes", |b| {
        b.iter(|| black_box(tx.canonical_bytes()))
    });

    c.bench_function("transaction_txid", |b| {
        b.iter(|| black_box(tx.txid()))
    });

    c.bench_function("block_canonical_bytes", |b| {
        b.iter(|| black_box(block.canonical_bytes()))
    });

    c.bench_function("block_hash", |b| {
        b.iter(|| black_box(block.header.block_hash()))
    });
}

criterion_group!(benches, bench_core_hot_paths);
criterion_main!(benches);
