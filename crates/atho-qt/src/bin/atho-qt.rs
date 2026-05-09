use std::path::Path;

use atho_core::network::Network;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct QtCli {
    network: Option<Network>,
    rpc_address: Option<String>,
    local_node: bool,
    data_dir: Option<String>,
    p2p_addr: Option<String>,
    renderer: Option<RendererChoice>,
    peers: Vec<String>,
}

impl QtCli {
    fn apply_env(&self) {
        if let Some(data_dir) = &self.data_dir {
            std::env::set_var(atho_storage::path::ATHO_DATA_DIR_ENV, data_dir);
        }
        if let Some(p2p_addr) = &self.p2p_addr {
            std::env::set_var("ATHO_P2P_ADDR", p2p_addr);
        }
        if !self.peers.is_empty() {
            std::env::set_var("ATHO_P2P_PEERS", self.peers.join(","));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RendererChoice {
    Glow,
    Wgpu,
}

impl RendererChoice {
    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "glow" => Some(Self::Glow),
            "wgpu" => Some(Self::Wgpu),
            _ => None,
        }
    }
}

impl From<RendererChoice> for eframe::Renderer {
    fn from(value: RendererChoice) -> Self {
        match value {
            RendererChoice::Glow => Self::Glow,
            RendererChoice::Wgpu => Self::Wgpu,
        }
    }
}

fn main() {
    let _ = atho_node::dev::append_log("atho-qt", "starting atho-qt");
    if std::env::args().any(|arg| arg == "--help" || arg == "-h") {
        print_usage();
        return;
    }
    let mut cli = parse_args().unwrap_or_else(|err| {
        eprintln!("{err}");
        std::process::exit(1);
    });
    let network = cli.network.unwrap_or_else(default_network);
    if let Err(err) = network.operator_launch_allowed() {
        eprintln!("{err}");
        std::process::exit(1);
    }
    if should_auto_launch_local_node(&cli) {
        cli.local_node = true;
    }
    cli.apply_env();
    if cli.local_node {
        std::env::set_var("ATHO_QT_LOCAL", "1");
        std::env::set_var("ATHO_QT_FORCE_RPC", "1");
        let _ = atho_node::dev::append_log(
            "atho-qt",
            &format!("starting managed local node mode for {}", network.id()),
        );
    }
    let renderer = cli
        .renderer
        .unwrap_or_else(|| default_renderer_choice(cfg!(target_os = "windows")));
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Atho")
            .with_inner_size([860.0, 560.0])
            .with_min_inner_size([720.0, 460.0])
            .with_icon(atho_qt::resources::app_icon()),
        renderer: renderer.into(),
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
                cli.rpc_address.clone(),
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

fn should_auto_launch_local_node(cli: &QtCli) -> bool {
    if cli.local_node || cli.rpc_address.is_some() {
        return false;
    }
    launched_from_macos_app_bundle() || launched_from_windows_client_entrypoint()
}

fn launched_from_macos_app_bundle() -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    is_macos_app_bundle_executable(&exe)
}

fn is_macos_app_bundle_executable(exe: &Path) -> bool {
    let Some(app_root) = exe.parent().and_then(Path::parent).and_then(Path::parent) else {
        return false;
    };
    app_root.extension().and_then(|ext| ext.to_str()) == Some("app")
}

fn launched_from_windows_client_entrypoint() -> bool {
    if !cfg!(target_os = "windows") {
        return false;
    }
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    is_windows_client_entrypoint_executable(&exe)
}

fn is_windows_client_entrypoint_executable(exe: &Path) -> bool {
    exe.file_name().and_then(|name| name.to_str()) == Some("Atho.exe")
}

fn parse_args() -> Result<QtCli, String> {
    parse_args_from(std::env::args().skip(1).collect())
}

fn parse_args_from(args: Vec<String>) -> Result<QtCli, String> {
    let mut cli = QtCli::default();
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            value if Network::parse(value).is_some() => {
                cli.network = Network::parse(value);
                i += 1;
            }
            "--network" | "-n" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| "missing network value".to_string())?;
                cli.network = Network::parse(value);
                if cli.network.is_none() {
                    return Err(format!("invalid network {value}"));
                }
                i += 2;
            }
            "--rpc-addr" => {
                cli.rpc_address = Some(
                    args.get(i + 1)
                        .ok_or_else(|| "missing rpc address value".to_string())?
                        .clone(),
                );
                i += 2;
            }
            "--local-node" | "--embedded-node" => {
                cli.local_node = true;
                i += 1;
            }
            "--data-dir" => {
                cli.data_dir = Some(
                    args.get(i + 1)
                        .ok_or_else(|| "missing data directory value".to_string())?
                        .clone(),
                );
                i += 2;
            }
            "--p2p-addr" => {
                cli.p2p_addr = Some(
                    args.get(i + 1)
                        .ok_or_else(|| "missing p2p address value".to_string())?
                        .clone(),
                );
                i += 2;
            }
            "--renderer" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| "missing renderer value".to_string())?;
                cli.renderer = RendererChoice::parse(value);
                if cli.renderer.is_none() {
                    return Err(format!("invalid renderer {value}; expected glow or wgpu"));
                }
                i += 2;
            }
            "--peer" => {
                cli.peers.push(
                    args.get(i + 1)
                        .ok_or_else(|| "missing peer address value".to_string())?
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

fn default_network() -> Network {
    std::env::var("ATHO_NETWORK")
        .ok()
        .and_then(|raw| Network::parse(&raw))
        .unwrap_or_else(Network::operator_default)
}

fn default_renderer_choice(is_windows: bool) -> RendererChoice {
    if is_windows {
        // Windows is the only platform currently failing in the field on the
        // WGL ES-context path. Prefer wgpu there so source and packaged builds
        // avoid the missing-extension startup failure.
        RendererChoice::Wgpu
    } else {
        RendererChoice::Glow
    }
}

fn print_usage() {
    eprintln!("usage:");
    eprintln!(
        "  atho-qt [--network <testnet|regnet|prunetest>] [--rpc-addr HOST:PORT] [--data-dir PATH] [--renderer <glow|wgpu>]"
    );
    eprintln!(
        "  atho-qt --local-node [--network <testnet|regnet|prunetest>] [--peer HOST:PORT] [--p2p-addr HOST:PORT] [--data-dir PATH] [--renderer <glow|wgpu>]"
    );
    eprintln!("    --local-node starts a managed athod child process over RPC");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qt_cli_parses_local_node_options() {
        let args = vec![
            String::from("--local-node"),
            String::from("--network"),
            String::from("regnet"),
            String::from("--rpc-addr"),
            String::from("127.0.0.1:9210"),
            String::from("--p2p-addr"),
            String::from("0.0.0.0:9200"),
            String::from("--renderer"),
            String::from("wgpu"),
            String::from("--peer"),
            String::from("127.0.0.1:9300"),
            String::from("--data-dir"),
            String::from("/tmp/atho"),
        ];
        let parsed = parse_args_from(args).expect("parse");
        assert!(parsed.local_node);
        assert_eq!(parsed.network, Some(Network::Regnet));
        assert_eq!(parsed.rpc_address.as_deref(), Some("127.0.0.1:9210"));
        assert_eq!(parsed.p2p_addr.as_deref(), Some("0.0.0.0:9200"));
        assert_eq!(parsed.renderer, Some(RendererChoice::Wgpu));
        assert_eq!(parsed.peers, vec![String::from("127.0.0.1:9300")]);
        assert_eq!(parsed.data_dir.as_deref(), Some("/tmp/atho"));
    }

    #[test]
    fn macos_app_bundle_path_is_recognized() {
        let app_executable = Path::new("/Applications/Atho.app/Contents/MacOS/Atho");
        assert!(is_macos_app_bundle_executable(app_executable));

        let plain_executable = Path::new("/usr/local/bin/atho-qt");
        assert!(!is_macos_app_bundle_executable(plain_executable));
    }

    #[test]
    fn windows_client_entrypoint_path_is_recognized() {
        let client_executable = Path::new("C:/Program Files/Atho/Atho.exe");
        assert!(is_windows_client_entrypoint_executable(client_executable));

        let plain_executable = Path::new("C:/Program Files/Atho/atho-qt.exe");
        assert!(!is_windows_client_entrypoint_executable(plain_executable));
    }

    #[test]
    fn qt_cli_accepts_prunetest_network() {
        let args = vec![
            String::from("--network"),
            String::from("prune-test"),
            String::from("--local-node"),
        ];
        let parsed = parse_args_from(args).expect("parse");
        assert_eq!(parsed.network, Some(Network::Prunetest));
        assert!(parsed.local_node);
    }

    #[test]
    fn qt_cli_rejects_invalid_renderer() {
        let args = vec![String::from("--renderer"), String::from("metal")];
        assert!(parse_args_from(args).is_err());
    }

    #[test]
    fn default_renderer_prefers_wgpu_on_windows() {
        assert_eq!(default_renderer_choice(true), RendererChoice::Wgpu);
        assert_eq!(default_renderer_choice(false), RendererChoice::Glow);
    }
}
