use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CommandPermission {
    PublicRead,
    LocalRead,
    LocalWrite,
    WalletRead,
    WalletWrite,
    WalletSecret,
    NodeAdmin,
    TestOnly,
    DangerousMainnetBlocked,
}

impl CommandPermission {
    pub const fn label(self) -> &'static str {
        match self {
            Self::PublicRead => "PUBLIC_READ",
            Self::LocalRead => "LOCAL_READ",
            Self::LocalWrite => "LOCAL_WRITE",
            Self::WalletRead => "WALLET_READ",
            Self::WalletWrite => "WALLET_WRITE",
            Self::WalletSecret => "WALLET_SECRET",
            Self::NodeAdmin => "NODE_ADMIN",
            Self::TestOnly => "TEST_ONLY",
            Self::DangerousMainnetBlocked => "DANGEROUS_MAINNET_BLOCKED",
        }
    }

    pub const fn requires_mutable_access(self) -> bool {
        match self {
            Self::PublicRead | Self::LocalRead | Self::WalletRead => false,
            Self::LocalWrite
            | Self::WalletWrite
            | Self::WalletSecret
            | Self::NodeAdmin
            | Self::TestOnly
            | Self::DangerousMainnetBlocked => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandGroup {
    Blockchain,
    Control,
    Mining,
    Network,
    Mempool,
    RawTransactions,
    Wallet,
    Storage,
    Pruning,
    Reorg,
    Snapshot,
    Util,
    Debug,
}

impl CommandGroup {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Blockchain => "blockchain",
            Self::Control => "control",
            Self::Mining => "mining",
            Self::Network => "network",
            Self::Mempool => "mempool",
            Self::RawTransactions => "rawtransactions",
            Self::Wallet => "wallet",
            Self::Storage => "storage",
            Self::Pruning => "pruning",
            Self::Reorg => "reorg",
            Self::Snapshot => "snapshot",
            Self::Util => "util",
            Self::Debug => "debug",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandDefinition {
    pub name: &'static str,
    pub group: CommandGroup,
    pub description: &'static str,
    pub usage: &'static str,
    pub args_schema: &'static str,
    pub result_schema: &'static str,
    pub permission: CommandPermission,
    pub mainnet_allowed: bool,
    pub wallet_required: bool,
    pub auth_required: bool,
    pub dangerous: bool,
    pub test_only: bool,
    pub audit_log_required: bool,
    pub examples: &'static [&'static str],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandHelpEntry {
    pub name: String,
    pub group: CommandGroup,
    pub description: String,
    pub usage: String,
    pub args_schema: String,
    pub result_schema: String,
    pub permission: CommandPermission,
    pub mainnet_allowed: bool,
    pub wallet_required: bool,
    pub auth_required: bool,
    pub dangerous: bool,
    pub test_only: bool,
    pub audit_log_required: bool,
    pub examples: Vec<String>,
}

impl From<&'static CommandDefinition> for CommandHelpEntry {
    fn from(definition: &'static CommandDefinition) -> Self {
        Self {
            name: definition.name.to_string(),
            group: definition.group,
            description: definition.description.to_string(),
            usage: definition.usage.to_string(),
            args_schema: definition.args_schema.to_string(),
            result_schema: definition.result_schema.to_string(),
            permission: definition.permission,
            mainnet_allowed: definition.mainnet_allowed,
            wallet_required: definition.wallet_required,
            auth_required: definition.auth_required,
            dangerous: definition.dangerous,
            test_only: definition.test_only,
            audit_log_required: definition.audit_log_required,
            examples: definition
                .examples
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandInvocation {
    pub name: String,
    pub args: Vec<String>,
    pub confirmed: bool,
}

impl CommandInvocation {
    pub fn new(name: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            name: name.into(),
            args,
            confirmed: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandResponse {
    pub command: String,
    pub group: CommandGroup,
    pub permission: CommandPermission,
    pub dangerous: bool,
    pub network: String,
    pub data: serde_json::Value,
}

pub const COMMANDS: &[CommandDefinition] = &[
    CommandDefinition {
        name: "help",
        group: CommandGroup::Control,
        description: "List all supported Atho RPC commands or show help for one command.",
        usage: "help [command|group]",
        args_schema: "optional string query",
        result_schema: "command metadata or grouped command summaries",
        permission: CommandPermission::PublicRead,
        mainnet_allowed: true,
        wallet_required: false,
        auth_required: false,
        dangerous: false,
        test_only: false,
        audit_log_required: false,
        examples: &["help", "help getblockchaininfo", "help mining"],
    },
    CommandDefinition {
        name: "getstatus",
        group: CommandGroup::Control,
        description: "Return the current Atho node status summary.",
        usage: "getstatus",
        args_schema: "no arguments",
        result_schema: "node status object",
        permission: CommandPermission::PublicRead,
        mainnet_allowed: true,
        wallet_required: false,
        auth_required: false,
        dangerous: false,
        test_only: false,
        audit_log_required: false,
        examples: &["getstatus"],
    },
    CommandDefinition {
        name: "gethealth",
        group: CommandGroup::Control,
        description: "Return a user-facing health summary for the local Atho node.",
        usage: "gethealth",
        args_schema: "no arguments",
        result_schema: "health object",
        permission: CommandPermission::PublicRead,
        mainnet_allowed: true,
        wallet_required: false,
        auth_required: false,
        dangerous: false,
        test_only: false,
        audit_log_required: false,
        examples: &["gethealth"],
    },
    CommandDefinition {
        name: "getversion",
        group: CommandGroup::Control,
        description: "Return the local Atho build version and protocol compatibility.",
        usage: "getversion",
        args_schema: "no arguments",
        result_schema: "version object",
        permission: CommandPermission::PublicRead,
        mainnet_allowed: true,
        wallet_required: false,
        auth_required: false,
        dangerous: false,
        test_only: false,
        audit_log_required: false,
        examples: &["getversion"],
    },
    CommandDefinition {
        name: "geterrorcodes",
        group: CommandGroup::Control,
        description: "Return the Atho structured error-code registry.",
        usage: "geterrorcodes",
        args_schema: "no arguments",
        result_schema: "array of error-code descriptors",
        permission: CommandPermission::PublicRead,
        mainnet_allowed: true,
        wallet_required: false,
        auth_required: false,
        dangerous: false,
        test_only: false,
        audit_log_required: false,
        examples: &["geterrorcodes"],
    },
    CommandDefinition {
        name: "getblockcount",
        group: CommandGroup::Blockchain,
        description: "Return the current canonical chain height.",
        usage: "getblockcount",
        args_schema: "no arguments",
        result_schema: "u64 height",
        permission: CommandPermission::PublicRead,
        mainnet_allowed: true,
        wallet_required: false,
        auth_required: false,
        dangerous: false,
        test_only: false,
        audit_log_required: false,
        examples: &["getblockcount"],
    },
    CommandDefinition {
        name: "getbestblockhash",
        group: CommandGroup::Blockchain,
        description: "Return the current canonical tip hash.",
        usage: "getbestblockhash",
        args_schema: "no arguments",
        result_schema: "48-byte hash hex string",
        permission: CommandPermission::PublicRead,
        mainnet_allowed: true,
        wallet_required: false,
        auth_required: false,
        dangerous: false,
        test_only: false,
        audit_log_required: false,
        examples: &["getbestblockhash"],
    },
    CommandDefinition {
        name: "getblockhash",
        group: CommandGroup::Blockchain,
        description: "Return the block hash at the requested height.",
        usage: "getblockhash <height>",
        args_schema: "height as unsigned integer",
        result_schema: "48-byte hash hex string",
        permission: CommandPermission::PublicRead,
        mainnet_allowed: true,
        wallet_required: false,
        auth_required: false,
        dangerous: false,
        test_only: false,
        audit_log_required: false,
        examples: &["getblockhash 0", "getblockhash 250"],
    },
    CommandDefinition {
        name: "getblock",
        group: CommandGroup::Blockchain,
        description: "Return a full Atho block by hash or height.",
        usage: "getblock <hash|height>",
        args_schema: "block hash hex or unsigned height",
        result_schema: "block object",
        permission: CommandPermission::PublicRead,
        mainnet_allowed: true,
        wallet_required: false,
        auth_required: false,
        dangerous: false,
        test_only: false,
        audit_log_required: false,
        examples: &["getblock 0", "getblock 0000abcd..."],
    },
    CommandDefinition {
        name: "getblockheader",
        group: CommandGroup::Blockchain,
        description: "Return an Atho block header by hash or height.",
        usage: "getblockheader <hash|height>",
        args_schema: "block hash hex or unsigned height",
        result_schema: "block header object",
        permission: CommandPermission::PublicRead,
        mainnet_allowed: true,
        wallet_required: false,
        auth_required: false,
        dangerous: false,
        test_only: false,
        audit_log_required: false,
        examples: &["getblockheader 0", "getblockheader 0000abcd..."],
    },
    CommandDefinition {
        name: "getblockchaininfo",
        group: CommandGroup::Blockchain,
        description: "Return chain, ruleset, difficulty-target, and verification status info.",
        usage: "getblockchaininfo",
        args_schema: "no arguments",
        result_schema: "blockchain info object",
        permission: CommandPermission::PublicRead,
        mainnet_allowed: true,
        wallet_required: false,
        auth_required: false,
        dangerous: false,
        test_only: false,
        audit_log_required: false,
        examples: &["getblockchaininfo"],
    },
    CommandDefinition {
        name: "getnetworkinfo",
        group: CommandGroup::Network,
        description: "Return the current Atho network diagnostics summary.",
        usage: "getnetworkinfo",
        args_schema: "no arguments",
        result_schema: "network diagnostics object",
        permission: CommandPermission::LocalRead,
        mainnet_allowed: true,
        wallet_required: false,
        auth_required: false,
        dangerous: false,
        test_only: false,
        audit_log_required: false,
        examples: &["getnetworkinfo"],
    },
    CommandDefinition {
        name: "getconnectioncount",
        group: CommandGroup::Network,
        description: "Return the current number of connected peers.",
        usage: "getconnectioncount",
        args_schema: "no arguments",
        result_schema: "peer count object",
        permission: CommandPermission::LocalRead,
        mainnet_allowed: true,
        wallet_required: false,
        auth_required: false,
        dangerous: false,
        test_only: false,
        audit_log_required: false,
        examples: &["getconnectioncount"],
    },
    CommandDefinition {
        name: "getpeerinfo",
        group: CommandGroup::Network,
        description: "Return safe per-peer diagnostics for the local Atho node.",
        usage: "getpeerinfo",
        args_schema: "no arguments",
        result_schema: "array of peer diagnostic objects",
        permission: CommandPermission::LocalRead,
        mainnet_allowed: true,
        wallet_required: false,
        auth_required: false,
        dangerous: false,
        test_only: false,
        audit_log_required: false,
        examples: &["getpeerinfo"],
    },
    CommandDefinition {
        name: "getmempoolinfo",
        group: CommandGroup::Mempool,
        description: "Return current mempool counts and total fee data.",
        usage: "getmempoolinfo",
        args_schema: "no arguments",
        result_schema: "mempool summary object",
        permission: CommandPermission::PublicRead,
        mainnet_allowed: true,
        wallet_required: false,
        auth_required: false,
        dangerous: false,
        test_only: false,
        audit_log_required: false,
        examples: &["getmempoolinfo"],
    },
    CommandDefinition {
        name: "getblocktemplate",
        group: CommandGroup::Mining,
        description: "Return the current candidate block template from the local node.",
        usage: "getblocktemplate",
        args_schema: "no arguments",
        result_schema: "block template object",
        permission: CommandPermission::LocalRead,
        mainnet_allowed: true,
        wallet_required: false,
        auth_required: false,
        dangerous: false,
        test_only: false,
        audit_log_required: false,
        examples: &["getblocktemplate"],
    },
    CommandDefinition {
        name: "gettemplateinfo",
        group: CommandGroup::Mining,
        description: "Return a mining-oriented summary of the current Atho block template.",
        usage: "gettemplateinfo",
        args_schema: "no arguments",
        result_schema: "template summary object",
        permission: CommandPermission::LocalRead,
        mainnet_allowed: true,
        wallet_required: false,
        auth_required: false,
        dangerous: false,
        test_only: false,
        audit_log_required: false,
        examples: &["gettemplateinfo"],
    },
    CommandDefinition {
        name: "getmininginfo",
        group: CommandGroup::Mining,
        description: "Return current mining-related chain information for the local node.",
        usage: "getmininginfo",
        args_schema: "no arguments",
        result_schema: "mining summary object",
        permission: CommandPermission::LocalRead,
        mainnet_allowed: true,
        wallet_required: false,
        auth_required: false,
        dangerous: false,
        test_only: false,
        audit_log_required: false,
        examples: &["getmininginfo"],
    },
    CommandDefinition {
        name: "getnetworkparams",
        group: CommandGroup::Blockchain,
        description: "Return Atho network identity, ports, prefixes, and wire-parameter data.",
        usage: "getnetworkparams",
        args_schema: "no arguments",
        result_schema: "network parameter object",
        permission: CommandPermission::PublicRead,
        mainnet_allowed: true,
        wallet_required: false,
        auth_required: false,
        dangerous: false,
        test_only: false,
        audit_log_required: false,
        examples: &["getnetworkparams"],
    },
    CommandDefinition {
        name: "getgenesisinfo",
        group: CommandGroup::Blockchain,
        description: "Return genesis block metadata for the active Atho network.",
        usage: "getgenesisinfo",
        args_schema: "no arguments",
        result_schema: "genesis info object",
        permission: CommandPermission::PublicRead,
        mainnet_allowed: true,
        wallet_required: false,
        auth_required: false,
        dangerous: false,
        test_only: false,
        audit_log_required: false,
        examples: &["getgenesisinfo"],
    },
    CommandDefinition {
        name: "validateathoaddress",
        group: CommandGroup::Util,
        description: "Validate an Atho address and return its decoded network information.",
        usage: "validateathoaddress <address>",
        args_schema: "address string",
        result_schema: "address validation object",
        permission: CommandPermission::PublicRead,
        mainnet_allowed: true,
        wallet_required: false,
        auth_required: false,
        dangerous: false,
        test_only: false,
        audit_log_required: false,
        examples: &["validateathoaddress A...", "validateathoaddress T..."],
    },
    CommandDefinition {
        name: "sha3_384",
        group: CommandGroup::Util,
        description: "Hash a UTF-8 string or hex payload with SHA3-384.",
        usage: "sha3_384 <input>",
        args_schema: "hex string or raw string",
        result_schema: "hash object",
        permission: CommandPermission::PublicRead,
        mainnet_allowed: true,
        wallet_required: false,
        auth_required: false,
        dangerous: false,
        test_only: false,
        audit_log_required: false,
        examples: &["sha3_384 ABC", "sha3_384 0x414243"],
    },
];

pub fn command_definition(name: &str) -> Option<&'static CommandDefinition> {
    COMMANDS
        .iter()
        .find(|definition| definition.name.eq_ignore_ascii_case(name))
}

pub fn search_commands(query: &str) -> Vec<&'static CommandDefinition> {
    let normalized = query.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return COMMANDS.iter().collect();
    }
    COMMANDS
        .iter()
        .filter(|definition| {
            definition.name.contains(&normalized)
                || definition.group.label().contains(&normalized)
                || definition
                    .description
                    .to_ascii_lowercase()
                    .contains(&normalized)
        })
        .collect()
}

pub fn command_requires_mutable_access(name: &str) -> bool {
    command_definition(name)
        .map(|definition| definition.permission.requires_mutable_access())
        .unwrap_or(false)
}

pub fn help_payload(query: Option<&str>) -> Result<serde_json::Value, String> {
    if let Some(query) = query {
        if let Some(definition) = command_definition(query) {
            return serde_json::to_value(CommandHelpEntry::from(definition))
                .map_err(|err| err.to_string());
        }

        let matches = search_commands(query);
        if matches.is_empty() {
            return Err(format!("unknown command or group {query}"));
        }

        let entries: Vec<_> = matches.into_iter().map(CommandHelpEntry::from).collect();
        return Ok(json!({
            "query": query,
            "count": entries.len(),
            "commands": entries,
        }));
    }

    let entries: Vec<_> = search_commands("")
        .into_iter()
        .map(CommandHelpEntry::from)
        .collect();
    let mut groups = BTreeMap::<String, Vec<CommandHelpEntry>>::new();
    for entry in entries {
        groups
            .entry(entry.group.label().to_string())
            .or_default()
            .push(entry);
    }

    Ok(json!({
        "count": groups.values().map(Vec::len).sum::<usize>(),
        "groups": groups,
    }))
}

pub fn parse_command_line(line: &str) -> Result<CommandInvocation, String> {
    let tokens = tokenize_command_line(line)?;
    if tokens.is_empty() {
        return Err(String::from("empty command"));
    }
    Ok(CommandInvocation {
        name: tokens[0].clone(),
        args: tokens[1..].to_vec(),
        confirmed: false,
    })
}

fn tokenize_command_line(line: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = None;
    let mut escape = false;

    for ch in line.chars() {
        if escape {
            current.push(ch);
            escape = false;
            continue;
        }

        match ch {
            '\\' => {
                escape = true;
            }
            '"' | '\'' => {
                if let Some(active) = in_quotes {
                    if active == ch {
                        in_quotes = None;
                    } else {
                        current.push(ch);
                    }
                } else {
                    in_quotes = Some(ch);
                }
            }
            ch if ch.is_whitespace() && in_quotes.is_none() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }

    if escape {
        return Err(String::from("unfinished escape sequence"));
    }
    if in_quotes.is_some() {
        return Err(String::from("unterminated quoted argument"));
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn command_registry_has_unique_names_and_usage() {
        let mut names = BTreeSet::new();
        for definition in COMMANDS {
            assert!(
                names.insert(definition.name),
                "duplicate {}",
                definition.name
            );
            assert!(!definition.description.is_empty());
            assert!(!definition.usage.is_empty());
            assert!(!definition.args_schema.is_empty());
            assert!(!definition.result_schema.is_empty());
        }
    }

    #[test]
    fn command_parser_handles_quotes_and_hex_like_inputs() {
        let parsed = parse_command_line(r#"validateathoaddress "A example""#).unwrap();
        assert_eq!(parsed.name, "validateathoaddress");
        assert_eq!(parsed.args, vec![String::from("A example")]);

        let parsed = parse_command_line(r#"sha3_384 0x414243"#).unwrap();
        assert_eq!(parsed.name, "sha3_384");
        assert_eq!(parsed.args, vec![String::from("0x414243")]);
    }

    #[test]
    fn mutable_access_is_false_for_initial_query_commands() {
        assert!(!command_requires_mutable_access("getstatus"));
        assert!(!command_requires_mutable_access("getblocktemplate"));
        assert!(!command_requires_mutable_access("sha3_384"));
    }

    #[test]
    fn help_payload_lists_commands_and_groups() {
        let payload = help_payload(None).expect("help payload");
        assert!(payload["count"].as_u64().unwrap_or_default() > 0);
        assert!(payload["groups"].is_object());

        let specific = help_payload(Some("getblockchaininfo")).expect("specific help");
        assert_eq!(specific["name"], "getblockchaininfo");

        let group = help_payload(Some("mining")).expect("group help");
        assert!(group["count"].as_u64().unwrap_or_default() > 0);
    }
}
