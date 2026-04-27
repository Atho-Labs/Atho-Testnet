use atho_core::network::Network;

fn main() {
    let _ = atho_node::dev::append_log("atho-qt", "starting atho-qt");
    if std::env::args().any(|arg| arg == "--help" || arg == "-h") {
        print_usage();
        return;
    }
    let (network, rpc_address, local_node) = parse_args();
    if local_node {
        std::env::set_var("ATHO_QT_LOCAL", "1");
        let _ = atho_node::dev::append_log(
            "atho-qt",
            &format!("starting embedded local node mode for {}", network.id()),
        );
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Atho")
            .with_inner_size([1000.0, 660.0])
            .with_min_inner_size([700.0, 440.0])
            .with_icon(atho_qt::resources::app_icon()),
        follow_system_theme: false,
        default_theme: eframe::Theme::Light,
        ..Default::default()
    };
    let result = eframe::run_native(
        "Atho",
        options,
        Box::new(move |cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Box::new(atho_qt::app::DesktopApp::new_with_rpc(
                network,
                rpc_address.clone(),
            ))
        }),
    );

    if let Err(err) = result {
        let _ = atho_node::dev::append_log("atho-qt", &format!("failed to launch: {err}"));
        eprintln!("failed to launch atho-qt: {err}");
        std::process::exit(1);
    }

    let _ = atho_node::dev::append_log("atho-qt", "stopped atho-qt");
}

fn parse_args() -> (Network, Option<String>, bool) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut network = default_network();
    let mut rpc_address = None;
    let mut local_node = false;

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "mainnet" => {
                network = Network::Mainnet;
                i += 1;
            }
            "testnet" => {
                network = Network::Testnet;
                i += 1;
            }
            "regnet" | "regtest" => {
                network = Network::Regnet;
                i += 1;
            }
            "--network" | "-n" => {
                if let Some(value) = args.get(i + 1) {
                    network = match value.as_str() {
                        "mainnet" => Network::Mainnet,
                        "testnet" => Network::Testnet,
                        "regnet" | "regtest" => Network::Regnet,
                        _ => network,
                    };
                }
                i += 2;
            }
            "--rpc-addr" => {
                if let Some(value) = args.get(i + 1) {
                    rpc_address = Some(value.clone());
                }
                i += 2;
            }
            "--local-node" | "--embedded-node" => {
                local_node = true;
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    (network, rpc_address, local_node)
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
    eprintln!("  atho-qt [mainnet|testnet|regnet]");
    eprintln!("  atho-qt --network <mainnet|testnet|regnet> [--rpc-addr HOST:PORT]");
    eprintln!("  atho-qt --local-node");
}
