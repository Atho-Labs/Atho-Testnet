use crate::config::NodeConfig;
use crate::mempool::MempoolEntry;
use crate::miner::Miner;
use crate::node::Node;
use crate::validation::encode_input_reference;
use atho_core::block::Block;
use atho_core::consensus::pow::{clamp_target, initial_target_for_network, DIFFICULTY_PROFILE};
use atho_core::network::Network;
use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness};
use atho_crypto::falcon::{generate_from_seed, sign, FalconKeypair};
use atho_p2p::relay::RelayLoop;
use atho_storage::chainstate::Chainstate;
use atho_storage::utxo::UtxoEntry;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

const DEV_ROOT: &str = "dev";

fn dev_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub fn dev_root() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(DEV_ROOT)
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
    fs::create_dir_all(logs_dir())?;
    fs::create_dir_all(chain_dir())?;
    fs::create_dir_all(wallet_dir())?;
    fs::create_dir_all(audit_dir())?;
    Ok(())
}

pub fn append_log(component: &str, line: &str) -> std::io::Result<()> {
    let _guard = dev_lock().lock().expect("dev lock poisoned");
    ensure_layout()?;
    let path = logs_dir().join(format!("{component}.log"));
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")?;
    Ok(())
}

pub fn record_block(height: u64, block: &Block) -> std::io::Result<()> {
    let _guard = dev_lock().lock().expect("dev lock poisoned");
    ensure_layout()?;
    append_block_row(&chain_blocks_file(), height, block)?;
    append_transaction_rows(height, block)?;
    Ok(())
}

pub fn wipe_chain_and_keys() -> std::io::Result<()> {
    let _guard = dev_lock().lock().expect("dev lock poisoned");
    if chain_dir().exists() {
        fs::remove_dir_all(chain_dir())?;
    }
    if wallet_dir().exists() {
        fs::remove_dir_all(wallet_dir())?;
    }
    if audit_dir().exists() {
        fs::remove_dir_all(audit_dir())?;
    }
    ensure_layout()
}

pub fn export_chain(_chainstate: &Chainstate) -> std::io::Result<PathBuf> {
    let _guard = dev_lock().lock().expect("dev lock poisoned");
    ensure_layout()?;
    let path = audit_dir().join("chain.tsv");
    copy_or_init(
        &chain_blocks_file(),
        &path,
        "height\tblock_hash\tprevious_block_hash\tmerkle_root\ttimestamp\ttarget\tnonce\ttx_count",
    )?;
    Ok(path)
}

pub fn export_transactions(_chainstate: &Chainstate) -> std::io::Result<PathBuf> {
    let _guard = dev_lock().lock().expect("dev lock poisoned");
    ensure_layout()?;
    let path = audit_dir().join("transactions.tsv");
    copy_or_init(&chain_transactions_file(), &path, "height\tblock_hash\ttx_index\ttxid\tversion\tlock_time\tinput_count\toutput_count\tcanonical_bytes_hex")?;
    Ok(path)
}

pub fn export_transaction_details(_chainstate: &Chainstate) -> std::io::Result<(PathBuf, PathBuf)> {
    let _guard = dev_lock().lock().expect("dev lock poisoned");
    ensure_layout()?;
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
    ensure_layout()?;
    let chain = audit_dir().join("chain.tsv");
    let txs = audit_dir().join("transactions.tsv");
    let inputs = audit_dir().join("transaction_inputs.tsv");
    let outputs = audit_dir().join("transaction_outputs.tsv");
    copy_or_init(
        &chain_blocks_file(),
        &chain,
        "height\tblock_hash\tprevious_block_hash\tmerkle_root\ttimestamp\ttarget\tnonce\ttx_count",
    )?;
    copy_or_init(&chain_transactions_file(), &txs, "height\tblock_hash\ttx_index\ttxid\tversion\tlock_time\tinput_count\toutput_count\tcanonical_bytes_hex")?;
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
    ensure_layout()?;
    let target = initial_target_for_network(network);
    let clamped_target = clamp_target(target);
    append_log(
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

    let mut node = Node::new(NodeConfig::new(network));
    let (seed_txid, seed_value, seed_script) = seed_utxo(network);
    node.chainstate
        .insert_utxo(UtxoEntry {
            network,
            txid: seed_txid,
            output_index: 0,
            value_atoms: seed_value,
            locking_script: vec![seed_script],
        })
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err.to_string()))?;

    let tx = signed_spend_transaction(network, seed_txid, seed_value, seed_script)?;

    let txid = node
        .admit_transaction(MempoolEntry {
            transaction: tx,
            fee_atoms: 500,
        })
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err.to_string()))?;

    let miner = Miner::new(4);
    let block = node
        .mine_and_connect_candidate_block(&miner)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err.to_string()))?;

    let relay = RelayLoop::new(network);
    relay.prime();
    relay.sync_headers(node.chainstate.height);
    relay.relay_transaction(&txid);
    relay.relay_block(&block.header.block_hash(), block.transactions.len());
    append_log(
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

fn signed_spend_transaction(
    network: Network,
    seed_txid: [u8; 48],
    seed_value: u64,
    seed_script: u8,
) -> std::io::Result<Transaction> {
    let keypair = signing_keypair(network, seed_txid, seed_script)?;
    let mut tx = Transaction {
        version: 1,
        inputs: vec![TxInput {
            previous_txid: seed_txid,
            output_index: 0,
            unlocking_script: vec![seed_script],
        }],
        outputs: vec![TxOutput {
            value_atoms: seed_value.saturating_sub(500),
            locking_script: vec![seed_script.saturating_add(1)],
        }],
        lock_time: 0,
        witness: vec![],
    };
    let digest = tx.signing_digest();
    let signature = sign(&keypair.secret_key, &digest).map_err(|err| {
        std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("falcon sign failed: {err:?}"),
        )
    })?;
    tx.witness = TxWitness {
        signature: signature.0,
        pubkey: keypair.public_key.0,
        input_refs: vec![encode_input_reference(&seed_txid, 0)],
    }
    .canonical_bytes();
    Ok(tx)
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
            format!("falcon keygen failed: {err:?}"),
        )
    })
}

fn seed_utxo(network: Network) -> ([u8; 48], u64, u8) {
    match network {
        Network::Mainnet => ([0x11; 48], 2_000, 0x11),
        Network::Testnet => ([0x22; 48], 1_500, 0x22),
        Network::Regnet => ([0x33; 48], 1_000, 0x33),
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
            "height\tblock_hash\tprevious_block_hash\tmerkle_root\ttimestamp\ttarget\tnonce\ttx_count"
        )?;
    }
    writeln!(
        file,
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        height,
        hex::encode(block.header.block_hash()),
        hex::encode(block.header.previous_block_hash),
        hex::encode(block.header.merkle_root),
        block.header.timestamp,
        hex::encode(block.header.difficulty_target_or_bits),
        block.header.nonce,
        block.transactions.len()
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
            "height\tblock_hash\ttx_index\ttxid\tversion\tlock_time\tinput_count\toutput_count\tcanonical_bytes_hex"
        )?;
    }
    let block_hash = hex::encode(block.header.block_hash());
    for (tx_index, tx) in block.transactions.iter().enumerate() {
        writeln!(
            file,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            height,
            block_hash,
            tx_index,
            hex::encode(tx.txid()),
            tx.version,
            tx.lock_time,
            tx.inputs.len(),
            tx.outputs.len(),
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
    let mut offsets: std::collections::BTreeMap<PathBuf, u64> = std::collections::BTreeMap::new();
    loop {
        for entry in fs::read_dir(logs_dir())? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("log") {
                continue;
            }
            let offset = offsets.entry(path.clone()).or_insert(0);
            let mut file = File::open(&path)?;
            file.seek(SeekFrom::Start(*offset))?;
            let mut reader = BufReader::new(file);
            let mut line = String::new();
            while reader.read_line(&mut line)? > 0 {
                print!("{}", line);
                *offset += line.len() as u64;
                line.clear();
            }
        }
        thread::sleep(Duration::from_millis(500));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_layout_can_be_created_and_wiped() {
        ensure_layout().unwrap();
        append_log("athod", "test line").unwrap();
        wipe_chain_and_keys().unwrap();
        assert!(logs_dir().exists());
        assert!(!chain_dir().exists() || chain_dir().is_dir());
        assert!(!wallet_dir().exists() || wallet_dir().is_dir());
    }
}
