use crate::config::NodeConfig;
use crate::mempool::MempoolEntry;
use crate::miner::Miner;
use crate::node::Node;
use crate::validation::{derive_sig_ref_short, derive_witness_commit_ref};
use atho_core::block::Block;
use atho_core::consensus::pow::{clamp_target, initial_target_for_network, DIFFICULTY_PROFILE};
use atho_core::consensus::signatures::{transaction_signing_digest, AthoSignatureDomain};
use atho_core::constants::MIN_TX_FEE_PER_VBYTE_ATOMS;
use atho_core::network::Network;
use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness};
use atho_crypto::falcon::{generate_from_seed, sign, FalconKeypair};
use atho_errors::AthoError;
use atho_p2p::relay::RelayLoop;
use atho_storage::chainstate::Chainstate;
use atho_storage::utxo::UtxoEntry;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

const ACTIVITY_LOG: &str = "activity.log";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityLine {
    pub timestamp: String,
    pub component: String,
    pub line: String,
}

fn dev_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub fn dev_root() -> PathBuf {
    atho_storage::path::sandbox_root()
}

pub fn logs_dir() -> PathBuf {
    dev_root().join("logs")
}

pub fn chain_dir() -> PathBuf {
    dev_root().join("chain")
}

pub fn wallet_dir() -> PathBuf {
    dev_root().join("wallet")
}

pub fn db_dir() -> PathBuf {
    dev_root().join("db")
}

pub fn audit_dir() -> PathBuf {
    dev_root().join("audit")
}

fn chain_blocks_file() -> PathBuf {
    chain_dir().join("blocks.tsv")
}

fn chain_transactions_file() -> PathBuf {
    chain_dir().join("transactions.tsv")
}

fn chain_inputs_file() -> PathBuf {
    chain_dir().join("transaction_inputs.tsv")
}

fn chain_outputs_file() -> PathBuf {
    chain_dir().join("transaction_outputs.tsv")
}

pub fn ensure_layout() -> std::io::Result<()> {
    let _guard = dev_lock().lock().expect("dev lock poisoned");
    ensure_layout_locked()
}

fn ensure_layout_locked() -> std::io::Result<()> {
    ensure_layout_for(&dev_root())
}

fn ensure_layout_for(root: &Path) -> std::io::Result<()> {
    fs::create_dir_all(root.join("logs"))?;
    fs::create_dir_all(root.join("chain"))?;
    fs::create_dir_all(root.join("wallet"))?;
    fs::create_dir_all(root.join("db"))?;
    fs::create_dir_all(root.join("audit"))?;
    fs::create_dir_all(root.join("quarantine"))?;
    Ok(())
}

pub fn append_log(component: &str, line: &str) -> std::io::Result<()> {
    let _guard = dev_lock().lock().expect("dev lock poisoned");
    append_log_locked(component, line)
}

pub fn append_atho_error(component: &str, error: &AthoError) -> std::io::Result<()> {
    append_log(component, &error.log_line())
}

fn append_log_locked(component: &str, line: &str) -> std::io::Result<()> {
    ensure_layout_locked()?;
    let component_path = logs_dir().join(format!("{component}.log"));
    append_line(&component_path, line)?;
    let activity_path = logs_dir().join(ACTIVITY_LOG);
    append_line(
        &activity_path,
        &format!("{}|{}|{}", activity_timestamp(), component, line),
    )?;
    Ok(())
}

pub fn summarize_transaction(tx: &Transaction, fee_atoms: Option<u64>) -> String {
    let txid = hex::encode(tx.txid());
    let wtxid = hex::encode(tx.wtxid());
    let size_bytes = tx.full_size_bytes();
    let weight_bytes = tx.weight_bytes();
    let vsize_bytes = tx.vsize_bytes();
    let witness_bytes = tx.witness_bytes();
    let output_total_atoms = tx.output_value_atoms();
    let fee_text = fee_atoms
        .map(|fee| fee.to_string())
        .unwrap_or_else(|| String::from("n/a"));
    format!(
        "txid={txid} wtxid={wtxid} version={} inputs={} outputs={} lock_time={} size={} weight={} vsize={} witness_bytes={} output_total_atoms={} fee_atoms={}",
        tx.version,
        tx.inputs.len(),
        tx.outputs.len(),
        tx.lock_time,
        size_bytes,
        weight_bytes,
        vsize_bytes,
        witness_bytes,
        output_total_atoms,
        fee_text
    )
}

pub fn summarize_block(block: &Block) -> String {
    format!(
        "hash={} height={} prev={} merkle={} witness={} txs={} size={} weight={} vsize={} nonce={} target={} fees_total={} fees_miner={} fees_burned={} fees_pool={}",
        hex::encode(block.header.block_hash()),
        block.header.height,
        hex::encode(block.header.previous_block_hash),
        hex::encode(block.header.merkle_root),
        hex::encode(block.header.witness_root),
        block.transactions.len(),
        block.size_bytes(),
        block.weight_bytes(),
        block.vsize_bytes(),
        block.header.nonce,
        hex::encode(block.header.difficulty_target_or_bits),
        block.fees_total_atoms,
        block.fees_miner_atoms,
        block.fees_burned_atoms,
        block.fees_pool_atoms
    )
}

fn append_line(path: &PathBuf, line: &str) -> std::io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")?;
    Ok(())
}

fn activity_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}", now.as_secs(), now.subsec_millis())
}

pub fn record_block(height: u64, block: &Block) -> std::io::Result<()> {
    let _guard = dev_lock().lock().expect("dev lock poisoned");
    ensure_layout_locked()?;
    append_log_locked(
        "chain",
        &format!("block accepted {}", summarize_block(block)),
    )?;
    for (index, tx) in block.transactions.iter().enumerate() {
        append_log_locked(
            "chain",
            &format!("block tx index={index} {}", summarize_transaction(tx, None)),
        )?;
    }
    append_block_row(&chain_blocks_file(), height, block)?;
    append_transaction_rows(height, block)?;
    Ok(())
}

pub fn wipe_chain_and_keys() -> std::io::Result<()> {
    let _guard = dev_lock().lock().expect("dev lock poisoned");
    wipe_root_locked(&dev_root())
}

pub fn wipe_root(root: &Path) -> std::io::Result<()> {
    let _guard = dev_lock().lock().expect("dev lock poisoned");
    wipe_root_locked(root)
}

fn wipe_root_locked(root: &Path) -> std::io::Result<()> {
    remove_tree(&root.join("chain"))?;
    remove_tree(&root.join("wallet"))?;
    remove_tree(&root.join("db"))?;
    remove_tree(&root.join("audit"))?;
    remove_tree(&root.join("logs"))?;
    remove_tree(&root.join("quarantine"))?;
    ensure_layout_for(root)
}

fn remove_tree(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_file() || path.is_symlink() {
        fs::remove_file(path)?;
        return Ok(());
    }
    for _ in 0..8 {
        match fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(err)
                if err.kind() == std::io::ErrorKind::DirectoryNotEmpty
                    || err.kind() == std::io::ErrorKind::PermissionDenied =>
            {
                thread::sleep(Duration::from_millis(10));
            }
            Err(err) => return Err(err),
        }
    }
    fs::remove_dir_all(path)
}

pub fn export_chain(_chainstate: &Chainstate) -> std::io::Result<PathBuf> {
    let _guard = dev_lock().lock().expect("dev lock poisoned");
    ensure_layout_locked()?;
    let path = audit_dir().join("chain.tsv");
    copy_or_init(
        &chain_blocks_file(),
        &path,
        "height\tblock_hash\tprevious_block_hash\tmerkle_root\twitness_root\ttimestamp\ttarget\tnonce\ttx_count\tsize_bytes\tweight_bytes\tvsize_bytes\tfees_total_atoms\tfees_miner_atoms\tfees_burned_atoms\tfees_pool_atoms",
    )?;
    Ok(path)
}

pub fn export_transactions(_chainstate: &Chainstate) -> std::io::Result<PathBuf> {
    let _guard = dev_lock().lock().expect("dev lock poisoned");
    ensure_layout_locked()?;
    let path = audit_dir().join("transactions.tsv");
    copy_or_init(&chain_transactions_file(), &path, "height\tblock_hash\ttx_index\ttxid\twtxid\tversion\tlock_time\tinput_count\toutput_count\tsize_bytes\tweight_bytes\tvsize_bytes\twitness_bytes\toutput_value_atoms\tcanonical_bytes_hex")?;
    Ok(path)
}

pub fn export_transaction_details(_chainstate: &Chainstate) -> std::io::Result<(PathBuf, PathBuf)> {
    let _guard = dev_lock().lock().expect("dev lock poisoned");
    ensure_layout_locked()?;
    let inputs_path = audit_dir().join("transaction_inputs.tsv");
    let outputs_path = audit_dir().join("transaction_outputs.tsv");
    copy_or_init(&chain_inputs_file(), &inputs_path, "height\tblock_hash\ttx_index\tinput_index\tprevious_txid\toutput_index\tunlocking_script_hex")?;
    copy_or_init(
        &chain_outputs_file(),
        &outputs_path,
        "height\tblock_hash\ttx_index\toutput_index\tvalue_atoms\tlocking_script_hex",
    )?;
    Ok((inputs_path, outputs_path))
}

pub fn publish_audit_exports() -> std::io::Result<(PathBuf, PathBuf, PathBuf, PathBuf)> {
    let _guard = dev_lock().lock().expect("dev lock poisoned");
    ensure_layout_locked()?;
    let chain = audit_dir().join("chain.tsv");
    let txs = audit_dir().join("transactions.tsv");
    let inputs = audit_dir().join("transaction_inputs.tsv");
    let outputs = audit_dir().join("transaction_outputs.tsv");
    copy_or_init(
        &chain_blocks_file(),
        &chain,
        "height\tblock_hash\tprevious_block_hash\tmerkle_root\twitness_root\ttimestamp\ttarget\tnonce\ttx_count\tsize_bytes\tweight_bytes\tvsize_bytes\tfees_total_atoms\tfees_miner_atoms\tfees_burned_atoms\tfees_pool_atoms",
    )?;
    copy_or_init(&chain_transactions_file(), &txs, "height\tblock_hash\ttx_index\ttxid\twtxid\tversion\tlock_time\tinput_count\toutput_count\tsize_bytes\tweight_bytes\tvsize_bytes\twitness_bytes\toutput_value_atoms\tcanonical_bytes_hex")?;
    copy_or_init(&chain_inputs_file(), &inputs, "height\tblock_hash\ttx_index\tinput_index\tprevious_txid\toutput_index\tunlocking_script_hex")?;
    copy_or_init(
        &chain_outputs_file(),
        &outputs,
        "height\tblock_hash\ttx_index\toutput_index\tvalue_atoms\tlocking_script_hex",
    )?;
    Ok((chain, txs, inputs, outputs))
}

pub fn mine_once(network: Network) -> std::io::Result<PathBuf> {
    let _guard = dev_lock().lock().expect("dev lock poisoned");
    ensure_layout_locked()?;
    let target = initial_target_for_network(network);
    let clamped_target = clamp_target(target);
    append_log_locked(
        "p2p",
        &format!(
            "dev mine network={} target={} min={} max={} tx_alloc_bps={}",
            network.id(),
            hex::encode(clamped_target),
            hex::encode(DIFFICULTY_PROFILE.min_difficulty_target),
            hex::encode(DIFFICULTY_PROFILE.max_difficulty_target),
            DIFFICULTY_PROFILE.standard_transaction_allocation_bps
        ),
    )?;

    let mut node = Node::load_or_new(NodeConfig::new(network));
    let (seed_txid, seed_value, seed_script) = seed_utxo(network);
    node.dev_seed_chainstate(
        6,
        node.tip_hash(),
        [UtxoEntry::new(
            network,
            seed_txid,
            0,
            seed_value,
            vec![seed_script],
            0,
            false,
        )],
    )
    .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err.to_string()))?;

    let tx = signed_spend_transaction(network, seed_txid, seed_value, seed_script)?;
    let tx_fee = tx.vsize_bytes() as u64 * MIN_TX_FEE_PER_VBYTE_ATOMS;

    let txid = node
        .admit_transaction(MempoolEntry::new(tx, tx_fee))
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err.to_string()))?;

    let miner = Miner::new(4);
    let block = node
        .mine_and_connect_candidate_block(&miner)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err.to_string()))?;

    let mut relay = RelayLoop::new(network);
    relay.prime(node.blocks());
    let _ = relay.relay_transaction(&txid);
    let _ = relay.relay_block(&block.header.block_hash(), block.transactions.len());
    append_log_locked(
        "athod",
        &format!(
            "dev mine connected network={} block={} txid={}",
            network.id(),
            hex::encode(block.header.block_hash()),
            hex::encode(txid)
        ),
    )?;
    publish_audit_exports()?;
    Ok(audit_dir().join("chain.tsv"))
}

pub(crate) fn signed_spend_transaction(
    network: Network,
    seed_txid: [u8; 48],
    seed_value: u64,
    seed_script: u8,
) -> std::io::Result<Transaction> {
    let keypair = signing_keypair(network, seed_txid, seed_script)?;
    let mut output_atoms = seed_value.saturating_sub(1);
    let mut last_fee = 0u64;
    for _ in 0..4 {
        let mut tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: seed_txid,
                output_index: 0,
                unlocking_script: vec![seed_script],
            }],
            outputs: vec![TxOutput {
                value_atoms: output_atoms,
                locking_script: vec![seed_script.saturating_add(1)],
            }],
            lock_time: 0,
            witness: vec![],
        };
        let digest = transaction_signing_digest(&tx);
        let signature = sign(
            AthoSignatureDomain::Transaction,
            &keypair.secret_key,
            &digest,
        )
        .map_err(|err: atho_crypto::error::CryptoError| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("falcon-512 rs sign failed: {err:?}"),
            )
        })?;
        let txid = tx.txid();
        let sig_bytes = signature.0.clone();
        tx.witness = TxWitness {
            signature: sig_bytes.clone(),
            pubkey: keypair.public_key.0.clone(),
            input_refs: vec![atho_core::transaction::WitnessInputRef {
                sig_ref_short: derive_sig_ref_short(&txid, &sig_bytes, 0),
                witness_commit_ref: [0; 16],
            }],
        }
        .canonical_bytes();
        let witness_root = tx.witness_commitment_hash();
        tx.witness = TxWitness {
            signature: sig_bytes.clone(),
            pubkey: keypair.public_key.0.clone(),
            input_refs: vec![atho_core::transaction::WitnessInputRef {
                sig_ref_short: derive_sig_ref_short(&txid, &sig_bytes, 0),
                witness_commit_ref: derive_witness_commit_ref(&txid, &witness_root, 0),
            }],
        }
        .canonical_bytes();
        let fee = tx.vsize_bytes() as u64 * MIN_TX_FEE_PER_VBYTE_ATOMS;
        if fee == last_fee {
            if seed_value < fee {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "seed utxo too small for fee",
                ));
            }
            tx.outputs[0].value_atoms = seed_value - fee;
            return Ok(tx);
        }
        last_fee = fee;
        if seed_value < fee {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "seed utxo too small for fee",
            ));
        }
        output_atoms = seed_value - fee;
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::Other,
        "failed to stabilize dev spend fee",
    ))
}

fn signing_keypair(
    network: Network,
    seed_txid: [u8; 48],
    seed_script: u8,
) -> std::io::Result<FalconKeypair> {
    let mut seed = Vec::with_capacity(network.id().len() + seed_txid.len() + 1);
    seed.extend_from_slice(network.id().as_bytes());
    seed.extend_from_slice(&seed_txid);
    seed.push(seed_script);
    generate_from_seed(&seed).map_err(|err| {
        std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("falcon-512 rs keygen failed: {err:?}"),
        )
    })
}

pub(crate) fn seed_utxo(network: Network) -> ([u8; 48], u64, u8) {
    match network {
        Network::Mainnet => ([0x11; 48], 2_000, 0x11),
        Network::Testnet => ([0x22; 48], 1_500, 0x22),
        Network::Regnet => ([0x33; 48], 1_000, 0x33),
        Network::Prunetest => ([0x44; 48], 750, 0x44),
    }
}

fn copy_or_init(source: &PathBuf, target: &PathBuf, header: &str) -> std::io::Result<()> {
    if source.exists() {
        fs::copy(source, target)?;
        return Ok(());
    }
    let mut file = File::create(target)?;
    writeln!(file, "{header}")?;
    Ok(())
}

fn append_block_row(path: &PathBuf, height: u64, block: &Block) -> std::io::Result<()> {
    let exists = path.exists();
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    if !exists {
        writeln!(
            file,
            "height\tblock_hash\tprevious_block_hash\tmerkle_root\twitness_root\ttimestamp\ttarget\tnonce\ttx_count\tsize_bytes\tweight_bytes\tvsize_bytes\tfees_total_atoms\tfees_miner_atoms\tfees_burned_atoms\tfees_pool_atoms"
        )?;
    }
    writeln!(
        file,
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        height,
        hex::encode(block.header.block_hash()),
        hex::encode(block.header.previous_block_hash),
        hex::encode(block.header.merkle_root),
        hex::encode(block.header.witness_root),
        block.header.timestamp,
        hex::encode(block.header.difficulty_target_or_bits),
        block.header.nonce,
        block.transactions.len(),
        block.size_bytes(),
        block.weight_bytes(),
        block.vsize_bytes(),
        block.fees_total_atoms,
        block.fees_miner_atoms,
        block.fees_burned_atoms,
        block.fees_pool_atoms
    )?;
    Ok(())
}

fn append_transaction_rows(height: u64, block: &Block) -> std::io::Result<()> {
    append_tx_rows(&chain_transactions_file(), height, block)?;
    append_input_rows(&chain_inputs_file(), height, block)?;
    append_output_rows(&chain_outputs_file(), height, block)
}

fn append_tx_rows(path: &PathBuf, height: u64, block: &Block) -> std::io::Result<()> {
    let exists = path.exists();
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    if !exists {
        writeln!(
            file,
            "height\tblock_hash\ttx_index\ttxid\twtxid\tversion\tlock_time\tinput_count\toutput_count\tsize_bytes\tweight_bytes\tvsize_bytes\twitness_bytes\toutput_value_atoms\tcanonical_bytes_hex"
        )?;
    }
    let block_hash = hex::encode(block.header.block_hash());
    for (tx_index, tx) in block.transactions.iter().enumerate() {
        writeln!(
            file,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            height,
            block_hash,
            tx_index,
            hex::encode(tx.txid()),
            hex::encode(tx.wtxid()),
            tx.version,
            tx.lock_time,
            tx.inputs.len(),
            tx.outputs.len(),
            tx.full_size_bytes(),
            tx.weight_bytes(),
            tx.vsize_bytes(),
            tx.witness_bytes(),
            tx.output_value_atoms(),
            hex::encode(tx.canonical_bytes())
        )?;
    }
    Ok(())
}

fn append_input_rows(path: &PathBuf, height: u64, block: &Block) -> std::io::Result<()> {
    let exists = path.exists();
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    if !exists {
        writeln!(
            file,
            "height\tblock_hash\ttx_index\tinput_index\tprevious_txid\toutput_index\tunlocking_script_hex"
        )?;
    }
    let block_hash = hex::encode(block.header.block_hash());
    for (tx_index, tx) in block.transactions.iter().enumerate() {
        for (input_index, input) in tx.inputs.iter().enumerate() {
            writeln!(
                file,
                "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                height,
                block_hash,
                tx_index,
                input_index,
                hex::encode(input.previous_txid),
                input.output_index,
                hex::encode(&input.unlocking_script)
            )?;
        }
    }
    Ok(())
}

fn append_output_rows(path: &PathBuf, height: u64, block: &Block) -> std::io::Result<()> {
    let exists = path.exists();
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    if !exists {
        writeln!(
            file,
            "height\tblock_hash\ttx_index\toutput_index\tvalue_atoms\tlocking_script_hex"
        )?;
    }
    let block_hash = hex::encode(block.header.block_hash());
    for (tx_index, tx) in block.transactions.iter().enumerate() {
        for (output_index, output) in tx.outputs.iter().enumerate() {
            writeln!(
                file,
                "{}\t{}\t{}\t{}\t{}\t{}",
                height,
                block_hash,
                tx_index,
                output_index,
                output.value_atoms,
                hex::encode(&output.locking_script)
            )?;
        }
    }
    Ok(())
}

pub fn watch_logs() -> std::io::Result<()> {
    ensure_layout()?;
    let path = logs_dir().join(ACTIVITY_LOG);
    let mut offset = 0u64;
    loop {
        if let Ok(mut file) = File::open(&path) {
            file.seek(SeekFrom::Start(offset))?;
            let mut reader = BufReader::new(file);
            let mut line = String::new();
            while reader.read_line(&mut line)? > 0 {
                print!("{}", line);
                offset += line.len() as u64;
                line.clear();
            }
        }
        thread::sleep(Duration::from_millis(500));
    }
}

pub fn recent_activity_lines(limit: usize) -> std::io::Result<Vec<ActivityLine>> {
    ensure_layout()?;
    let path = logs_dir().join(ACTIVITY_LOG);
    if !path.exists() || limit == 0 {
        return Ok(Vec::new());
    }

    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut offset = reader.get_ref().metadata()?.len();
    let mut buffer = Vec::new();
    let mut lines = std::collections::VecDeque::with_capacity(limit);

    while offset > 0 && lines.len() < limit {
        let step = offset.min(8 * 1024);
        offset -= step;
        reader.seek(SeekFrom::Start(offset))?;
        buffer.resize(step as usize, 0);
        reader.read_exact(&mut buffer)?;
        let chunk = String::from_utf8_lossy(&buffer);
        for raw in chunk.lines().rev() {
            if let Some(line) = parse_activity_line(raw.trim_end_matches('\r')) {
                lines.push_front(line);
                if lines.len() == limit {
                    break;
                }
            }
        }
        if offset == 0 {
            break;
        }
    }

    if lines.len() < limit {
        let file = File::open(logs_dir().join(ACTIVITY_LOG))?;
        let reader = BufReader::new(file);
        lines.clear();
        for raw in reader.lines() {
            let raw = raw?;
            if let Some(line) = parse_activity_line(&raw) {
                if lines.len() == limit {
                    let _ = lines.pop_front();
                }
                lines.push_back(line);
            }
        }
    }

    Ok(lines.into_iter().collect())
}

fn parse_activity_line(line: &str) -> Option<ActivityLine> {
    let mut parts = line.splitn(3, '|');
    let timestamp = parts.next()?.to_string();
    let component = parts.next()?.to_string();
    let message = parts.next()?.to_string();
    Some(ActivityLine {
        timestamp,
        component,
        line: message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::acquire_global_test_lock;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn dev_layout_can_be_created_and_wiped() {
        let _lock = acquire_global_test_lock();
        ensure_layout().unwrap();
        append_log("athod", "test line").unwrap();
        wipe_chain_and_keys().unwrap();
        assert!(logs_dir().exists());
        assert!(!chain_dir().exists() || chain_dir().is_dir());
        assert!(!wallet_dir().exists() || wallet_dir().is_dir());
    }

    #[test]
    fn wipe_root_recreates_an_explicit_sandbox_tree() {
        let _lock = acquire_global_test_lock();
        let root = std::env::temp_dir().join(format!(
            "atho-dev-wipe-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(root.join("logs")).expect("logs");
        fs::write(root.join("logs").join("athod.log"), "temp").expect("log");
        fs::create_dir_all(root.join("db").join("nested")).expect("db");

        wipe_root(&root).expect("wipe");

        assert!(root.join("logs").exists());
        assert!(root.join("chain").exists());
        assert!(root.join("wallet").exists());
        assert!(root.join("db").exists());
        assert!(root.join("audit").exists());
        assert!(root.join("quarantine").exists());
        assert!(!root.join("logs").join("athod.log").exists());
    }
}
