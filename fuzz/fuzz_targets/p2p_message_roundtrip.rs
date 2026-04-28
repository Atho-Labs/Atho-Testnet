#![no_main]

use atho_core::network::Network;
use atho_p2p::codec::WireCodec;
use atho_p2p::protocol::{
    Hash48, InventoryKind, InventoryVector, MessageCommand, MessagePayload, NetworkMessage,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() < 13 {
        return;
    }

    let mut command_bytes = [0u8; 12];
    command_bytes.copy_from_slice(&data[..12]);
    let Ok(command) = MessageCommand::from_bytes(command_bytes) else {
        return;
    };

    let message = match command {
        MessageCommand::Ping => {
            let mut nonce = [0u8; 8];
            let nonce_bytes = &data[12..data.len().min(20)];
            nonce[..nonce_bytes.len()].copy_from_slice(nonce_bytes);
            NetworkMessage::new(
                Network::Regnet,
                MessagePayload::Ping {
                    nonce: u64::from_le_bytes(nonce),
                },
            )
        }
        MessageCommand::Pong => {
            let mut nonce = [0u8; 8];
            let nonce_bytes = &data[12..data.len().min(20)];
            nonce[..nonce_bytes.len()].copy_from_slice(nonce_bytes);
            NetworkMessage::new(
                Network::Regnet,
                MessagePayload::Pong {
                    nonce: u64::from_le_bytes(nonce),
                },
            )
        }
        MessageCommand::Verack => NetworkMessage::new(Network::Regnet, MessagePayload::Verack),
        MessageCommand::GetAddr => {
            NetworkMessage::new(Network::Regnet, MessagePayload::GetAddr)
        }
        _ => NetworkMessage::new(
            Network::Regnet,
            MessagePayload::Inv {
                inventory: vec![InventoryVector {
                    kind: InventoryKind::Block,
                    hash: Hash48::from([data[12]; 48]),
                }],
            },
        ),
    };

    let encoded = WireCodec::encode(&message).expect("encode fuzz message");
    let decoded = WireCodec::decode(&encoded).expect("decode encoded fuzz message");
    let reencoded = WireCodec::encode(&decoded).expect("reencode decoded fuzz message");
    assert_eq!(encoded, reencoded);
});
