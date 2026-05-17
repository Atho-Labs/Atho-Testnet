// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Read-only explorer index derived from canonical chainstate data.
//!
//! The explorer index avoids full-chain scans on every address lookup by
//! caching canonical address history and current UTXO ownership in memory. The
//! node remains the source of truth; the index is rebuilt whenever the
//! canonical chain tip changes. Volatile mempool state is tracked separately so
//! address lookups do not force a full chain rebuild when only the mempool
//! changes.
use crate::error::NodeError;
use crate::node::Node;
use atho_core::address::encode_base56_address;
use atho_core::block::Block;
use atho_core::constants::ATOMS_PER_ATHO;
use atho_core::network::Network;
use atho_core::transaction::Transaction;
use atho_storage::utxo::UtxoEntry;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplorerIndex {
    network: Option<Network>,
    tip_height: u64,
    #[serde(with = "serde_big_array::BigArray")]
    tip_hash: [u8; 48],
    canonical_hashes: Vec<CanonicalHash>,
    known_outputs: BTreeMap<OutputRef, KnownOutput>,
    addresses: BTreeMap<String, IndexedAddressRecord>,
}

impl Default for ExplorerIndex {
    fn default() -> Self {
        Self {
            network: None,
            tip_height: 0,
            tip_hash: [0; 48],
            canonical_hashes: Vec::new(),
            known_outputs: BTreeMap::new(),
            addresses: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct IndexedAddressRecord {
    #[serde(with = "serde_big_array::BigArray")]
    payment_digest: [u8; 32],
    transactions: Vec<AddressTransactionEntry>,
    utxos: Vec<UtxoEntry>,
    balance_atoms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AddressTransactionEntry {
    txid: String,
    source: String,
    block_hash: Option<String>,
    height: Option<u64>,
    timestamp: Option<u64>,
    kind: String,
    received_atoms: u64,
    sent_atoms: u64,
    net_atoms: String,
    received_atho: String,
    sent_atho: String,
    net_atho: String,
    fee_atoms: Option<u64>,
    confirmed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CanonicalHash {
    #[serde(with = "serde_big_array::BigArray")]
    hash: [u8; 48],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct OutputRef {
    #[serde(with = "serde_big_array::BigArray")]
    txid: [u8; 48],
    output_index: u32,
}

impl OutputRef {
    fn new(txid: [u8; 48], output_index: u32) -> Self {
        Self { txid, output_index }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct KnownOutput {
    address: Option<String>,
    value_atoms: u64,
    locking_script: Vec<u8>,
    created_height: u64,
    is_coinbase: bool,
}

#[derive(Debug, Clone, Default)]
struct AddressTxAccumulator {
    sent_atoms: u64,
    received_atoms: u64,
}

impl ExplorerIndex {
    pub fn default_for_network(network: Network) -> Self {
        Self {
            network: Some(network),
            ..Self::default()
        }
    }

    pub fn needs_refresh(&self, network: Network, tip_height: u64, tip_hash: [u8; 48]) -> bool {
        self.network != Some(network) || self.tip_height != tip_height || self.tip_hash != tip_hash
    }

    pub fn rebuild(node: &Node) -> Result<Self, NodeError> {
        let network = node.network();
        let tip_height = node.height();
        let tip_hash = node.tip_hash();
        let blocks = node.canonical_blocks()?;

        let mut addresses = BTreeMap::<String, IndexedAddressRecord>::new();
        let mut known_outputs = BTreeMap::<OutputRef, KnownOutput>::new();
        let mut canonical_hashes = Vec::with_capacity(blocks.len());

        for block in &blocks {
            canonical_hashes.push(CanonicalHash {
                hash: block.header.block_hash(),
            });
            for tx in &block.transactions {
                index_transaction(
                    &mut addresses,
                    &mut known_outputs,
                    network,
                    tx,
                    TransactionIndexContext {
                        height: Some(block.header.height),
                        block_hash: Some(block.header.block_hash()),
                        timestamp: Some(block.header.timestamp),
                        fee_atoms: if tx.is_coinbase() { Some(0) } else { None },
                        source: "chain",
                    },
                );
            }
        }

        for utxo in node.utxo_entries() {
            let Some(address) = script_address_hint(network, &utxo.locking_script) else {
                continue;
            };
            let record = addresses
                .entry(address)
                .or_insert_with(|| indexed_address_record_from_script(&utxo.locking_script));
            record.utxos.push(utxo.clone());
            record.balance_atoms = record.balance_atoms.saturating_add(utxo.value_atoms);
        }

        for record in addresses.values_mut() {
            sort_address_record(record);
        }

        Ok(Self {
            network: Some(network),
            tip_height,
            tip_hash,
            canonical_hashes,
            known_outputs,
            addresses,
        })
    }

    pub fn try_refresh_incremental(&mut self, node: &Node) -> Result<bool, NodeError> {
        let network = node.network();
        let tip_height = node.height();
        let tip_hash = node.tip_hash();
        if !self.needs_refresh(network, tip_height, tip_hash) {
            return Ok(true);
        }
        if self.network != Some(network)
            || self.canonical_hashes.is_empty()
            || self.canonical_hashes.len() != self.tip_height.saturating_add(1) as usize
        {
            return Ok(false);
        }

        let Some(common_ancestor_height) = self.common_ancestor_height(node) else {
            return Ok(false);
        };
        let mut disconnected = Vec::new();
        for height in common_ancestor_height.saturating_add(1)..=self.tip_height {
            let Some(hash) = self
                .canonical_hashes
                .get(height as usize)
                .map(|entry| entry.hash)
            else {
                return Ok(false);
            };
            let Some(block) = node.block_by_hash(hash) else {
                return Ok(false);
            };
            disconnected.push(block);
        }

        let mut connected = Vec::new();
        for height in common_ancestor_height.saturating_add(1)..=tip_height {
            let Some(block) = node.block_by_height(height) else {
                return Ok(false);
            };
            connected.push(block);
        }

        for block in disconnected.iter().rev() {
            self.disconnect_block(network, block);
        }
        self.canonical_hashes
            .truncate(common_ancestor_height.saturating_add(1) as usize);

        for block in &connected {
            self.connect_block(network, block);
            self.canonical_hashes.push(CanonicalHash {
                hash: block.header.block_hash(),
            });
        }

        self.tip_height = tip_height;
        self.tip_hash = tip_hash;
        Ok(true)
    }

    pub fn network(&self) -> Option<Network> {
        self.network
    }

    pub fn tip_height(&self) -> u64 {
        self.tip_height
    }

    pub fn tip_hash(&self) -> [u8; 48] {
        self.tip_hash
    }

    pub fn address_summary_value(
        &self,
        network: Network,
        address: &str,
        limit: usize,
        offset: usize,
    ) -> Option<Value> {
        let record = self.addresses.get(address)?;
        let slice = paginate(&record.transactions, limit, offset);
        let (spendable_atoms, immature_atoms) =
            record
                .utxos
                .iter()
                .fold((0u64, 0u64), |(spendable, immature), utxo| {
                    if utxo.is_spendable_at(self.tip_height) {
                        (spendable.saturating_add(utxo.value_atoms), immature)
                    } else {
                        (spendable, immature.saturating_add(utxo.value_atoms))
                    }
                });
        Some(json!({
            "address": address,
            "network": network.domain_tag(),
            "payment_digest_hex": hex::encode(record.payment_digest),
            "tx_count": record.transactions.len(),
            "utxo_count": record.utxos.len(),
            "balance_atoms": record.balance_atoms,
            "balance_atho": format_atoms_decimal(record.balance_atoms),
            "spendable_atoms": spendable_atoms,
            "spendable_atho": format_atoms_decimal(spendable_atoms),
            "immature_atoms": immature_atoms,
            "immature_atho": format_atoms_decimal(immature_atoms),
            "pending_delta_atoms": "0",
            "pending_delta_atho": "0.000000000000",
            "transactions": slice,
            "page": {
                "limit": limit,
                "offset": offset,
                "returned": slice.len(),
                "total": record.transactions.len(),
            }
        }))
    }

    pub fn address_utxos_value(
        &self,
        network: Network,
        current_height: u64,
        address: &str,
        limit: usize,
        offset: usize,
    ) -> Option<Value> {
        let record = self.addresses.get(address)?;
        let utxos = paginate(&record.utxos, limit, offset)
            .iter()
            .map(|entry| render_utxo_value(current_height, network, entry))
            .collect::<Vec<_>>();
        Some(json!({
            "address": address,
            "network": network.domain_tag(),
            "utxo_count": record.utxos.len(),
            "utxos": utxos,
            "page": {
                "limit": limit,
                "offset": offset,
                "returned": utxos.len(),
                "total": record.utxos.len(),
            }
        }))
    }

    fn common_ancestor_height(&self, node: &Node) -> Option<u64> {
        let mut height = self.tip_height.min(node.height());
        loop {
            let indexed_hash = self.canonical_hashes.get(height as usize)?.hash;
            let chain_hash = node.block_by_height(height)?.header.block_hash();
            if indexed_hash == chain_hash {
                return Some(height);
            }
            if height == 0 {
                return None;
            }
            height = height.saturating_sub(1);
        }
    }

    fn connect_block(&mut self, network: Network, block: &Block) {
        let mut touched = BTreeSet::new();
        for tx in &block.transactions {
            connect_transaction(
                &mut self.addresses,
                &mut self.known_outputs,
                &mut touched,
                network,
                tx,
                TransactionIndexContext {
                    height: Some(block.header.height),
                    block_hash: Some(block.header.block_hash()),
                    timestamp: Some(block.header.timestamp),
                    fee_atoms: if tx.is_coinbase() { Some(0) } else { None },
                    source: "chain",
                },
            );
        }
        finalize_touched_addresses(&mut self.addresses, touched);
    }

    fn disconnect_block(&mut self, network: Network, block: &Block) {
        let mut touched = BTreeSet::new();
        for tx in block.transactions.iter().rev() {
            disconnect_transaction(
                &mut self.addresses,
                &mut self.known_outputs,
                &mut touched,
                network,
                tx,
                TransactionIndexContext {
                    height: Some(block.header.height),
                    block_hash: Some(block.header.block_hash()),
                    timestamp: Some(block.header.timestamp),
                    fee_atoms: if tx.is_coinbase() { Some(0) } else { None },
                    source: "chain",
                },
            );
        }
        finalize_touched_addresses(&mut self.addresses, touched);
    }
}

#[derive(Debug, Clone, Copy)]
struct TransactionIndexContext {
    height: Option<u64>,
    block_hash: Option<[u8; 48]>,
    timestamp: Option<u64>,
    fee_atoms: Option<u64>,
    source: &'static str,
}

fn index_transaction(
    addresses: &mut BTreeMap<String, IndexedAddressRecord>,
    known_outputs: &mut BTreeMap<OutputRef, KnownOutput>,
    network: Network,
    tx: &Transaction,
    context: TransactionIndexContext,
) {
    let TransactionIndexContext {
        height,
        block_hash,
        timestamp,
        fee_atoms,
        source,
    } = context;
    let txid = tx.txid();
    let mut per_address = BTreeMap::<String, AddressTxAccumulator>::new();

    for input in &tx.inputs {
        if let Some(previous) =
            known_outputs.get(&OutputRef::new(input.previous_txid, input.output_index))
        {
            if let Some(address) = &previous.address {
                let accumulator = per_address.entry(address.clone()).or_default();
                accumulator.sent_atoms =
                    accumulator.sent_atoms.saturating_add(previous.value_atoms);
            }
        }
    }

    for (output_index, output) in tx.outputs.iter().enumerate() {
        let address = script_address_hint(network, &output.locking_script);
        known_outputs.insert(
            OutputRef::new(txid, output_index as u32),
            KnownOutput {
                address: address.clone(),
                value_atoms: output.value_atoms,
                locking_script: output.locking_script.clone(),
                created_height: height.unwrap_or_default(),
                is_coinbase: tx.is_coinbase(),
            },
        );
        if let Some(address) = address {
            let accumulator = per_address.entry(address).or_default();
            accumulator.received_atoms = accumulator
                .received_atoms
                .saturating_add(output.value_atoms);
        }
    }

    for (address, accumulator) in per_address {
        let net_atoms = accumulator.received_atoms as i128 - accumulator.sent_atoms as i128;
        let kind = if tx.is_coinbase() && accumulator.received_atoms > 0 {
            "mined"
        } else if accumulator.sent_atoms > 0 && accumulator.received_atoms > 0 {
            "self_transfer"
        } else if accumulator.sent_atoms > 0 {
            "sent"
        } else {
            "received"
        };
        let entry = AddressTransactionEntry {
            txid: hex::encode(txid),
            source: source.to_string(),
            block_hash: block_hash.map(hex::encode),
            height,
            timestamp,
            kind: kind.to_string(),
            received_atoms: accumulator.received_atoms,
            sent_atoms: accumulator.sent_atoms,
            net_atoms: net_atoms.to_string(),
            received_atho: format_atoms_decimal(accumulator.received_atoms),
            sent_atho: format_atoms_decimal(accumulator.sent_atoms),
            net_atho: format_signed_atoms_decimal(net_atoms),
            fee_atoms,
            confirmed: source == "chain",
        };
        let record = addresses
            .entry(address.clone())
            .or_insert_with(|| indexed_address_record_from_address(&address));
        record.transactions.push(entry);
    }
}

fn connect_transaction(
    addresses: &mut BTreeMap<String, IndexedAddressRecord>,
    known_outputs: &mut BTreeMap<OutputRef, KnownOutput>,
    touched: &mut BTreeSet<String>,
    network: Network,
    tx: &Transaction,
    context: TransactionIndexContext,
) {
    let TransactionIndexContext {
        height,
        block_hash: _,
        timestamp: _,
        fee_atoms: _,
        source: _,
    } = context;
    let txid = tx.txid();
    let mut per_address = BTreeMap::<String, AddressTxAccumulator>::new();

    for input in &tx.inputs {
        if let Some(previous) =
            known_outputs.get(&OutputRef::new(input.previous_txid, input.output_index))
        {
            if let Some(address) = previous.address.as_ref() {
                touched.insert(address.clone());
                let record = addresses
                    .entry(address.clone())
                    .or_insert_with(|| indexed_address_record_from_address(address));
                if let Some(index) = record.utxos.iter().position(|utxo| {
                    utxo.txid == input.previous_txid && utxo.output_index == input.output_index
                }) {
                    let spent = record.utxos.remove(index);
                    record.balance_atoms = record.balance_atoms.saturating_sub(spent.value_atoms);
                }
                let accumulator = per_address.entry(address.clone()).or_default();
                accumulator.sent_atoms =
                    accumulator.sent_atoms.saturating_add(previous.value_atoms);
            }
        }
    }

    for (output_index, output) in tx.outputs.iter().enumerate() {
        let output_ref = OutputRef::new(txid, output_index as u32);
        let address = script_address_hint(network, &output.locking_script);
        known_outputs.insert(
            output_ref,
            KnownOutput {
                address: address.clone(),
                value_atoms: output.value_atoms,
                locking_script: output.locking_script.clone(),
                created_height: height.unwrap_or_default(),
                is_coinbase: tx.is_coinbase(),
            },
        );
        if let Some(address) = address {
            touched.insert(address.clone());
            let record = addresses
                .entry(address.clone())
                .or_insert_with(|| indexed_address_record_from_script(&output.locking_script));
            record.utxos.push(UtxoEntry::new(
                network,
                txid,
                output_index as u32,
                output.value_atoms,
                output.locking_script.clone(),
                height.unwrap_or_default(),
                tx.is_coinbase(),
            ));
            record.balance_atoms = record.balance_atoms.saturating_add(output.value_atoms);
            let accumulator = per_address.entry(address).or_default();
            accumulator.received_atoms = accumulator
                .received_atoms
                .saturating_add(output.value_atoms);
        }
    }

    push_transaction_entries(addresses, tx, context, per_address);
}

fn disconnect_transaction(
    addresses: &mut BTreeMap<String, IndexedAddressRecord>,
    known_outputs: &mut BTreeMap<OutputRef, KnownOutput>,
    touched: &mut BTreeSet<String>,
    network: Network,
    tx: &Transaction,
    context: TransactionIndexContext,
) {
    let txid = tx.txid();
    let txid_hex = hex::encode(txid);

    for (output_index, _) in tx.outputs.iter().enumerate() {
        let output_ref = OutputRef::new(txid, output_index as u32);
        let Some(known_output) = known_outputs.remove(&output_ref) else {
            continue;
        };
        if let Some(address) = known_output.address {
            touched.insert(address.clone());
            if let Some(record) = addresses.get_mut(&address) {
                if let Some(index) = record
                    .utxos
                    .iter()
                    .position(|utxo| utxo.txid == txid && utxo.output_index == output_index as u32)
                {
                    let removed = record.utxos.remove(index);
                    record.balance_atoms = record.balance_atoms.saturating_sub(removed.value_atoms);
                }
            }
        }
    }

    for input in &tx.inputs {
        if let Some(previous) =
            known_outputs.get(&OutputRef::new(input.previous_txid, input.output_index))
        {
            if let Some(address) = previous.address.as_ref() {
                touched.insert(address.clone());
                let record = addresses
                    .entry(address.clone())
                    .or_insert_with(|| indexed_address_record_from_address(address));
                record.utxos.push(UtxoEntry::new(
                    network,
                    input.previous_txid,
                    input.output_index,
                    previous.value_atoms,
                    previous.locking_script.clone(),
                    previous.created_height,
                    previous.is_coinbase,
                ));
                record.balance_atoms = record.balance_atoms.saturating_add(previous.value_atoms);
            }
        }
    }

    for address in touched.iter() {
        if let Some(record) = addresses.get_mut(address) {
            record
                .transactions
                .retain(|entry| !(entry.txid == txid_hex && entry.source == context.source));
        }
    }
}

fn push_transaction_entries(
    addresses: &mut BTreeMap<String, IndexedAddressRecord>,
    tx: &Transaction,
    context: TransactionIndexContext,
    per_address: BTreeMap<String, AddressTxAccumulator>,
) {
    let TransactionIndexContext {
        height,
        block_hash,
        timestamp,
        fee_atoms,
        source,
    } = context;
    let txid = tx.txid();

    for (address, accumulator) in per_address {
        let net_atoms = accumulator.received_atoms as i128 - accumulator.sent_atoms as i128;
        let kind = if tx.is_coinbase() && accumulator.received_atoms > 0 {
            "mined"
        } else if accumulator.sent_atoms > 0 && accumulator.received_atoms > 0 {
            "self_transfer"
        } else if accumulator.sent_atoms > 0 {
            "sent"
        } else {
            "received"
        };
        let entry = AddressTransactionEntry {
            txid: hex::encode(txid),
            source: source.to_string(),
            block_hash: block_hash.map(hex::encode),
            height,
            timestamp,
            kind: kind.to_string(),
            received_atoms: accumulator.received_atoms,
            sent_atoms: accumulator.sent_atoms,
            net_atoms: net_atoms.to_string(),
            received_atho: format_atoms_decimal(accumulator.received_atoms),
            sent_atho: format_atoms_decimal(accumulator.sent_atoms),
            net_atho: format_signed_atoms_decimal(net_atoms),
            fee_atoms,
            confirmed: source == "chain",
        };
        let record = addresses
            .entry(address.clone())
            .or_insert_with(|| indexed_address_record_from_address(&address));
        record.transactions.push(entry);
    }
}

fn finalize_touched_addresses(
    addresses: &mut BTreeMap<String, IndexedAddressRecord>,
    touched: BTreeSet<String>,
) {
    for address in touched {
        let should_remove = if let Some(record) = addresses.get_mut(&address) {
            sort_address_record(record);
            record.transactions.is_empty() && record.utxos.is_empty()
        } else {
            false
        };
        if should_remove {
            addresses.remove(&address);
        }
    }
}

fn sort_address_record(record: &mut IndexedAddressRecord) {
    record.transactions.sort_by(|left, right| {
        right
            .confirmed
            .cmp(&left.confirmed)
            .then(right.timestamp.cmp(&left.timestamp))
            .then(right.height.cmp(&left.height))
            .then(right.txid.cmp(&left.txid))
    });
    record.utxos.sort_by(|left, right| {
        right
            .created_height
            .cmp(&left.created_height)
            .then(left.txid.cmp(&right.txid))
            .then(left.output_index.cmp(&right.output_index))
    });
}

fn indexed_address_record_from_address(address: &str) -> IndexedAddressRecord {
    let payment_digest = atho_core::address::decode_base56_address(address)
        .map(|(payment_digest, _)| payment_digest)
        .unwrap_or([0; 32]);
    IndexedAddressRecord {
        payment_digest,
        ..IndexedAddressRecord::default()
    }
}

fn indexed_address_record_from_script(locking_script: &[u8]) -> IndexedAddressRecord {
    let payment_digest = locking_script.try_into().unwrap_or([0; 32]);
    IndexedAddressRecord {
        payment_digest,
        ..IndexedAddressRecord::default()
    }
}

fn script_address_hint(network: Network, locking_script: &[u8]) -> Option<String> {
    let digest: [u8; 32] = locking_script.try_into().ok()?;
    Some(encode_base56_address(network, &digest))
}

fn render_utxo_value(spend_height: u64, network: Network, entry: &UtxoEntry) -> Value {
    json!({
        "txid": hex::encode(entry.txid),
        "vout": entry.output_index,
        "value_atoms": entry.value_atoms,
        "value_atho": format_atoms_decimal(entry.value_atoms),
        "confirmations": entry.confirmation_count(spend_height),
        "coinbase": entry.is_coinbase,
        "spendable": entry.is_spendable_at(spend_height),
        "created_height": entry.created_height,
        "locking_script_hex": hex::encode(&entry.locking_script),
        "address_hint": script_address_hint(network, &entry.locking_script),
    })
}

fn format_atoms_decimal(atoms: u64) -> String {
    let whole = atoms / ATOMS_PER_ATHO;
    let fractional = atoms % ATOMS_PER_ATHO;
    format!("{whole}.{fractional:012}")
}

fn format_signed_atoms_decimal(atoms: i128) -> String {
    let negative = atoms.is_negative();
    let magnitude = atoms.unsigned_abs();
    let whole = magnitude / ATOMS_PER_ATHO as u128;
    let fractional = magnitude % ATOMS_PER_ATHO as u128;
    if negative {
        format!("-{whole}.{fractional:012}")
    } else {
        format!("{whole}.{fractional:012}")
    }
}

fn paginate<T>(items: &[T], limit: usize, offset: usize) -> &[T] {
    if offset >= items.len() {
        return &items[0..0];
    }
    let end = offset.saturating_add(limit).min(items.len());
    &items[offset..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NodeConfig;
    use crate::service::NodeService;
    use atho_core::address::encode_base56_address;
    use atho_storage::path::ATHO_DATA_DIR_ENV;
    use atho_storage::utxo::UtxoEntry;
    use std::ffi::OsString;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
        _lock: crate::test_support::TestLockGuard,
    }

    impl EnvVarGuard {
        fn set_path(key: &'static str, value: &std::path::Path) -> Self {
            let lock = crate::test_support::acquire_global_test_lock();
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

    fn temp_data_dir(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "atho-explorer-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    #[test]
    fn rebuild_indexes_genesis_reward_address() {
        let root = temp_data_dir("genesis-address");
        fs::create_dir_all(&root).expect("root");
        let _guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);

        let mut service = NodeService::new(NodeConfig::new(Network::Regnet));
        let digest = [7u8; 32];
        let address = encode_base56_address(Network::Regnet, &digest);
        service.sandbox_with_node_mut(|node| {
            node.dev_seed_chainstate(
                node.height(),
                node.tip_hash(),
                [UtxoEntry::new(
                    Network::Regnet,
                    [0x33; 48],
                    0,
                    12_345,
                    digest.to_vec(),
                    node.height(),
                    false,
                )],
            )
            .expect("seed visible utxo");
        });
        let index = ExplorerIndex::rebuild(service.node_ref()).expect("index");
        let summary = index
            .address_summary_value(Network::Regnet, &address, 10, 0)
            .expect("address summary");
        assert_eq!(summary["address"], address);
        assert_eq!(summary["utxo_count"], 1);
        assert_eq!(summary["balance_atoms"], 12_345);
    }
}
