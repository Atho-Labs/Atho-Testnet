use atho_core::network::Network;
use atho_rpc::command::{help_payload, parse_command_line};
use atho_rpc::request::RpcRequest;
use atho_rpc::response::RpcResponse;
use atho_rpc::transport::RpcClient;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Json,
    Pretty,
    Table,
}

impl OutputFormat {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "json" => Some(Self::Json),
            "pretty" => Some(Self::Pretty),
            "table" => Some(Self::Table),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliConfig {
    network: Network,
    rpc_address: Option<String>,
    format: OutputFormat,
    command_line: Option<String>,
    confirmed: bool,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            network: Network::Mainnet,
            rpc_address: None,
            format: OutputFormat::Pretty,
            command_line: None,
            confirmed: false,
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
    let config = parse_cli(&args)?;
    let Some(command_line) = config.command_line.as_deref() else {
        print_usage();
        return Ok(());
    };

    if command_line == "help" {
        print_value(
            &help_payload(None).map_err(|err| err.to_string())?,
            config.format,
        );
        return Ok(());
    }
    if let Some(query) = command_line.strip_prefix("help ") {
        print_value(
            &help_payload(Some(query.trim())).map_err(|err| err.to_string())?,
            config.format,
        );
        return Ok(());
    }

    let mut invocation = parse_command_line(command_line)?;
    invocation.confirmed = config.confirmed;

    let client = RpcClient::new(
        config
            .rpc_address
            .unwrap_or_else(|| atho_node::runtime::default_rpc_bind_address(config.network)),
    );
    match client.call(&RpcRequest::ExecuteCommand(invocation)) {
        Ok(RpcResponse::Command(response)) => {
            print_value(&response.data, config.format);
            Ok(())
        }
        Ok(RpcResponse::Error(error)) => {
            let payload = serde_json::json!({
                "success": false,
                "error": {
                    "code": error.code,
                    "title": error.title,
                    "message": error.message,
                    "severity": error.severity,
                    "details": error.details,
                }
            });
            print_value(&payload, config.format);
            Err(String::from("command failed"))
        }
        Ok(other) => Err(format!("unexpected rpc response: {other:?}")),
        Err(err) => Err(err.to_string()),
    }
}

fn parse_cli(args: &[String]) -> Result<CliConfig, String> {
    let mut config = CliConfig::default();
    let mut index = 0usize;
    let mut command = Vec::new();
    while index < args.len() {
        match args[index].as_str() {
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            "--network" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| String::from("--network requires a value"))?;
                config.network =
                    Network::parse(value).ok_or_else(|| format!("unknown network {value}"))?;
            }
            "--rpc-url" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| String::from("--rpc-url requires a value"))?;
                config.rpc_address = Some(normalize_rpc_address(value)?);
            }
            "--format" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| String::from("--format requires a value"))?;
                config.format = OutputFormat::parse(value)
                    .ok_or_else(|| format!("unknown output format {value}"))?;
            }
            "--confirm" => {
                config.confirmed = true;
            }
            "--cookie-auth" | "--rpc-user" | "--rpc-password" | "--timeout" => {
                return Err(format!(
                    "{} is not supported by the current Atho local RPC transport yet",
                    args[index]
                ));
            }
            "--verbose" | "--debug" => {}
            "--" => {
                command.extend(args[index + 1..].iter().cloned());
                break;
            }
            value if value.starts_with("--") => {
                return Err(format!("unknown flag {value}"));
            }
            _ => {
                command.extend(args[index..].iter().cloned());
                break;
            }
        }
        index += 1;
    }

    if !command.is_empty() {
        config.command_line = Some(command.join(" "));
    }
    Ok(config)
}

fn normalize_rpc_address(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    let trimmed = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))
        .unwrap_or(trimmed);
    if trimmed.is_empty() {
        return Err(String::from("rpc address cannot be empty"));
    }
    Ok(trimmed.to_string())
}

fn print_value(value: &serde_json::Value, format: OutputFormat) {
    match format {
        OutputFormat::Json => println!("{value}"),
        OutputFormat::Pretty => {
            println!(
                "{}",
                serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
            );
        }
        OutputFormat::Table => {
            if let Some(table) = format_table_value(value) {
                println!("{table}");
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
                );
            }
        }
    }
}

fn format_table_value(value: &serde_json::Value) -> Option<String> {
    let rows = value.as_array()?;
    if rows.is_empty() {
        return Some(String::from("(empty)"));
    }
    let objects: Vec<_> = rows.iter().map(|row| row.as_object()).collect();
    if objects.iter().any(|row| row.is_none()) {
        return None;
    }
    let mut columns = Vec::<String>::new();
    for row in objects.iter().flatten() {
        for key in row.keys() {
            if !columns.contains(key) {
                columns.push(key.clone());
            }
        }
    }
    if columns.is_empty() {
        return None;
    }

    let mut widths = columns
        .iter()
        .map(|column| column.len())
        .collect::<Vec<_>>();
    let rendered_rows = objects
        .into_iter()
        .flatten()
        .map(|row| {
            columns
                .iter()
                .enumerate()
                .map(|(index, column)| {
                    let cell = row
                        .get(column)
                        .map(render_table_cell)
                        .unwrap_or_else(|| String::from("-"));
                    widths[index] = widths[index].max(cell.len());
                    cell
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    let mut output = String::new();
    output.push_str(&render_table_row(&columns, &widths));
    output.push('\n');
    output.push_str(&render_table_separator(&widths));
    for row in rendered_rows {
        output.push('\n');
        output.push_str(&render_table_row(&row, &widths));
    }
    Some(output)
}

fn render_table_row(row: &[String], widths: &[usize]) -> String {
    row.iter()
        .zip(widths.iter())
        .map(|(cell, width)| format!("{cell:<width$}", width = *width))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn render_table_separator(widths: &[usize]) -> String {
    widths
        .iter()
        .map(|width| "-".repeat(*width))
        .collect::<Vec<_>>()
        .join("-+-")
}

fn render_table_cell(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::from("-"),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => value.clone(),
        _ => serde_json::to_string(value).unwrap_or_else(|_| String::from("?")),
    }
}

fn print_usage() {
    println!("Atho CLI");
    println!();
    println!("Usage:");
    println!("  atho-cli [--network <mainnet|testnet|regnet|prunetest>] [--rpc-url <host:port>] [--format <json|pretty|table>] <command> [args]");
    println!("  atho-cli help [command|group]");
    println!();
    println!("Flags:");
    println!("  --network      Select the network and default local RPC port");
    println!("  --rpc-url      Override the local RPC address");
    println!("  --format       Output format: json, pretty, or table");
    println!("  --confirm      Confirm dangerous commands when supported");
    println!("  --help         Show this usage text");
    println!();
    println!("Examples:");
    println!("  atho-cli --network testnet getblockchaininfo");
    println!("  atho-cli getpeerinfo --format table");
    println!("  atho-cli help getblocktemplate");
    println!();
    println!("Current local RPC note:");
    println!(
        "  Authentication flags are not implemented in the current Atho local RPC transport yet."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cli_accepts_network_and_command() {
        let config = parse_cli(&[
            String::from("--network"),
            String::from("regtest"),
            String::from("getblockchaininfo"),
        ])
        .expect("parse cli");
        assert_eq!(config.network, Network::Regnet);
        assert_eq!(config.command_line.as_deref(), Some("getblockchaininfo"));
    }

    #[test]
    fn parse_cli_normalizes_rpc_urls() {
        let config = parse_cli(&[
            String::from("--rpc-url"),
            String::from("http://127.0.0.1:9210"),
            String::from("getstatus"),
        ])
        .expect("parse cli");
        assert_eq!(config.rpc_address.as_deref(), Some("127.0.0.1:9210"));
    }

    #[test]
    fn parse_cli_rejects_unsupported_auth_flags() {
        let err = parse_cli(&[String::from("--cookie-auth"), String::from("getstatus")])
            .expect_err("unsupported");
        assert!(err.contains("not supported"));
    }
}
