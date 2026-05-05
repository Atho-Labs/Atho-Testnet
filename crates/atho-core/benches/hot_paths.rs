use atho_core::block::{merkle_root, witness_root, Block, BlockHeader};
use atho_core::consensus::pow;
use atho_core::consensus::subsidy;
use atho_core::consensus::tx_policy::{
    minimum_required_fee_atoms, solve_transaction_pow, transaction_pow_is_valid,
    transaction_pow_preimage,
};
use atho_core::network::Network;
use atho_core::transaction::{Transaction, TxInput, TxOutput};
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
        tx_pow_nonce: 0,
        tx_pow_bits: 0,
    }
}

fn sample_block() -> Block {
    let coinbase = Transaction {
        version: 1,
        inputs: vec![],
        outputs: vec![TxOutput {
            value_atoms: subsidy::block_subsidy_atoms(0),
            locking_script: vec![0],
        }],
        lock_time: 0,
        witness: vec![],
        tx_pow_nonce: 0,
        tx_pow_bits: 0,
    };
    let tx = sample_transaction();
    let transactions = vec![coinbase, tx.clone(), tx];
    let merkle = merkle_root(&transactions);
    Block::new(
        BlockHeader {
            version: 1,
            network_id: Network::Mainnet,
            height: 1,
            previous_block_hash: [2; 48],
            merkle_root: merkle,
            witness_root: witness_root(&transactions),
            timestamp: 75,
            difficulty_target_or_bits: pow::DIFFICULTY_PROFILE.min_difficulty_target,
            nonce: 42,
        },
        transactions,
    )
}

fn bench_core_hot_paths(c: &mut Criterion) {
    let tx = sample_transaction();
    let block = sample_block();
    let pow_preimage = transaction_pow_preimage(Network::Regnet, &tx);
    let mut pow_tx = tx.clone();
    let pow_fee = minimum_required_fee_atoms(Network::Regnet, &pow_tx);
    solve_transaction_pow(Network::Regnet, &mut pow_tx, pow_fee);

    c.bench_function("transaction_canonical_bytes", |b| {
        b.iter(|| black_box(tx.canonical_bytes()))
    });

    c.bench_function("transaction_txid", |b| b.iter(|| black_box(tx.txid())));

    c.bench_function("transaction_pow_preimage", |b| {
        b.iter(|| black_box(transaction_pow_preimage(Network::Regnet, &tx)))
    });

    c.bench_function("transaction_pow_verify_2_output_low_fee", |b| {
        b.iter(|| black_box(transaction_pow_is_valid(Network::Regnet, &pow_tx, pow_fee)))
    });

    c.bench_function("transaction_pow_solve_2_output_min_fee", |b| {
        b.iter(|| {
            let mut candidate = tx.clone();
            black_box(solve_transaction_pow(
                Network::Regnet,
                &mut candidate,
                pow_fee,
            ))
        })
    });

    c.bench_function("block_canonical_bytes", |b| {
        b.iter(|| black_box(block.canonical_bytes()))
    });

    c.bench_function("block_hash", |b| {
        b.iter(|| black_box(block.header.block_hash()))
    });

    c.bench_function("transaction_pow_hash_ready_nonce", |b| {
        b.iter(|| {
            black_box(atho_core::consensus::tx_policy::transaction_pow_hash(
                &pow_preimage,
                pow_tx.tx_pow_nonce,
            ))
        })
    });
}

criterion_group!(benches, bench_core_hot_paths);
criterion_main!(benches);
