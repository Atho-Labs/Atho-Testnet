// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

use atho_core::address::{address_parts_from_public_key, decode_base56_address};
use atho_core::block::{merkle_root, witness_root, Block, BlockHeader};
use atho_core::consensus::signatures::transaction_signing_digest;
use atho_core::consensus::subsidy;
use atho_core::consensus::tx_policy::solve_transaction_pow;
use atho_core::consensus::{pow, rules};
use atho_core::constants::{
    ADDRESS_DIGEST_BYTES, COINBASE_MATURITY_BLOCKS, MAX_TRANSACTION_SIZE_BYTES,
    MIN_TX_FEE_PER_VBYTE_ATOMS, STANDARD_TX_CONFIRMATIONS,
};
use atho_core::genesis;
use atho_core::network::Network;
use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness, WitnessInputRef};
use atho_crypto::falcon::{generate_from_seed, sign, FalconKeypair};
use atho_node::error::NodeError;
use atho_node::mempool::{Mempool, MempoolEntry};
use atho_node::miner::Miner;
use atho_node::node::Node;
use atho_node::validation::{
    derive_sig_ref_short, finalize_witness_commit_refs, validate_block_with_context,
    validate_block_without_pow, validate_transaction, validate_transaction_with_context,
    ValidationError,
};
use atho_p2p::config::{network_params as p2p_network_params, MIN_SUPPORTED_PROTOCOL_VERSION};
use atho_p2p::protocol::{
    validate_version_message, CompactBlockMessage, GetHeadersMessage, Hash48 as P2pHash48,
    MessageCommand, MessagePayload, NetworkMessage, PeerAddress as P2pPeerAddress, ProtocolError,
    VersionMessage as P2pVersionMessage, LOCAL_NODE_SERVICES,
};
use atho_storage::chainstate::Chainstate as StorageChainstate;
use atho_storage::db::{ChainstateSnapshot, Database};
use atho_storage::error::StorageError;
use atho_storage::path::ATHO_DATA_DIR_ENV;
use atho_storage::utxo::{UtxoEntry, UtxoSet};
use bincode::Options;
use std::ffi::OsString;
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_CASES: usize = 52_000;
const BASE_TX_VALUE: u64 = 100_000;
const BASE_TX_OUTPUT: u64 = 90_000;
const BASE_TX_FEE: u64 = BASE_TX_VALUE - BASE_TX_OUTPUT;
const BASE_TXID: [u8; 48] = [7; 48];
const BASE_UNLOCKING_SCRIPT: [u8; 4] = [1, 2, 3, 4];
const BASE_LOCKING_SCRIPT: [u8; 4] = [4, 3, 2, 1];
const BASE_REWARD_SCRIPT: [u8; 4] = [9, 9, 9, 9];

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    std::panic::set_hook(Box::new(|_| {}));
    let args: Vec<String> = std::env::args().collect();
    let cases = parse_flag(&args, "--cases").unwrap_or(DEFAULT_CASES);
    let seed = parse_flag_u64(&args, "--seed").unwrap_or(0x5a17_8d3c_2e91_44b7);
    let heavy_cases = cases / 10;
    let light_cases = 100usize;

    let mut totals = CampaignTotals::default();
    let reports = vec![
        tx_structure_attack(heavy_cases, seed ^ 0x01)?,
        serialization_attack(heavy_cases, seed ^ 0x02)?,
        signature_attack(heavy_cases, seed ^ 0x03)?,
        utxo_attack(heavy_cases, seed ^ 0x04)?,
        fee_attack(heavy_cases, seed ^ 0x05)?,
        coinbase_attack(heavy_cases, seed ^ 0x06)?,
        block_template_attack(heavy_cases, seed ^ 0x07)?,
        header_pow_attack(heavy_cases, seed ^ 0x08)?,
        chain_acceptance_attack(light_cases, seed ^ 0x09)?,
        confirmation_attack(heavy_cases, seed ^ 0x0a)?,
        genesis_attack(light_cases, seed ^ 0x0b)?,
        persistence_attack(light_cases, seed ^ 0x0c)?,
        determinism_attack(heavy_cases, seed ^ 0x0d)?,
        network_protocol_attack(light_cases, seed ^ 0x0e)?,
    ];

    for report in &reports {
        totals.cases += report.cases;
        totals.passed += report.passed;
        totals.unexpected_accept += report.unexpected_accept;
        totals.unexpected_reject += report.unexpected_reject;
        totals.panics += report.panics;
        totals.mismatches += report.mismatches;
        totals.silent_accepts += report.silent_accepts;
        totals.silent_state_divergence += report.silent_state_divergence;
    }

    println!("campaign_seed={seed:#x}");
    println!("campaign_cases={}", totals.cases);
    println!("campaign_passed={}", totals.passed);
    println!("campaign_unexpected_accept={}", totals.unexpected_accept);
    println!("campaign_unexpected_reject={}", totals.unexpected_reject);
    println!("campaign_panics={}", totals.panics);
    println!("campaign_mismatches={}", totals.mismatches);
    println!("campaign_silent_accepts={}", totals.silent_accepts);
    println!(
        "campaign_silent_state_divergence={}",
        totals.silent_state_divergence
    );

    for report in reports {
        print_report(&report);
    }

    Ok(())
}

fn parse_flag(args: &[String], flag: &str) -> Option<usize> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .and_then(|pair| pair[1].parse().ok())
}

fn parse_flag_u64(args: &[String], flag: &str) -> Option<u64> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .and_then(|pair| pair[1].parse().ok())
}

#[derive(Default)]
struct CampaignTotals {
    cases: u64,
    passed: u64,
    unexpected_accept: u64,
    unexpected_reject: u64,
    panics: u64,
    mismatches: u64,
    silent_accepts: u64,
    silent_state_divergence: u64,
}

struct CategoryReport {
    name: &'static str,
    cases: u64,
    passed: u64,
    unexpected_accept: u64,
    unexpected_reject: u64,
    panics: u64,
    mismatches: u64,
    silent_accepts: u64,
    silent_state_divergence: u64,
    examples: Vec<String>,
}

impl CategoryReport {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            cases: 0,
            passed: 0,
            unexpected_accept: 0,
            unexpected_reject: 0,
            panics: 0,
            mismatches: 0,
            silent_accepts: 0,
            silent_state_divergence: 0,
            examples: Vec::new(),
        }
    }

    fn note(&mut self, text: impl Into<String>) {
        if self.examples.len() < 12 {
            self.examples.push(text.into());
        }
    }

    fn fail_accept(&mut self, detail: impl Into<String>) {
        self.unexpected_accept += 1;
        self.note(detail);
    }

    fn fail_reject(&mut self, detail: impl Into<String>) {
        self.unexpected_reject += 1;
        self.note(detail);
    }

    fn fail_panic(&mut self, detail: impl Into<String>) {
        self.panics += 1;
        self.note(detail);
    }

    fn pass(&mut self) {
        self.passed += 1;
    }
}

fn print_report(report: &CategoryReport) {
    println!(
        "category={} cases={} passed={} unexpected_accept={} unexpected_reject={} panics={} mismatches={} silent_accepts={} silent_state_divergence={}",
        report.name,
        report.cases,
        report.passed,
        report.unexpected_accept,
        report.unexpected_reject,
        report.panics,
        report.mismatches,
        report.silent_accepts,
        report.silent_state_divergence
    );
    for example in &report.examples {
        println!("  example={example}");
    }
}

#[derive(Clone)]
struct SplitMix64(u64);

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }

    fn next_usize(&mut self, upper: usize) -> usize {
        if upper == 0 {
            return 0;
        }
        (self.next_u64() as usize) % upper
    }
}

fn hash_seed(seed: u64, tag: u64) -> u64 {
    let mut prng = SplitMix64::new(seed ^ tag);
    prng.next_u64()
}

fn with_cwd<T>(path: &PathBuf, f: impl FnOnce() -> T) -> Result<T, String> {
    let previous = std::env::current_dir().map_err(|err| err.to_string())?;
    std::env::set_current_dir(path).map_err(|err| err.to_string())?;
    let result = f();
    std::env::set_current_dir(previous).map_err(|err| err.to_string())?;
    Ok(result)
}

fn persistence_data_dir(root: &Path) -> PathBuf {
    root.join("dev")
}

fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|err| err.into_inner())
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
    _lock: MutexGuard<'static, ()>,
}

impl EnvVarGuard {
    fn set_path(key: &'static str, value: &std::path::Path) -> Self {
        let lock = env_lock();
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self {
            key,
            previous,
            _lock: lock,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

fn signed_base_tx(keypair: &FalconKeypair, input_txid: [u8; 48], output_value: u64) -> Transaction {
    let mut tx = Transaction {
        version: 1,
        inputs: vec![TxInput {
            previous_txid: input_txid,
            output_index: 0,
            unlocking_script: BASE_UNLOCKING_SCRIPT.to_vec(),
        }],
        outputs: vec![TxOutput {
            value_atoms: output_value,
            locking_script: BASE_LOCKING_SCRIPT.to_vec(),
        }],
        lock_time: 0,
        witness: Vec::new(),
        tx_pow_nonce: 0,
        tx_pow_bits: 0,
    };
    let signature = sign(
        atho_core::consensus::signatures::AthoSignatureDomain::Transaction,
        &keypair.secret_key,
        &transaction_signing_digest(Network::Mainnet, &tx),
    )
    .expect("falcon signature");
    let sig_bytes = signature.0.clone();
    let witness = TxWitness {
        signature: sig_bytes.clone(),
        pubkey: keypair.public_key.0.clone(),
        input_refs: vec![WitnessInputRef {
            input_index: 0,
            sig_ref_short: derive_sig_ref_short(&tx.txid(), &sig_bytes, 0),
            witness_commit_ref: [0; 16],
        }],
        additional_signers: vec![],
    };
    tx.witness = witness.canonical_bytes();
    tx
}

fn finalize_tx_for_block(tx: &Transaction, witness_root: [u8; 48]) -> Transaction {
    finalize_witness_commit_refs(tx, witness_root)
}

fn valid_utxo(network: Network, created_height: u64, value_atoms: u64) -> UtxoEntry {
    UtxoEntry::new(
        network,
        BASE_TXID,
        0,
        value_atoms,
        BASE_UNLOCKING_SCRIPT.to_vec(),
        created_height,
        false,
    )
}

fn valid_coinbase(_network: Network, height: u64, fee_atoms: u64) -> Transaction {
    Transaction {
        version: 1,
        inputs: vec![],
        outputs: vec![TxOutput {
            value_atoms: subsidy::block_subsidy_atoms(height).saturating_add(fee_atoms),
            locking_script: BASE_REWARD_SCRIPT.to_vec(),
        }],
        lock_time: height as u32,
        witness: vec![],
        tx_pow_nonce: 0,
        tx_pow_bits: 0,
    }
}

fn valid_block_timestamp(network: Network, height: u64) -> u64 {
    let genesis_timestamp = genesis::genesis_state(network).block.header.timestamp;
    genesis_timestamp.saturating_add(height.max(1))
}

fn valid_block(network: Network, height: u64, txs: Vec<Transaction>) -> Block {
    let header = BlockHeader {
        version: 1,
        network_id: network,
        height,
        previous_block_hash: [0; 48],
        merkle_root: merkle_root(&txs),
        witness_root: witness_root(&txs),
        timestamp: valid_block_timestamp(network, height),
        difficulty_target_or_bits: pow::target_for_height(network, height),
        nonce: 0,
    };
    Block::new(header, txs)
}

fn valid_spend_fixture() -> (FalconKeypair, UtxoEntry, Transaction, u64) {
    let keypair = generate_from_seed(b"atho-adversarial-spend").expect("keypair");
    let utxo = valid_utxo(Network::Mainnet, 0, BASE_TX_VALUE);
    let mut tx = signed_base_tx(&keypair, utxo.txid, BASE_TX_OUTPUT);
    solve_transaction_pow(Network::Mainnet, &mut tx, BASE_TX_FEE);
    (keypair, utxo, tx, BASE_TX_FEE)
}

fn valid_spend_fixture_two_inputs() -> (FalconKeypair, UtxoEntry, UtxoEntry, Transaction, u64) {
    let keypair = generate_from_seed(b"atho-adversarial-spend-two").expect("keypair");
    let utxo_a = UtxoEntry::new(
        Network::Mainnet,
        [8; 48],
        0,
        u64::MAX / 2,
        BASE_UNLOCKING_SCRIPT.to_vec(),
        0,
        false,
    );
    let utxo_b = UtxoEntry::new(
        Network::Mainnet,
        [9; 48],
        1,
        u64::MAX / 2,
        BASE_UNLOCKING_SCRIPT.to_vec(),
        0,
        false,
    );
    let mut tx = Transaction {
        version: 1,
        inputs: vec![
            TxInput {
                previous_txid: utxo_a.txid,
                output_index: utxo_a.output_index,
                unlocking_script: utxo_a.locking_script.clone(),
            },
            TxInput {
                previous_txid: utxo_b.txid,
                output_index: utxo_b.output_index,
                unlocking_script: utxo_b.locking_script.clone(),
            },
        ],
        outputs: vec![TxOutput {
            value_atoms: u64::MAX / 2 - 10,
            locking_script: BASE_LOCKING_SCRIPT.to_vec(),
        }],
        lock_time: 0,
        witness: vec![],
        tx_pow_nonce: 0,
        tx_pow_bits: 0,
    };
    let signature = sign(
        atho_core::consensus::signatures::AthoSignatureDomain::Transaction,
        &keypair.secret_key,
        &transaction_signing_digest(Network::Mainnet, &tx),
    )
    .expect("falcon signature");
    let sig_bytes = signature.0.clone();
    let witness = TxWitness {
        signature: sig_bytes.clone(),
        pubkey: keypair.public_key.0.clone(),
        input_refs: (0..tx.inputs.len())
            .map(|index| WitnessInputRef {
                input_index: index as u32,
                sig_ref_short: derive_sig_ref_short(&tx.txid(), &sig_bytes, index as u32),
                witness_commit_ref: [0; 16],
            })
            .collect(),
        additional_signers: vec![],
    };
    tx.witness = witness.canonical_bytes();
    solve_transaction_pow(Network::Mainnet, &mut tx, 20);
    (keypair, utxo_a, utxo_b, tx, 20)
}

fn tx_structure_attack(cases: usize, seed: u64) -> Result<CategoryReport, String> {
    let mut report = CategoryReport::new("tx_structure");
    let (_keypair, utxo, base_tx, base_fee) = valid_spend_fixture();
    for i in 0..cases {
        report.cases += 1;
        let mut prng = SplitMix64::new(hash_seed(seed, i as u64));
        let kind = prng.next_usize(15);
        let spend_height = if kind == 13 {
            utxo.created_height + STANDARD_TX_CONFIRMATIONS - 2
        } else {
            10
        };
        let mut tx = base_tx.clone();
        let mut fee_atoms = base_fee;
        let expected = match kind {
            0 => Ok(()),
            1 => {
                tx.inputs.clear();
                tx.witness.clear();
                Err(ValidationError::NoInputs)
            }
            2 => {
                tx.outputs.clear();
                tx.witness.clear();
                fee_atoms = 0;
                Err(ValidationError::NoOutputs)
            }
            3 => {
                tx.inputs.push(tx.inputs[0].clone());
                let witness = tx.witness_payload().expect("witness");
                let mut refs = witness.input_refs.clone();
                refs.push(refs[0].clone());
                tx.witness = TxWitness {
                    input_refs: refs,
                    ..witness
                }
                .canonical_bytes();
                Err(ValidationError::DuplicateInput)
            }
            4 => {
                tx.outputs[0].value_atoms = 0;
                Err(ValidationError::ZeroValueOutput)
            }
            5 => {
                fee_atoms = 0;
                Err(ValidationError::FeeBelowMinimum)
            }
            6 => {
                tx.outputs[0].locking_script = vec![0; MAX_TRANSACTION_SIZE_BYTES + 1];
                Err(ValidationError::TransactionTooLarge)
            }
            7 => {
                tx.witness.clear();
                Err(ValidationError::InvalidWitness)
            }
            8 => {
                let mut witness = tx.witness_payload().expect("witness");
                witness.signature.truncate(1);
                tx.witness = witness.canonical_bytes();
                Err(ValidationError::InvalidWitness)
            }
            9 => {
                let mut witness = tx.witness_payload().expect("witness");
                witness.pubkey.truncate(1);
                tx.witness = witness.canonical_bytes();
                Err(ValidationError::InvalidWitness)
            }
            10 => {
                let mut witness = tx.witness_payload().expect("witness");
                witness.input_refs[0].sig_ref_short = [0, 0];
                tx.witness = witness.canonical_bytes();
                Err(ValidationError::WitnessInputReferenceMismatch)
            }
            11 => {
                tx.inputs[0].unlocking_script = vec![0xff];
                Err(ValidationError::InvalidWitness)
            }
            12 => {
                tx.inputs[0].unlocking_script = utxo.locking_script.clone();
                tx.witness = tx.witness.clone();
                Ok(())
            }
            13 => {
                fee_atoms = base_fee;
                Ok(())
            }
            _ => Ok(()),
        };

        let outcome = catch_unwind(AssertUnwindSafe(|| {
            validate_transaction(&tx, fee_atoms, Network::Mainnet)
        }));
        match outcome {
            Ok(actual) => check_validation_result(
                &mut report,
                i,
                "validate_transaction",
                expected.is_ok(),
                actual,
            ),
            Err(_) => {
                report.fail_panic(format!(
                    "case={i} kind={kind} panic in validate_transaction"
                ));
            }
        }

        let lookup = |txid: &[u8; 48], output_index: u32| {
            if *txid == utxo.txid && output_index == utxo.output_index {
                Some(utxo.clone())
            } else {
                None
            }
        };
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            validate_transaction_with_context(
                &tx,
                fee_atoms,
                Network::Mainnet,
                spend_height,
                lookup,
            )
        }));
        let expected_ctx = match kind {
            12 => Ok(base_fee),
            13 => Err(ValidationError::InsufficientConfirmations),
            _ => expected.clone().map(|_| base_fee),
        };
        match outcome {
            Ok(actual) => check_validation_result_u64(
                &mut report,
                i,
                "validate_transaction_with_context",
                expected_ctx.is_ok(),
                actual,
            ),
            Err(_) => report.fail_panic(format!(
                "case={i} kind={kind} panic in validate_transaction_with_context"
            )),
        }
    }
    Ok(report)
}

fn serialization_attack(cases: usize, seed: u64) -> Result<CategoryReport, String> {
    let mut report = CategoryReport::new("serialization");
    let (_, _, base_tx, _) = valid_spend_fixture();
    let coinbase = valid_coinbase(Network::Mainnet, 1, 0);
    let base_block = valid_block(Network::Mainnet, 1, vec![coinbase.clone(), base_tx.clone()]);
    for i in 0..cases {
        report.cases += 1;
        let kind = hash_seed(seed, i as u64) as usize % 14;
        let outcome = catch_unwind(AssertUnwindSafe(|| match kind {
            0 => {
                let encoded = serde_json::to_string(&base_tx).unwrap();
                let decoded: Transaction = serde_json::from_str(&encoded).unwrap();
                assert_eq!(decoded, base_tx);
            }
            1 => {
                let witness = base_tx.witness_payload().unwrap();
                let encoded = witness.canonical_bytes();
                let decoded = TxWitness::from_bytes(&encoded).unwrap();
                assert_eq!(decoded, witness);
            }
            2 => {
                assert_eq!(base_tx.txid(), base_tx.txid());
                assert_eq!(base_tx.signing_digest(), base_tx.signing_digest());
            }
            3 => {
                let mut mutated = base_tx.clone();
                mutated.witness[0] ^= 1;
                assert_eq!(base_tx.txid(), mutated.txid());
                assert_ne!(base_tx.wtxid(), mutated.wtxid());
            }
            4 => {
                let encoded = serde_json::to_string(&base_block.header).unwrap();
                let decoded: BlockHeader = serde_json::from_str(&encoded).unwrap();
                assert_eq!(decoded, base_block.header);
            }
            5 => {
                let encoded = serde_json::to_string(&base_block).unwrap();
                let decoded: Block = serde_json::from_str(&encoded).unwrap();
                assert_eq!(decoded.header, base_block.header);
                assert_eq!(decoded.transactions, base_block.transactions);
                assert_eq!(decoded.header.witness_root, base_block.header.witness_root);
                assert!(decoded.witnesses.is_empty());
            }
            6 => {
                assert!(base_block.compact_bytes().len() <= base_block.full_bytes().len());
                assert_eq!(
                    base_block.header.block_hash(),
                    base_block.header.block_hash()
                );
            }
            7 => {
                let witness = base_tx.witness_payload().unwrap();
                let mut bytes = witness.canonical_bytes();
                bytes.truncate(bytes.len().saturating_sub(1));
                assert!(TxWitness::from_bytes(&bytes).is_none());
            }
            8 => {
                let parts = address_parts_from_public_key(Network::Mainnet, &[1u8; 32]);
                let (digest, network) = decode_base56_address(&parts.base56_address).unwrap();
                assert_eq!(digest, parts.payment_digest);
                assert_eq!(network, Network::Mainnet);
            }
            9 => {
                let bytes = serde_json::to_vec(&base_tx).unwrap();
                let decoded: Transaction = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(decoded.full_bytes(), base_tx.full_bytes());
            }
            10 => {
                let encoded = base_tx.full_bytes();
                let decoded = Transaction::from_full_bytes(&encoded).unwrap();
                assert_eq!(decoded.full_bytes(), encoded);
                assert_eq!(decoded.txid(), base_tx.txid());
                assert_eq!(decoded.wtxid(), base_tx.wtxid());
            }
            11 => {
                let mut encoded = base_tx.full_bytes();
                encoded.push(0);
                assert!(Transaction::from_full_bytes(&encoded).is_none());
            }
            12 => {
                let encoded = base_block.canonical_bytes();
                let decoded = Block::from_canonical_bytes(&encoded).unwrap();
                assert_eq!(decoded.canonical_bytes(), encoded);
                assert_eq!(decoded.header.block_hash(), base_block.header.block_hash());
            }
            13 => {
                let mut encoded = base_block.canonical_bytes();
                encoded.truncate(encoded.len().saturating_sub(1));
                assert!(Block::from_canonical_bytes(&encoded).is_none());
            }
            _ => unreachable!(),
        }));
        match outcome {
            Ok(()) => report.pass(),
            Err(_) => report.fail_panic(format!("case={i} kind={kind} panic in serialization")),
        }
    }
    Ok(report)
}

fn signature_attack(cases: usize, seed: u64) -> Result<CategoryReport, String> {
    let mut report = CategoryReport::new("signature_witness");
    let (keypair, utxo, base_tx, base_fee) = valid_spend_fixture();
    let coinbase = valid_coinbase(Network::Mainnet, 1, base_fee);
    let base_block = valid_block(
        Network::Mainnet,
        1,
        vec![coinbase.clone(), finalize_tx_for_block(&base_tx, [0; 48])],
    );
    for i in 0..cases {
        report.cases += 1;
        let kind = hash_seed(seed, i as u64) as usize % 8;
        let mut tx = base_tx.clone();
        let expected = match kind {
            0 => Ok(()),
            1 => {
                let mut witness = tx.witness_payload().unwrap();
                witness.signature[0] ^= 0xff;
                tx.witness = witness.canonical_bytes();
                Err(ValidationError::InvalidWitness)
            }
            2 => {
                let mut witness = tx.witness_payload().unwrap();
                witness.pubkey.truncate(1);
                tx.witness = witness.canonical_bytes();
                Err(ValidationError::InvalidWitness)
            }
            3 => {
                let mut witness = tx.witness_payload().unwrap();
                witness.input_refs[0].sig_ref_short = [0, 0];
                tx.witness = witness.canonical_bytes();
                Err(ValidationError::WitnessInputReferenceMismatch)
            }
            4 => {
                let mut witness = tx.witness_payload().unwrap();
                witness.input_refs.clear();
                tx.witness = witness.canonical_bytes();
                Err(ValidationError::InvalidWitness)
            }
            5 => {
                tx.outputs[0].value_atoms = tx.outputs[0].value_atoms.saturating_sub(1);
                Err(ValidationError::InvalidWitness)
            }
            6 => {
                let digest = atho_core::consensus::signatures::block_signing_digest(&base_block);
                let signature = sign(
                    atho_core::consensus::signatures::AthoSignatureDomain::Block,
                    &keypair.secret_key,
                    &digest,
                )
                .unwrap();
                let mut witness = tx.witness_payload().unwrap();
                witness.signature = signature.0;
                tx.witness = witness.canonical_bytes();
                Err(ValidationError::InvalidWitness)
            }
            _ => {
                let mut block_tx = tx.clone();
                let block_witness_root = witness_root(&[coinbase.clone(), block_tx.clone()]);
                block_tx = finalize_tx_for_block(&block_tx, block_witness_root);
                let mut block = valid_block(Network::Mainnet, 1, vec![coinbase.clone(), block_tx]);
                block.fees_miner_atoms = base_fee;
                block.transactions[1].witness = tx.witness.clone();
                Ok(())
            }
        };

        let outcome = catch_unwind(AssertUnwindSafe(|| {
            validate_transaction(&tx, base_fee, Network::Mainnet)
        }));
        match outcome {
            Ok(actual) => check_validation_result(
                &mut report,
                i,
                "validate_transaction",
                expected.is_ok(),
                actual,
            ),
            Err(_) => report.fail_panic(format!(
                "case={i} kind={kind} panic in signature validation"
            )),
        }

        let mut lookup = |txid: &[u8; 48], output_index: u32| {
            if *txid == utxo.txid && output_index == utxo.output_index {
                Some(utxo.clone())
            } else {
                None
            }
        };
        let actual = catch_unwind(AssertUnwindSafe(|| {
            validate_transaction_with_context(&tx, base_fee, Network::Mainnet, 10, &mut lookup)
        }));
        match actual {
            Ok(res) => {
                let expected_ctx = expected.is_ok();
                check_validation_result_u64(
                    &mut report,
                    i,
                    "validate_transaction_with_context",
                    expected_ctx,
                    res,
                )
            }
            Err(_) => {
                report.fail_panic(format!("case={i} kind={kind} panic in context validation"))
            }
        }
    }
    Ok(report)
}

fn utxo_attack(cases: usize, seed: u64) -> Result<CategoryReport, String> {
    let mut report = CategoryReport::new("utxo_state");
    let (_, utxo, tx, fee) = valid_spend_fixture();
    let tx2 = tx.clone();
    let mut set = UtxoSet::new(Network::Mainnet);
    set.insert(utxo.clone()).unwrap();
    for i in 0..cases {
        report.cases += 1;
        let kind = hash_seed(seed, i as u64) as usize % 8;
        let outcome = catch_unwind(AssertUnwindSafe(|| match kind {
            0 => {
                let actual = validate_transaction_with_context(
                    &tx,
                    fee,
                    Network::Mainnet,
                    10,
                    |txid, output_index| {
                        if *txid == utxo.txid && output_index == utxo.output_index {
                            Some(utxo.clone())
                        } else {
                            None
                        }
                    },
                );
                assert!(actual.is_ok());
            }
            1 => {
                assert!(validate_transaction_with_context(
                    &tx,
                    fee,
                    Network::Mainnet,
                    10,
                    |_, _| None,
                )
                .is_err());
            }
            2 => {
                let mut bad = tx.clone();
                bad.inputs[0].unlocking_script = vec![0xff];
                let actual = validate_transaction_with_context(
                    &bad,
                    fee,
                    Network::Mainnet,
                    10,
                    |txid, output_index| {
                        if *txid == utxo.txid && output_index == utxo.output_index {
                            Some(utxo.clone())
                        } else {
                            None
                        }
                    },
                );
                assert!(actual.is_err());
            }
            3 => {
                let immature =
                    UtxoEntry::coinbase(Network::Mainnet, [1; 48], 0, BASE_TX_VALUE, vec![1], 0);
                let mut immature_tx = tx.clone();
                immature_tx.inputs[0].previous_txid = immature.txid;
                immature_tx.inputs[0].unlocking_script = immature.locking_script.clone();
                let actual = validate_transaction_with_context(
                    &immature_tx,
                    fee,
                    Network::Mainnet,
                    COINBASE_MATURITY_BLOCKS - 2,
                    |_, _| Some(immature.clone()),
                );
                assert!(actual.is_err());
            }
            4 => {
                let err = set.insert(utxo.clone()).unwrap_err();
                assert!(matches!(err, StorageError::DuplicateUtxo));
            }
            5 => {
                let mut cross = utxo.clone();
                cross.network = Network::Testnet;
                let err = set.insert(cross).unwrap_err();
                assert!(matches!(err, StorageError::CrossNetworkReplay));
            }
            6 => {
                let block = valid_block(
                    Network::Mainnet,
                    1,
                    vec![valid_coinbase(Network::Mainnet, 1, 0), tx2.clone()],
                );
                let mut working = set.clone();
                let before = working.len();
                let result = working.apply_block(&block);
                assert!(result.is_ok());
                assert!(working.len() >= before);
                working.disconnect_block(result.unwrap());
            }
            _ => {
                let block = valid_block(
                    Network::Mainnet,
                    1,
                    vec![
                        valid_coinbase(Network::Mainnet, 1, 0),
                        tx.clone(),
                        tx.clone(),
                    ],
                );
                let mut working = set.clone();
                let before = working.len();
                let result = working.apply_block(&block);
                assert!(result.is_err());
                assert_eq!(working.len(), before);
            }
        }));
        match outcome {
            Ok(()) => report.pass(),
            Err(_) => report.fail_panic(format!("case={i} kind={kind} panic in utxo state")),
        }
    }
    Ok(report)
}

fn fee_attack(cases: usize, seed: u64) -> Result<CategoryReport, String> {
    let mut report = CategoryReport::new("fee_emission");
    let (_, utxo, tx, fee) = valid_spend_fixture();
    let (k2, utxo_a, utxo_b, tx_overflow, overflow_fee) = valid_spend_fixture_two_inputs();
    let _ = k2;
    for i in 0..cases {
        report.cases += 1;
        let kind = hash_seed(seed, i as u64) as usize % 8;
        let outcome = catch_unwind(AssertUnwindSafe(|| match kind {
            0 => {
                let minimum_fee = tx.vsize_bytes() as u64 * MIN_TX_FEE_PER_VBYTE_ATOMS;
                assert!(
                    validate_transaction(&tx, minimum_fee.saturating_sub(1), Network::Mainnet,)
                        .is_err()
                );
            }
            1 => {
                assert!(validate_transaction(&tx, fee, Network::Mainnet).is_ok());
            }
            2 => {
                let mut bad = tx.clone();
                bad.outputs[0].value_atoms = bad.outputs[0].value_atoms.saturating_add(1);
                let actual = validate_transaction_with_context(
                    &bad,
                    fee,
                    Network::Mainnet,
                    10,
                    |txid, output_index| {
                        if *txid == utxo.txid && output_index == utxo.output_index {
                            Some(utxo.clone())
                        } else {
                            None
                        }
                    },
                );
                assert!(actual.is_err());
            }
            3 => {
                let actual = validate_transaction_with_context(
                    &tx_overflow,
                    overflow_fee,
                    Network::Mainnet,
                    10,
                    |txid, output_index| {
                        if *txid == utxo_a.txid && output_index == utxo_a.output_index {
                            Some(utxo_a.clone())
                        } else if *txid == utxo_b.txid && output_index == utxo_b.output_index {
                            Some(utxo_b.clone())
                        } else {
                            None
                        }
                    },
                );
                assert!(actual.is_err());
            }
            4 => {
                let mut zero = tx.clone();
                zero.outputs[0].value_atoms = 0;
                assert!(validate_transaction(&zero, fee, Network::Mainnet).is_err());
            }
            5 => {
                assert_eq!(
                    subsidy::max_supply_atoms_for_network(Network::Mainnet),
                    None
                );
            }
            6 => {
                assert_eq!(
                    subsidy::get_block_reward_atoms(1_679_999),
                    6_250_000_000_000
                );
                assert_eq!(
                    subsidy::get_block_reward_atoms(1_680_000),
                    3_125_000_000_000
                );
            }
            _ => {
                let _rate = tx.feerate_atoms_per_vbyte(fee);
            }
        }));
        match outcome {
            Ok(()) => report.pass(),
            Err(_) => report.fail_panic(format!("case={i} kind={kind} panic in fee attack")),
        }
    }
    Ok(report)
}

fn coinbase_attack(cases: usize, seed: u64) -> Result<CategoryReport, String> {
    let mut report = CategoryReport::new("coinbase_reward");
    for i in 0..cases {
        report.cases += 1;
        let kind = hash_seed(seed, i as u64) as usize % 6;
        let height = 7;
        let outcome = catch_unwind(AssertUnwindSafe(|| match kind {
            0 => {
                let block = valid_block(
                    Network::Mainnet,
                    height,
                    vec![valid_coinbase(Network::Mainnet, height, 0)],
                );
                assert!(validate_block_without_pow(&block, height, Network::Mainnet).is_ok());
            }
            1 => {
                let mut coinbase = valid_coinbase(Network::Mainnet, height, 0);
                coinbase.outputs[0].value_atoms += 1;
                let block = valid_block(Network::Mainnet, height, vec![coinbase]);
                assert!(validate_block_without_pow(&block, height, Network::Mainnet).is_err());
            }
            2 => {
                let coinbase_a = valid_coinbase(Network::Mainnet, height, 0);
                let coinbase_b = valid_coinbase(Network::Mainnet, height, 0);
                let block = valid_block(Network::Mainnet, height, vec![coinbase_a, coinbase_b]);
                assert!(validate_block_without_pow(&block, height, Network::Mainnet).is_err());
            }
            3 => {
                let block = Block::default();
                assert!(validate_block_without_pow(&block, height, Network::Mainnet).is_err());
            }
            4 => {
                let mut coinbase = valid_coinbase(Network::Mainnet, height, 0);
                coinbase.outputs.push(TxOutput {
                    value_atoms: 1,
                    locking_script: vec![1; ADDRESS_DIGEST_BYTES],
                });
                let block = valid_block(Network::Mainnet, height, vec![coinbase]);
                assert!(validate_block_without_pow(&block, height, Network::Mainnet).is_err());
            }
            _ => {
                let block = valid_block(
                    Network::Mainnet,
                    height,
                    vec![valid_coinbase(Network::Mainnet, height, 0)],
                );
                assert_eq!(block.transactions[0].outputs.len(), 1);
            }
        }));
        match outcome {
            Ok(()) => report.pass(),
            Err(_) => report.fail_panic(format!("case={i} kind={kind} panic in coinbase attack")),
        }
    }
    Ok(report)
}

fn block_template_attack(cases: usize, seed: u64) -> Result<CategoryReport, String> {
    let mut report = CategoryReport::new("block_template_body");
    let (_, _utxo, tx, fee) = valid_spend_fixture();
    let coinbase = valid_coinbase(Network::Mainnet, 1, fee);
    let tx = finalize_tx_for_block(&tx, [0; 48]);
    let mut base = valid_block(Network::Mainnet, 1, vec![coinbase.clone(), tx.clone()]);
    base.fees_total_atoms = fee;
    base.fees_miner_atoms = fee;
    for i in 0..cases {
        report.cases += 1;
        let kind = hash_seed(seed, i as u64) as usize % 8;
        let outcome = catch_unwind(AssertUnwindSafe(|| match kind {
            0 => {
                assert!(validate_block_without_pow(&base, 1, Network::Mainnet).is_ok());
            }
            1 => {
                let mut bad = base.clone();
                bad.header.merkle_root[0] ^= 1;
                assert!(validate_block_without_pow(&bad, 1, Network::Mainnet).is_err());
            }
            2 => {
                let mut bad = base.clone();
                bad.header.witness_root[0] ^= 1;
                assert!(validate_block_without_pow(&bad, 1, Network::Mainnet).is_err());
            }
            3 => {
                let mut bad = base.clone();
                bad.transactions.swap(0, 1);
                bad.header.merkle_root = merkle_root(&bad.transactions);
                bad.header.witness_root = witness_root(&bad.transactions);
                assert!(validate_block_without_pow(&bad, 1, Network::Mainnet).is_err());
            }
            4 => {
                let mut bad = base.clone();
                bad.transactions[1].inputs[0].unlocking_script = vec![0xff];
                bad.header.merkle_root = merkle_root(&bad.transactions);
                bad.header.witness_root = witness_root(&bad.transactions);
                assert!(validate_block_with_context(
                    &bad,
                    1,
                    Network::Mainnet,
                    [0; 48],
                    pow::target_for_height(Network::Mainnet, 1),
                    &[],
                    UtxoSet::new(Network::Mainnet),
                )
                .is_err());
            }
            5 => {
                let mut bad = base.clone();
                bad.transactions[1].witness.clear();
                bad.header.witness_root = witness_root(&bad.transactions);
                assert!(validate_block_without_pow(&bad, 1, Network::Mainnet).is_err());
            }
            6 => {
                let mut bad = base.clone();
                bad.header.previous_block_hash = [1; 48];
                assert!(validate_block_with_context(
                    &bad,
                    1,
                    Network::Mainnet,
                    [0; 48],
                    pow::target_for_height(Network::Mainnet, 1),
                    &[],
                    UtxoSet::new(Network::Mainnet),
                )
                .is_err());
            }
            _ => {
                let mut bad = base.clone();
                bad.fees_miner_atoms += 1;
                assert!(validate_block_with_context(
                    &bad,
                    1,
                    Network::Mainnet,
                    [0; 48],
                    pow::target_for_height(Network::Mainnet, 1),
                    &[],
                    UtxoSet::new(Network::Mainnet),
                )
                .is_err());
            }
        }));
        match outcome {
            Ok(()) => report.pass(),
            Err(_) => report.fail_panic(format!("case={i} kind={kind} panic in block template")),
        }
    }
    Ok(report)
}

fn header_pow_attack(cases: usize, seed: u64) -> Result<CategoryReport, String> {
    let mut report = CategoryReport::new("header_pow_timestamp");
    let block = valid_block(
        Network::Mainnet,
        1,
        vec![valid_coinbase(Network::Mainnet, 1, 0)],
    );
    for i in 0..cases {
        report.cases += 1;
        let kind = hash_seed(seed, i as u64) as usize % 8;
        let mut header = block.header.clone();
        let outcome = catch_unwind(AssertUnwindSafe(|| match kind {
            0 => {
                assert_eq!(
                    validate_block_without_pow(&block, 1, Network::Mainnet),
                    Ok(())
                );
            }
            1 => {
                header.timestamp += 1;
                let bad = Block::new(header, block.transactions.clone());
                assert_eq!(
                    validate_block_without_pow(&bad, 1, Network::Mainnet),
                    Ok(())
                );
            }
            2 => {
                header.version = 0;
                let bad = Block::new(header, block.transactions.clone());
                assert_eq!(
                    validate_block_without_pow(&bad, 1, Network::Mainnet),
                    Err(ValidationError::InvalidBlockVersion)
                );
            }
            3 => {
                header.network_id = Network::Testnet;
                let bad = Block::new(header, block.transactions.clone());
                assert_eq!(
                    validate_block_without_pow(&bad, 1, Network::Mainnet),
                    Err(ValidationError::BlockNetworkMismatch)
                );
            }
            4 => {
                header.height = 2;
                let bad = Block::new(header, block.transactions.clone());
                assert_eq!(
                    validate_block_without_pow(&bad, 1, Network::Mainnet),
                    Err(ValidationError::InvalidBlockHeight)
                );
            }
            5 => {
                header.difficulty_target_or_bits = [0; 48];
                let bad = Block::new(header, block.transactions.clone());
                assert_eq!(
                    validate_block_without_pow(&bad, 1, Network::Mainnet),
                    Err(ValidationError::BlockTargetOutOfBounds)
                );
            }
            6 => {
                assert!(!pow::meets_target(
                    &[0xff; 48],
                    &pow::DIFFICULTY_PROFILE.min_difficulty_target
                ));
            }
            _ => {
                assert!(pow::target_within_bounds(&pow::initial_target_for_network(
                    Network::Mainnet
                )));
            }
        }));
        match outcome {
            Ok(()) => report.pass(),
            Err(_) => report.fail_panic(format!("case={i} kind={kind} panic in header/pow")),
        }
    }
    Ok(report)
}

fn chain_acceptance_attack(cases: usize, seed: u64) -> Result<CategoryReport, String> {
    let mut report = CategoryReport::new("chain_acceptance_rollback");
    for i in 0..cases {
        report.cases += 1;
        let kind = hash_seed(seed, i as u64) as usize % 6;
        let height = 1;
        let mut block = valid_block(
            Network::Mainnet,
            height,
            vec![valid_coinbase(Network::Mainnet, height, 0)],
        );
        block.header.previous_block_hash = genesis::genesis_state(Network::Mainnet).block_hash;
        match kind {
            0 => block.transactions[0].outputs[0].value_atoms += 1,
            1 => block.header.timestamp = 0,
            2 => block.header.network_id = Network::Testnet,
            3 => block.header.previous_block_hash = [1; 48],
            4 => block.header.height = 2,
            _ => {}
        }
        block.header.merkle_root = merkle_root(&block.transactions);
        block.header.witness_root = witness_root(&block.transactions);
        let block = if kind == 5 {
            Miner::new(1).solve_block(block)
        } else {
            block
        };

        let mut node = Node::new(atho_node::config::NodeConfig::new(Network::Mainnet));
        let node_before = (node.height(), node.tip_hash());
        let node_result = catch_unwind(AssertUnwindSafe(|| node.submit_block(&block)));
        match node_result {
            Ok(Ok(())) => {
                if kind == 5 {
                    report.pass();
                } else {
                    report.fail_accept(format!("case={i} kind={kind} node accepted invalid block"));
                }
            }
            Ok(Err(NodeError::Validation(_))) => {
                let after = (node.height(), node.tip_hash());
                if after != node_before {
                    report
                        .fail_reject(format!("case={i} kind={kind} node mutated state on reject"));
                } else {
                    report.pass();
                }
            }
            Ok(Err(other)) => {
                report.fail_reject(format!(
                    "case={i} kind={kind} unexpected node error {other}"
                ));
            }
            Err(_) => report.fail_panic(format!("case={i} kind={kind} panic in node path")),
        }

        let mut storage = StorageChainstate::new(Network::Mainnet);
        let storage_result = catch_unwind(AssertUnwindSafe(|| storage.connect_block(&block)));
        match storage_result {
            Ok(Ok(())) => {
                if kind == 5 {
                    report.pass();
                } else {
                    report.fail_accept(format!(
                        "case={i} kind={kind} storage accepted invalid block"
                    ));
                }
            }
            Ok(Err(err)) => {
                if kind == 5 {
                    report.fail_reject(format!(
                        "case={i} kind={kind} storage rejected valid block unexpectedly: {err}"
                    ));
                } else {
                    report.pass();
                }
            }
            Err(_) => report.fail_panic(format!("case={i} kind={kind} panic in storage path")),
        }
    }
    Ok(report)
}

fn confirmation_attack(cases: usize, seed: u64) -> Result<CategoryReport, String> {
    let mut report = CategoryReport::new("ordering_confirmations");
    let (_, utxo, tx, fee) = valid_spend_fixture();
    for i in 0..cases {
        report.cases += 1;
        let kind = hash_seed(seed, i as u64) as usize % 6;
        let outcome = catch_unwind(AssertUnwindSafe(|| match kind {
            0 => {
                let spend_height = utxo.created_height + STANDARD_TX_CONFIRMATIONS - 1;
                assert_eq!(
                    validate_transaction_with_context(
                        &tx,
                        fee,
                        Network::Mainnet,
                        spend_height,
                        |txid, output_index| {
                            if *txid == utxo.txid && output_index == utxo.output_index {
                                Some(utxo.clone())
                            } else {
                                None
                            }
                        }
                    ),
                    Ok(fee)
                );
            }
            1 => {
                let spend_height = utxo.created_height + STANDARD_TX_CONFIRMATIONS - 2;
                let err = validate_transaction_with_context(
                    &tx,
                    fee,
                    Network::Mainnet,
                    spend_height,
                    |txid, output_index| {
                        if *txid == utxo.txid && output_index == utxo.output_index {
                            Some(utxo.clone())
                        } else {
                            None
                        }
                    },
                )
                .unwrap_err();
                assert_eq!(err, ValidationError::InsufficientConfirmations);
            }
            2 => {
                let coinbase =
                    UtxoEntry::coinbase(Network::Mainnet, [1; 48], 0, BASE_TX_VALUE, vec![1], 0);
                assert_eq!(
                    coinbase.confirmation_count(COINBASE_MATURITY_BLOCKS - 1),
                    COINBASE_MATURITY_BLOCKS
                );
                assert!(coinbase.is_coinbase_mature(COINBASE_MATURITY_BLOCKS - 1));
            }
            3 => {
                let coinbase =
                    UtxoEntry::coinbase(Network::Mainnet, [1; 48], 0, BASE_TX_VALUE, vec![1], 0);
                assert!(!coinbase.is_coinbase_mature(COINBASE_MATURITY_BLOCKS - 2));
            }
            4 => {
                let future = UtxoEntry::new(Network::Mainnet, [2; 48], 0, 1, vec![1], 10, false);
                assert_eq!(future.confirmation_count(0), 1);
            }
            _ => {
                assert_eq!(utxo.required_confirmations(), STANDARD_TX_CONFIRMATIONS);
            }
        }));
        match outcome {
            Ok(()) => report.pass(),
            Err(_) => {
                report.fail_panic(format!("case={i} kind={kind} panic in confirmation attack"))
            }
        }
    }
    Ok(report)
}

fn genesis_attack(cases: usize, seed: u64) -> Result<CategoryReport, String> {
    let mut report = CategoryReport::new("genesis_constants");
    for i in 0..cases {
        report.cases += 1;
        let network = match hash_seed(seed, i as u64) % 3 {
            0 => Network::Mainnet,
            1 => Network::Testnet,
            _ => Network::Regnet,
        };
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let state = genesis::genesis_state(network);
            let profile = genesis::regenerate_genesis_profile(network);
            assert_eq!(state.block_hash, profile.block_hash);
            assert_eq!(state.coinbase_txid, profile.coinbase_txid);
            assert_eq!(state.reward_address, profile.reward_address);
            assert_eq!(state.block.header.block_hash(), state.block_hash);
            let address = address_parts_from_public_key(network, &[42u8; 32]).base56_address;
            let decoded = decode_base56_address(&address).unwrap().1;
            assert_eq!(decoded, network);
            assert_eq!(
                Network::from_consensus_id(network.consensus_id()),
                Some(network)
            );
            assert_eq!(
                network.visible_prefix(),
                match network {
                    Network::Mainnet => 'A',
                    Network::Testnet => 'T',
                    Network::Regnet => 'R',
                    Network::Prunetest => 'P',
                }
            );
        }));
        match outcome {
            Ok(()) => report.pass(),
            Err(_) => report.fail_panic(format!(
                "case={i} network={network:?} panic in genesis attack"
            )),
        }
    }
    Ok(report)
}

fn persistence_attack(cases: usize, seed: u64) -> Result<CategoryReport, String> {
    let mut report = CategoryReport::new("persistence_state");
    let root = std::env::temp_dir().join(format!(
        "atho-adversarial-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).map_err(|err| err.to_string())?;
    let result = with_cwd(&root, || {
        let _data_dir = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &persistence_data_dir(&root));
        for i in 0..cases {
            report.cases += 1;
            let kind = hash_seed(seed, i as u64) as usize % 4;
            cleanup_dev_layout();
            let outcome = catch_unwind(AssertUnwindSafe(|| match kind {
                0 => {
                    write_legacy_snapshot_files(&root, true, false)?;
                    match atho_storage::chainstate::Chainstate::try_load_or_new(Network::Mainnet) {
                        Ok(_) => {
                            report.fail_accept(format!(
                                "case={i} kind={kind} incomplete legacy snapshot accepted"
                            ));
                        }
                        Err(StorageError::CorruptData)
                        | Err(StorageError::IncompleteBlockHistory)
                        | Err(StorageError::LegacyStorageLayout) => report.pass(),
                        Err(err) => {
                            report.fail_reject(format!(
                                "case={i} kind={kind} incomplete legacy snapshot wrong error {err}"
                            ));
                        }
                    }
                    Ok::<(), String>(())
                }
                1 => {
                    write_legacy_snapshot_files(&root, false, true)?;
                    match atho_storage::chainstate::Chainstate::try_load_or_new(Network::Mainnet) {
                        Ok(_) => {
                            report.fail_accept(format!(
                                "case={i} kind={kind} malformed legacy snapshot accepted"
                            ));
                        }
                        Err(StorageError::CorruptData) | Err(StorageError::LegacyStorageLayout) => {
                            report.pass()
                        }
                        Err(err) => {
                            report.fail_reject(format!(
                                "case={i} kind={kind} malformed legacy snapshot wrong error {err}"
                            ));
                        }
                    }
                    Ok(())
                }
                2 => {
                    let db = Database::open(Network::Mainnet).map_err(|err| err.to_string())?;
                    let snapshot = ChainstateSnapshot {
                        height: 42,
                        tip_hash: [3; 48],
                        tip_header: None,
                    };
                    let mut utxos = vec![
                        UtxoEntry::new(Network::Mainnet, [11; 48], 0, 100, vec![1], 1, false),
                        UtxoEntry::new(Network::Testnet, [12; 48], 1, 200, vec![2], 2, false),
                        UtxoEntry::new(Network::Mainnet, [11; 48], 0, 300, vec![3], 3, false),
                    ];
                    db.save_chainstate_snapshot(&snapshot, &utxos)
                        .map_err(|err| err.to_string())?;
                    match atho_storage::chainstate::Chainstate::try_load_or_new(Network::Mainnet) {
                        Ok(_) => {
                            report.fail_accept(format!(
                                "case={i} kind={kind} persisted invalid utxos accepted"
                            ));
                        }
                        Err(StorageError::CrossNetworkReplay) => report.pass(),
                        Err(err) => {
                            report.fail_reject(format!(
                                "case={i} kind={kind} invalid utxo snapshot wrong error {err}"
                            ));
                        }
                    }
                    utxos.clear();
                    Ok(())
                }
                _ => {
                    let genesis = genesis::genesis_state(Network::Mainnet);
                    let db = Database::open(Network::Mainnet).map_err(|err| err.to_string())?;
                    let snapshot = ChainstateSnapshot {
                        height: 0,
                        tip_hash: genesis.block_hash,
                        tip_header: Some(genesis.block.header.clone()),
                    };
                    db.append_block(0, &genesis.block)
                        .map_err(|err| err.to_string())?;
                    let utxos = vec![UtxoEntry::coinbase(
                        Network::Mainnet,
                        genesis.coinbase_txid,
                        0,
                        atho_core::consensus::subsidy::genesis_coinbase_atoms_for_network(
                            Network::Mainnet,
                        ),
                        genesis.block.transactions[0].outputs[0]
                            .locking_script
                            .clone(),
                        0,
                    )];
                    db.save_chainstate_snapshot(&snapshot, &utxos)
                        .map_err(|err| err.to_string())?;
                    let state =
                        atho_storage::chainstate::Chainstate::try_load_or_new(Network::Mainnet)
                            .map_err(|err| err.to_string())?;
                    if state.height == 0 && state.tip_hash == genesis.block_hash {
                        report.pass();
                    } else {
                        report.fail_reject(format!(
                            "case={i} kind={kind} valid snapshot did not reload"
                        ));
                    }
                    Ok(())
                }
            }));
            if outcome.is_err() {
                report.fail_panic(format!("case={i} kind={kind} panic in persistence attack"));
            }
        }
        Ok::<(), String>(())
    });
    result??;
    Ok(report)
}

fn determinism_attack(cases: usize, seed: u64) -> Result<CategoryReport, String> {
    let mut report = CategoryReport::new("determinism_split_brain");
    let (_, utxo, base_tx, fee) = valid_spend_fixture();
    let coinbase = valid_coinbase(Network::Mainnet, 1, fee);
    let tx = finalize_tx_for_block(&base_tx, [0; 48]);
    let mut base_block = valid_block(Network::Mainnet, 1, vec![coinbase.clone(), tx.clone()]);
    base_block.fees_total_atoms = fee;
    base_block.fees_miner_atoms = fee;
    for i in 0..cases {
        report.cases += 1;
        let kind = hash_seed(seed, i as u64) as usize % 7;
        let outcome = catch_unwind(AssertUnwindSafe(|| match kind {
            0 => {
                let mut mempool = Mempool::new();
                let entry = MempoolEntry::new(base_tx.clone(), fee);
                let txid = mempool
                    .admit(entry, Network::Mainnet, 10, |txid, output_index| {
                        if *txid == utxo.txid && output_index == utxo.output_index {
                            Some(utxo.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap();
                assert!(mempool.contains(&txid));
            }
            1 => {
                let mut node = Node::new(atho_node::config::NodeConfig::new(Network::Mainnet));
                node.dev_seed_chainstate(6, node.tip_hash(), [utxo.clone()])
                    .unwrap();
                let entry = MempoolEntry::new(base_tx.clone(), fee);
                let txid = node.admit_transaction(entry).unwrap();
                assert_eq!(node.mempool_len(), 1);
                assert_eq!(node.mempool_spent_inputs().len(), 1);
                assert_eq!(txid, base_tx.txid());
            }
            2 => {
                assert_eq!(base_tx.txid(), base_tx.txid());
                assert_eq!(
                    base_block.header.block_hash(),
                    base_block.header.block_hash()
                );
            }
            3 => {
                let mut block = base_block.clone();
                block.transactions[1].witness[0] ^= 1;
                assert_eq!(block.header.block_hash(), base_block.header.block_hash());
                assert_eq!(
                    validate_block_without_pow(&base_block, 1, Network::Mainnet),
                    Ok(())
                );
            }
            4 => {
                let candidate =
                    valid_block(Network::Mainnet, 1, vec![coinbase.clone(), tx.clone()]);
                assert_eq!(
                    candidate.header.merkle_root,
                    merkle_root(&candidate.transactions)
                );
                assert_eq!(
                    candidate.header.witness_root,
                    witness_root(&candidate.transactions)
                );
            }
            5 => {
                let address =
                    address_parts_from_public_key(Network::Mainnet, &[1u8; 32]).base56_address;
                let (digest, network) = decode_base56_address(&address).unwrap();
                assert_eq!(network, Network::Mainnet);
                assert!(address.len() > 10);
                assert_eq!(digest.len(), 32);
            }
            _ => {
                let mut block = base_block.clone();
                block.header.timestamp = 75;
                assert_eq!(
                    block.header.canonical_bytes().len(),
                    base_block.header.canonical_bytes().len()
                );
            }
        }));
        match outcome {
            Ok(()) => report.pass(),
            Err(_) => {
                report.fail_panic(format!("case={i} kind={kind} panic in determinism attack"))
            }
        }
    }
    Ok(report)
}

fn network_protocol_attack(cases: usize, seed: u64) -> Result<CategoryReport, String> {
    let mut report = CategoryReport::new("network_protocol_chaos");
    for i in 0..cases {
        report.cases += 1;
        let kind = hash_seed(seed, i as u64) as usize % 8;
        let outcome = catch_unwind(AssertUnwindSafe(|| match kind {
            0 => {
                assert_eq!(
                    NetworkMessage::decode(Network::Mainnet, MessageCommand::GetAddr, &[1]),
                    Err(ProtocolError::UnexpectedPayload)
                );
            }
            1 => {
                let addresses = (0..=p2p_network_params(Network::Mainnet)
                    .limits
                    .max_addr_per_message)
                    .map(|index| P2pPeerAddress {
                        host: format!("203.0.113.{}", index % 255),
                        port: 56000,
                        services: 0,
                        last_seen_unix: 1_700_000_000,
                    })
                    .collect::<Vec<_>>();
                let payload = bincode::DefaultOptions::new()
                    .serialize(&addresses)
                    .expect("serialize addr mutation");
                assert_eq!(
                    NetworkMessage::decode(Network::Mainnet, MessageCommand::Addr, &payload),
                    Err(ProtocolError::TooManyPeerAddresses)
                );
            }
            2 => {
                let message = NetworkMessage::new(
                    Network::Mainnet,
                    MessagePayload::GetHeaders(GetHeadersMessage {
                        locator_hashes: vec![P2pHash48::ZERO; 33],
                        stop_hash: P2pHash48::ZERO,
                    }),
                );
                assert_eq!(
                    message.encode_payload(),
                    Err(ProtocolError::TooManyLocatorHashes)
                );
            }
            3 => {
                let block = valid_block(
                    Network::Mainnet,
                    1,
                    vec![valid_coinbase(Network::Mainnet, 1, 0)],
                );
                let message = NetworkMessage::new(
                    Network::Mainnet,
                    MessagePayload::CompactBlock(CompactBlockMessage {
                        header: block.header,
                        tx_count: 0,
                        short_ids: Vec::new(),
                        prefilled_transactions: Vec::new(),
                        fees_total_atoms: 0,
                        fees_miner_atoms: 0,
                    }),
                );
                assert_eq!(
                    message.encode_payload(),
                    Err(ProtocolError::InvalidCompactBlock)
                );
            }
            4 => {
                let version = P2pVersionMessage {
                    protocol_version: rules::PROTOCOL_VERSION,
                    min_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
                    services: LOCAL_NODE_SERVICES,
                    timestamp_unix: 1_700_000_000,
                    network: Network::Testnet,
                    user_agent: String::from("/Atho:0.1.0/"),
                    best_height: 0,
                    ruleset_version: rules::RULESET_VERSION_V1,
                    relay: true,
                    genesis_hash: P2pHash48::from(genesis::genesis_hash(Network::Testnet)),
                    tip_hash: P2pHash48::ZERO,
                    chainwork: P2pHash48::ZERO,
                };
                assert_eq!(
                    validate_version_message(&version, Network::Mainnet),
                    Err(ProtocolError::UnsupportedNetwork)
                );
            }
            5 => {
                let version = P2pVersionMessage {
                    protocol_version: 0,
                    min_protocol_version: 0,
                    services: LOCAL_NODE_SERVICES,
                    timestamp_unix: 1_700_000_000,
                    network: Network::Mainnet,
                    user_agent: String::from("/Atho:0.1.0/"),
                    best_height: 0,
                    ruleset_version: rules::RULESET_VERSION_V1,
                    relay: true,
                    genesis_hash: P2pHash48::from(genesis::genesis_hash(Network::Mainnet)),
                    tip_hash: P2pHash48::ZERO,
                    chainwork: P2pHash48::ZERO,
                };
                assert_eq!(
                    validate_version_message(&version, Network::Mainnet),
                    Err(ProtocolError::UnsupportedProtocolVersion)
                );
            }
            6 => {
                let version = P2pVersionMessage {
                    protocol_version: rules::PROTOCOL_VERSION,
                    min_protocol_version: MIN_SUPPORTED_PROTOCOL_VERSION,
                    services: LOCAL_NODE_SERVICES,
                    timestamp_unix: 1_700_000_000,
                    network: Network::Mainnet,
                    user_agent: "A".repeat(
                        p2p_network_params(Network::Mainnet)
                            .limits
                            .max_user_agent_bytes
                            + 1,
                    ),
                    best_height: 0,
                    ruleset_version: rules::RULESET_VERSION_V1,
                    relay: true,
                    genesis_hash: P2pHash48::from(genesis::genesis_hash(Network::Mainnet)),
                    tip_hash: P2pHash48::ZERO,
                    chainwork: P2pHash48::ZERO,
                };
                assert_eq!(
                    validate_version_message(&version, Network::Mainnet),
                    Err(ProtocolError::UserAgentTooLong)
                );
            }
            _ => {
                assert_eq!(
                    NetworkMessage::decode(Network::Mainnet, MessageCommand::Ping, &[]),
                    Err(ProtocolError::MalformedPayload)
                );
            }
        }));
        match outcome {
            Ok(()) => report.pass(),
            Err(_) => report.fail_panic(format!(
                "case={i} kind={kind} panic in network protocol chaos"
            )),
        }
    }
    Ok(report)
}

fn check_validation_result(
    report: &mut CategoryReport,
    case: usize,
    where_: &str,
    expected_ok: bool,
    actual: Result<(), ValidationError>,
) {
    match (expected_ok, actual.is_ok()) {
        (true, true) | (false, false) => report.pass(),
        (true, false) => {
            report.fail_reject(format!("case={case} {where_} valid input rejected"));
        }
        (false, true) => {
            report.fail_accept(format!("case={case} {where_} invalid input accepted"));
        }
    }
}

fn check_validation_result_u64(
    report: &mut CategoryReport,
    case: usize,
    where_: &str,
    expected_ok: bool,
    actual: Result<u64, ValidationError>,
) {
    match (expected_ok, actual.is_ok()) {
        (true, true) | (false, false) => report.pass(),
        (true, false) => {
            report.fail_reject(format!("case={case} {where_} valid input rejected"));
        }
        (false, true) => {
            report.fail_accept(format!("case={case} {where_} invalid input accepted"));
        }
    }
}

fn cleanup_dev_layout() {
    let _ = fs::remove_dir_all("dev");
}

fn write_legacy_snapshot_files(
    root: &Path,
    valid_state: bool,
    malformed_utxo: bool,
) -> Result<(), String> {
    let chain_dir = root.join("dev").join("chain");
    fs::create_dir_all(&chain_dir).map_err(|err| err.to_string())?;
    let state_path = chain_dir.join("chainstate-atho-mainnet.tsv");
    let utxo_path = chain_dir.join("utxos-atho-mainnet.tsv");
    if valid_state {
        fs::write(
            &state_path,
            "height\ttip_hash\n42\t030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303\n",
        )
        .map_err(|err| err.to_string())?;
    } else {
        fs::write(&state_path, "height\ttip_hash\n42\tnot-hex\n").map_err(|err| err.to_string())?;
    }
    if malformed_utxo {
        fs::write(
            &utxo_path,
            "txid\toutput_index\tvalue_atoms\tlocking_script_hex\tcreated_height\tis_coinbase\n\
             010101\t0\t100\tzzzz\t1\t0\n",
        )
        .map_err(|err| err.to_string())?;
    } else {
        fs::write(
            &utxo_path,
            "txid\toutput_index\tvalue_atoms\tlocking_script_hex\tcreated_height\tis_coinbase\n\
             010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101\t0\t100\t01\t1\t0\n",
        )
        .map_err(|err| err.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{persistence_data_dir, EnvVarGuard};
    use atho_storage::path::{chain_dir, data_root, ATHO_DATA_DIR_ENV};
    use std::path::PathBuf;

    #[test]
    fn persistence_attack_uses_explicit_data_root() {
        let root = PathBuf::from("/tmp/atho-adversarial-test-root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &persistence_data_dir(&root));
        assert_eq!(data_root(), root.join("dev"));
        assert_eq!(chain_dir(), root.join("dev").join("chain"));
    }
}
