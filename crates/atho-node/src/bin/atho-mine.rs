use atho_core::network::Network;
use atho_node::miner::Miner;
use atho_rpc::request::RpcRequest;
use atho_rpc::response::RpcResponse;
use atho_rpc::transport::RpcClient;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MinerCli {
    network: Option<Network>,
    rpc_addr: Option<String>,
    data_dir: Option<String>,
    cores: Option<usize>,
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
    let network = cli.network.unwrap_or_else(default_network);
    let cores = cli.cores.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(1)
    });
    let miner = Miner::new(cores as u32);
    let rpc_address = cli
        .rpc_addr
        .clone()
        .unwrap_or_else(|| default_rpc_address(network));
    let client = RpcClient::new(rpc_address.clone());
    let _ = atho_node::dev::append_log(
        "miner",
        &format!(
            "cli mining request network={} rpc={} cores={cores}",
            network.id(),
            rpc_address
        ),
    );
    println!(
        "mining on {} rpc={} cores={cores}",
        network.id(),
        rpc_address
    );
    println!("requesting block template...");
    let template = match client.call(&RpcRequest::GetBlockTemplate) {
        Ok(RpcResponse::BlockTemplate(template)) => template,
        Ok(RpcResponse::Error(err)) => return Err(err.to_string()),
        Ok(other) => return Err(format!("unexpected rpc response: {other:?}")),
        Err(err) => return Err(err.to_string()),
    };
    println!("solving block at height {}", template.height);
    let block = miner.solve_block(template.block);
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

fn parse_cli(args: &[String]) -> Result<MinerCli, String> {
    let mut cli = MinerCli::default();
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "mainnet" | "testnet" | "regnet" | "regtest" => {
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
    eprintln!("  atho-mine [--network <mainnet|testnet|regnet>] [--rpc-addr HOST:PORT] [--cores N] [--data-dir PATH]");
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
        ];
        let parsed = parse_cli(&args).expect("parse");
        assert_eq!(parsed.network, Some(Network::Regnet));
        assert_eq!(parsed.rpc_addr.as_deref(), Some("127.0.0.1:9210"));
        assert_eq!(parsed.cores, Some(4));
        assert_eq!(parsed.data_dir.as_deref(), Some("/tmp/atho"));
    }
}
