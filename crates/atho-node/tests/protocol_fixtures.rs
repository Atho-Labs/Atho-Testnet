use atho_core::block::{merkle_root, Block, BlockHeader};
use atho_core::consensus::params::CONSENSUS_PARAMS;
use atho_core::consensus::subsidy;
use atho_core::consensus::pow;
use atho_core::network::Network;
use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness};
use atho_node::validation::{validate_block, validate_transaction};
use atho_crypto::falcon::{FALCON_512_PUBLIC_KEY_BYTES, FALCON_512_SIGNATURE_MIN_BYTES};

fn witness_bytes(inputs: usize) -> Vec<u8> {
    TxWitness {
        signature: vec![9; FALCON_512_SIGNATURE_MIN_BYTES],
        pubkey: vec![8; FALCON_512_PUBLIC_KEY_BYTES],
        input_refs: (0..inputs).map(|_| vec![7, 7]).collect(),
    }
    .canonical_bytes()
}

fn fixture_transaction() -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![TxInput {
            previous_txid: [1; 48],
            output_index: 0,
            unlocking_script: vec![1, 2, 3],
        }],
        outputs: vec![TxOutput {
            value_atoms: 1_000,
            locking_script: vec![4, 5],
        }],
        lock_time: 0,
        witness: witness_bytes(1),
    }
}

fn fixture_block() -> Block {
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
    let tx = fixture_transaction();
    let transactions = vec![coinbase, tx];
    let root = merkle_root(&transactions);
    Block::new(
        BlockHeader {
            version: 1,
            previous_block_hash: [2; 48],
            merkle_root: root,
            timestamp: 75,
            target: pow::DIFFICULTY_PROFILE.min_difficulty_target,
            nonce: 42,
        },
        transactions,
    )
}

#[test]
fn protocol_fixture_freezes_core_parameters_and_validation() {
    std::env::set_var("ATHO_SKIP_POW_VALIDATION", "1");
    std::env::set_var("ATHO_SKIP_FALCON_VALIDATION", "1");
    assert_eq!(CONSENSUS_PARAMS.max_supply_atho, 168_000_000);
    assert_eq!(CONSENSUS_PARAMS.halving_interval_blocks, 1_680_000);
    assert_eq!(CONSENSUS_PARAMS.min_tx_fee_atoms, 500);
    assert_eq!(validate_transaction(&fixture_transaction(), 500), Ok(()));
    assert_eq!(validate_block(&fixture_block(), 0, Network::Mainnet), Ok(()));
}
