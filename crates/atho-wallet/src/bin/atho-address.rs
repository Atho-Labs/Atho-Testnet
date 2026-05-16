// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

use atho_core::address::{address_checksum, decode_base56_address, hashed_public_key_from_digest};
use atho_core::constants::ADDRESS_CHECKSUM_BASE56_CHARS;
use atho_core::network::Network;
use atho_wallet::hd::WalletSeed;
use atho_wallet::mnemonic::MnemonicPhrase;
use atho_wallet::wallet::{Wallet, WalletAddress};

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("inspect") => inspect_address(&args[1..]),
        Some("generate") => generate_addresses(&args[1..]),
        Some("--help") | Some("-h") => {
            print_usage();
            Ok(())
        }
        _ => generate_addresses(&args),
    }
}

fn generate_addresses(args: &[String]) -> Result<(), String> {
    if matches!(
        args.first().map(String::as_str),
        Some("--help") | Some("-h")
    ) {
        print_usage();
        return Ok(());
    }
    let opts = parse_generate_options(args)?;
    let mut wallet = match (&opts.seed_hex, &opts.phrase) {
        (Some(seed_hex), None) => {
            Wallet::from_seed(WalletSeed(parse_seed_hex(seed_hex)?), opts.network)
        }
        (None, Some(phrase)) => {
            let mnemonic = MnemonicPhrase::parse(phrase).map_err(|err| err.to_string())?;
            Wallet::from_mnemonic(mnemonic, &opts.passphrase, opts.network)
        }
        _ => return Err("provide exactly one of --seed-hex or --phrase".to_string()),
    };

    for index in 0..opts.count {
        let address = if opts.change {
            wallet.checkout_change_address()
        } else {
            wallet.checkout_receive_address()
        };
        print_address(index, &address)?;
    }
    Ok(())
}

fn inspect_address(args: &[String]) -> Result<(), String> {
    let address = args
        .first()
        .ok_or_else(|| "usage: atho-address inspect <address>".to_string())?;
    let (digest, network) = decode_base56_address(address).map_err(|err| err.to_string())?;
    let prefix = address
        .chars()
        .next()
        .ok_or_else(|| "address is empty".to_string())?;
    let body = &address[1..address.len().saturating_sub(ADDRESS_CHECKSUM_BASE56_CHARS)];
    let checksum = address_checksum(prefix, body);
    println!("base56_address={address}");
    println!("network={}", network.id());
    println!("visible_prefix={prefix}");
    println!("payment_digest={}", hex::encode(digest));
    println!(
        "hashed_public_key={}",
        hashed_public_key_from_digest(network, &digest)
    );
    println!("checksum={}", hex::encode(checksum));
    println!("body={body}");
    Ok(())
}

fn print_address(index: usize, address: &WalletAddress) -> Result<(), String> {
    let (decoded_digest, decoded_network) =
        decode_base56_address(&address.address).map_err(|err| err.to_string())?;
    if decoded_digest != address.payment_digest {
        return Err("address digest verification failed".to_string());
    }
    if decoded_network != address.network {
        return Err("address network verification failed".to_string());
    }

    println!("address[{index}]");
    println!("  base56_address={}", address.address);
    println!("  network={}", address.network.id());
    println!("  visible_prefix={}", address.visible_prefix);
    println!("  path={:?}", address.path);
    println!("  hashed_public_key={}", address.hashed_public_key);
    println!("  public_key={}", hex::encode(&address.public_key));
    println!("  payment_digest={}", hex::encode(address.payment_digest));
    println!("  checksum={}", hex::encode(address.checksum));
    Ok(())
}

fn parse_generate_options(args: &[String]) -> Result<GenerateOptions, String> {
    let mut opts = GenerateOptions {
        network: default_network(),
        phrase: None,
        passphrase: String::new(),
        seed_hex: None,
        count: 1,
        change: false,
    };

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "mainnet" => {
                opts.network = Network::Mainnet;
                i += 1;
            }
            "testnet" => {
                opts.network = Network::Testnet;
                i += 1;
            }
            "regnet" | "regtest" => {
                opts.network = Network::Regnet;
                i += 1;
            }
            "--network" | "-n" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| "missing network value".to_string())?;
                opts.network =
                    parse_network(value).ok_or_else(|| format!("invalid network {value}"))?;
                i += 2;
            }
            "--phrase" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| "missing phrase value".to_string())?;
                opts.phrase = Some(value.clone());
                i += 2;
            }
            "--passphrase" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| "missing passphrase value".to_string())?;
                opts.passphrase = value.clone();
                i += 2;
            }
            "--seed-hex" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| "missing seed hex value".to_string())?;
                opts.seed_hex = Some(value.clone());
                i += 2;
            }
            "--count" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| "missing count value".to_string())?;
                opts.count = value
                    .parse::<usize>()
                    .map_err(|_| "invalid count".to_string())?;
                i += 2;
            }
            "--change" => {
                opts.change = true;
                i += 1;
            }
            "--receive" => {
                opts.change = false;
                i += 1;
            }
            value => {
                return Err(format!("unrecognized argument {value}"));
            }
        }
    }

    if opts.seed_hex.is_none() && opts.phrase.is_none() {
        return Err("provide --seed-hex or --phrase".to_string());
    }
    if opts.count == 0 {
        return Err("count must be at least 1".to_string());
    }

    Ok(opts)
}

fn parse_seed_hex(seed_hex: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(seed_hex).map_err(|err| err.to_string())?;
    if bytes.len() != 32 {
        return Err("seed hex must be exactly 32 bytes".to_string());
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&bytes);
    Ok(seed)
}

fn parse_network(value: &str) -> Option<Network> {
    match value {
        "mainnet" => Some(Network::Mainnet),
        "testnet" => Some(Network::Testnet),
        "regnet" | "regtest" => Some(Network::Regnet),
        _ => None,
    }
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
    eprintln!(
        "  atho-address generate [mainnet|testnet|regnet] --seed-hex <hex> [--count N] [--change]"
    );
    eprintln!("  atho-address generate [mainnet|testnet|regnet] --phrase <mnemonic> [--passphrase <text>]");
    eprintln!("  atho-address inspect <address>");
}

struct GenerateOptions {
    network: Network,
    phrase: Option<String>,
    passphrase: String,
    seed_hex: Option<String>,
    count: usize,
    change: bool,
}
