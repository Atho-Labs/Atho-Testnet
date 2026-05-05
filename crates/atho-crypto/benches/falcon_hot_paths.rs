use atho_core::consensus::signatures::{transaction_signing_digest, AthoSignatureDomain};
use atho_core::transaction::{Transaction, TxInput, TxOutput};
use atho_crypto::falcon::{generate_from_seed, sign, verify};
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

fn bench_falcon_hot_paths(c: &mut Criterion) {
    let tx = sample_transaction();
    let digest = transaction_signing_digest(&tx);
    let keypair = generate_from_seed(b"atho-falcon-bench").expect("falcon keypair");
    let signature = sign(
        AthoSignatureDomain::Transaction,
        &keypair.secret_key,
        &digest,
    )
    .expect("falcon signature");

    c.bench_function("falcon_generate_from_seed", |b| {
        b.iter(|| black_box(generate_from_seed(black_box(b"atho-falcon-bench"))))
    });

    c.bench_function("falcon_sign_transaction", |b| {
        b.iter(|| {
            black_box(
                sign(
                    AthoSignatureDomain::Transaction,
                    black_box(&keypair.secret_key),
                    black_box(&digest),
                )
                .expect("falcon signature"),
            )
        })
    });

    c.bench_function("falcon_verify_transaction", |b| {
        b.iter(|| {
            black_box(
                verify(
                    AthoSignatureDomain::Transaction,
                    black_box(&keypair.public_key),
                    black_box(&digest),
                    black_box(&signature),
                )
                .expect("falcon verification"),
            )
        })
    });
}

criterion_group!(benches, bench_falcon_hot_paths);
criterion_main!(benches);
