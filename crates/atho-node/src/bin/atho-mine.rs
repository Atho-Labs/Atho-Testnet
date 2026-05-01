use atho_core::network::Network;
use atho_node::mining_backend::{MiningAcceleratorInfo, MiningBackendKind, MiningController};
use atho_rpc::request::RpcRequest;
use atho_rpc::response::RpcResponse;
use atho_rpc::transport::RpcClient;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MinerCli {
    network: Option<Network>,
    rpc_addr: Option<String>,
    data_dir: Option<String>,
    cores: Option<usize>,
    backend: Option<MiningBackendKind>,
    probe_gpu: bool,
}

impl MinerCli {
    fn apply_env(&self) {
        if let Some(network) = self.network {
            std::env::set_var("ATHO_NETWORK", network.cli_arg());
        }
        if let Some(data_dir) = &self.data_dir {
            std::env::set_var(atho_storage::path::ATHO_DATA_DIR_ENV, data_dir);
        }
        if let Some(rpc_addr) = &self.rpc_addr {
            std::env::set_var("ATHO_RPC_ADDR", rpc_addr);
        }
    }
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_usage();
        return Ok(());
    }
    let cli = parse_cli(&args)?;
    cli.apply_env();
    let requested_backend = cli
        .backend
        .or_else(MiningBackendKind::from_env)
        .unwrap_or_default();
    if cli.probe_gpu {
        let info = MiningController::new(requested_backend, 1).gpu_probe_info();
        print_gpu_probe(&info);
        return Ok(());
    }
    let network = cli.network.unwrap_or_else(default_network);
    let cores = cli.cores.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(1)
    });
    let backend = requested_backend;
    let controller = MiningController::new(backend, cores as u32);
    let gpu_info = controller.gpu_probe_info();
    let rpc_address = cli
        .rpc_addr
        .clone()
        .unwrap_or_else(|| default_rpc_address(network));
    let client = RpcClient::new(rpc_address.clone());
    let _ = atho_node::dev::append_log(
        "miner",
        &format!(
            "cli mining request network={} rpc={} cores={cores} backend={}",
            network.id(),
            rpc_address,
            controller.backend().label()
        ),
    );
    println!(
        "mining on {} rpc={} cores={cores} backend={}",
        network.id(),
        rpc_address,
        controller.backend().label()
    );
    if !matches!(controller.backend(), MiningBackendKind::Cpu) {
        println!("{}", gpu_info.summary());
    }
    println!("requesting block template...");
    let template = match client.call(&RpcRequest::GetBlockTemplate) {
        Ok(RpcResponse::BlockTemplate(template)) => template,
        Ok(RpcResponse::Error(err)) => return Err(err.to_string()),
        Ok(other) => return Err(format!("unexpected rpc response: {other:?}")),
        Err(err) => return Err(err.to_string()),
    };
    println!(
        "solving block at height {} with requested backend {}",
        template.height,
        controller.backend().label()
    );
    let report = controller
        .mine_block_reported(template, Arc::new(AtomicBool::new(false)))
        .map_err(|err| err.to_string())?;
    let effective_backend = report.backend_used.label();
    let fallback_reason = report.fallback_reason.clone();
    let accelerator = report.accelerator.clone();
    if let Some(reason) = fallback_reason.as_deref() {
        println!("backend fallback: {reason}");
    }
    let block = report.block;
    match client.call(&RpcRequest::SubmitBlock(block.clone())) {
        Ok(RpcResponse::BlockSubmitted { accepted: true, .. }) => {}
        Ok(RpcResponse::BlockSubmitted {
            accepted: false, ..
        }) => return Err("block rejected".to_string()),
        Ok(RpcResponse::Error(err)) => return Err(err.to_string()),
        Ok(other) => return Err(format!("unexpected rpc response: {other:?}")),
        Err(err) => return Err(err.to_string()),
    }

    println!("network={}", network.id());
    println!("cores={cores}");
    println!("requested_backend={}", controller.backend().label());
    println!("effective_backend={effective_backend}");
    if let Some(accelerator) = &accelerator {
        print_gpu_probe(accelerator);
    }
    println!("rpc_address={rpc_address}");
    println!("height={}", block.header.height);
    println!("hash={}", hex::encode(block.header.block_hash()));
    println!(
        "previous_hash={}",
        hex::encode(block.header.previous_block_hash)
    );
    println!("merkle_root={}", hex::encode(block.header.merkle_root));
    println!("witness_root={}", hex::encode(block.header.witness_root));
    println!(
        "target={}",
        hex::encode(block.header.difficulty_target_or_bits)
    );
    println!("nonce={}", block.header.nonce);
    println!("tx_count={}", block.transactions.len());
    println!("block_bytes_hex={}", hex::encode(block.full_bytes()));
    let _ = atho_node::dev::append_log(
        "miner",
        &format!(
            "cli mining complete hash={} height={}",
            hex::encode(block.header.block_hash()),
            block.header.height
        ),
    );
    Ok(())
}

fn print_gpu_probe(info: &MiningAcceleratorInfo) {
    println!("gpu_backend={}", info.backend);
    println!("gpu_usable={}", info.usable);
    println!("gpu_device_type={}", info.device_type.label());
    if let Some(name) = &info.device_name {
        println!("gpu_device_name={name}");
    }
    if let Some(vendor) = &info.vendor {
        println!("gpu_vendor={vendor}");
    }
    if let Some(driver) = &info.driver {
        println!("gpu_driver={driver}");
    }
    if let Some(compute_units) = info.compute_units {
        println!("gpu_compute_units={compute_units}");
    }
    if let Some(global_mem_mb) = info.global_mem_mb {
        println!("gpu_global_mem_mb={global_mem_mb}");
    }
    if let Some(local_mem_kb) = info.local_mem_kb {
        println!("gpu_local_mem_kb={local_mem_kb}");
    }
    if let Some(clock_mhz) = info.clock_mhz {
        println!("gpu_clock_mhz={clock_mhz}");
    }
    if let Some(kernel_path) = &info.kernel_path {
        println!("gpu_kernel_path={}", kernel_path.display());
    }
    println!("gpu_supports_fixed={}", info.supports_fixed);
    println!("gpu_supports_template={}", info.supports_template);
    if let Some(max_batch) = info.max_batch {
        println!("gpu_max_batch={max_batch}");
    }
    if let Some(template_max_bytes) = info.template_max_bytes {
        println!("gpu_template_max_bytes={template_max_bytes}");
    }
    if let Some(code) = &info.reason_code {
        println!("gpu_unavailable_code={code}");
    }
    if let Some(reason) = &info.reason_if_not {
        println!("gpu_unavailable_reason={reason}");
    }
}

fn parse_cli(args: &[String]) -> Result<MinerCli, String> {
    let mut cli = MinerCli::default();
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "mainnet" | "testnet" | "regnet" | "regtest" | "prunetest" | "prune-test"
            | "prune_test" => {
                cli.network = parse_network(&args[i]);
                i += 1;
            }
            "--network" | "-n" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| "missing network value".to_string())?;
                cli.network = parse_network(value);
                if cli.network.is_none() {
                    return Err(format!("invalid network {value}"));
                }
                i += 2;
            }
            "--cores" | "-c" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| "missing cores value".to_string())?;
                let cores = value
                    .parse::<usize>()
                    .map_err(|_| "invalid cores".to_string())?;
                if cores == 0 {
                    return Err("cores must be at least 1".to_string());
                }
                cli.cores = Some(cores);
                i += 2;
            }
            "--rpc-addr" => {
                cli.rpc_addr = Some(
                    args.get(i + 1)
                        .ok_or_else(|| "missing rpc address value".to_string())?
                        .clone(),
                );
                i += 2;
            }
            "--data-dir" => {
                cli.data_dir = Some(
                    args.get(i + 1)
                        .ok_or_else(|| "missing data directory value".to_string())?
                        .clone(),
                );
                i += 2;
            }
            "--backend" | "-b" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| "missing backend value".to_string())?;
                cli.backend = MiningBackendKind::parse(value);
                if cli.backend.is_none() {
                    return Err(format!("invalid backend {value}"));
                }
                i += 2;
            }
            "--probe-gpu" => {
                cli.probe_gpu = true;
                i += 1;
            }
            value => {
                return Err(format!("unrecognized argument {value}"));
            }
        }
    }
    Ok(cli)
}

fn parse_network(value: &str) -> Option<Network> {
    Network::parse(value)
}

fn default_network() -> Network {
    Network::parse(&std::env::var("ATHO_NETWORK").unwrap_or_else(|_| String::from("mainnet")))
        .unwrap_or(Network::Mainnet)
}

fn print_usage() {
    eprintln!("usage:");
    eprintln!(
        "  atho-mine [--network <mainnet|testnet|regnet|prunetest>] [--rpc-addr HOST:PORT] [--cores N] [--data-dir PATH] [--backend <cpu|gpu|auto>] [--probe-gpu]  (default backend: auto)"
    );
}

fn default_rpc_address(network: Network) -> String {
    atho_node::runtime::rpc_bind_address(network)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn miner_cli_parses_runtime_flags() {
        let args = vec![
            String::from("--network"),
            String::from("regnet"),
            String::from("--rpc-addr"),
            String::from("127.0.0.1:9210"),
            String::from("--cores"),
            String::from("4"),
            String::from("--data-dir"),
            String::from("/tmp/atho"),
            String::from("--backend"),
            String::from("gpu"),
        ];
        let parsed = parse_cli(&args).expect("parse");
        assert_eq!(parsed.network, Some(Network::Regnet));
        assert_eq!(parsed.rpc_addr.as_deref(), Some("127.0.0.1:9210"));
        assert_eq!(parsed.cores, Some(4));
        assert_eq!(parsed.data_dir.as_deref(), Some("/tmp/atho"));
        assert_eq!(parsed.backend, Some(MiningBackendKind::Gpu));
    }

    #[test]
    fn miner_cli_defaults_to_auto_when_backend_not_set() {
        let cli = parse_cli(&[String::from("--network"), String::from("regnet")]).unwrap();
        assert_eq!(cli.backend, None);
    }

    #[test]
    fn miner_cli_parses_probe_gpu_flag() {
        let cli = parse_cli(&[String::from("--probe-gpu")]).unwrap();
        assert!(cli.probe_gpu);
    }

    #[test]
    fn miner_cli_accepts_prunetest_shorthand() {
        let cli = parse_cli(&[String::from("prune-test")]).unwrap();
        assert_eq!(cli.network, Some(Network::Prunetest));
    }
}
