use atho_core::block::{merkle_root, witness_root, Block, BlockHeader};
use atho_core::consensus::params::CONSENSUS_PARAMS;
use atho_core::consensus::pow;
use atho_core::consensus::signatures::{transaction_signing_digest, AthoSignatureDomain};
use atho_core::consensus::subsidy;
use atho_core::constants::MIN_TX_FEE_PER_VBYTE_ATOMS;
use atho_core::network::Network;
use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness, WitnessInputRef};
use atho_crypto::falcon::{generate_from_seed, sign, FalconKeypair};
use atho_node::validation::{
    derive_sig_ref_short, derive_witness_commit_ref, validate_block_without_pow,
    validate_transaction,
};

fn signing_keypair() -> FalconKeypair {
    generate_from_seed(b"atho-protocol-fixture").expect("deterministic falcon keypair")
}

fn witness_bytes(tx: &Transaction) -> Vec<u8> {
    let keypair = signing_keypair();
    let txid = tx.txid();
    let digest = transaction_signing_digest(&tx);
    let signature = sign(
        AthoSignatureDomain::Transaction,
        &keypair.secret_key,
        &digest,
    )
    .expect("deterministic falcon signature");
    let sig_bytes = signature.0.clone();
    let staged = TxWitness {
        signature: sig_bytes.clone(),
        pubkey: keypair.public_key.0.clone(),
        input_refs: (0..tx.inputs.len())
            .map(|index| WitnessInputRef {
                sig_ref_short: derive_sig_ref_short(&txid, &sig_bytes, index as u32),
                witness_commit_ref: [0; 16],
            })
            .collect(),
    };
    let staged_tx = Transaction {
        witness: staged.canonical_bytes(),
        ..tx.clone()
    };
    let witness_root = staged_tx.witness_commitment_hash();
    TxWitness {
        signature: sig_bytes.clone(),
        pubkey: keypair.public_key.0,
        input_refs: (0..tx.inputs.len())
            .map(|index| WitnessInputRef {
                sig_ref_short: derive_sig_ref_short(&txid, &sig_bytes, index as u32),
                witness_commit_ref: derive_witness_commit_ref(&txid, &witness_root, index as u32),
            })
            .collect(),
    }
    .canonical_bytes()
}

fn fixture_transaction() -> Transaction {
    let tx = Transaction {
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
        witness: vec![],
    };
    Transaction {
        witness: witness_bytes(&tx),
        ..tx
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
    assert_eq!(CONSENSUS_PARAMS.min_tx_fee_atoms, 1);
    let tx = fixture_transaction();
    let minimum_fee = tx.vsize_bytes() as u64 * MIN_TX_FEE_PER_VBYTE_ATOMS;
    assert_eq!(validate_transaction(&tx, minimum_fee), Ok(()));
    assert_eq!(
        validate_block_without_pow(&fixture_block(), 0, Network::Mainnet),
        Ok(())
    );
}
