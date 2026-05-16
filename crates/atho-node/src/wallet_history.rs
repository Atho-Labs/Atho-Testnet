// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Wallet activity extraction from on-chain and mempool data.
use atho_core::block::Block;
use atho_rpc::request::WalletHistoryAddress;
use atho_rpc::response::{WalletActivityEntry, WalletActivityKind};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct OutpointKey {
    txid: [u8; 48],
    output_index: u32,
}

pub fn derive_wallet_activity(
    blocks: &[Block],
    addresses: &[WalletHistoryAddress],
) -> Vec<WalletActivityEntry> {
    // Wallet history is derived from canonical blocks instead of dev TSV exports so the UI and
    // RPC paths see the same on-chain truth that consensus accepted.
    if blocks.is_empty() || addresses.is_empty() {
        return Vec::new();
    }

    let address_map = addresses
        .iter()
        .map(|address| (address.payment_digest, address.address.clone()))
        .collect::<HashMap<_, _>>();
    let mut known_outputs: HashMap<OutpointKey, u64> = HashMap::new();
    let mut activities = Vec::new();

    for block in blocks {
        for (tx_index, tx) in block.transactions.iter().enumerate() {
            let txid = tx.txid();
            let mut wallet_input_total = 0u64;
            for input in &tx.inputs {
                if let Some(value) = known_outputs.get(&OutpointKey {
                    txid: input.previous_txid,
                    output_index: input.output_index,
                }) {
                    wallet_input_total = wallet_input_total.saturating_add(*value);
                }
            }

            let mut wallet_output_total = 0u64;
            let mut wallet_output_labels = Vec::new();
            let mut wallet_output_count = 0usize;
            for (output_index, output) in tx.outputs.iter().enumerate() {
                let digest: [u8; 32] = match output.locking_script.as_slice().try_into() {
                    Ok(digest) => digest,
                    Err(_) => continue,
                };
                if let Some(address) = address_map.get(&digest) {
                    wallet_output_total = wallet_output_total.saturating_add(output.value_atoms);
                    wallet_output_count = wallet_output_count.saturating_add(1);
                    wallet_output_labels.push(address.clone());
                    known_outputs.insert(
                        OutpointKey {
                            txid,
                            output_index: output_index as u32,
                        },
                        output.value_atoms,
                    );
                }
            }

            if tx.is_coinbase() && tx_index == 0 && wallet_output_total > 0 {
                activities.push(WalletActivityEntry {
                    height: block.header.height,
                    kind: WalletActivityKind::Mined,
                    label: format_wallet_label("coinbase reward", &wallet_output_labels),
                    amount_atoms: wallet_output_total as i128,
                    txid,
                });
                continue;
            }

            if wallet_input_total > 0 {
                let debit = wallet_input_total.saturating_sub(wallet_output_total);
                if debit == 0 {
                    continue;
                }
                let external_output_count = tx.outputs.len().saturating_sub(wallet_output_count);
                activities.push(WalletActivityEntry {
                    height: block.header.height,
                    kind: WalletActivityKind::Sent,
                    label: if external_output_count == 0 {
                        String::from("self transfer")
                    } else {
                        format!("{external_output_count} external output(s)")
                    },
                    amount_atoms: -(debit as i128),
                    txid,
                });
                continue;
            }

            if wallet_output_total > 0 {
                activities.push(WalletActivityEntry {
                    height: block.header.height,
                    kind: WalletActivityKind::Received,
                    label: format_wallet_label("incoming transfer", &wallet_output_labels),
                    amount_atoms: wallet_output_total as i128,
                    txid,
                });
            }
        }
    }

    activities.reverse();
    activities
}

fn format_wallet_label(prefix: &str, labels: &[String]) -> String {
    match labels {
        [] => prefix.to_string(),
        [label] => format!("{prefix} to {label}"),
        _ => format!("{prefix} to {} outputs", labels.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atho_core::block::{merkle_root, witness_root, Block, BlockHeader};
    use atho_core::network::Network;
    use atho_core::transaction::{Transaction, TxInput, TxOutput};

    fn block(height: u64, previous_block_hash: [u8; 48], transactions: Vec<Transaction>) -> Block {
        let header = BlockHeader {
            version: 1,
            network_id: Network::Regnet,
            height,
            previous_block_hash,
            merkle_root: merkle_root(&transactions),
            witness_root: witness_root(&transactions),
            timestamp: height,
            difficulty_target_or_bits: [0x0f; 48],
            nonce: height,
        };
        Block::new(header, transactions)
    }

    fn coinbase(locking_script: Vec<u8>, value_atoms: u64) -> Transaction {
        Transaction {
            version: 1,
            inputs: Vec::new(),
            outputs: vec![TxOutput {
                value_atoms,
                locking_script,
            }],
            lock_time: 0,
            witness: Vec::new(),
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        }
    }

    fn transfer(
        previous_txid: [u8; 48],
        previous_index: u32,
        input_script: Vec<u8>,
        outputs: Vec<TxOutput>,
    ) -> Transaction {
        Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid,
                output_index: previous_index,
                unlocking_script: input_script,
            }],
            outputs,
            lock_time: 0,
            witness: Vec::new(),
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        }
    }

    #[test]
    fn canonical_history_derives_mined_received_and_sent_rows() {
        let wallet_digest = [1u8; 32];
        let external_digest = [2u8; 32];
        let tracked = vec![WalletHistoryAddress {
            payment_digest: wallet_digest,
            address: String::from("R-test-wallet"),
        }];

        let mined = coinbase(wallet_digest.to_vec(), 500);
        let block_one = block(1, [0; 48], vec![mined.clone()]);

        let external_seed = coinbase(external_digest.to_vec(), 900);
        let funding = transfer(
            external_seed.txid(),
            0,
            external_digest.to_vec(),
            vec![TxOutput {
                value_atoms: 400,
                locking_script: wallet_digest.to_vec(),
            }],
        );
        let block_two = block(
            2,
            block_one.header.block_hash(),
            vec![external_seed, funding.clone()],
        );

        let spend = transfer(
            funding.txid(),
            0,
            wallet_digest.to_vec(),
            vec![TxOutput {
                value_atoms: 250,
                locking_script: external_digest.to_vec(),
            }],
        );
        let block_three = block(
            3,
            block_two.header.block_hash(),
            vec![coinbase(vec![9], 50), spend.clone()],
        );

        let activity = derive_wallet_activity(&[block_one, block_two, block_three], &tracked);
        assert_eq!(activity.len(), 3);
        assert_eq!(activity[0].kind, WalletActivityKind::Sent);
        assert_eq!(activity[0].amount_atoms, -400);
        assert_eq!(activity[0].txid, spend.txid());
        assert_eq!(activity[1].kind, WalletActivityKind::Received);
        assert_eq!(activity[1].amount_atoms, 400);
        assert_eq!(activity[1].txid, funding.txid());
        assert_eq!(activity[2].kind, WalletActivityKind::Mined);
        assert_eq!(activity[2].amount_atoms, 500);
    }
}
