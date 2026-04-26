use atho_core::block::{merkle_root, witness_root, Block, BlockHeader};
use atho_core::consensus::params::CONSENSUS_PARAMS;
use atho_core::consensus::pow;
use atho_core::consensus::subsidy;
use atho_core::network::Network;
use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness};
use atho_crypto::falcon::{generate_from_seed, sign, FalconKeypair};
use atho_node::validation::{
    encode_input_reference, validate_block_without_pow, validate_transaction,
};

fn signing_keypair() -> FalconKeypair {
    generate_from_seed(b"atho-protocol-fixture").expect("deterministic falcon keypair")
}

fn witness_bytes(previous_txid: [u8; 48], output_index: u32) -> Vec<u8> {
    let keypair = signing_keypair();
    let tx = Transaction {
        version: 1,
        inputs: vec![TxInput {
            previous_txid,
            output_index,
            unlocking_script: vec![1, 2, 3],
        }],
        outputs: vec![TxOutput {
            value_atoms: 1_000,
            locking_script: vec![4, 5],
        }],
        lock_time: 0,
        witness: vec![],
    };
    let digest = tx.signing_digest();
    let signature = sign(&keypair.secret_key, &digest).expect("deterministic falcon signature");
    TxWitness {
        signature: signature.0,
        pubkey: keypair.public_key.0,
        input_refs: vec![encode_input_reference(&previous_txid, output_index)],
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
        witness: witness_bytes([1; 48], 0),
    }
}

fn fixture_block() -> Block {
    let coinbase = Transaction {
        version: 1,
        inputs: vec![],
        outputs: vec![TxOutput {
            value_atoms: subsidy::block_subsidy_atoms(0),
            locking_script: vec![0],
        }],
        lock_time: 0,
        witness: vec![],
    };
    let tx = fixture_transaction();
    let transactions = vec![coinbase, tx];
    let root = merkle_root(&transactions);
    let header = BlockHeader {
        version: 1,
        network_id: Network::Mainnet,
        height: 0,
        previous_block_hash: [2; 48],
        merkle_root: root,
        witness_root: witness_root(&transactions),
        timestamp: 75,
        difficulty_target_or_bits: pow::DIFFICULTY_PROFILE.min_difficulty_target,
        nonce: 0,
    };
    Block::new(header, transactions)
}

#[test]
fn protocol_fixture_freezes_core_parameters_and_validation() {
    assert_eq!(CONSENSUS_PARAMS.max_supply_atho, 168_000_000);
    assert_eq!(CONSENSUS_PARAMS.halving_interval_blocks, 1_680_000);
    assert_eq!(CONSENSUS_PARAMS.min_tx_fee_atoms, 500);
    assert_eq!(validate_transaction(&fixture_transaction(), 500), Ok(()));
    assert_eq!(
        validate_block_without_pow(&fixture_block(), 0, Network::Mainnet),
        Ok(())
    );
}
