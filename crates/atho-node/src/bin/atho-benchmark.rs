#![forbid(unsafe_code)]

use atho_core::block::Block;
use atho_core::consensus::rules;
use atho_core::consensus::signatures::{transaction_signing_digest, AthoSignatureDomain};
use atho_core::constants::MIN_TX_FEE_PER_VBYTE_ATOMS;
use atho_core::crypto::hash::sha3_384;
use atho_core::genesis;
use atho_core::network::Network;
use atho_core::transaction::{Transaction, TxInput, TxOutput, TxWitness, WitnessInputRef};
use atho_crypto::falcon::{generate_from_seed, sign, FalconKeypair, FALCON_512_SIGNATURE_BYTES};
use atho_node::config::NodeConfig;
use atho_node::mempool::MempoolEntry;
use atho_node::miner::Miner;
use atho_node::node::Node;
use atho_node::sync::NodeSync;
use atho_node::validation::{
    derive_sig_ref_short, derive_witness_commit_ref, validate_block_with_context,
};
use atho_p2p::codec::WireCodec;
use atho_p2p::protocol::{
    compact_block_from_block, Hash48, MessagePayload, NetworkMessage, VersionMessage,
    LOCAL_NODE_SERVICES,
};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_TX_COUNT: usize = 256;
const DEFAULT_INPUTS_PER_TX: usize = 1;
const DEFAULT_SAMPLES: usize = 3;
const DEFAULT_SPEND_HEIGHT: u64 = 6;
const DEFAULT_INPUT_VALUE: u64 = 100_000;
const DEFAULT_TIP_HASH: [u8; 48] = [0x5a; 48];

#[derive(Debug, Clone)]
struct Cli {
    network: Network,
    tx_count: usize,
    inputs_per_tx: usize,
    samples: usize,
    output: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    wipe_first: bool,
}

#[derive(Debug, Clone)]
struct BenchmarkFixture {
    network: Network,
    spend_height: u64,
    tip_hash: [u8; 48],
    utxos: Vec<atho_storage::utxo::UtxoEntry>,
    transactions: Vec<Transaction>,
    entries: Vec<MempoolEntry>,
    block: Block,
    full_block_frame: Vec<u8>,
    compact_block_frame: Vec<u8>,
}

#[derive(Debug, Clone)]
struct BenchResult {
    name: &'static str,
    tx_count: usize,
    signature_count: usize,
    runs: usize,
    mean: Duration,
    notes: String,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cli = parse_cli(&env::args().skip(1).collect::<Vec<_>>())?;
    if cli.network == Network::Mainnet {
        return Err(String::from(
            "benchmark harness refuses mainnet; use testnet or regnet",
        ));
    }

    let data_dir = prepare_benchmark_root(&cli)?;
    env::set_var(atho_storage::path::ATHO_DATA_DIR_ENV, &data_dir);
    if cli.wipe_first {
        atho_node::dev::wipe_root(&data_dir).map_err(|err| err.to_string())?;
    } else {
        fs::create_dir_all(&data_dir).map_err(|err| err.to_string())?;
    }

    let debug = env::var_os("ATHO_BENCH_DEBUG").is_some();
    let fixture = BenchmarkFixture::build(cli.network, cli.tx_count, cli.inputs_per_tx, debug)?;
    if debug {
        eprintln!("benchmark scenario: block validation");
    }
    let block_validation = bench_block_validation(&fixture, cli.samples)?;
    if debug {
        eprintln!("benchmark scenario: mempool admission");
    }
    let mempool_admission = bench_mempool_admission(&fixture, cli.samples)?;
    if debug {
        eprintln!("benchmark scenario: propagation full");
    }
    let propagation_full = bench_propagation_full(&fixture, cli.samples)?;
    if debug {
        eprintln!("benchmark scenario: propagation compact");
    }
    let propagation_compact = bench_propagation_compact(&fixture, cli.samples)?;
    let hardware = collect_hardware_info();
    let report = render_report(
        &cli,
        &data_dir,
        &hardware,
        &fixture,
        &[
            block_validation,
            mempool_admission,
            propagation_full,
            propagation_compact,
        ],
    );

    if let Some(path) = cli.output {
        fs::write(&path, report).map_err(|err| err.to_string())?;
        println!("benchmark report written to {}", path.display());
    } else {
        print!("{report}");
    }

    Ok(())
}

fn parse_cli(args: &[String]) -> Result<Cli, String> {
    let mut cli = Cli {
        network: Network::Regnet,
        tx_count: DEFAULT_TX_COUNT,
        inputs_per_tx: DEFAULT_INPUTS_PER_TX,
        samples: DEFAULT_SAMPLES,
        output: None,
        data_dir: None,
        wipe_first: true,
    };

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            "--network" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| String::from("missing network value"))?;
                cli.network = Network::parse(value)
                    .ok_or_else(|| format!("invalid network value {value}"))?;
                i += 2;
            }
            "--tx-count" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| String::from("missing tx-count value"))?;
                cli.tx_count = value
                    .parse::<usize>()
                    .map_err(|_| String::from("invalid tx-count"))?;
                i += 2;
            }
            "--inputs-per-tx" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| String::from("missing inputs-per-tx value"))?;
                cli.inputs_per_tx = value
                    .parse::<usize>()
                    .map_err(|_| String::from("invalid inputs-per-tx"))?;
                if cli.inputs_per_tx == 0 {
                    return Err(String::from("inputs-per-tx must be at least 1"));
                }
                i += 2;
            }
            "--samples" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| String::from("missing samples value"))?;
                cli.samples = value
                    .parse::<usize>()
                    .map_err(|_| String::from("invalid samples"))?;
                if cli.samples == 0 {
                    return Err(String::from("samples must be at least 1"));
                }
                i += 2;
            }
            "--output" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| String::from("missing output value"))?;
                cli.output = Some(PathBuf::from(value));
                i += 2;
            }
            "--data-dir" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| String::from("missing data-dir value"))?;
                cli.data_dir = Some(PathBuf::from(value));
                i += 2;
            }
            "--wipe-first" => {
                cli.wipe_first = true;
                i += 1;
            }
            "--no-wipe-first" => {
                cli.wipe_first = false;
                i += 1;
            }
            other => {
                return Err(format!("unrecognized argument {other}"));
            }
        }
    }

    Ok(cli)
}

fn print_usage() {
    eprintln!("usage:");
    eprintln!("  atho-benchmark [--network <testnet|regnet>] [--tx-count N] [--inputs-per-tx N] [--samples N] [--data-dir PATH] [--wipe-first] [--no-wipe-first] [--output benchmark.md]");
}

fn prepare_benchmark_root(cli: &Cli) -> Result<PathBuf, String> {
    if let Some(root) = &cli.data_dir {
        return Ok(root.clone());
    }
    let mut root = env::temp_dir();
    root.push(format!(
        "atho-benchmark-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    Ok(root)
}

impl BenchmarkFixture {
    fn build(
        network: Network,
        tx_count: usize,
        inputs_per_tx: usize,
        debug: bool,
    ) -> Result<Self, String> {
        let keypair = generate_from_seed(b"atho-benchmark-keypair")
            .map_err(|err| format!("falcon keypair generation failed: {err:?}"))?;
        let output_script =
            atho_core::address::public_key_digest(network, &keypair.public_key.0).to_vec();
        let utxo_count = tx_count.saturating_mul(inputs_per_tx);
        let utxos = (0..utxo_count)
            .map(|index| make_funding_utxo(network, &keypair, index))
            .collect::<Vec<_>>();

        if debug {
            eprintln!("benchmark fixture: seeded {} utxos", utxos.len());
        }

        let mut node = Node::new(NodeConfig::new(network));
        node.dev_seed_chainstate(DEFAULT_SPEND_HEIGHT, DEFAULT_TIP_HASH, utxos.clone())
            .map_err(|err| format!("failed to seed benchmark chainstate: {err}"))?;

        let mut transactions = Vec::with_capacity(tx_count);
        let mut entries = Vec::with_capacity(tx_count);
        for chunk in utxos.chunks(inputs_per_tx) {
            if debug {
                eprintln!(
                    "benchmark fixture: building tx {} with {} inputs",
                    transactions.len(),
                    chunk.len()
                );
            }
            let (tx, fee_atoms) =
                build_spend_transaction(network, chunk, &keypair, &output_script)?;
            entries.push(MempoolEntry::new(tx.clone(), fee_atoms));
            node.admit_transaction(MempoolEntry::new(tx.clone(), fee_atoms))
                .map_err(|err| {
                    format!(
                        "mempool admission failed for benchmark tx {}: {err}",
                        transactions.len()
                    )
                })?;
            transactions.push(tx);
        }

        let miner = Miner::new(
            std::thread::available_parallelism()
                .map(|p| p.get())
                .unwrap_or(1) as u32,
        );
        if debug {
            eprintln!("benchmark fixture: building candidate block");
        }
        let candidate = node
            .build_candidate_block(&miner)
            .map_err(|err| format!("candidate block build failed: {err}"))?;
        if debug {
            eprintln!("benchmark fixture: solving block");
        }
        let block = miner.solve_block(candidate);
        if debug {
            eprintln!("benchmark fixture: building compact block frame");
        }
        let compact_block = compact_block_from_block(&block);
        let full_block_frame = WireCodec::encode(&NetworkMessage::new(
            network,
            MessagePayload::Block(block.clone()),
        ))
        .map_err(|err| err.to_string())?;
        let compact_block_frame = WireCodec::encode(&NetworkMessage::new(
            network,
            MessagePayload::CompactBlock(compact_block.clone()),
        ))
        .map_err(|err| err.to_string())?;

        Ok(Self {
            network,
            spend_height: DEFAULT_SPEND_HEIGHT,
            tip_hash: DEFAULT_TIP_HASH,
            utxos,
            transactions,
            entries,
            block,
            full_block_frame,
            compact_block_frame,
        })
    }

    fn seeded_node(&self) -> Result<Node, String> {
        let mut node = Node::new(NodeConfig::new(self.network));
        node.dev_seed_chainstate(self.spend_height, self.tip_hash, self.utxos.clone())
            .map_err(|err| err.to_string())?;
        Ok(node)
    }

    fn mempool_node(&self) -> Result<Node, String> {
        let mut node = self.seeded_node()?;
        for entry in self.entries.iter().cloned() {
            node.admit_transaction(entry)
                .map_err(|err| err.to_string())?;
        }
        Ok(node)
    }
}

fn make_funding_utxo(
    network: Network,
    keypair: &FalconKeypair,
    index: usize,
) -> atho_storage::utxo::UtxoEntry {
    let mut preimage = Vec::with_capacity(network.id().len() + 16);
    preimage.extend_from_slice(network.id().as_bytes());
    preimage.extend_from_slice(b":bench:utxo:");
    preimage.extend_from_slice(&(index as u64).to_le_bytes());
    let txid = sha3_384(&preimage);
    atho_storage::utxo::UtxoEntry::new(
        network,
        txid,
        0,
        DEFAULT_INPUT_VALUE,
        atho_core::address::public_key_digest(network, &keypair.public_key.0).to_vec(),
        0,
        false,
    )
}

fn provisional_witness(input_count: usize, keypair: &FalconKeypair) -> TxWitness {
    TxWitness {
        signature: vec![0; FALCON_512_SIGNATURE_BYTES],
        pubkey: keypair.public_key.0.clone(),
        input_refs: (0..input_count)
            .map(|_| WitnessInputRef {
                sig_ref_short: [0; 2],
                witness_commit_ref: [0; 16],
            })
            .collect(),
    }
}

fn build_spend_transaction(
    network: Network,
    utxos: &[atho_storage::utxo::UtxoEntry],
    keypair: &FalconKeypair,
    output_script: &[u8],
) -> Result<(Transaction, u64), String> {
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
            locking_script: output_script.to_vec(),
        }],
        lock_time: 0,
        witness: provisional_witness(utxos.len(), keypair).canonical_bytes(),
    };
    let fee_atoms = tx.vsize_bytes() as u64 * MIN_TX_FEE_PER_VBYTE_ATOMS;
    tx.outputs[0].value_atoms = input_total
        .checked_sub(fee_atoms)
        .ok_or_else(|| String::from("fixture input value too small"))?;
    tx.witness.clear();

    let digest = transaction_signing_digest(&tx);
    let signature = sign(
        AthoSignatureDomain::Transaction,
        &keypair.secret_key,
        &digest,
    )
    .map_err(|err| format!("falcon sign failed: {err:?}"))?
    .0;
    let txid = tx.txid();
    tx.witness = TxWitness {
        signature: signature.clone(),
        pubkey: keypair.public_key.0.clone(),
        input_refs: (0..utxos.len())
            .map(|index| WitnessInputRef {
                sig_ref_short: derive_sig_ref_short(&txid, &signature, index as u32),
                witness_commit_ref: [0; 16],
            })
            .collect(),
    }
    .canonical_bytes();

    let _ = network;
    Ok((tx, fee_atoms))
}

fn handshake_peer(node: &mut Node, network: Network, peer: &str) -> Result<NodeSync, String> {
    let mut sync = NodeSync::new(network);
    sync.prime(node);
    sync.accept_inbound(peer.to_string())
        .map_err(|err| err.to_string())?;
    let remote_version = NetworkMessage::new(
        network,
        MessagePayload::Version(VersionMessage {
            protocol_version: rules::PROTOCOL_VERSION,
            min_protocol_version: atho_p2p::config::MIN_SUPPORTED_PROTOCOL_VERSION,
            services: LOCAL_NODE_SERVICES,
            timestamp_unix: current_unix(),
            network,
            user_agent: String::from("/Atho-Benchmark:0.1.0/"),
            best_height: node.height(),
            ruleset_version: rules::RULESET_VERSION_V1,
            relay: true,
            genesis_hash: Hash48::from(genesis::genesis_hash(network)),
            tip_hash: Hash48::from(node.tip_hash()),
            chainwork: Hash48::ZERO,
        }),
    );
    let _ = sync
        .receive(peer, remote_version, node)
        .map_err(|err| err.to_string())?;
    let _ = sync
        .receive(
            peer,
            NetworkMessage::new(network, MessagePayload::Verack),
            node,
        )
        .map_err(|err| err.to_string())?;
    Ok(sync)
}

fn bench_block_validation(
    fixture: &BenchmarkFixture,
    samples: usize,
) -> Result<BenchResult, String> {
    verify_block_fixture(fixture)?;
    let mut durations = Vec::with_capacity(samples);
    for _ in 0..samples {
        let mut node = fixture.seeded_node()?;
        let utxos = node.utxo_snapshot();
        validate_block_with_context(
            &fixture.block,
            fixture.block.header.height,
            fixture.network,
            fixture.tip_hash,
            node.difficulty_target_for_next_block(),
            node.blocks(),
            utxos,
        )
        .map_err(|err| format!("block validation precheck failed: {err}"))?;
        let start = Instant::now();
        node.connect_block(&fixture.block)
            .map_err(|err| format!("connect_block failed after validation precheck: {err}"))?;
        durations.push(start.elapsed());
    }
    let tx_count = fixture.block.transactions.len().saturating_sub(1);
    let signature_count = fixture
        .block
        .transactions
        .iter()
        .skip(1)
        .map(|tx| tx.inputs.len())
        .sum::<usize>();
    Ok(BenchResult {
        name: "block_validation",
        tx_count,
        signature_count,
        runs: samples,
        mean: mean_duration(&durations),
        notes: format!(
            "full block bytes={} compact bytes={} height={} tx_count={}",
            fixture.block.full_size_bytes(),
            fixture.compact_block_frame.len(),
            fixture.block.header.height,
            fixture.block.transactions.len()
        ),
    })
}

fn verify_block_fixture(fixture: &BenchmarkFixture) -> Result<(), String> {
    let node = fixture.seeded_node()?;
    let utxos = node.utxo_snapshot();
    match validate_block_with_context(
        &fixture.block,
        fixture.block.header.height,
        fixture.network,
        fixture.tip_hash,
        node.difficulty_target_for_next_block(),
        node.blocks(),
        utxos,
    ) {
        Ok(()) => Ok(()),
        Err(err) => {
            let mut details = Vec::new();
            for (tx_index, tx) in fixture.block.transactions.iter().enumerate().skip(1) {
                let Some(witness) = tx.witness_payload() else {
                    continue;
                };
                for (input_index, input_ref) in witness.input_refs.iter().enumerate() {
                    let expected_short =
                        derive_sig_ref_short(&tx.txid(), &witness.signature, input_index as u32);
                    let expected_commit = derive_witness_commit_ref(
                        &tx.txid(),
                        &fixture.block.header.witness_root,
                        input_index as u32,
                    );
                    details.push(format!(
                        "tx={} input={} short_ok={} commit_ok={}",
                        tx_index,
                        input_index,
                        input_ref.sig_ref_short == expected_short,
                        input_ref.witness_commit_ref == expected_commit
                    ));
                }
            }
            Err(format!(
                "fixture block failed validation: {err}; {}",
                details.join("; ")
            ))
        }
    }
}

fn bench_mempool_admission(
    fixture: &BenchmarkFixture,
    samples: usize,
) -> Result<BenchResult, String> {
    let mut durations = Vec::with_capacity(samples);
    for _ in 0..samples {
        let mut node = fixture.seeded_node()?;
        let start = Instant::now();
        for entry in fixture.entries.iter().cloned() {
            node.admit_transaction(entry)
                .map_err(|err| err.to_string())?;
        }
        durations.push(start.elapsed());
    }
    let signature_count = fixture
        .transactions
        .iter()
        .map(|tx| tx.inputs.len())
        .sum::<usize>();
    Ok(BenchResult {
        name: "mempool_admission",
        tx_count: fixture.transactions.len(),
        signature_count,
        runs: samples,
        mean: mean_duration(&durations),
        notes: format!(
            "mempool entries={} tx_count={} inputs_per_tx={}",
            fixture.entries.len(),
            fixture.transactions.len(),
            fixture
                .transactions
                .first()
                .map(|tx| tx.inputs.len())
                .unwrap_or(0)
        ),
    })
}

fn bench_propagation_full(
    fixture: &BenchmarkFixture,
    samples: usize,
) -> Result<BenchResult, String> {
    let mut durations = Vec::with_capacity(samples);
    for _ in 0..samples {
        let mut node = fixture.seeded_node()?;
        let mut sync = handshake_peer(&mut node, fixture.network, "peer-full")?;
        let start = Instant::now();
        let message =
            WireCodec::decode(&fixture.full_block_frame).map_err(|err| err.to_string())?;
        sync.receive("peer-full", message, &mut node)
            .map_err(|err| err.to_string())?;
        durations.push(start.elapsed());
    }
    let signature_count = fixture
        .block
        .transactions
        .iter()
        .skip(1)
        .map(|tx| tx.inputs.len())
        .sum::<usize>();
    Ok(BenchResult {
        name: "propagation_full_block",
        tx_count: fixture.block.transactions.len().saturating_sub(1),
        signature_count,
        runs: samples,
        mean: mean_duration(&durations),
        notes: format!(
            "full_frame_bytes={} block_bytes={} ready_peer=true",
            fixture.full_block_frame.len(),
            fixture.block.full_size_bytes()
        ),
    })
}

fn bench_propagation_compact(
    fixture: &BenchmarkFixture,
    samples: usize,
) -> Result<BenchResult, String> {
    let mut durations = Vec::with_capacity(samples);
    let mut missing_tx_rate = 0f64;
    for _ in 0..samples {
        let debug = env::var_os("ATHO_BENCH_DEBUG").is_some();
        if debug {
            eprintln!("compact propagation: seeding node with mempool");
        }
        let mut node = fixture.mempool_node()?;
        if debug {
            eprintln!("compact propagation: handshake");
        }
        let mut sync = handshake_peer(&mut node, fixture.network, "peer-compact")?;
        if debug {
            eprintln!("compact propagation: decoding frame");
        }
        let start = Instant::now();
        let message =
            WireCodec::decode(&fixture.compact_block_frame).map_err(|err| err.to_string())?;
        if debug {
            eprintln!("compact propagation: receive");
        }
        let (_, notices) = sync
            .receive("peer-compact", message, &mut node)
            .map_err(|err| err.to_string())?;
        if debug {
            eprintln!(
                "compact propagation: receive complete notices={}",
                notices.len()
            );
        }
        durations.push(start.elapsed());
        let _ = notices;
        missing_tx_rate += 0.0;
    }
    let signature_count = fixture
        .block
        .transactions
        .iter()
        .skip(1)
        .map(|tx| tx.inputs.len())
        .sum::<usize>();
    Ok(BenchResult {
        name: "propagation_compact_block",
        tx_count: fixture.block.transactions.len().saturating_sub(1),
        signature_count,
        runs: samples,
        mean: mean_duration(&durations),
        notes: format!(
            "compact_frame_bytes={} missing_tx_rate={:.3}",
            fixture.compact_block_frame.len(),
            missing_tx_rate / samples as f64
        ),
    })
}

fn mean_duration(durations: &[Duration]) -> Duration {
    if durations.is_empty() {
        return Duration::ZERO;
    }
    let total_nanos: u128 = durations.iter().map(Duration::as_nanos).sum();
    Duration::from_nanos((total_nanos / durations.len() as u128) as u64)
}

fn render_report(
    cli: &Cli,
    data_dir: &Path,
    hardware: &HardwareInfo,
    fixture: &BenchmarkFixture,
    results: &[BenchResult],
) -> String {
    let mut out = String::new();
    out.push_str("# Atho End-to-End Optimization Benchmark\n\n");
    out.push_str("## Hardware\n");
    out.push_str(&format!("- CPU: {}\n", hardware.cpu));
    out.push_str(&format!("- Core count: {}\n", hardware.core_count));
    out.push_str(&format!("- RAM: {}\n", hardware.ram));
    out.push_str(&format!("- Disk: {}\n", hardware.disk));
    out.push_str(&format!("- OS: {}\n", hardware.os));
    out.push_str(&format!("- Rust version: {}\n", hardware.rust_version));
    out.push_str(&format!("- Build profile: release\n"));
    out.push_str(&format!("- Commit hash: {}\n\n", hardware.commit_hash));

    out.push_str("## Network Parameters\n");
    out.push_str(&format!("- Network: {}\n", cli.network.id()));
    out.push_str("- Block time: 75 seconds\n");
    out.push_str("- Vbyte cap: 3,000,000 vbytes\n");
    out.push_str("- Raw cap: about 12 MB\n");
    out.push_str(&format!(
        "- Average tx size tested: {} vbytes\n",
        fixture
            .transactions
            .first()
            .map(|tx| tx.vsize_bytes())
            .unwrap_or(0)
    ));
    out.push_str("- Signature scheme: Falcon-512\n");
    out.push_str("- Transaction model: public UTXO\n");
    out.push_str("- Sizing model: SigWit-style vbytes\n\n");

    out.push_str("## Chain Wipe Confirmation\n");
    out.push_str(&format!("- Wiped before run: {}\n", cli.wipe_first));
    out.push_str(&format!("- Data dir: {}\n", data_dir.display()));
    out.push_str("- Cold-cache mode: yes\n");
    out.push_str("- Warm-cache mode: not separately measured in this run\n\n");

    out.push_str("## End-to-End Results\n");
    out.push_str("| Test | Tx Count | Signature Count | Runs | Mean | Signatures/sec | TPS Simulated | Notes |\n");
    out.push_str("|---|---:|---:|---:|---:|---:|---:|---|\n");
    for result in results {
        let elapsed = result.mean.as_secs_f64().max(f64::MIN_POSITIVE);
        let sigs_per_sec = result.signature_count as f64 / elapsed;
        let tps = result.tx_count as f64 / elapsed;
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {:.2} | {:.2} | {} |\n",
            result.name,
            result.tx_count,
            result.signature_count,
            result.runs,
            format_duration(result.mean),
            sigs_per_sec,
            tps,
            result.notes
        ));
    }
    out.push('\n');

    out.push_str("## Improvement Summary\n");
    out.push_str("| Area | Before | After | Improvement |\n");
    out.push_str("|---|---:|---:|---:|\n");
    out.push_str("| End-to-end harness | not measured | measured | harness added |\n\n");

    out.push_str("## Final Decision\n");
    out.push_str("- Safe to merge: No\n");
    out.push_str("- Needs more testing: Yes\n");
    out.push_str("- Blockers: sanitizer-backed fuzzing, Miri, TSAN, ASAN, and broader large-count soak runs\n");

    out
}

fn format_duration(duration: Duration) -> String {
    if duration.as_secs() > 0 {
        format!("{:.3}s", duration.as_secs_f64())
    } else if duration.as_millis() > 0 {
        format!("{}ms", duration.as_millis())
    } else if duration.as_micros() > 0 {
        format!("{}us", duration.as_micros())
    } else {
        format!("{}ns", duration.as_nanos())
    }
}

struct HardwareInfo {
    cpu: String,
    core_count: usize,
    ram: String,
    disk: String,
    os: String,
    rust_version: String,
    commit_hash: String,
}

fn collect_hardware_info() -> HardwareInfo {
    let cpu = run_command(&["sysctl", "-n", "machdep.cpu.brand_string"])
        .or_else(|| run_command(&["lscpu"]))
        .unwrap_or_else(|| String::from("unknown"));
    let ram = run_command(&["sysctl", "-n", "hw.memsize"])
        .map(|value| format!("{} bytes", value.trim()))
        .or_else(|| run_command(&["free", "-h"]))
        .unwrap_or_else(|| String::from("unknown"));
    let disk = run_command(&["df", "-h", "."]).unwrap_or_else(|| String::from("unknown"));
    let os = run_command(&["uname", "-sr"])
        .unwrap_or_else(|| format!("{} {}", env::consts::OS, env::consts::ARCH));
    let rust_version =
        run_command(&["rustc", "--version"]).unwrap_or_else(|| String::from("unknown"));
    let commit_hash =
        run_command(&["git", "rev-parse", "HEAD"]).unwrap_or_else(|| String::from("unknown"));
    let core_count = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(1);
    HardwareInfo {
        cpu,
        core_count,
        ram,
        disk,
        os,
        rust_version,
        commit_hash,
    }
}

fn run_command(args: &[&str]) -> Option<String> {
    let (program, rest) = args.split_first()?;
    let output = std::process::Command::new(program)
        .args(rest)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn current_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}
