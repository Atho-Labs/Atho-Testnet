use atho_core::network::Network;
use atho_node::miner::Miner;
use atho_rpc::request::RpcRequest;
use atho_rpc::response::RpcResponse;
use atho_rpc::transport::RpcClient;

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
    let network = network_from_args(&args)?.unwrap_or_else(default_network);
    let cores = cores_from_args(&args)?.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(1)
    });
    let miner = Miner::new(cores as u32);
    let rpc_address = rpc_addr_from_args(&args)?.unwrap_or_else(|| default_rpc_address(network));
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

fn network_from_args(args: &[String]) -> Result<Option<Network>, String> {
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "mainnet" | "testnet" | "regnet" | "regtest" => {
                return Ok(parse_network(&args[i]));
            }
            "--network" | "-n" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| "missing network value".to_string())?;
                return parse_network(value)
                    .ok_or_else(|| format!("invalid network {value}"))
                    .map(Some);
            }
            "--cores" | "-c" => {
                i += 2;
            }
            value if value.starts_with('-') => {
                return Err(format!("unrecognized argument {value}"));
            }
            _ => {
                i += 1;
            }
        }
    }
    Ok(None)
}

fn parse_network(value: &str) -> Option<Network> {
    match value {
        "mainnet" => Some(Network::Mainnet),
        "testnet" => Some(Network::Testnet),
        "regnet" | "regtest" => Some(Network::Regnet),
        _ => None,
    }
}

fn cores_from_args(args: &[String]) -> Result<Option<usize>, String> {
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
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
                return Ok(Some(cores));
            }
            _ => i += 1,
        }
    }
    Ok(None)
}

fn rpc_addr_from_args(args: &[String]) -> Result<Option<String>, String> {
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--rpc-addr" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| "missing rpc address value".to_string())?;
                return Ok(Some(value.clone()));
            }
            _ => i += 1,
        }
    }
    Ok(None)
}

fn default_network() -> Network {
    match std::env::var("ATHO_NETWORK")
        .unwrap_or_else(|_| String::from("mainnet"))
        .as_str()
    {
        "testnet" => Network::Testnet,
        "regnet" | "regtest" => Network::Regnet,
        _ => Network::Mainnet,
    }
}

fn print_usage() {
    eprintln!("usage:");
    eprintln!("  atho-mine [mainnet|testnet|regnet]");
    eprintln!("  atho-mine --network <mainnet|testnet|regnet> [--cores N] [--rpc-addr HOST:PORT]");
    eprintln!("  ATHO_NETWORK=<network> atho-mine [--cores N]");
}

fn default_rpc_address(network: Network) -> String {
    atho_node::runtime::rpc_bind_address(network)
}
