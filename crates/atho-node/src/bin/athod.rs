fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("dev") => match args.next().as_deref() {
            Some("wipe") => {
                let _ = atho_node::dev::append_log("athod", "dev wipe requested");
                if let Err(err) = atho_node::dev::wipe_chain_and_keys() {
                    let _ = atho_node::dev::append_log("athod", &format!("dev wipe failed: {err}"));
                    eprintln!("failed to wipe dev state: {err}");
                    std::process::exit(1);
                }
                let _ = atho_node::dev::append_log("athod", "dev wipe completed");
                println!("dev state wiped");
            }
            Some("watch") => {
                let _ = atho_node::dev::append_log("athod", "dev watch started");
                if let Err(err) = atho_node::dev::watch_logs() {
                    eprintln!("failed to watch dev logs: {err}");
                    std::process::exit(1);
                }
            }
            Some("export") => match args.next().as_deref() {
                Some("chain") => match atho_node::dev::publish_audit_exports() {
                    Ok((chain, _, _, _)) => println!("{}", chain.display()),
                    Err(err) => {
                        eprintln!("failed to export chain audit files: {err}");
                        std::process::exit(1);
                    }
                },
                Some("tx") => match atho_node::dev::publish_audit_exports() {
                    Ok((_, txs, inputs, outputs)) => {
                        println!("{}", txs.display());
                        println!("{}", inputs.display());
                        println!("{}", outputs.display());
                    }
                    Err(err) => {
                        eprintln!("failed to export tx audit files: {err}");
                        std::process::exit(1);
                    }
                },
                _ => {
                    eprintln!("usage: athod dev export <chain|tx>");
                    std::process::exit(1);
                }
            },
            Some("mine") => {
                let network = parse_network(args.next().as_deref().unwrap_or("mainnet"));
                match network {
                    Some(network) => match atho_node::dev::mine_once(network) {
                        Ok(path) => println!("{}", path.display()),
                        Err(err) => {
                            eprintln!("failed to mine dev block: {err}");
                            std::process::exit(1);
                        }
                    },
                    None => {
                        eprintln!("usage: athod dev mine <mainnet|testnet|regnet>");
                        std::process::exit(1);
                    }
                }
            }
            _ => {
                eprintln!("usage: athod dev <wipe|watch|export|mine>");
                std::process::exit(1);
            }
        },
        _ => {
            if let Err(err) = atho_node::runtime::run() {
                eprintln!("{err}");
                std::process::exit(1);
            }
        }
    }
}

fn parse_network(value: &str) -> Option<atho_core::network::Network> {
    match value {
        "mainnet" => Some(atho_core::network::Network::Mainnet),
        "testnet" => Some(atho_core::network::Network::Testnet),
        "regnet" | "regtest" => Some(atho_core::network::Network::Regnet),
        _ => None,
    }
}
