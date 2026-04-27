use atho_core::genesis;
use atho_core::network::Network;
use atho_node::config::NodeConfig;

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => atho_node::runtime::run().map_err(|err| err.to_string()),
        Some("run") => run_node(&args[1..]),
        Some("verify") => verify_node(&args[1..]),
        Some("dev") => run_dev(&args[1..]),
        Some("--help") | Some("-h") => {
            print_usage();
            Ok(())
        }
        Some(value) if parse_network(value).is_some() => {
            let network = parse_network(value).expect("validated above");
            atho_node::runtime::run_with_config(NodeConfig::new(network))
                .map_err(|err| err.to_string())
        }
        Some(value) => Err(format!("unrecognized command {value}")),
    }
}

fn run_node(args: &[String]) -> Result<(), String> {
    let network = network_from_args(args)?;
    match network {
        Some(network) => atho_node::runtime::run_with_config(NodeConfig::new(network))
            .map_err(|err| err.to_string()),
        None => atho_node::runtime::run().map_err(|err| err.to_string()),
    }
}

fn verify_node(args: &[String]) -> Result<(), String> {
    let network = match network_from_args(args)? {
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

    if node.chainstate.height != 0 {
        return Err(format!(
            "unexpected chain height {}",
            node.chainstate.height
        ));
    }
    if node.chainstate.tip_hash != genesis.block_hash {
        return Err("genesis tip hash mismatch".to_string());
    }
    if node.chainstate.utxo_count() != 1 {
        return Err(format!(
            "unexpected genesis utxo count {}",
            node.chainstate.utxo_count()
        ));
    }
    if node.chainstate.blocks().len() != 1 {
        return Err(format!(
            "unexpected genesis block count {}",
            node.chainstate.blocks().len()
        ));
    }

    println!("node verification ok");
    println!("network={}", network.id());
    println!("genesis_hash={}", hex::encode(genesis.block_hash));
    println!("genesis_height={}", node.chainstate.height);
    println!(
        "genesis_target={}",
        hex::encode(genesis.block.header.difficulty_target_or_bits)
    );
    Ok(())
}

fn run_dev(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("genesis") => {
            let network = network_from_args(&args[1..])?.unwrap_or(Network::Mainnet);
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
            let _ = atho_node::dev::append_log("athod", "dev wipe requested");
            atho_node::dev::wipe_chain_and_keys().map_err(|err| err.to_string())?;
            let _ = atho_node::dev::append_log("athod", "dev wipe completed");
            println!("dev state wiped");
            Ok(())
        }
        Some("reset") => {
            let network = network_from_args(&args[1..])?.unwrap_or(Network::Mainnet);
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
        Some("watch") => atho_node::dev::watch_logs().map_err(|err| err.to_string()),
        Some("export") => match args.get(1).map(String::as_str) {
            Some("chain") => {
                let (chain, _, _, _) =
                    atho_node::dev::publish_audit_exports().map_err(|err| err.to_string())?;
                println!("{}", chain.display());
                Ok(())
            }
            Some("tx") => {
                let (_, txs, inputs, outputs) =
                    atho_node::dev::publish_audit_exports().map_err(|err| err.to_string())?;
                println!("{}", txs.display());
                println!("{}", inputs.display());
                println!("{}", outputs.display());
                Ok(())
            }
            _ => Err("usage: athod dev export <chain|tx>".to_string()),
        },
        Some("mine") => {
            let network = network_from_args(&args[1..])?.unwrap_or(Network::Mainnet);
            let path = atho_node::dev::mine_once(network).map_err(|err| err.to_string())?;
            println!("{}", path.display());
            Ok(())
        }
        _ => Err("usage: athod dev <genesis|wipe|reset|watch|export|mine>".to_string()),
    }
}

fn network_from_args(args: &[String]) -> Result<Option<Network>, String> {
    match args.first().map(String::as_str) {
        Some(value) if parse_network(value).is_some() => Ok(parse_network(value)),
        Some("--network") | Some("-n") => {
            let value = args
                .get(1)
                .ok_or_else(|| "missing network value".to_string())?;
            parse_network(value)
                .ok_or_else(|| format!("invalid network {value}"))
                .map(Some)
        }
        Some(value) => Err(format!("unrecognized argument {value}")),
        None => Ok(None),
    }
}

fn parse_network(value: &str) -> Option<Network> {
    match value {
        "mainnet" => Some(Network::Mainnet),
        "testnet" => Some(Network::Testnet),
        "regnet" | "regtest" => Some(Network::Regnet),
        _ => None,
    }
}

fn print_usage() {
    eprintln!("usage:");
    eprintln!("  athod");
    eprintln!("  athod run [mainnet|testnet|regnet]");
    eprintln!("  athod verify [mainnet|testnet|regnet]");
    eprintln!("  athod dev <genesis|wipe|reset|watch|export|mine>");
}
