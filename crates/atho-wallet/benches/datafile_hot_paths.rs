// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

use atho_core::network::Network;
use atho_wallet::mnemonic::{MnemonicLength, MnemonicPhrase};
use atho_wallet::wallet::{datafile, Wallet};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::env;
use std::fs;

fn sample_wallet() -> Wallet {
    let mnemonic = MnemonicPhrase::from_entropy(&[0u8; 32], MnemonicLength::Words24).unwrap();
    let mut wallet = Wallet::from_mnemonic(mnemonic, "", Network::Mainnet);
    wallet.checkout_receive_address();
    wallet.checkout_change_address();
    wallet
}

fn bench_wallet_datafile(c: &mut Criterion) {
    let wallet = sample_wallet();
    let dir = env::temp_dir();
    let path = dir.join("atho-wallet-bench.datafile");

    c.bench_function("wallet_datafile_save", |b| {
        b.iter(|| {
            datafile::WalletDataFile::save(
                black_box(&wallet),
                black_box("password"),
                black_box(&path),
            )
            .unwrap();
        })
    });

    datafile::WalletDataFile::save(&wallet, "password", &path).unwrap();

    c.bench_function("wallet_datafile_load", |b| {
        b.iter(|| {
            let loaded =
                datafile::WalletDataFile::load(black_box(&path), black_box("password")).unwrap();
            black_box(loaded);
        })
    });

    let _ = fs::remove_file(&path);
}

criterion_group!(benches, bench_wallet_datafile);
criterion_main!(benches);
