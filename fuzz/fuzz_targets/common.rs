#![allow(dead_code)]

use atho_core::address::public_key_digest;
use atho_core::block::{merkle_root, witness_root, Block, BlockHeader};
use atho_core::consensus::pow;
use atho_core::consensus::signatures::{transaction_signing_digest, AthoSignatureDomain};
use atho_core::consensus::tx_policy::{minimum_required_fee_atoms, solve_transaction_pow};
use atho_core::crypto::hash::sha3_384;
use atho_core::network::Network;
use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness, WitnessInputRef};
use atho_crypto::falcon::{
    generate_from_seed, sign, FalconKeypair, FALCON_512_SIGNATURE_BYTES,
};
use atho_node::config::NodeConfig;
use atho_node::miner::Miner;
use atho_node::node::Node;
use atho_node::validation::{derive_sig_ref_short, finalize_witness_commit_refs};
use atho_p2p::protocol::{compact_block_from_block, CompactBlockMessage};
use atho_storage::utxo::UtxoEntry;
use std::sync::OnceLock;

pub const NETWORK: Network = Network::Regnet;
pub const SPEND_HEIGHT: u64 = 6;
pub const FIXTURE_TX_COUNT: usize = 2;
pub const FIXTURE_INPUT_COUNT: usize = 1;
pub const FIXTURE_INPUT_VALUE: u64 = 100_000;
pub const FIXTURE_OUTPUT_LOCKING_SCRIPT: [u8; 4] = [0x42, 0x43, 0x44, 0x45];
pub const FIXTURE_TIP_HASH: [u8; 48] = [0x5a; 48];

#[derive(Debug, Clone)]
pub struct ValidationFixture {
    pub network: Network,
    pub spend_height: u64,
    pub tip_hash: [u8; 48],
    pub utxos: Vec<UtxoEntry>,
    pub transactions: Vec<Transaction>,
    pub fees: Vec<u64>,
    pub block: Block,
    pub compact_block: CompactBlockMessage,
}

impl ValidationFixture {
    pub fn seed_node(&self) -> Node {
        let mut node = Node::new(NodeConfig::new(self.network));
        node.dev_seed_chainstate(self.spend_height, self.tip_hash, self.utxos.clone())
            .expect("seed node");
        node
    }

    pub fn mempool_node(&self) -> Node {
        let mut node = self.seed_node();
        for (tx, fee) in self.transactions.iter().cloned().zip(self.fees.iter().copied()) {
            node.admit_transaction(atho_node::mempool::MempoolEntry::new(tx, fee))
                .expect("seed mempool");
        }
        node
    }
}

fn make_keypair(tag: &[u8]) -> FalconKeypair {
    generate_from_seed(tag).expect("falcon keypair")
}

fn make_utxo(network: Network, keypair: &FalconKeypair, index: usize) -> UtxoEntry {
    let mut preimage = Vec::with_capacity(network.id().len() + 16);
    preimage.extend_from_slice(network.id().as_bytes());
    preimage.extend_from_slice(b":fuzz:utxo:");
    preimage.extend_from_slice(&(index as u64).to_le_bytes());
    let txid = sha3_384(&preimage);
    UtxoEntry::new(
        network,
        txid,
        0,
        FIXTURE_INPUT_VALUE,
        public_key_digest(network, &keypair.public_key.0).to_vec(),
        0,
        false,
    )
}

fn provisional_witness(input_count: usize, keypair: &FalconKeypair) -> TxWitness {
    TxWitness {
        signature: vec![0; FALCON_512_SIGNATURE_BYTES],
        pubkey: keypair.public_key.0.clone(),
        input_refs: (0..input_count)
            .map(|index| WitnessInputRef {
                input_index: index as u32,
                sig_ref_short: [0; 2],
                witness_commit_ref: [0; 16],
            })
            .collect(),
        additional_signers: vec![],
    }
}

fn build_spend_transaction(
    network: Network,
    utxos: &[UtxoEntry],
    keypair: &FalconKeypair,
    output_locking_script: Vec<u8>,
) -> (Transaction, u64) {
    let input_total = utxos.iter().map(|utxo| utxo.value_atoms).sum::<u64>();
    let inputs = utxos
        .iter()
        .map(|utxo| TxInput {
            previous_txid: utxo.txid,
            output_index: utxo.output_index,
            unlocking_script: utxo.locking_script.clone(),
        })
        .collect::<Vec<_>>();

    let mut tx = Transaction {
        version: 1,
        inputs,
        outputs: vec![TxOutput {
            value_atoms: input_total,
            locking_script: output_locking_script,
        }],
        lock_time: 0,
        witness: provisional_witness(utxos.len(), keypair).canonical_bytes(),
        tx_pow_nonce: 0,
        tx_pow_bits: 0,
    };
    let fee_atoms = minimum_required_fee_atoms(network, &tx);
    tx.outputs[0].value_atoms = input_total
        .checked_sub(fee_atoms)
        .expect("fixture input covers fee");
    tx.witness.clear();

    let digest = transaction_signing_digest(&tx);
    let signature = sign(AthoSignatureDomain::Transaction, &keypair.secret_key, &digest)
        .expect("fixture signature")
        .0;
    let txid = tx.txid();
    tx.witness = TxWitness {
        signature: signature.clone(),
        pubkey: keypair.public_key.0.clone(),
        input_refs: (0..utxos.len())
            .map(|index| WitnessInputRef {
                input_index: index as u32,
                sig_ref_short: derive_sig_ref_short(&txid, &signature, index as u32),
                witness_commit_ref: [0; 16],
            })
            .collect(),
        additional_signers: vec![],
    }
    .canonical_bytes();

    solve_transaction_pow(network, &mut tx, fee_atoms);
    (tx, fee_atoms)
}

fn build_coinbase(network: Network, height: u64, reward_atoms: u64) -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![],
        outputs: vec![TxOutput {
            value_atoms: reward_atoms,
            locking_script: public_key_digest(network, &make_keypair(b"atho-fuzz-coinbase").public_key.0)
                .to_vec(),
        }],
        lock_time: height as u32,
        witness: vec![],
        tx_pow_nonce: 0,
        tx_pow_bits: 0,
    }
}

fn solve_block(block: Block) -> Block {
    Miner::new(std::thread::available_parallelism().map(|p| p.get()).unwrap_or(1) as u32)
        .solve_block(block)
}

fn build_fixture_inner() -> ValidationFixture {
    let network = NETWORK;
    let keypair = make_keypair(b"atho-fuzz-validation-keypair");
    let output_script = public_key_digest(network, &keypair.public_key.0).to_vec();
    let utxos = (0..(FIXTURE_TX_COUNT * FIXTURE_INPUT_COUNT))
        .map(|index| make_utxo(network, &keypair, index))
        .collect::<Vec<_>>();
    let mut node = Node::new(NodeConfig::new(network));
    node.dev_seed_chainstate(SPEND_HEIGHT, FIXTURE_TIP_HASH, utxos.clone())
        .expect("seed chainstate");

    let mut transactions = Vec::with_capacity(FIXTURE_TX_COUNT);
    let mut fees = Vec::with_capacity(FIXTURE_TX_COUNT);
    for chunk in utxos.chunks(FIXTURE_INPUT_COUNT) {
        let (tx, fee_atoms) = build_spend_transaction(network, chunk, &keypair, output_script.clone());
        node.admit_transaction(atho_node::mempool::MempoolEntry::new(tx.clone(), fee_atoms))
            .expect("seed mempool transaction");
        transactions.push(tx);
        fees.push(fee_atoms);
    }

    let candidate = node.build_candidate_block().expect("build candidate block");
    let solved = solve_block(candidate);
    let compact_block = compact_block_from_block(&solved);

    ValidationFixture {
        network,
        spend_height: SPEND_HEIGHT,
        tip_hash: FIXTURE_TIP_HASH,
        utxos,
        transactions,
        fees,
        block: solved,
        compact_block,
    }
}

pub fn validation_fixture() -> &'static ValidationFixture {
    static FIXTURE: OnceLock<ValidationFixture> = OnceLock::new();
    FIXTURE.get_or_init(build_fixture_inner)
}

pub fn fixture_node() -> Node {
    validation_fixture().seed_node()
}

pub fn fixture_mempool_node() -> Node {
    validation_fixture().mempool_node()
}

pub fn fixture_block() -> Block {
    validation_fixture().block.clone()
}

pub fn fixture_compact_block() -> CompactBlockMessage {
    validation_fixture().compact_block.clone()
}

pub fn fixture_transactions() -> Vec<Transaction> {
    validation_fixture().transactions.clone()
}

pub fn fixture_utxos() -> Vec<UtxoEntry> {
    validation_fixture().utxos.clone()
}

pub fn fixture_fees() -> Vec<u64> {
    validation_fixture().fees.clone()
}

pub fn build_finalized_block_from_transactions(transactions: &[Transaction]) -> Block {
    let height = SPEND_HEIGHT.saturating_add(1);
    let subsidy = atho_core::consensus::subsidy::block_subsidy_atoms(height);
    let fees_total = transactions
        .iter()
        .enumerate()
        .map(|(index, _)| validation_fixture().fees.get(index).copied().unwrap_or(0))
        .sum::<u64>();
    let coinbase = build_coinbase(NETWORK, height, subsidy.saturating_add(fees_total));
    let staged_transactions = std::iter::once(coinbase.clone())
        .chain(transactions.iter().cloned())
        .collect::<Vec<_>>();
    let witness_root = witness_root(&staged_transactions);
    let transactions = staged_transactions
        .into_iter()
        .map(|tx| finalize_witness_commit_refs(&tx, witness_root))
        .collect::<Vec<_>>();
    let header = BlockHeader {
        version: 1,
        network_id: NETWORK,
        height,
        previous_block_hash: FIXTURE_TIP_HASH,
        merkle_root: merkle_root(&transactions),
        witness_root,
        timestamp: atho_core::genesis::genesis_state(NETWORK)
            .block
            .header
            .timestamp
            .saturating_add(1),
        difficulty_target_or_bits: pow::initial_target_for_network(NETWORK),
        nonce: 0,
    };
    let mut block = Block::new(header, transactions);
    block.fees_total_atoms = fees_total;
    block.fees_miner_atoms = fees_total;
    block
}
