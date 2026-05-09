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
    network_overrides_local: bool,
    all: bool,
    include_wallets: bool,
    dangerously_allow_mainnet: bool,
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
        Some("wipe") => run_wipe(&args[1..]),
        Some("dev") => run_dev(&args[1..]),
        Some("run") => run_node(&args[1..]),
        _ => run_node(&args),
    }
}

fn run_node(args: &[String]) -> Result<(), String> {
    let runtime = parse_runtime_cli(args)?;
    runtime.apply_env();
    let config = runtime_node_config(&runtime)?;
    config
        .network
        .operator_launch_allowed()
        .map_err(str::to_string)?;
    apply_network_override_if_requested(&runtime, config.network)?;
    start_managed_parent_monitor();
    atho_node::runtime::run_with_config(config).map_err(|err| err.to_string())
}

fn runtime_node_config(runtime: &RuntimeCli) -> Result<NodeConfig, String> {
    let network = match runtime.network {
        Some(network) => network,
        None => {
            atho_node::runtime::load_config_from_env()
                .map_err(|err| err.to_string())?
                .network
        }
    };
    Ok(NodeConfig::from_env(network))
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
    network.operator_launch_allowed().map_err(str::to_string)?;
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
    network.operator_launch_allowed().map_err(str::to_string)?;
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
    println!(
        "chain_synced={}",
        status.running && status.headers_synced && status.block_count >= status.sync_best_height
    );
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
            let network = runtime.network.unwrap_or_else(Network::operator_default);
            network.operator_launch_allowed().map_err(str::to_string)?;
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
        Some("wipe") => run_wipe(&args[1..]),
        Some("reset") => {
            let runtime = parse_runtime_cli(&args[1..])?;
            runtime.apply_env();
            let network = runtime.network.unwrap_or_else(Network::operator_default);
            network.operator_launch_allowed().map_err(str::to_string)?;
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
            let network = runtime.network.unwrap_or_else(Network::operator_default);
            network.operator_launch_allowed().map_err(str::to_string)?;
            let path = atho_node::dev::mine_once(network).map_err(|err| err.to_string())?;
            println!("{}", path.display());
            Ok(())
        }
        _ => Err("usage: athod dev <genesis|wipe|reset|watch|export|mine>".to_string()),
    }
}

fn run_wipe(args: &[String]) -> Result<(), String> {
    let runtime = parse_runtime_cli(args)?;
    runtime.apply_env();
    let network = runtime
        .network
        .ok_or_else(|| "wipe requires --network <mainnet|testnet|regnet>".to_string())?;
    if network == Network::Mainnet && !runtime.dangerously_allow_mainnet {
        return Err("refusing to wipe mainnet without --dangerously-allow-mainnet".to_string());
    }
    if !runtime.all {
        return Err("wipe requires --all".to_string());
    }
    let data_dir = runtime
        .data_dir
        .ok_or_else(|| "wipe requires --data-dir PATH".to_string())?;
    let rpc_address = runtime
        .rpc_addr
        .unwrap_or_else(|| atho_node::runtime::default_rpc_bind_address(network));
    if local_rpc_endpoint_matches_network(network, &rpc_address)? {
        return Err(format!(
            "refusing to wipe {} while a live local node is still serving {} on rpc={rpc_address}; stop the node first",
            data_dir,
            network.id()
        ));
    }
    let root = std::path::PathBuf::from(data_dir);
    if runtime.include_wallets {
        atho_node::dev::wipe_root_including_wallets(&root).map_err(|err| err.to_string())?;
    } else {
        atho_node::dev::wipe_root(&root).map_err(|err| err.to_string())?;
    }
    println!("wiped {}", root.display());
    Ok(())
}

fn local_rpc_endpoint_matches_network(network: Network, rpc_address: &str) -> Result<bool, String> {
    let client = RpcClient::new(rpc_address.to_string());
    match client.call(&RpcRequest::GetNodeStatus) {
        Ok(RpcResponse::NodeStatus(status)) => {
            if status.network == network {
                return Ok(true);
            }
            return Err(format!(
                "rpc address {rpc_address} is serving {}; expected {}",
                status.network.id(),
                network.id()
            ));
        }
        Ok(RpcResponse::Error(err)) => {
            return Err(format!(
                "rpc address {rpc_address} refused wipe preflight for {}: {err}",
                network.id()
            ));
        }
        Ok(_) => {}
        Err(_) => {}
    }

    match client.call(&RpcRequest::GetNetwork) {
        Ok(RpcResponse::Network(label)) => {
            if label == network.id() {
                Ok(true)
            } else {
                Err(format!(
                    "rpc address {rpc_address} is serving {label}; expected {}",
                    network.id()
                ))
            }
        }
        Ok(RpcResponse::Error(err)) => Err(format!(
            "rpc address {rpc_address} refused wipe preflight for {}: {err}",
            network.id()
        )),
        Ok(_) => Ok(false),
        Err(_) => Ok(false),
    }
}

fn apply_network_override_if_requested(
    runtime: &RuntimeCli,
    network: Network,
) -> Result<(), String> {
    if !network_override_requested(runtime) {
        return Ok(());
    }

    let root = atho_storage::path::sandbox_root();
    atho_node::dev::wipe_root(&root).map_err(|err| err.to_string())?;
    let _ = atho_node::dev::append_log(
        "athod",
        &format!(
            "network override resync wiped local chain databases network={} root={}",
            network.id(),
            root.display()
        ),
    );
    Ok(())
}

fn network_override_requested(runtime: &RuntimeCli) -> bool {
    runtime.network_overrides_local
        || std::env::var("ATHO_NETWORK_OVERRIDES_LOCAL")
            .ok()
            .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
}

fn start_managed_parent_monitor() {
    let Some(parent_pid) = std::env::var("ATHO_MANAGED_PARENT_PID")
        .ok()
        .and_then(|raw| raw.parse::<u32>().ok())
        .filter(|pid| *pid > 0)
    else {
        return;
    };

    std::thread::Builder::new()
        .name(String::from("atho-managed-parent-monitor"))
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            if managed_parent_is_alive(parent_pid) {
                continue;
            }
            let _ = atho_node::dev::append_log(
                "athod",
                &format!(
                    "managed parent pid={} exited; shutting down athod",
                    parent_pid
                ),
            );
            std::process::exit(0);
        })
        .ok();
}

fn managed_parent_is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    #[cfg(windows)]
    {
        let output = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output();
        match output {
            Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
                .lines()
                .any(|line| line.split_whitespace().any(|part| part == pid.to_string())),
            _ => false,
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        true
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
            "--network-overrides-local" | "--force-network-resync" => {
                runtime.network_overrides_local = true;
                i += 1;
            }
            "--all" => {
                runtime.all = true;
                i += 1;
            }
            "--include-wallets" => {
                runtime.include_wallets = true;
                i += 1;
            }
            "--dangerously-allow-mainnet" => {
                runtime.dangerously_allow_mainnet = true;
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
    eprintln!("  athod [--network <mainnet|testnet|regnet|prunetest>] [--data-dir PATH] [--rpc-addr HOST:PORT] [--p2p-addr HOST:PORT] [--peer HOST:PORT] [--public-rpc] [--network-overrides-local]");
    eprintln!("  athod wipe --network <mainnet|testnet|regnet|prunetest> --data-dir PATH --all [--include-wallets] [--dangerously-allow-mainnet]");
    eprintln!("  athod status [--network <mainnet|testnet|regnet|prunetest>] [--rpc-addr HOST:PORT] [--data-dir PATH]");
    eprintln!("  athod verify [--network <mainnet|testnet|regnet|prunetest>] [--data-dir PATH]");
    eprintln!("  athod dev <genesis|wipe|reset|watch|export|mine> [options]");
    eprintln!();
    eprintln!("legacy compatibility:");
    eprintln!("  athod run [options]");
}

#[cfg(test)]
mod tests {
    use super::*;
    use atho_rpc::transport::{read_message, write_message};
    use std::ffi::OsString;
    use std::io::BufReader;
    use std::net::TcpListener;
    use std::thread;

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
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
    fn wipe_flags_parse_explicit_sandbox_guard() {
        let args = [
            String::from("wipe"),
            String::from("--network"),
            String::from("regnet"),
            String::from("--data-dir"),
            String::from("/tmp/atho-dev"),
            String::from("--all"),
            String::from("--include-wallets"),
            String::from("--dangerously-allow-mainnet"),
        ];
        let parsed = parse_runtime_cli(&args[1..]).expect("parse");
        assert_eq!(parsed.network, Some(Network::Regnet));
        assert_eq!(parsed.data_dir.as_deref(), Some("/tmp/atho-dev"));
        assert!(parsed.all);
        assert!(parsed.include_wallets);
        assert!(parsed.dangerously_allow_mainnet);
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

    #[test]
    fn runtime_cli_accepts_prunetest_network() {
        let args = vec![String::from("--network"), String::from("prune-test")];
        let parsed = parse_runtime_cli(&args).expect("parse");
        assert_eq!(parsed.network, Some(Network::Prunetest));
    }

    #[test]
    fn runtime_cli_accepts_network_override_resync() {
        let args = vec![
            String::from("--network"),
            String::from("testnet"),
            String::from("--network-overrides-local"),
        ];
        let parsed = parse_runtime_cli(&args).expect("parse");
        assert_eq!(parsed.network, Some(Network::Testnet));
        assert!(parsed.network_overrides_local);
    }

    #[test]
    fn network_override_resync_wipes_chain_dbs_but_keeps_wallets() {
        let root =
            std::env::temp_dir().join(format!("atho-network-override-{}", std::process::id()));
        let _data_dir = EnvVarGuard::set(
            atho_storage::path::ATHO_DATA_DIR_ENV,
            root.to_str().expect("utf8 temp path"),
        );
        let _env_override = EnvVarGuard::set("ATHO_NETWORK_OVERRIDES_LOCAL", "0");
        std::fs::create_dir_all(root.join("db").join("testnet")).expect("db dir");
        std::fs::write(root.join("db").join("testnet").join("data.mdb"), "db").expect("db");
        std::fs::create_dir_all(root.join("testnet")).expect("direct network dir");
        std::fs::write(root.join("testnet").join("data.mdb"), "db").expect("direct db");
        std::fs::create_dir_all(root.join("wallet")).expect("wallet dir");
        std::fs::write(root.join("wallet").join("wallet.dat"), "wallet").expect("wallet");

        let runtime = RuntimeCli {
            network_overrides_local: true,
            ..RuntimeCli::default()
        };
        apply_network_override_if_requested(&runtime, Network::Testnet).expect("wipe override");

        assert!(!root.join("db").join("testnet").join("data.mdb").exists());
        assert!(!root.join("testnet").exists());
        assert!(root.join("wallet").join("wallet.dat").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_node_config_honors_api_env_overrides() {
        let _port = EnvVarGuard::set("ATHO_API_PORT", "18080");
        let _bind = EnvVarGuard::set("ATHO_API_BIND", "127.0.0.2");
        let runtime = RuntimeCli {
            network: Some(Network::Regnet),
            ..RuntimeCli::default()
        };
        let config = runtime_node_config(&runtime).expect("config");
        assert_eq!(config.network, Network::Regnet);
        assert_eq!(config.api.port, 18080);
        assert_eq!(config.api.bind, "127.0.0.2");
    }

    #[test]
    fn managed_parent_probe_recognizes_current_process() {
        assert!(managed_parent_is_alive(std::process::id()));
    }

    #[test]
    fn wipe_preflight_detects_live_same_network_rpc_endpoint() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let rpc_address = listener.local_addr().expect("local addr").to_string();
        let server = thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept");
                let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
                let _request = read_message::<_, RpcRequest>(&mut reader).expect("read request");
                write_message(
                    &mut stream,
                    &RpcResponse::Network(Network::Testnet.id().to_string()),
                )
                .expect("write response");
            }
        });

        assert!(local_rpc_endpoint_matches_network(Network::Testnet, &rpc_address).expect("probe"));
        server.join().expect("join server");
    }
}
