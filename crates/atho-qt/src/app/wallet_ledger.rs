use super::models::{WalletActivityKind, WalletActivityRow, WalletBalanceSummary};
use super::widgets::short_hash;
use atho_core::crypto::hash::sha3_256;
use atho_node::dev::chain_dir;
use atho_storage::utxo::UtxoEntry;
use atho_wallet::wallet::WalletAddress;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct WalletLedgerSnapshot {
    summary: WalletBalanceSummary,
    activities: Vec<WalletActivityRow>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LedgerFingerprint {
    tx_bytes: u64,
    input_bytes: u64,
    output_bytes: u64,
    address_hash: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct LedgerKey {
    height: u64,
    block_hash: [u8; 48],
    tx_index: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct OutpointKey {
    txid: [u8; 48],
    output_index: u32,
}

#[derive(Debug, Clone)]
struct TxRecord {
    key: LedgerKey,
    txid: [u8; 48],
    input_count: usize,
    output_count: usize,
}

#[derive(Debug, Clone)]
struct InputRecord {
    previous_txid: [u8; 48],
    output_index: u32,
}

#[derive(Debug, Clone)]
struct OutputRecord {
    value_atoms: u64,
    locking_script: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct WalletLedgerCache {
    fingerprint: Option<LedgerFingerprint>,
    snapshot: WalletLedgerSnapshot,
}

impl WalletLedgerCache {
    pub(crate) fn refresh(
        &mut self,
        addresses: &[WalletAddress],
        summary: WalletBalanceSummary,
    ) -> io::Result<()> {
        let fingerprint = LedgerFingerprint::capture(addresses)?;
        if self.fingerprint.as_ref() == Some(&fingerprint) {
            self.snapshot.summary = summary;
            return Ok(());
        }

        let mut snapshot = scan_wallet_ledger(addresses)?;
        snapshot.summary = summary;
        self.snapshot = snapshot;
        self.fingerprint = Some(fingerprint);
        Ok(())
    }

    pub(crate) fn summary(&self) -> &WalletBalanceSummary {
        &self.snapshot.summary
    }

    pub(crate) fn activities(&self) -> &[WalletActivityRow] {
        &self.snapshot.activities
    }
}

impl LedgerFingerprint {
    fn capture(addresses: &[WalletAddress]) -> io::Result<Self> {
        let mut state = Vec::with_capacity(addresses.len().saturating_mul(48));
        let mut sorted = addresses.to_vec();
        sorted.sort_by(|left, right| {
            left.payment_digest
                .cmp(&right.payment_digest)
                .then(left.path.account.cmp(&right.path.account))
                .then(left.path.index.cmp(&right.path.index))
        });
        for address in sorted {
            state.extend_from_slice(&address.payment_digest);
            state.extend_from_slice(&address.path.account.to_le_bytes());
            state.push(match address.path.kind {
                atho_wallet::hd::AddressKind::Receive => 0,
                atho_wallet::hd::AddressKind::Change => 1,
            });
            state.extend_from_slice(&address.path.index.to_le_bytes());
        }

        let mut fingerprint = Self::default();
        fingerprint.address_hash = sha3_256(&state);

        let txs = ledger_file_size("transactions.tsv")?;
        let inputs = ledger_file_size("transaction_inputs.tsv")?;
        let outputs = ledger_file_size("transaction_outputs.tsv")?;
        fingerprint.tx_bytes = txs;
        fingerprint.input_bytes = inputs;
        fingerprint.output_bytes = outputs;
        Ok(fingerprint)
    }
}

fn scan_wallet_ledger(addresses: &[WalletAddress]) -> io::Result<WalletLedgerSnapshot> {
    let address_map = address_map(addresses);
    let canonical_blocks = load_block_rows()?;
    let tx_rows = load_tx_rows()?;
    let mut input_rows = load_input_rows()?;
    let mut output_rows = load_output_rows()?;
    let mut known_outputs: HashMap<OutpointKey, u64> = HashMap::new();
    let mut activities = Vec::new();

    for (key, tx) in tx_rows {
        match canonical_blocks.get(&key.height) {
            Some(block_hash) if block_hash == &key.block_hash => {}
            _ => continue,
        }
        let inputs = input_rows.remove(&key).unwrap_or_default();
        let outputs = output_rows.remove(&key).unwrap_or_default();
        let txid_short = short_hash(&tx.txid);
        let is_coinbase = tx.input_count == 0 && key.tx_index == 0;

        let mut wallet_input_total = 0u64;
        for input in &inputs {
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
        for (output_index, output) in outputs.iter().enumerate() {
            let digest: [u8; 32] = match output.locking_script.as_slice().try_into() {
                Ok(digest) => digest,
                Err(_) => continue,
            };
            if let Some(address) = address_map.get(&digest) {
                wallet_output_total = wallet_output_total.saturating_add(output.value_atoms);
                wallet_output_count = wallet_output_count.saturating_add(1);
                wallet_output_labels.push(address.address.clone());
                known_outputs.insert(
                    OutpointKey {
                        txid: tx.txid,
                        output_index: output_index as u32,
                    },
                    output.value_atoms,
                );
            }
        }

        let when = format!("H{}", key.height);
        if is_coinbase && wallet_output_total > 0 {
            activities.push(WalletActivityRow {
                when,
                kind: WalletActivityKind::Mined,
                label: format_wallet_label("coinbase reward", &wallet_output_labels),
                amount_atoms: wallet_output_total as i128,
                reference: txid_short,
            });
            continue;
        }

        if wallet_input_total > 0 {
            let debit = wallet_input_total.saturating_sub(wallet_output_total);
            if debit == 0 {
                continue;
            }
            let external_output_count = tx.output_count.saturating_sub(wallet_output_count);
            activities.push(WalletActivityRow {
                when,
                kind: WalletActivityKind::Sent,
                label: if external_output_count == 0 {
                    String::from("self transfer")
                } else {
                    format!("{external_output_count} external output(s)")
                },
                amount_atoms: -(debit as i128),
                reference: txid_short,
            });
            continue;
        }

        if wallet_output_total > 0 {
            activities.push(WalletActivityRow {
                when,
                kind: WalletActivityKind::Received,
                label: format_wallet_label("incoming transfer", &wallet_output_labels),
                amount_atoms: wallet_output_total as i128,
                reference: txid_short,
            });
        }
    }

    activities.reverse();
    Ok(WalletLedgerSnapshot {
        summary: WalletBalanceSummary::default(),
        activities,
    })
}

fn load_block_rows() -> io::Result<BTreeMap<u64, [u8; 48]>> {
    let path = chain_dir().join("blocks.tsv");
    let mut rows = BTreeMap::new();
    for line in read_lines(path)? {
        let mut fields = line.split('\t');
        let height = match fields.next().and_then(|value| value.parse::<u64>().ok()) {
            Some(height) => height,
            None => continue,
        };
        let block_hash = match fields.next().and_then(|value| hex::decode(value).ok()) {
            Some(bytes) => match bytes.as_slice().try_into() {
                Ok(hash) => hash,
                Err(_) => continue,
            },
            None => continue,
        };
        rows.insert(height, block_hash);
    }
    Ok(rows)
}

fn address_map(addresses: &[WalletAddress]) -> HashMap<[u8; 32], WalletAddress> {
    let mut map = HashMap::with_capacity(addresses.len());
    for address in addresses {
        map.insert(address.payment_digest, address.clone());
    }
    map
}

fn ledger_file_size(name: &str) -> io::Result<u64> {
    let path = chain_dir().join(name);
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(err) => Err(err),
    }
}

fn load_tx_rows() -> io::Result<BTreeMap<LedgerKey, TxRecord>> {
    let mut rows = BTreeMap::new();
    for line in read_lines(chain_dir().join("transactions.tsv"))? {
        if let Some(record) = parse_tx_row(&line) {
            rows.insert(record.key, record);
        }
    }
    Ok(rows)
}

fn load_input_rows() -> io::Result<BTreeMap<LedgerKey, Vec<InputRecord>>> {
    let mut rows: BTreeMap<LedgerKey, Vec<InputRecord>> = BTreeMap::new();
    for line in read_lines(chain_dir().join("transaction_inputs.tsv"))? {
        if let Some((key, record)) = parse_input_row(&line) {
            rows.entry(key).or_default().push(record);
        }
    }
    Ok(rows)
}

fn load_output_rows() -> io::Result<BTreeMap<LedgerKey, Vec<OutputRecord>>> {
    let mut rows: BTreeMap<LedgerKey, Vec<OutputRecord>> = BTreeMap::new();
    for line in read_lines(chain_dir().join("transaction_outputs.tsv"))? {
        if let Some((key, record)) = parse_output_row(&line) {
            rows.entry(key).or_default().push(record);
        }
    }
    Ok(rows)
}

fn read_lines(path: PathBuf) -> io::Result<Vec<String>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut lines = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.starts_with("height\t") || line.trim().is_empty() {
            continue;
        }
        lines.push(line);
    }
    Ok(lines)
}

fn parse_tx_row(line: &str) -> Option<TxRecord> {
    let mut fields = line.split('\t');
    let height = parse_u64(fields.next()?)?;
    let block_hash = parse_fixed_hex::<48>(fields.next()?)?;
    let tx_index = parse_u32(fields.next()?)?;
    let txid = parse_fixed_hex::<48>(fields.next()?)?;
    let _wtxid = fields.next()?;
    let _version = fields.next()?;
    let _lock_time = fields.next()?;
    let input_count = parse_usize(fields.next()?)?;
    let output_count = parse_usize(fields.next()?)?;
    let _size_bytes = fields.next()?;
    let _weight_bytes = fields.next()?;
    let _vsize_bytes = fields.next()?;
    let _witness_bytes = fields.next()?;
    let _output_value_atoms = fields.next()?;
    let _canonical_bytes_hex = fields.next()?;
    Some(TxRecord {
        key: LedgerKey {
            height,
            block_hash,
            tx_index,
        },
        txid,
        input_count,
        output_count,
    })
}

fn parse_input_row(line: &str) -> Option<(LedgerKey, InputRecord)> {
    let mut fields = line.split('\t');
    let height = parse_u64(fields.next()?)?;
    let block_hash = parse_fixed_hex::<48>(fields.next()?)?;
    let tx_index = parse_u32(fields.next()?)?;
    let _input_index = fields.next()?;
    let previous_txid = parse_fixed_hex::<48>(fields.next()?)?;
    let output_index = parse_u32(fields.next()?)?;
    let _unlocking_script_hex = fields.next()?;
    Some((
        LedgerKey {
            height,
            block_hash,
            tx_index,
        },
        InputRecord {
            previous_txid,
            output_index,
        },
    ))
}

fn parse_output_row(line: &str) -> Option<(LedgerKey, OutputRecord)> {
    let mut fields = line.split('\t');
    let height = parse_u64(fields.next()?)?;
    let block_hash = parse_fixed_hex::<48>(fields.next()?)?;
    let tx_index = parse_u32(fields.next()?)?;
    let _output_index = fields.next()?;
    let value_atoms = parse_u64(fields.next()?)?;
    let locking_script = hex::decode(fields.next()?).ok()?;
    Some((
        LedgerKey {
            height,
            block_hash,
            tx_index,
        },
        OutputRecord {
            value_atoms,
            locking_script,
        },
    ))
}

fn parse_u64(value: &str) -> Option<u64> {
    value.parse::<u64>().ok()
}

fn parse_u32(value: &str) -> Option<u32> {
    value.parse::<u32>().ok()
}

fn parse_usize(value: &str) -> Option<usize> {
    value.parse::<usize>().ok()
}

fn parse_fixed_hex<const N: usize>(value: &str) -> Option<[u8; N]> {
    let bytes = hex::decode(value).ok()?;
    bytes.as_slice().try_into().ok()
}

fn format_wallet_label(prefix: &str, labels: &[String]) -> String {
    match labels {
        [] => prefix.to_string(),
        [label] => format!("{prefix} to {label}"),
        _ => format!("{prefix} to {} outputs", labels.len()),
    }
}

pub(crate) fn summarize_wallet_utxos(
    utxos: &[UtxoEntry],
    current_height: u64,
) -> WalletBalanceSummary {
    let mut available_atoms = 0u64;
    let mut pending_atoms = 0u64;

    for utxo in utxos {
        if utxo.is_spendable_at(current_height) {
            available_atoms = available_atoms.saturating_add(utxo.value_atoms);
        } else {
            pending_atoms = pending_atoms.saturating_add(utxo.value_atoms);
        }
    }

    WalletBalanceSummary {
        available_atoms,
        pending_atoms,
        total_atoms: available_atoms.saturating_add(pending_atoms),
    }
}
