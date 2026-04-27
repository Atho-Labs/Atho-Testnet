use crate::connection::{ConnectionStatus, ReadOnlyNodeConnection, StatusMonitor};
use crate::error::QtError;
use crate::state::UiState;
use crate::view::ViewModel;
use atho_core::address::decode_base56_address;
use atho_core::constants::MIN_TX_FEE_PER_VBYTE_ATOMS;
use atho_core::network::Network;
use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness};
use atho_crypto::falcon::{
    sign, FalconKeypair, FALCON_512_PUBLIC_KEY_BYTES, FALCON_512_SIGNATURE_MAX_BYTES,
};
use atho_node::miner::Miner;
use atho_node::validation::{derive_sig_ref_short, derive_witness_commit_ref};
use atho_rpc::request::RpcRequest;
use atho_rpc::response::RpcResponse;
use atho_rpc::transport::RpcClient;
use atho_storage::utxo::UtxoEntry;
use atho_wallet::hd::AddressKind;
use atho_wallet::mnemonic::{MnemonicLength, MnemonicPhrase};
use atho_wallet::wallet::{Wallet, WalletAddress};
use eframe::egui;
use getrandom::getrandom;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

mod dialogs;
mod models;
mod pages;
mod shell;
mod startup;
mod theme;
mod widgets;
pub(crate) use models::{
    CreateWalletForm, ImportWalletForm, LaunchPage, MiningJob, MiningOutcome, NavTab,
    OpenWalletForm, ReceiveRequestRecord, SendJob, SendOutcome, WalletActivityRow,
};

pub struct DesktopApp {
    pub connection: ReadOnlyNodeConnection,
    status_monitor: StatusMonitor,
    pub ui_state: UiState,
    pub view_model: ViewModel,
    wallet: Option<Wallet>,
    wallet_path: Option<String>,
    receive_addresses: Vec<WalletAddress>,
    current_receive_address: Option<WalletAddress>,
    launch_page: LaunchPage,
    create_form: CreateWalletForm,
    import_form: ImportWalletForm,
    open_form: OpenWalletForm,
    last_error: Option<String>,
    active_tab: NavTab,
    send_to: String,
    send_label: String,
    send_amount: String,
    send_subtract_fee: bool,
    send_fee: String,
    receive_label: String,
    receive_amount: String,
    receive_message: String,
    send_status: String,
    send_job: Option<SendJob>,
    mining_status: String,
    mining_job: Option<MiningJob>,
    last_mined_block_hash: Option<[u8; 48]>,
    requested_payments: Vec<ReceiveRequestRecord>,
    selected_receive_request: Option<usize>,
    transaction_search: String,
    transaction_min_amount: String,
    transaction_date_filter: usize,
    transaction_type_filter: usize,
    show_about_dialog: bool,
    wallet_utxos_cache: Vec<UtxoEntry>,
    wallet_activity_cache: Vec<WalletActivityRow>,
    wallet_balance_cache: u64,
    theme_initialized: bool,
    compact_viewport: bool,
}

impl DesktopApp {
    pub fn new(network: Network) -> Self {
        Self::new_with_rpc(network, None)
    }

    pub fn new_with_rpc(network: Network, rpc_address: Option<String>) -> Self {
        let connection = match rpc_address {
            Some(address) => ReadOnlyNodeConnection::with_rpc_address(network, address),
            None => ReadOnlyNodeConnection::new(network),
        };
        let status_monitor = connection.spawn_status_monitor(Duration::from_secs(1));
        let launch_page = LaunchPage::Welcome;

        let mut app = Self {
            connection,
            status_monitor,
            ui_state: UiState {
                mining_cores: 4,
                ..UiState::default()
            },
            view_model: ViewModel::default(),
            wallet: None,
            wallet_path: None,
            receive_addresses: Vec::new(),
            current_receive_address: None,
            launch_page,
            create_form: CreateWalletForm::new(network),
            import_form: ImportWalletForm::new(network),
            open_form: OpenWalletForm::new(network),
            last_error: None,
            active_tab: NavTab::Overview,
            send_to: String::new(),
            send_label: String::new(),
            send_amount: String::new(),
            send_subtract_fee: false,
            send_fee: String::new(),
            receive_label: String::new(),
            receive_amount: String::new(),
            receive_message: String::new(),
            send_status: String::from("Enter a destination and integer atom amounts."),
            send_job: None,
            mining_status: String::from("Idle"),
            mining_job: None,
            last_mined_block_hash: None,
            requested_payments: Vec::new(),
            selected_receive_request: None,
            transaction_search: String::new(),
            transaction_min_amount: String::new(),
            transaction_date_filter: 0,
            transaction_type_filter: 0,
            show_about_dialog: false,
            wallet_utxos_cache: Vec::new(),
            wallet_activity_cache: Vec::new(),
            wallet_balance_cache: 0,
            theme_initialized: false,
            compact_viewport: false,
        };

        app.view_model.network_label = app.connection.network().id().to_string();
        app.view_model.sync_stage = if app.connection.has_local_node() {
            String::from("Starting node")
        } else {
            String::from("Disconnected")
        };
        app.try_open_existing_wallet_on_startup();
        app
    }

    pub fn refresh(&mut self) -> Result<(), QtError> {
        let status = self.connection.status();
        self.apply_connection_status(status);
        if self.wallet.is_some() {
            self.refresh_wallet_cache();
        }
        Ok(())
    }

    fn apply_connection_status(&mut self, status: ConnectionStatus) {
        self.view_model.network_label = status.network.id().to_string();
        self.view_model.block_count = status.block_count;
        self.view_model.mempool_count = status.mempool_count;
        self.view_model.sync_best_height = status.block_count;
        self.view_model.sync_stage = if status.connected {
            String::from("Connected")
        } else if self.connection.has_local_node() {
            String::from("Starting node")
        } else {
            String::from("Disconnected")
        };

        if let Some(wallet) = &self.wallet {
            self.view_model.ui_state.wallet_snapshot = wallet.snapshot.clone();
            self.ui_state.wallet_snapshot = wallet.snapshot.clone();
        }
        self.ui_state.set_connected(status.connected);
    }

    fn wallet_mut(&mut self) -> Option<&mut Wallet> {
        self.wallet.as_mut()
    }

    fn wallet_ref(&self) -> Option<&Wallet> {
        self.wallet.as_ref()
    }

    fn wallet_file_label(&self) -> String {
        self.wallet_path
            .as_ref()
            .and_then(|path| {
                PathBuf::from(path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| String::from("wallet.dat"))
    }

    fn refresh_wallet_cache(&mut self) {
        let Some(_wallet) = self.wallet_ref() else {
            self.wallet_utxos_cache.clear();
            self.wallet_activity_cache.clear();
            self.wallet_balance_cache = 0;
            return;
        };
        let RpcResponse::Utxos(utxos) = self.connection.request(RpcRequest::ListUtxos) else {
            return;
        };

        let (owned, activities, balance) = {
            let wallet = self.wallet_ref().expect("wallet checked above");
            let mut owned = utxos
                .into_iter()
                .filter(|utxo| match utxo.locking_script.as_slice().try_into() {
                    Ok(digest) => wallet.address_for_payment_digest(&digest).is_some(),
                    Err(_) => false,
                })
                .collect::<Vec<_>>();
            owned.sort_by(|left, right| {
                right
                    .txid
                    .cmp(&left.txid)
                    .then(right.output_index.cmp(&left.output_index))
            });
            let balance = owned.iter().map(|utxo| utxo.value_atoms).sum();
            let activities = owned
                .iter()
                .map(|utxo| {
                    let label = match utxo.locking_script.as_slice().try_into() {
                        Ok(digest) => wallet
                            .address_for_payment_digest(&digest)
                            .map(|address| address.address)
                            .unwrap_or_else(|| widgets::short_hash(&utxo.txid)),
                        Err(_) => widgets::short_hash(&utxo.txid),
                    };

                    WalletActivityRow {
                        when: String::from("Current"),
                        kind: "Received",
                        label,
                        amount_atoms: utxo.value_atoms,
                        reference: widgets::short_hash(&utxo.txid),
                    }
                })
                .collect();
            (owned, activities, balance)
        };

        self.wallet_utxos_cache = owned;
        self.wallet_activity_cache = activities;
        self.wallet_balance_cache = balance;
    }

    fn wallet_balance_atoms(&self) -> u64 {
        self.wallet_balance_cache
    }

    fn wallet_activity_rows(&self) -> &[WalletActivityRow] {
        &self.wallet_activity_cache
    }

    fn attach_wallet(&mut self, mut wallet: Wallet, wallet_path: String) {
        let has_receive = wallet
            .address_book
            .snapshot()
            .iter()
            .any(|record| record.path.kind == AddressKind::Receive);

        if !has_receive {
            let first = wallet.checkout_receive_address();
            self.receive_addresses = vec![first.clone()];
            self.current_receive_address = Some(first);
        } else {
            self.receive_addresses = wallet
                .address_book
                .snapshot()
                .into_iter()
                .filter(|record| record.path.kind == AddressKind::Receive)
                .map(|record| wallet.address_for_path(record.path))
                .collect();
            self.current_receive_address = self.receive_addresses.last().cloned();
            if self.receive_addresses.is_empty() {
                let first = wallet.checkout_receive_address();
                self.receive_addresses = vec![first.clone()];
                self.current_receive_address = Some(first);
            }
        }

        self.wallet_path = Some(wallet_path);
        self.wallet = Some(wallet);
        self.sync_wallet_state();
        self.refresh_wallet_cache();
        self.active_tab = NavTab::Overview;
        self.launch_page = LaunchPage::Welcome;
    }

    fn sync_wallet_state(&mut self) {
        if let Some(wallet) = &self.wallet {
            self.ui_state.wallet_snapshot = wallet.snapshot.clone();
            self.view_model.ui_state.wallet_snapshot = wallet.snapshot.clone();
            self.current_receive_address = self.receive_addresses.last().cloned();
        }
    }

    fn poll_mining_job(&mut self) {
        let Some(job) = self.mining_job.take() else {
            return;
        };

        match job.receiver.try_recv() {
            Ok(Ok(outcome)) => {
                self.last_mined_block_hash = Some(outcome.block_hash);
                self.mining_status = format!("{} at height {}", outcome.message, outcome.height);
                if outcome.accepted {
                    self.last_error = None;
                    self.refresh_wallet_cache();
                    if self.ui_state.generate_coins {
                        self.start_mining_job();
                    }
                }
            }
            Ok(Err(err)) => {
                self.mining_status = String::from("Mining failed");
                self.last_error = Some(err);
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.mining_job = Some(job);
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.mining_status = String::from("Mining worker disconnected");
                self.last_error = Some(String::from("mining worker disconnected"));
            }
        }

        if self.mining_job.is_none() {
            let elapsed = job.started_at.elapsed();
            self.mining_status = format!("{} ({}s)", self.mining_status, elapsed.as_secs());
        }
    }

    fn poll_send_job(&mut self) {
        let Some(job) = self.send_job.take() else {
            return;
        };

        match job.receiver.try_recv() {
            Ok(Ok(outcome)) => {
                self.send_fee = outcome.fee_atoms.to_string();
                self.send_status = outcome.message;
                self.last_error = None;
                self.refresh_wallet_cache();
            }
            Ok(Err(err)) => {
                self.send_status = String::from("Submission failed");
                self.last_error = Some(err);
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.send_job = Some(job);
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.send_status = String::from("Submission worker disconnected");
                self.last_error = Some(String::from("submission worker disconnected"));
            }
        }

        if self.send_job.is_none() {
            let elapsed = job.started_at.elapsed();
            self.send_status = format!("{} ({}s)", self.send_status, elapsed.as_secs());
        }
    }

    fn start_mining_job(&mut self) {
        if self.mining_job.is_some() {
            self.mining_status = String::from("Mining already running");
            return;
        }
        if !self.ui_state.connected {
            self.mining_status = String::from("Node is not connected yet");
            return;
        }

        let rpc_address = self.connection.rpc_address().to_string();
        let cores = self.ui_state.mining_cores.max(1);
        let (sender, receiver) = mpsc::channel();
        self.mining_status = format!("Starting generation with {} thread(s)", cores);
        self.last_error = None;

        std::thread::spawn(move || {
            let result = mine_via_rpc(rpc_address, cores);
            let _ = sender.send(result);
        });

        self.mining_job = Some(MiningJob {
            started_at: Instant::now(),
            receiver,
        });
    }

    fn generate_create_mnemonic(&mut self) -> Result<(), String> {
        let mut entropy = [0u8; 32];
        getrandom(&mut entropy).map_err(|_| String::from("failed to gather wallet entropy"))?;
        let mnemonic = MnemonicPhrase::from_entropy(&entropy, MnemonicLength::Words24)
            .map_err(|err| err.to_string())?;
        self.create_form.mnemonic_text = mnemonic.as_sentence();
        self.create_form.acknowledged_backup = false;
        Ok(())
    }

    fn make_wallet_from_mnemonic(&self, mnemonic: MnemonicPhrase, passphrase: &str) -> Wallet {
        Wallet::from_mnemonic(mnemonic, passphrase, self.connection.network())
    }

    fn save_wallet_to_path(
        wallet: &Wallet,
        wallet_path: &str,
        password: &str,
    ) -> Result<(), String> {
        let path = PathBuf::from(wallet_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        wallet
            .save_to_datafile(&path, password)
            .map_err(|err| err.to_string())
    }

    fn open_wallet_from_path(&self, wallet_path: &str, password: &str) -> Result<Wallet, String> {
        let path = PathBuf::from(wallet_path);
        let wallet = Wallet::load_from_datafile(&path, password).map_err(|err| err.to_string())?;
        if wallet.network != self.connection.network() {
            return Err(format!(
                "wallet belongs to {} not {}",
                wallet.network.id(),
                self.connection.network().id()
            ));
        }
        Ok(wallet)
    }

    fn try_open_existing_wallet_on_startup(&mut self) {
        let path = default_wallet_path(self.connection.network());
        if !path.exists() {
            self.launch_page = LaunchPage::Welcome;
            return;
        }

        let wallet_path = path.to_string_lossy().into_owned();
        self.open_form.wallet_path = wallet_path.clone();
        self.open_form.wallet_password.clear();

        match self.open_wallet_from_path(&wallet_path, "") {
            Ok(wallet) => {
                self.load_or_create_wallet(wallet, wallet_path);
                self.last_error = None;
            }
            Err(_) => {
                self.launch_page = LaunchPage::OpenWallet;
                self.last_error = None;
            }
        }
    }

    fn load_or_create_wallet(&mut self, wallet: Wallet, wallet_path: String) {
        self.attach_wallet(wallet, wallet_path);
        self.send_status = String::from("Wallet loaded");
        self.last_error = None;
    }

    fn generate_receive_address(&mut self) {
        let label = if self.receive_label.trim().is_empty() {
            None
        } else {
            Some(self.receive_label.trim().to_owned())
        };

        let (address, snapshot) = {
            let Some(wallet) = self.wallet_mut() else {
                self.send_status = String::from("Load or create a wallet first");
                return;
            };
            let address = wallet.checkout_receive_address_with_label(label);
            let snapshot = wallet.snapshot.clone();
            (address, snapshot)
        };
        self.receive_addresses.push(address.clone());
        self.current_receive_address = Some(address);
        self.ui_state.wallet_snapshot = snapshot.clone();
        self.view_model.ui_state.wallet_snapshot = snapshot;
        self.send_status = String::from("Receive address generated");
        self.receive_label.clear();
    }

    fn create_receive_request(&mut self) {
        if self.wallet.is_none() {
            self.last_error = Some(String::from("Load or create a wallet first"));
            return;
        }

        let requested_amount = if self.receive_amount.trim().is_empty() {
            None
        } else {
            match self.receive_amount.trim().parse::<u64>() {
                Ok(value) if value > 0 => Some(value),
                _ => {
                    self.last_error = Some(String::from(
                        "Requested amount must be an integer atom value",
                    ));
                    return;
                }
            }
        };

        let label = self.receive_label.trim().to_owned();
        let message = self.receive_message.trim().to_owned();
        self.generate_receive_address();
        let address = self.current_receive_address_text();
        if address.is_empty() {
            return;
        }
        let sequence = self
            .requested_payments
            .iter()
            .map(|request| request.sequence)
            .max()
            .unwrap_or(0)
            + 1;
        self.requested_payments.push(ReceiveRequestRecord {
            sequence,
            label,
            message,
            amount_atoms: requested_amount,
            address,
        });
        self.selected_receive_request = Some(sequence);
        self.receive_amount.clear();
        self.receive_message.clear();
    }

    #[allow(dead_code)]
    fn generate_change_address(&mut self) {
        let (address, snapshot) = {
            let Some(wallet) = self.wallet_mut() else {
                self.send_status = String::from("Load or create a wallet first");
                return;
            };
            let address = wallet.checkout_change_address_with_label(None);
            let snapshot = wallet.snapshot.clone();
            (address, snapshot)
        };
        self.receive_addresses.push(address);
        self.ui_state.wallet_snapshot = snapshot.clone();
        self.view_model.ui_state.wallet_snapshot = snapshot;
        self.send_status = String::from("Change address generated");
    }

    fn submit_send_transaction(&mut self) -> Result<(), String> {
        if self.send_job.is_some() {
            return Err(String::from("A send submission is already in progress"));
        }

        let destination = self.send_to.trim();
        if destination.is_empty() {
            return Err(String::from("Enter a destination address"));
        }
        let amount = self
            .send_amount
            .trim()
            .parse::<u64>()
            .map_err(|_| String::from("Amount must be an integer atom value"))?;
        if amount == 0 {
            return Err(String::from("Amount must be greater than zero"));
        }

        let (recipient_digest, network) =
            decode_base56_address(destination).map_err(|err| err.to_string())?;
        if network != self.connection.network() {
            return Err(format!("Address belongs to {}", network.id()));
        }

        if self.wallet_utxos_cache.is_empty() {
            return Err(String::from(
                "No cached wallet UTXOs available; refresh the wallet first",
            ));
        }

        let (_selected_address, selected_utxos, keypair) = {
            let wallet = self
                .wallet_ref()
                .ok_or_else(|| String::from("Load or create a wallet first"))?;
            let mut grouped: BTreeMap<String, (WalletAddress, Vec<UtxoEntry>)> = BTreeMap::new();
            for utxo in self.wallet_utxos_cache.clone() {
                let digest: [u8; 32] = match utxo.locking_script.as_slice().try_into() {
                    Ok(digest) => digest,
                    Err(_) => continue,
                };
                if let Some(address) = wallet.address_for_payment_digest(&digest) {
                    let key = hex::encode(address.payment_digest);
                    grouped
                        .entry(key)
                        .or_insert((address, Vec::new()))
                        .1
                        .push(utxo);
                }
            }

            let Some((selected_address, selected_utxos)) =
                Self::select_wallet_utxos(grouped, amount)?
            else {
                return Err(String::from("No spendable wallet UTXOs available"));
            };

            let keypair = wallet.keypair_for_path(selected_address.path);
            (selected_address, selected_utxos, keypair)
        };

        let total_input_atoms: u64 = selected_utxos.iter().map(|utxo| utxo.value_atoms).sum();
        let fee_with_change = Self::estimate_fee(selected_utxos.len(), 2);
        if total_input_atoms < amount.saturating_add(fee_with_change) {
            return Err(String::from("selected inputs do not cover amount plus fee"));
        }
        let change_address = Some(
            self.wallet_mut()
                .ok_or_else(|| String::from("Load or create a wallet first"))?
                .checkout_change_address_with_label(None),
        );
        let provisional_change = total_input_atoms
            .checked_sub(amount)
            .and_then(|value| value.checked_sub(fee_with_change))
            .ok_or_else(|| String::from("selected inputs do not cover amount plus fee"))?;
        let provisional = Self::build_signed_spend_transaction(
            &keypair,
            &selected_utxos,
            recipient_digest,
            amount,
            provisional_change,
            change_address.clone(),
        )?;
        let fee_atoms = provisional.vsize_bytes() as u64 * MIN_TX_FEE_PER_VBYTE_ATOMS;
        let final_change = total_input_atoms
            .checked_sub(amount)
            .and_then(|value: u64| value.checked_sub(fee_atoms))
            .ok_or_else(|| String::from("selected inputs do not cover amount plus fee"))?;
        if final_change == 0 {
            return Err(String::from(
                "selected inputs do not leave change for privacy",
            ));
        }
        let final_transaction = Self::build_signed_spend_transaction(
            &keypair,
            &selected_utxos,
            recipient_digest,
            amount,
            final_change,
            change_address,
        )?;
        let fee_atoms = final_transaction.vsize_bytes() as u64 * MIN_TX_FEE_PER_VBYTE_ATOMS;
        let (sender, receiver) = mpsc::channel();
        let rpc_address = self.connection.rpc_address().to_string();
        std::thread::spawn(move || {
            let client = RpcClient::new(rpc_address);
            let result = match client.call(&RpcRequest::SubmitTransaction {
                transaction: final_transaction,
                fee_atoms,
            }) {
                Ok(RpcResponse::TransactionSubmitted(txid)) => Ok(SendOutcome {
                    fee_atoms,
                    message: format!("Transaction submitted {}", hex::encode(txid)),
                }),
                Ok(RpcResponse::Error(err)) => Err(err.to_string()),
                Ok(other) => Err(format!("unexpected rpc response: {other:?}")),
                Err(err) => Err(err.to_string()),
            };
            let _ = sender.send(result);
        });

        self.send_fee = fee_atoms.to_string();
        self.send_status = String::from("Submitting transaction...");
        self.last_error = None;
        self.send_job = Some(SendJob {
            started_at: Instant::now(),
            receiver,
        });
        Ok(())
    }

    fn select_wallet_utxos(
        grouped: BTreeMap<String, (WalletAddress, Vec<UtxoEntry>)>,
        amount_atoms: u64,
    ) -> Result<Option<(WalletAddress, Vec<UtxoEntry>)>, String> {
        let mut best: Option<(WalletAddress, Vec<UtxoEntry>, u64, usize)> = None;

        for (_key, (address, mut utxos)) in grouped {
            utxos.sort_by(|a, b| b.value_atoms.cmp(&a.value_atoms).then(a.txid.cmp(&b.txid)));
            let mut selected = Vec::new();
            let mut total = 0u64;
            for utxo in utxos {
                total = total.saturating_add(utxo.value_atoms);
                selected.push(utxo);
                let estimate_change_fee = Self::estimate_fee(selected.len(), 2);
                let estimate_exact_fee = Self::estimate_fee(selected.len(), 1);
                if total >= amount_atoms.saturating_add(estimate_change_fee) {
                    let candidate: (WalletAddress, Vec<UtxoEntry>, u64, usize) =
                        (address.clone(), selected.clone(), total, selected.len());
                    best = Self::prefer_candidate(best, candidate);
                    break;
                }
                if total >= amount_atoms.saturating_add(estimate_exact_fee) {
                    let candidate: (WalletAddress, Vec<UtxoEntry>, u64, usize) =
                        (address.clone(), selected.clone(), total, selected.len());
                    best = Self::prefer_candidate(best, candidate);
                }
            }
        }

        Ok(best.map(|(address, selected, _, _)| (address, selected)))
    }

    fn prefer_candidate(
        current: Option<(WalletAddress, Vec<UtxoEntry>, u64, usize)>,
        candidate: (WalletAddress, Vec<UtxoEntry>, u64, usize),
    ) -> Option<(WalletAddress, Vec<UtxoEntry>, u64, usize)> {
        match current {
            None => Some(candidate),
            Some(existing) => {
                let existing_inputs = existing.3;
                let candidate_inputs = candidate.3;
                if candidate_inputs < existing_inputs
                    || (candidate_inputs == existing_inputs && candidate.2 < existing.2)
                {
                    Some(candidate)
                } else {
                    Some(existing)
                }
            }
        }
    }

    fn estimate_fee(input_count: usize, output_count: usize) -> u64 {
        let mut inputs = Vec::with_capacity(input_count);
        for index in 0..input_count {
            inputs.push(TxInput {
                previous_txid: [0; 48],
                output_index: index as u32,
                unlocking_script: vec![0; 32],
            });
        }
        let mut outputs = Vec::with_capacity(output_count);
        for _ in 0..output_count {
            outputs.push(TxOutput {
                value_atoms: 1,
                locking_script: vec![0; 32],
            });
        }
        let witness = TxWitness {
            signature: vec![0; FALCON_512_SIGNATURE_MAX_BYTES],
            pubkey: vec![0; FALCON_512_PUBLIC_KEY_BYTES],
            input_refs: (0..input_count)
                .map(|_| atho_core::transaction::WitnessInputRef {
                    sig_ref_short: [0; 2],
                    witness_commit_ref: [0; 16],
                })
                .collect(),
        }
        .canonical_bytes();
        Transaction {
            version: 1,
            inputs,
            outputs,
            lock_time: 0,
            witness,
        }
        .vsize_bytes() as u64
            * MIN_TX_FEE_PER_VBYTE_ATOMS
    }

    fn build_signed_spend_transaction(
        keypair: &FalconKeypair,
        selected_utxos: &[UtxoEntry],
        recipient_digest: [u8; 32],
        amount_atoms: u64,
        change_atoms: u64,
        change_address: Option<WalletAddress>,
    ) -> Result<Transaction, String> {
        let mut outputs = vec![TxOutput {
            value_atoms: amount_atoms,
            locking_script: recipient_digest.to_vec(),
        }];
        if change_atoms > 0 {
            let change = change_address.ok_or_else(|| String::from("missing change address"))?;
            outputs.push(TxOutput {
                value_atoms: change_atoms,
                locking_script: change.payment_digest.to_vec(),
            });
        }

        let inputs: Vec<TxInput> = selected_utxos
            .iter()
            .map(|utxo| TxInput {
                previous_txid: utxo.txid,
                output_index: utxo.output_index,
                unlocking_script: utxo.locking_script.clone(),
            })
            .collect();

        let mut tx = Transaction {
            version: 1,
            inputs,
            outputs,
            lock_time: 0,
            witness: vec![],
        };
        let digest = tx.signing_digest();
        let signature = sign(&keypair.secret_key, &digest)
            .map_err(|err: atho_crypto::error::CryptoError| err.to_string())?;
        let txid = tx.txid();
        let sig_bytes = signature.0.clone();
        tx.witness = TxWitness {
            signature: sig_bytes.clone(),
            pubkey: keypair.public_key.0.clone(),
            input_refs: selected_utxos
                .iter()
                .enumerate()
                .map(|(index, _utxo)| atho_core::transaction::WitnessInputRef {
                    sig_ref_short: derive_sig_ref_short(&txid, &sig_bytes, index as u32),
                    witness_commit_ref: [0; 16],
                })
                .collect(),
        }
        .canonical_bytes();
        let witness_root = tx.witness_commitment_hash();
        let input_refs = selected_utxos
            .iter()
            .enumerate()
            .map(|(index, _utxo)| atho_core::transaction::WitnessInputRef {
                sig_ref_short: derive_sig_ref_short(&txid, &sig_bytes, index as u32),
                witness_commit_ref: derive_witness_commit_ref(&txid, &witness_root, index as u32),
            })
            .collect();
        tx.witness = TxWitness {
            signature: sig_bytes.clone(),
            pubkey: keypair.public_key.0.clone(),
            input_refs,
        }
        .canonical_bytes();
        Ok(tx)
    }

    fn current_receive_address_text(&self) -> String {
        self.current_receive_address
            .as_ref()
            .map(|address| address.address.clone())
            .unwrap_or_default()
    }

    fn selected_receive_request(&self) -> Option<&ReceiveRequestRecord> {
        let selected = self.selected_receive_request?;
        self.requested_payments
            .iter()
            .find(|request| request.sequence == selected)
    }

    fn select_receive_request(&mut self, sequence: usize) {
        self.selected_receive_request = Some(sequence);
    }

    fn remove_selected_receive_request(&mut self) {
        let Some(selected) = self.selected_receive_request else {
            return;
        };
        self.requested_payments
            .retain(|request| request.sequence != selected);
        self.selected_receive_request = self
            .requested_payments
            .last()
            .map(|request| request.sequence);
    }

    fn copy_text(ui: &mut egui::Ui, text: String) {
        ui.output_mut(|output| {
            output.copied_text = text;
        });
    }

    fn read_clipboard_text() -> Option<String> {
        let mut clipboard = arboard::Clipboard::new().ok()?;
        let text = clipboard.get_text().ok()?;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    }
}

impl eframe::App for DesktopApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.theme_initialized {
            theme::install_fonts(ctx);
            self.theme_initialized = true;
        }
        if self.wallet.is_none() && !self.compact_viewport {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(700.0, 440.0)));
            self.compact_viewport = true;
        } else if self.wallet.is_some() && self.compact_viewport {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1000.0, 660.0)));
            self.compact_viewport = false;
        }
        theme::apply_theme(ctx);
        self.drain_status_updates();
        self.poll_send_job();
        self.poll_mining_job();
        ctx.request_repaint_after(Duration::from_millis(200));

        if self.wallet.is_some() {
            shell::render_main_shell(self, ctx);
        } else {
            startup::render_startup_screen(self, ctx);
        }
    }
}

impl DesktopApp {
    fn drain_status_updates(&mut self) {
        while let Some(status) = self.status_monitor.try_recv_latest() {
            self.apply_connection_status(status);
        }
    }
}

fn default_wallet_path(network: Network) -> PathBuf {
    let base = home_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join(".atho")
        .join(network.id())
        .join(Wallet::datafile_name())
}

fn alternate_wallet_path(network: Network) -> PathBuf {
    let mut path = default_wallet_path(network);
    let file_name = format!("{}.2", Wallet::datafile_name());
    path.set_file_name(file_name);
    path
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}

fn mine_via_rpc(rpc_address: String, cores: u32) -> Result<MiningOutcome, String> {
    let client = RpcClient::new(rpc_address);
    let template = match client.call(&RpcRequest::GetBlockTemplate) {
        Ok(RpcResponse::BlockTemplate(template)) => template,
        Ok(RpcResponse::Error(err)) => return Err(err.to_string()),
        Ok(other) => return Err(format!("unexpected rpc response: {other:?}")),
        Err(err) => return Err(err.to_string()),
    };

    let miner = Miner::new(cores);
    let block = miner.solve_block(template.block);
    let block_hash = block.header.block_hash();
    match client.call(&RpcRequest::SubmitBlock(block)) {
        Ok(RpcResponse::BlockSubmitted { accepted: true, .. }) => Ok(MiningOutcome {
            height: template.height,
            block_hash,
            accepted: true,
            message: format!(
                "Block {} accepted at height {}",
                hex::encode(block_hash),
                template.height
            ),
        }),
        Ok(RpcResponse::BlockSubmitted {
            accepted: false, ..
        }) => Ok(MiningOutcome {
            height: template.height,
            block_hash,
            accepted: false,
            message: format!("Block {} rejected", hex::encode(block_hash)),
        }),
        Ok(RpcResponse::Error(err)) => Err(err.to_string()),
        Ok(other) => Err(format!("unexpected rpc response: {other:?}")),
        Err(err) => Err(err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_app_refreshes_view_state() {
        std::env::set_var("ATHO_QT_LOCAL", "1");
        let mut app = DesktopApp::new(Network::Mainnet);
        app.refresh().unwrap();
        assert!(app.ui_state.connected);
        assert_eq!(app.view_model.network_label, "atho-mainnet");
        assert!(matches!(
            app.launch_page,
            LaunchPage::Welcome | LaunchPage::OpenWallet
        ));
        std::env::remove_var("ATHO_QT_LOCAL");
    }
}
