// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

use atho_core::consensus::rules;
use atho_core::genesis;
use atho_core::network::Network;
use atho_p2p::codec::WireCodec;
use atho_p2p::config::MIN_SUPPORTED_PROTOCOL_VERSION;
use atho_p2p::downloader::BlockDownloadScheduler;
use atho_p2p::protocol::{
    Hash48, MessagePayload, NetworkMessage, VersionMessage, LOCAL_NODE_SERVICES,
};
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn version_message() -> NetworkMessage {
    NetworkMessage::new(
        Network::Regnet,
        MessagePayload::Version(VersionMessage {
            protocol_version: rules::PROTOCOL_VERSION,
            min_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
            services: LOCAL_NODE_SERVICES,
            timestamp_unix: 1_700_000_000,
            network: Network::Regnet,
            user_agent: String::from("/Atho:0.1.0/"),
            best_height: 512,
            ruleset_version: rules::RULESET_VERSION_V1,
            relay: true,
            genesis_hash: Hash48::from(genesis::genesis_hash(Network::Regnet)),
            tip_hash: Hash48::from([9; 48]),
            chainwork: Hash48::from([7; 48]),
        }),
    )
}

fn bench_p2p_hot_paths(c: &mut Criterion) {
    let message = version_message();
    let encoded = WireCodec::encode(&message).expect("encode version");

    c.bench_function("p2p_wire_encode_version", |b| {
        b.iter(|| black_box(WireCodec::encode(black_box(&message)).expect("encode version")))
    });

    c.bench_function("p2p_wire_decode_version", |b| {
        b.iter(|| black_box(WireCodec::decode(black_box(&encoded)).expect("decode version")))
    });

    let mut scheduler = BlockDownloadScheduler::default();
    scheduler.note_peer_ready("left");
    scheduler.note_peer_ready("right");
    scheduler.note_headers("left", (0..128).map(|value| [value as u8; 48]));
    scheduler.note_headers("right", (0..128).map(|value| [value as u8; 48]));

    c.bench_function("p2p_downloader_assignments_128", |b| {
        b.iter(|| {
            let mut local = scheduler.clone();
            black_box(local.assignments(black_box(64), black_box(16)))
        })
    });
}

criterion_group!(benches, bench_p2p_hot_paths);
criterion_main!(benches);
