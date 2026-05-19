// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Command-line RPC client for talking to a running Atho node.

use atho_core::network::Network;
use atho_rpc::command::{help_payload, parse_command_line};
use atho_rpc::request::RpcRequest;
use atho_rpc::response::RpcResponse;
use atho_rpc::transport::RpcClient;

/// Output rendering modes for command responses.
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

/// Parsed CLI settings for one `atho-cli` invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CliConfig {
    network: Network,
    rpc_address: Option<String>,
    rpc_user: Option<String>,
    rpc_password: Option<String>,
    cookie_auth: bool,
    format: OutputFormat,
    command_line: Option<String>,
    confirmed: bool,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            network: Network::Mainnet,
            rpc_address: None,
            rpc_user: None,
            rpc_password: None,
            cookie_auth: false,
            format: OutputFormat::Pretty,
            command_line: None,
            confirmed: false,
        }
    }
}

/// Entrypoint that converts process failures into a one-line stderr message.
fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

/// Parses arguments, executes the requested RPC command, and prints the result.
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

    let node_config = atho_node::config::NodeConfig::from_env(config.network);
    let rpc_address = config
        .rpc_address
        .unwrap_or_else(|| node_config.rpc_bind_address());
    let client = if node_config.rpc_auth.enabled {
        let prefer_cookie = config.cookie_auth
            || (node_config.rpc_auth.cookie_auth
                && config.rpc_user.is_none()
                && config.rpc_password.is_none());
        if prefer_cookie {
            match node_config
                .load_rpc_cookie_secret()
                .map_err(|err| err.to_string())?
            {
                Some(secret) => RpcClient::with_cookie(rpc_address, secret),
                None if config.cookie_auth => {
                    return Err(String::from(
                        "rpc cookie auth was requested, but the local .cookie file was not found",
                    ))
                }
                None => {
                    let rpc_user = config.rpc_user.unwrap_or(node_config.rpc_auth.username);
                    let rpc_password = config.rpc_password.unwrap_or(node_config.rpc_auth.password);
                    RpcClient::with_auth(rpc_address, rpc_user, rpc_password)
                }
            }
        } else {
            let rpc_user = config.rpc_user.unwrap_or(node_config.rpc_auth.username);
            let rpc_password = config.rpc_password.unwrap_or(node_config.rpc_auth.password);
            RpcClient::with_auth(rpc_address, rpc_user, rpc_password)
        }
    } else {
        RpcClient::new(rpc_address)
    };
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

/// Parses raw CLI arguments into a typed configuration.
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
            "--rpcuser" | "--rpc-user" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| String::from("--rpcuser requires a value"))?;
                config.rpc_user = Some(value.clone());
            }
            "--rpcpassword" | "--rpc-password" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| String::from("--rpcpassword requires a value"))?;
                config.rpc_password = Some(value.clone());
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
            "--cookie-auth" => {
                config.cookie_auth = true;
            }
            "--timeout" => {
                return Err(format!("{} is not supported yet", args[index]));
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

/// Normalizes a user-supplied RPC endpoint into a socket-address string.
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

/// Renders a JSON value according to the requested output format.
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

/// Converts a JSON value into a compact table cell when possible.
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

/// Renders a padded table row using precomputed column widths.
fn render_table_row(row: &[String], widths: &[usize]) -> String {
    row.iter()
        .zip(widths.iter())
        .map(|(cell, width)| format!("{cell:<width$}", width = *width))
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Renders the ASCII separator line for a table.
fn render_table_separator(widths: &[usize]) -> String {
    widths
        .iter()
        .map(|width| "-".repeat(*width))
        .collect::<Vec<_>>()
        .join("-+-")
}

/// Converts nested JSON into a compact single-cell string.
fn render_table_cell(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::from("-"),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => value.clone(),
        _ => serde_json::to_string(value).unwrap_or_else(|_| String::from("?")),
    }
}

/// Prints command-line usage and the supported flags.
fn print_usage() {
    println!("Atho CLI");
    println!();
    println!("Usage:");
    println!("  atho-cli [--network <mainnet|testnet|regnet|prunetest>] [--rpc-url <host:port>] [--cookie-auth] [--rpcuser USER] [--rpcpassword PASSWORD] [--format <json|pretty|table>] <command> [args]");
    println!("  atho-cli help [command|group]");
    println!();
    println!("Flags:");
    println!("  --network      Select the network and default local RPC port");
    println!("  --rpc-url      Override the local RPC address");
    println!("  --cookie-auth  Use the local node .cookie token when available");
    println!("  --rpcuser      RPC username when rpcauth=1");
    println!("  --rpcpassword  RPC password when rpcauth=1");
    println!("  --format       Output format: json, pretty, or table");
    println!("  --confirm      Confirm dangerous commands when supported");
    println!("  --help         Show this usage text");
    println!();
    println!("Examples:");
    println!("  atho-cli --network testnet getblockchaininfo");
    println!("  atho-cli getpeerinfo --format table");
    println!("  atho-cli help getblocktemplate");
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
    fn parse_cli_accepts_rpc_auth_flags() {
        let config = parse_cli(&[
            String::from("--rpcuser"),
            String::from("operator"),
            String::from("--rpcpassword"),
            String::from("secret"),
            String::from("getstatus"),
        ])
        .expect("parse cli");
        assert_eq!(config.rpc_user.as_deref(), Some("operator"));
        assert_eq!(config.rpc_password.as_deref(), Some("secret"));
    }

    #[test]
    fn parse_cli_accepts_cookie_auth_flag() {
        let config = parse_cli(&[String::from("--cookie-auth"), String::from("getstatus")])
            .expect("cookie auth");
        assert!(config.cookie_auth);
    }
}
