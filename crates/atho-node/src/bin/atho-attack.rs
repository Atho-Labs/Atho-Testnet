use atho_core::block::{merkle_root, witness_root, Block, BlockHeader};
use atho_core::consensus::signatures::{transaction_signing_digest, AthoSignatureDomain};
use atho_core::consensus::{pow, subsidy};
use atho_core::constants::{
    MAX_BLOCK_SIZE_BYTES, MAX_SUPPLY_ATOMS, MAX_TRANSACTION_SIZE_BYTES, MIN_TX_FEE_PER_VBYTE_ATOMS,
};
use atho_core::genesis;
use atho_core::network::Network;
use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness, WitnessInputRef};
use atho_crypto::falcon::{generate_from_seed, sign, FalconKeypair};
use atho_node::config::NodeConfig;
use atho_node::miner::Miner;
use atho_node::node::Node;
use atho_node::validation::{derive_sig_ref_short, finalize_witness_commit_refs, ValidationError};
use atho_storage::chainstate::Chainstate as StorageChainstate;
use atho_storage::utxo::UtxoEntry;

const ATTACK_TXID: [u8; 48] = [7; 48];
const ATTACK_UNLOCKING_SCRIPT: [u8; 4] = [1, 2, 3, 4];
const ATTACK_OUTPUT_SCRIPT: [u8; 4] = [4, 3, 2, 1];

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let network = parse_network();
    let miner = Miner::new(mining_cores());
    let mut passed = 0usize;
    let mut total = 0usize;

    total += 1;
    if tx_case("valid_tx_accepts", network, tx_valid())? {
        passed += 1;
    }

    total += 1;
    if tx_case("bad_signature_rejects", network, tx_bad_signature())? {
        passed += 1;
    }

    total += 1;
    if tx_case("bad_fee_rejects", network, tx_bad_fee())? {
        passed += 1;
    }

    total += 1;
    if tx_case("witness_ref_rejects", network, tx_bad_witness_ref())? {
        passed += 1;
    }

    total += 1;
    if immature_coinbase_spend_case(network)? {
        passed += 1;
    }

    total += 1;
    if tx_case("dust_like_spend_accepts", network, tx_dust_like())? {
        passed += 1;
    }

    total += 1;
    if tx_case("duplicate_input_rejects", network, tx_duplicate_input())? {
        passed += 1;
    }

    total += 1;
    if tx_case("zero_output_rejects", network, tx_zero_output())? {
        passed += 1;
    }

    total += 1;
    if tx_case("oversized_tx_rejects", network, tx_oversized())? {
        passed += 1;
    }

    total += 1;
    if block_case(
        "valid_block_accepts",
        network,
        &miner,
        build_coinbase_block(
            network,
            7,
            [0; 48],
            subsidy::block_subsidy_atoms(7),
            0,
            network,
            valid_block_timestamp(network, 7),
        ),
        Ok(()),
    )? {
        passed += 1;
    }

    total += 1;
    if block_case(
        "inflation_block_rejects",
        network,
        &miner,
        build_coinbase_block(
            network,
            7,
            [0; 48],
            subsidy::block_subsidy_atoms(7).saturating_add(1),
            0,
            network,
            valid_block_timestamp(network, 7),
        ),
        Err(ValidationError::CoinbaseRewardMismatch),
    )? {
        passed += 1;
    }

    total += 1;
    if block_case(
        "second_coinbase_rejects",
        network,
        &miner,
        build_double_coinbase_block(network, 7, [0; 48]),
        Err(ValidationError::NoInputs),
    )? {
        passed += 1;
    }

    total += 1;
    if block_case(
        "wrong_parent_rejects",
        network,
        &miner,
        build_coinbase_block(
            network,
            7,
            [1; 48],
            subsidy::block_subsidy_atoms(7),
            0,
            network,
            valid_block_timestamp(network, 7),
        ),
        Err(ValidationError::BlockParentHashMismatch),
    )? {
        passed += 1;
    }

    total += 1;
    if block_case(
        "wrong_network_rejects",
        network,
        &miner,
        build_coinbase_block(
            network,
            7,
            [0; 48],
            subsidy::block_subsidy_atoms(7),
            0,
            other_network(network),
            valid_block_timestamp(network, 7),
        ),
        Err(ValidationError::BlockNetworkMismatch),
    )? {
        passed += 1;
    }

    total += 1;
    if block_case(
        "invalid_height_rejects",
        network,
        &miner,
        build_coinbase_block(
            network,
            8,
            [0; 48],
            subsidy::block_subsidy_atoms(8),
            0,
            network,
            valid_block_timestamp(network, 8),
        ),
        Err(ValidationError::InvalidBlockHeight),
    )? {
        passed += 1;
    }

    total += 1;
    if block_case(
        "oversized_block_rejects",
        network,
        &miner,
        build_coinbase_block(
            network,
            7,
            [0; 48],
            subsidy::block_subsidy_atoms(7),
            MAX_BLOCK_SIZE_BYTES + 1,
            network,
            valid_block_timestamp(network, 7),
        ),
        Err(ValidationError::BlockTooLarge),
    )? {
        passed += 1;
    }

    total += 1;
    if block_case(
        "double_spend_block_rejects",
        network,
        &miner,
        build_double_spend_block(network),
        Err(ValidationError::MissingUtxo),
    )? {
        passed += 1;
    }

    total += 1;
    if block_case(
        "timestamp_warp_rejects",
        network,
        &miner,
        build_coinbase_block(
            network,
            7,
            [0; 48],
            subsidy::block_subsidy_atoms(7),
            0,
            network,
            genesis::genesis_state(network)
                .block
                .header
                .timestamp
                .saturating_sub(1),
        ),
        Err(ValidationError::InvalidBlockTimestamp),
    )? {
        passed += 1;
    }

    total += 1;
    if direct_storage_injection(network)? {
        passed += 1;
    }

    println!(
        "max_supply_bound={}",
        if atho_core::consensus::subsidy::cumulative_subsidy_atoms(u64::MAX / 2) <= MAX_SUPPLY_ATOMS
        {
            "holds"
        } else {
            "violated"
        }
    );
    println!("summary passed={passed} total={total}");
    Ok(())
}

fn tx_case(name: &str, network: Network, tx: BuiltTransaction) -> Result<bool, String> {
    let mut node = seeded_node(network);
    let expected = tx.expected;
    let result = node.submit_transaction(atho_node::mempool::MempoolEntry::new(
        tx.transaction,
        tx.fee_atoms,
    ));
    let verdict = match (result, expected) {
        (Ok(_), Ok(())) => "accept".to_string(),
        (Err(atho_node::error::NodeError::Validation(err)), Err(expected_err)) => {
            if err == expected_err {
                format!("reject {err}")
            } else {
                return Err(format!("{name}: expected {expected_err}, got {err}"));
            }
        }
        (Ok(_), Err(expected_err)) => {
            return Err(format!(
                "{name}: expected reject {expected_err}, got accept"
            ));
        }
        (Err(err), Ok(())) => {
            return Err(format!("{name}: expected accept, got reject {err}"));
        }
        (Err(other), Err(expected_err)) => {
            return Err(format!("{name}: expected {expected_err}, got {other}"));
        }
    };
    println!("tx_case={name} verdict={verdict}");
    Ok(true)
}

fn block_case(
    name: &str,
    network: Network,
    miner: &Miner,
    block: Block,
    expected: Result<(), ValidationError>,
) -> Result<bool, String> {
    let mut node = seeded_node(network);
    let should_mine = match &expected {
        Ok(()) => true,
        Err(ValidationError::CoinbaseRewardMismatch)
        | Err(ValidationError::BlockParentHashMismatch)
        | Err(ValidationError::NoInputs)
        | Err(ValidationError::MempoolConflict)
        | Err(ValidationError::MissingUtxo)
        | Err(ValidationError::InvalidBlockTimestamp) => true,
        _ => false,
    };
    let mined = if should_mine {
        miner.solve_block(block)
    } else {
        block
    };
    let result = node.submit_block(&mined);
    let verdict = match (result, expected) {
        (Ok(()), Ok(())) => "accept".to_string(),
        (Err(atho_node::error::NodeError::Validation(err)), Err(expected_err)) => {
            if err == expected_err {
                format!("reject {err}")
            } else {
                return Err(format!("{name}: expected {expected_err}, got {err}"));
            }
        }
        (Ok(()), Err(expected_err)) => {
            return Err(format!(
                "{name}: expected reject {expected_err}, got accept"
            ));
        }
        (Err(err), Ok(())) => {
            return Err(format!("{name}: expected accept, got reject {err}"));
        }
        (Err(other), Err(expected_err)) => {
            return Err(format!("{name}: expected {expected_err}, got {other}"));
        }
    };
    println!("block_case={name} verdict={verdict}");
    Ok(true)
}

fn direct_storage_injection(network: Network) -> Result<bool, String> {
    let mut chainstate = StorageChainstate::new(network);
    let inflated = build_coinbase_block(
        network,
        7,
        [0; 48],
        subsidy::block_subsidy_atoms(7).saturating_add(1),
        0,
        network,
        valid_block_timestamp(network, 7),
    );
    match chainstate.connect_block(&inflated) {
        Ok(()) => Err(String::from("storage_bypass=accepted_invalid_block")),
        Err(err) => {
            println!("storage_bypass=rejected_invalid_block reason={err}");
            Ok(true)
        }
    }
}

#[derive(Debug)]
struct BuiltTransaction {
    transaction: Transaction,
    fee_atoms: u64,
    expected: Result<(), ValidationError>,
}

fn tx_valid() -> BuiltTransaction {
    let input_value = 10_000u64;
    let output_value = 9_000u64;
    let tx = make_spend_tx(input_value, output_value, false, false, false);
    BuiltTransaction {
        transaction: tx,
        fee_atoms: input_value - output_value,
        expected: Ok(()),
    }
}

fn tx_bad_signature() -> BuiltTransaction {
    let input_value = 10_000u64;
    let output_value = 9_000u64;
    let tx = make_spend_tx(input_value, output_value, true, false, false);
    BuiltTransaction {
        transaction: tx,
        fee_atoms: input_value - output_value,
        expected: Err(ValidationError::InvalidWitness),
    }
}

fn tx_bad_fee() -> BuiltTransaction {
    let input_value = 10_000u64;
    let output_value = 10_000u64;
    let tx = make_spend_tx(input_value, output_value, false, false, false);
    BuiltTransaction {
        transaction: tx,
        fee_atoms: 0,
        expected: Err(ValidationError::FeeBelowMinimum),
    }
}

fn tx_bad_witness_ref() -> BuiltTransaction {
    let input_value = 10_000u64;
    let output_value = 9_000u64;
    let tx = make_spend_tx(input_value, output_value, false, true, false);
    BuiltTransaction {
        transaction: tx,
        fee_atoms: input_value - output_value,
        expected: Err(ValidationError::WitnessInputReferenceMismatch),
    }
}

fn tx_dust_like() -> BuiltTransaction {
    let input_value = 10_000u64;
    let output_value = 1u64;
    let tx = make_spend_tx(input_value, output_value, false, false, false);
    BuiltTransaction {
        transaction: tx,
        fee_atoms: input_value - output_value,
        expected: Ok(()),
    }
}

fn tx_duplicate_input() -> BuiltTransaction {
    let keypair = attack_keypair();
    let mut tx = Transaction {
        version: 1,
        inputs: vec![
            TxInput {
                previous_txid: ATTACK_TXID,
                output_index: 0,
                unlocking_script: ATTACK_UNLOCKING_SCRIPT.to_vec(),
            },
            TxInput {
                previous_txid: ATTACK_TXID,
                output_index: 0,
                unlocking_script: ATTACK_UNLOCKING_SCRIPT.to_vec(),
            },
        ],
        outputs: vec![TxOutput {
            value_atoms: 9_000,
            locking_script: ATTACK_OUTPUT_SCRIPT.to_vec(),
        }],
        lock_time: 0,
        witness: vec![],
    };
    let signature = sign(
        AthoSignatureDomain::Transaction,
        &keypair.secret_key,
        &transaction_signing_digest(&tx),
    )
    .expect("falcon signature");
    let signature_bytes = signature.0.clone();
    let witness = TxWitness {
        signature: signature_bytes.clone(),
        pubkey: keypair.public_key.0.clone(),
        input_refs: vec![
            WitnessInputRef {
                sig_ref_short: derive_sig_ref_short(&tx.txid(), &signature_bytes, 0),
                witness_commit_ref: [0; 16],
            },
            WitnessInputRef {
                sig_ref_short: derive_sig_ref_short(&tx.txid(), &signature_bytes, 1),
                witness_commit_ref: [0; 16],
            },
        ],
    };
    tx.witness = witness.canonical_bytes();
    BuiltTransaction {
        transaction: tx,
        fee_atoms: 1_000,
        expected: Err(ValidationError::DuplicateInput),
    }
}

fn tx_zero_output() -> BuiltTransaction {
    let input_value = 10_000u64;
    let tx = make_spend_tx(input_value, 0, false, false, false);
    BuiltTransaction {
        transaction: tx,
        fee_atoms: input_value,
        expected: Err(ValidationError::ZeroValueOutput),
    }
}

fn tx_oversized() -> BuiltTransaction {
    let input_value = 10_000u64;
    let output_value = 9_000u64;
    let tx = make_spend_tx(input_value, output_value, false, false, true);
    BuiltTransaction {
        transaction: tx,
        fee_atoms: input_value - output_value,
        expected: Err(ValidationError::TransactionTooLarge),
    }
}

fn make_spend_tx(
    input_value: u64,
    output_value: u64,
    mutate_signature: bool,
    bad_sig_ref: bool,
    huge_output_script: bool,
) -> Transaction {
    let keypair = attack_keypair();
    let mut tx = Transaction {
        version: 1,
        inputs: vec![TxInput {
            previous_txid: ATTACK_TXID,
            output_index: 0,
            unlocking_script: ATTACK_UNLOCKING_SCRIPT.to_vec(),
        }],
        outputs: vec![TxOutput {
            value_atoms: output_value,
            locking_script: if huge_output_script {
                vec![0; MAX_TRANSACTION_SIZE_BYTES + 1]
            } else {
                ATTACK_OUTPUT_SCRIPT.to_vec()
            },
        }],
        lock_time: 0,
        witness: vec![],
    };

    let signature = sign(
        AthoSignatureDomain::Transaction,
        &keypair.secret_key,
        &transaction_signing_digest(&tx),
    )
    .expect("falcon signature");
    let mut signature_bytes = signature.0.clone();
    if mutate_signature {
        signature_bytes[0] ^= 0xff;
    }
    let sig_ref_short = if bad_sig_ref {
        [0u8; 2]
    } else {
        derive_sig_ref_short(&tx.txid(), &signature_bytes, 0)
    };
    let witness = TxWitness {
        signature: signature_bytes,
        pubkey: keypair.public_key.0.clone(),
        input_refs: vec![WitnessInputRef {
            sig_ref_short,
            witness_commit_ref: [0; 16],
        }],
    };
    tx.witness = witness.canonical_bytes();
    let _ = input_value;
    tx
}

fn valid_block_timestamp(network: Network, height: u64) -> u64 {
    let genesis_timestamp = genesis::genesis_state(network).block.header.timestamp;
    genesis_timestamp.saturating_add(height.max(1))
}

fn build_coinbase_block(
    _network: Network,
    height: u64,
    previous_block_hash: [u8; 48],
    coinbase_value_atoms: u64,
    script_len: usize,
    block_network: Network,
    timestamp: u64,
) -> Block {
    let coinbase = Transaction {
        version: 1,
        inputs: vec![],
        outputs: vec![TxOutput {
            value_atoms: coinbase_value_atoms,
            locking_script: vec![0; script_len],
        }],
        lock_time: u32::try_from(height).unwrap_or(u32::MAX),
        witness: vec![],
    };
    let transactions = vec![coinbase];
    let header = BlockHeader {
        version: 1,
        network_id: block_network,
        height,
        previous_block_hash,
        merkle_root: merkle_root(&transactions),
        witness_root: witness_root(&transactions),
        timestamp,
        difficulty_target_or_bits: pow::target_for_height(block_network, height),
        nonce: 0,
    };
    Block::new(header, transactions)
}

fn build_double_coinbase_block(
    network: Network,
    height: u64,
    previous_block_hash: [u8; 48],
) -> Block {
    let coinbase_a = Transaction {
        version: 1,
        inputs: vec![],
        outputs: vec![TxOutput {
            value_atoms: subsidy::block_subsidy_atoms(height),
            locking_script: vec![0; 4],
        }],
        lock_time: u32::try_from(height).unwrap_or(u32::MAX),
        witness: vec![],
    };
    let coinbase_b = Transaction {
        version: 1,
        inputs: vec![],
        outputs: vec![TxOutput {
            value_atoms: 1,
            locking_script: vec![1; 4],
        }],
        lock_time: u32::try_from(height).unwrap_or(u32::MAX),
        witness: vec![],
    };
    let transactions = vec![coinbase_a, coinbase_b];
    let header = BlockHeader {
        version: 1,
        network_id: network,
        height,
        previous_block_hash,
        merkle_root: merkle_root(&transactions),
        witness_root: witness_root(&transactions),
        timestamp: valid_block_timestamp(network, height),
        difficulty_target_or_bits: pow::target_for_height(network, height),
        nonce: 0,
    };
    Block::new(header, transactions)
}

fn build_double_spend_block(network: Network) -> Block {
    let coinbase = Transaction {
        version: 1,
        inputs: vec![],
        outputs: vec![TxOutput {
            value_atoms: subsidy::block_subsidy_atoms(7),
            locking_script: vec![0; 4],
        }],
        lock_time: 7,
        witness: vec![],
    };
    let spend_a = make_block_spend_tx();
    let spend_b = spend_a.clone();
    let transactions = vec![coinbase, spend_a, spend_b];
    let block_witness_root = witness_root(&transactions);
    let transactions = transactions
        .into_iter()
        .map(|tx| finalize_witness_commit_refs(&tx, block_witness_root))
        .collect::<Vec<_>>();
    let header = BlockHeader {
        version: 1,
        network_id: network,
        height: 7,
        previous_block_hash: [0; 48],
        merkle_root: merkle_root(&transactions),
        witness_root: witness_root(&transactions),
        timestamp: valid_block_timestamp(network, 7),
        difficulty_target_or_bits: pow::target_for_height(network, 7),
        nonce: 0,
    };
    Block::new(header, transactions)
}

fn make_block_spend_tx() -> Transaction {
    let input_value = 10_000u64;
    let mut output_value = input_value.saturating_sub(1);
    for _ in 0..64 {
        let tx = make_spend_tx(input_value, output_value, false, false, false);
        let required_fee = tx.vsize_bytes() as u64 * MIN_TX_FEE_PER_VBYTE_ATOMS;
        let actual_fee = input_value.saturating_sub(tx.output_value_atoms());
        if actual_fee == required_fee {
            return tx;
        }
        output_value = input_value.saturating_sub(required_fee);
    }
    make_spend_tx(input_value, output_value, false, false, false)
}

fn immature_coinbase_spend_case(network: Network) -> Result<bool, String> {
    let mut node = Node::new(NodeConfig::new(network));
    node.dev_seed_chainstate(
        148,
        [0; 48],
        [UtxoEntry::coinbase(
            network,
            ATTACK_TXID,
            0,
            10_000,
            ATTACK_UNLOCKING_SCRIPT.to_vec(),
            0,
        )],
    )
    .map_err(|err| err.to_string())?;

    let tx = make_spend_tx(10_000, 9_000, false, false, false);
    let result = node.submit_transaction(atho_node::mempool::MempoolEntry::new(tx, 1_000));
    match result {
        Err(atho_node::error::NodeError::Validation(
            ValidationError::InsufficientConfirmations,
        )) => {
            println!(
                "tx_case=immature_coinbase_spend_rejects verdict=reject insufficient confirmations"
            );
            Ok(true)
        }
        Err(other) => Err(format!(
            "immature_coinbase_spend_rejects: expected insufficient confirmations, got {other}"
        )),
        Ok(_) => Err(String::from(
            "immature_coinbase_spend_rejects: expected reject, got accept",
        )),
    }
}

fn seeded_node(network: Network) -> Node {
    let mut node = Node::new(NodeConfig::new(network));
    node.dev_seed_chainstate(
        6,
        [0; 48],
        [UtxoEntry::new(
            network,
            ATTACK_TXID,
            0,
            10_000,
            ATTACK_UNLOCKING_SCRIPT.to_vec(),
            0,
            false,
        )],
    )
    .expect("seed utxo");
    node
}

fn attack_keypair() -> FalconKeypair {
    generate_from_seed(b"atho-attack-seed").expect("attack keypair")
}

fn parse_network() -> Network {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--network" | "-n" => {
                if let Some(value) = args.next() {
                    return match value.as_str() {
                        "mainnet" => Network::Mainnet,
                        "testnet" => Network::Testnet,
                        "regnet" | "regtest" => Network::Regnet,
                        _ => Network::Regnet,
                    };
                }
            }
            "mainnet" => return Network::Mainnet,
            "testnet" => return Network::Testnet,
            "regnet" | "regtest" => return Network::Regnet,
            _ => {}
        }
    }
    Network::Regnet
}

fn mining_cores() -> u32 {
    std::thread::available_parallelism()
        .map(|parallelism| parallelism.get().min(4) as u32)
        .unwrap_or(1)
}

fn other_network(network: Network) -> Network {
    match network {
        Network::Mainnet => Network::Testnet,
        Network::Testnet => Network::Regnet,
        Network::Regnet => Network::Mainnet,
    }
}
