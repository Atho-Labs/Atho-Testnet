use atho_core::network::Network;

fn main() {
    let _ = atho_node::dev::append_log("atho-qt", "starting atho-qt");
    let options = eframe::NativeOptions::default();
    let result = eframe::run_native(
        "Atho",
        options,
        Box::new(|_cc| Box::new(atho_qt::app::DesktopApp::new(Network::Mainnet))),
    );

    if let Err(err) = result {
        let _ = atho_node::dev::append_log("atho-qt", &format!("failed to launch: {err}"));
        eprintln!("failed to launch atho-qt: {err}");
        std::process::exit(1);
    }

    let _ = atho_node::dev::append_log("atho-qt", "stopped atho-qt");
}
