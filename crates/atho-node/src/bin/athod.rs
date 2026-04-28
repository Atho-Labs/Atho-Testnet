use atho_core::genesis;
use atho_core::network::Network;
use atho_node::config::NodeConfig;
use atho_rpc::request::RpcRequest;
use atho_rpc::response::RpcResponse;
use atho_rpc::transport::RpcClient;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RuntimeCli {
    network: Option<Network>,
    rpc_addr: Option<String>,
    p2p_addr: Option<String>,
    data_dir: Option<String>,
    peers: Vec<String>,
    public_rpc: bool,
}

impl RuntimeCli {
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
        if let Some(p2p_addr) = &self.p2p_addr {
            std::env::set_var("ATHO_P2P_ADDR", p2p_addr);
        }
        if !self.peers.is_empty() {
            std::env::set_var("ATHO_P2P_PEERS", self.peers.join(","));
        }
        if self.public_rpc {
            std::env::set_var("ATHO_RPC_ALLOW_PUBLIC", "1");
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct StatusCli {
    network: Option<Network>,
    rpc_addr: Option<String>,
    data_dir: Option<String>,
}

impl StatusCli {
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
    match args.first().map(String::as_str) {
        Some("--help") | Some("-h") => {
            print_usage();
            Ok(())
        }
        Some("status") => show_status(&args[1..]),
        Some("verify") => verify_node(&args[1..]),
        Some("dev") => run_dev(&args[1..]),
        Some("run") => run_node(&args[1..]),
        _ => run_node(&args),
    }
}

fn run_node(args: &[String]) -> Result<(), String> {
    let runtime = parse_runtime_cli(args)?;
    runtime.apply_env();
    match runtime.network {
        Some(network) => atho_node::runtime::run_with_config(NodeConfig::new(network))
            .map_err(|err| err.to_string()),
        None => atho_node::runtime::run().map_err(|err| err.to_string()),
    }
}

fn verify_node(args: &[String]) -> Result<(), String> {
    let runtime = parse_runtime_cli(args)?;
    runtime.apply_env();
    let network = match runtime.network {
        Some(network) => network,
        None => {
            atho_node::runtime::load_config_from_env()
                .map_err(|err| err.to_string())?
                .network
        }
    };
    let config = NodeConfig::new(network);
    let node = atho_node::node::Node::new(config);
    let genesis = genesis::genesis_state(network);

    atho_node::validation::validate_block(&genesis.block, 0, network)
        .map_err(|err| err.to_string())?;

    if node.height() != 0 {
        return Err(format!("unexpected chain height {}", node.height()));
    }
    if node.tip_hash() != genesis.block_hash {
        return Err("genesis tip hash mismatch".to_string());
    }
    if node.utxo_count() != 1 {
        return Err(format!(
            "unexpected genesis utxo count {}",
            node.utxo_count()
        ));
    }
    if node.blocks_len() != 1 {
        return Err(format!(
            "unexpected genesis block count {}",
            node.blocks_len()
        ));
    }

    println!("node verification ok");
    println!("network={}", network.id());
    println!("genesis_hash={}", hex::encode(genesis.block_hash));
    println!("genesis_height={}", node.height());
    println!(
        "genesis_target={}",
        hex::encode(genesis.block.header.difficulty_target_or_bits)
    );
    Ok(())
}

fn show_status(args: &[String]) -> Result<(), String> {
    let status_cli = parse_status_cli(args)?;
    status_cli.apply_env();
    let network = match status_cli.network {
        Some(network) => network,
        None => {
            atho_node::runtime::load_config_from_env()
                .map_err(|err| err.to_string())?
                .network
        }
    };
    let rpc_address = status_cli
        .rpc_addr
        .unwrap_or_else(|| atho_node::runtime::rpc_bind_address(network));
    let client = RpcClient::new(rpc_address.clone());
    let status = match client.call(&RpcRequest::GetNodeStatus) {
        Ok(RpcResponse::NodeStatus(status)) => status,
        Ok(RpcResponse::Error(err)) => return Err(err.to_string()),
        Ok(other) => return Err(format!("unexpected rpc response: {other:?}")),
        Err(err) => {
            let raw = err.to_string();
            let hint = if raw.contains("Connection refused") || raw.contains("connection refused") {
                format!(
                    "{raw}. start the node first with `cargo run -p atho-node --bin athod -- --network {}` or open the client with `cargo run -p atho-qt --bin atho-qt -- --network {} --local-node`",
                    network.cli_arg(),
                    network.cli_arg()
                )
            } else {
                raw
            };
            return Err(hint);
        }
    };

    println!("network={}", status.network.id());
    println!("rpc_address={rpc_address}");
    println!("running={}", status.running);
    println!("headers_synced={}", status.headers_synced);
    println!("block_count={}", status.block_count);
    println!("mempool_count={}", status.mempool_count);
    println!("mempool_total_fee_atoms={}", status.mempool_total_fee_atoms);
    println!("sync_best_height={}", status.sync_best_height);
    println!("peer_count={}", status.network_diagnostics.peer_count);
    println!(
        "peer_count_inbound={}",
        status.network_diagnostics.inbound_peer_count
    );
    println!(
        "peer_count_outbound={}",
        status.network_diagnostics.outbound_peer_count
    );
    println!("bytes_sent={}", status.network_diagnostics.bytes_sent);
    println!(
        "bytes_received={}",
        status.network_diagnostics.bytes_received
    );
    if !status.network_diagnostics.peers.is_empty() {
        println!("peers:");
        for peer in &status.network_diagnostics.peers {
            println!(
                "- {} dir={:?} ready={} height={:?} proto={:?} sent={} recv={} quality={:?}",
                peer.remote_addr,
                peer.direction,
                peer.handshake_ready,
                peer.best_height,
                peer.protocol_version,
                peer.bytes_sent,
                peer.bytes_received,
                peer.quality_score
            );
        }
    }

    if let Ok(lines) = atho_node::dev::recent_activity_lines(8) {
        if !lines.is_empty() {
            println!("activity:");
            for line in lines {
                println!("[{}] {} {}", line.timestamp, line.component, line.line);
            }
        }
    }

    Ok(())
}

fn run_dev(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("genesis") => {
            let runtime = parse_runtime_cli(&args[1..])?;
            runtime.apply_env();
            let network = runtime.network.unwrap_or(Network::Mainnet);
            let profile = genesis::regenerate_genesis_profile(network);
            println!("network={}", profile.network.id());
            println!("reward_address={}", profile.reward_address);
            println!("reward_script={}", hex::encode(profile.reward_script));
            println!("timestamp={}", profile.timestamp);
            println!("target={}", hex::encode(profile.target));
            println!("coinbase_txid={}", hex::encode(profile.coinbase_txid));
            println!("merkle_root={}", hex::encode(profile.merkle_root));
            println!("witness_root={}", hex::encode(profile.witness_root));
            println!("nonce={}", profile.nonce);
            println!("block_hash={}", hex::encode(profile.block_hash));
            Ok(())
        }
        Some("wipe") => {
            let runtime = parse_runtime_cli(&args[1..])?;
            runtime.apply_env();
            let _ = atho_node::dev::append_log("athod", "dev wipe requested");
            atho_node::dev::wipe_chain_and_keys().map_err(|err| err.to_string())?;
            let _ = atho_node::dev::append_log("athod", "dev wipe completed");
            println!("dev state wiped");
            Ok(())
        }
        Some("reset") => {
            let runtime = parse_runtime_cli(&args[1..])?;
            runtime.apply_env();
            let network = runtime.network.unwrap_or(Network::Mainnet);
            let _ = atho_node::dev::append_log(
                "athod",
                &format!("dev reset requested network={}", network.id()),
            );
            atho_node::dev::wipe_chain_and_keys().map_err(|err| err.to_string())?;
            let _ = atho_node::dev::append_log(
                "athod",
                &format!("dev reset completed network={}", network.id()),
            );
            atho_node::runtime::run_with_config(NodeConfig::new(network))
                .map_err(|err| err.to_string())
        }
        Some("watch") => {
            let runtime = parse_runtime_cli(&args[1..])?;
            runtime.apply_env();
            atho_node::dev::watch_logs().map_err(|err| err.to_string())
        }
        Some("export") => {
            let mode = args
                .get(1)
                .map(String::as_str)
                .ok_or_else(|| "usage: athod dev export <chain|tx>".to_string())?;
            let runtime = parse_runtime_cli(&args[2..])?;
            runtime.apply_env();
            match mode {
                "chain" => {
                    let (chain, _, _, _) =
                        atho_node::dev::publish_audit_exports().map_err(|err| err.to_string())?;
                    println!("{}", chain.display());
                    Ok(())
                }
                "tx" => {
                    let (_, txs, inputs, outputs) =
                        atho_node::dev::publish_audit_exports().map_err(|err| err.to_string())?;
                    println!("{}", txs.display());
                    println!("{}", inputs.display());
                    println!("{}", outputs.display());
                    Ok(())
                }
                _ => Err("usage: athod dev export <chain|tx>".to_string()),
            }
        }
        Some("mine") => {
            let runtime = parse_runtime_cli(&args[1..])?;
            runtime.apply_env();
            let network = runtime.network.unwrap_or(Network::Mainnet);
            let path = atho_node::dev::mine_once(network).map_err(|err| err.to_string())?;
            println!("{}", path.display());
            Ok(())
        }
        _ => Err("usage: athod dev <genesis|wipe|reset|watch|export|mine>".to_string()),
    }
}

fn parse_runtime_cli(args: &[String]) -> Result<RuntimeCli, String> {
    let mut runtime = RuntimeCli::default();
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            value if parse_network(value).is_some() => {
                runtime.network = parse_network(value);
                i += 1;
            }
            "--network" | "-n" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| "missing network value".to_string())?;
                runtime.network = parse_network(value);
                if runtime.network.is_none() {
                    return Err(format!("invalid network {value}"));
                }
                i += 2;
            }
            "--rpc-addr" => {
                runtime.rpc_addr = Some(
                    args.get(i + 1)
                        .ok_or_else(|| "missing rpc address value".to_string())?
                        .clone(),
                );
                i += 2;
            }
            "--p2p-addr" => {
                runtime.p2p_addr = Some(
                    args.get(i + 1)
                        .ok_or_else(|| "missing p2p address value".to_string())?
                        .clone(),
                );
                i += 2;
            }
            "--data-dir" => {
                runtime.data_dir = Some(
                    args.get(i + 1)
                        .ok_or_else(|| "missing data directory value".to_string())?
                        .clone(),
                );
                i += 2;
            }
            "--peer" => {
                runtime.peers.push(
                    args.get(i + 1)
                        .ok_or_else(|| "missing peer address value".to_string())?
                        .clone(),
                );
                i += 2;
            }
            "--public-rpc" => {
                runtime.public_rpc = true;
                i += 1;
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            value => return Err(format!("unrecognized argument {value}")),
        }
    }
    Ok(runtime)
}

fn parse_status_cli(args: &[String]) -> Result<StatusCli, String> {
    let mut status = StatusCli::default();
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            value if parse_network(value).is_some() => {
                status.network = parse_network(value);
                i += 1;
            }
            "--network" | "-n" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| "missing network value".to_string())?;
                status.network = parse_network(value);
                if status.network.is_none() {
                    return Err(format!("invalid network {value}"));
                }
                i += 2;
            }
            "--rpc-addr" => {
                status.rpc_addr = Some(
                    args.get(i + 1)
                        .ok_or_else(|| "missing rpc address value".to_string())?
                        .clone(),
                );
                i += 2;
            }
            "--data-dir" => {
                status.data_dir = Some(
                    args.get(i + 1)
                        .ok_or_else(|| "missing data directory value".to_string())?
                        .clone(),
                );
                i += 2;
            }
            value => return Err(format!("unrecognized argument {value}")),
        }
    }
    Ok(status)
}

fn parse_network(value: &str) -> Option<Network> {
    Network::parse(value)
}

fn print_usage() {
    eprintln!("usage:");
    eprintln!("  athod [--network <mainnet|testnet|regnet>] [--data-dir PATH] [--rpc-addr HOST:PORT] [--p2p-addr HOST:PORT] [--peer HOST:PORT] [--public-rpc]");
    eprintln!("  athod status [--network <mainnet|testnet|regnet>] [--rpc-addr HOST:PORT] [--data-dir PATH]");
    eprintln!("  athod verify [--network <mainnet|testnet|regnet>] [--data-dir PATH]");
    eprintln!("  athod dev <genesis|wipe|reset|watch|export|mine> [options]");
    eprintln!();
    eprintln!("legacy compatibility:");
    eprintln!("  athod run [options]");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_cli_parses_operator_flags() {
        let args = vec![
            String::from("--network"),
            String::from("regnet"),
            String::from("--rpc-addr"),
            String::from("127.0.0.1:9210"),
            String::from("--p2p-addr"),
            String::from("0.0.0.0:9200"),
            String::from("--data-dir"),
            String::from("/tmp/atho"),
            String::from("--peer"),
            String::from("127.0.0.1:9300"),
            String::from("--peer"),
            String::from("127.0.0.1:9301"),
            String::from("--public-rpc"),
        ];
        let parsed = parse_runtime_cli(&args).expect("parse");
        assert_eq!(parsed.network, Some(Network::Regnet));
        assert_eq!(parsed.rpc_addr.as_deref(), Some("127.0.0.1:9210"));
        assert_eq!(parsed.p2p_addr.as_deref(), Some("0.0.0.0:9200"));
        assert_eq!(parsed.data_dir.as_deref(), Some("/tmp/atho"));
        assert_eq!(parsed.peers.len(), 2);
        assert!(parsed.public_rpc);
    }

    #[test]
    fn status_cli_parses_network_and_rpc_address() {
        let args = vec![
            String::from("regnet"),
            String::from("--rpc-addr"),
            String::from("127.0.0.1:9210"),
            String::from("--data-dir"),
            String::from("/tmp/atho"),
        ];
        let parsed = parse_status_cli(&args).expect("parse");
        assert_eq!(parsed.network, Some(Network::Regnet));
        assert_eq!(parsed.rpc_addr.as_deref(), Some("127.0.0.1:9210"));
        assert_eq!(parsed.data_dir.as_deref(), Some("/tmp/atho"));
    }
}
