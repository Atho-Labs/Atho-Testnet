//! Atho network identity and isolation parameters.
//!
//! This module keeps the runtime names, consensus ids, ports, and address
//! prefixes for the supported Atho networks in one place.
//!
//! INVARIANT: Mainnet, testnet, regnet, and prunetest must never share the same
//! consensus id, port set, or visible address prefix.
use serde::{Deserialize, Serialize};

/// Network mode selected for the running node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Network {
    /// Production network with real economic value.
    Mainnet,
    /// Public test network.
    Testnet,
    /// Local deterministic developer network.
    Regnet,
    /// Low-difficulty storage and pruning test network.
    Prunetest,
}

impl Network {
    /// Parses a CLI or RPC network selector into a canonical [`Network`].
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "mainnet" | "atho-mainnet" => Some(Self::Mainnet),
            "testnet" | "atho-testnet" => Some(Self::Testnet),
            "regnet" | "regtest" | "atho-regnet" => Some(Self::Regnet),
            "prunetest" | "prune-test" | "prune_test" | "atho-prunetest" => Some(Self::Prunetest),
            _ => None,
        }
    }

    /// Returns the one-byte network id committed into block headers.
    pub fn consensus_id(self) -> u8 {
        match self {
            Self::Mainnet => 1,
            Self::Testnet => 2,
            Self::Regnet => 3,
            Self::Prunetest => 4,
        }
    }

    /// Decodes the consensus network id found in canonical block bytes.
    pub fn from_consensus_id(id: u8) -> Option<Self> {
        match id {
            1 => Some(Self::Mainnet),
            2 => Some(Self::Testnet),
            3 => Some(Self::Regnet),
            4 => Some(Self::Prunetest),
            _ => None,
        }
    }

    /// Returns the stable human-readable network identifier.
    pub fn id(self) -> &'static str {
        match self {
            Self::Mainnet => "atho-mainnet",
            Self::Testnet => "atho-testnet",
            Self::Regnet => "atho-regnet",
            Self::Prunetest => "atho-prunetest",
        }
    }

    /// Returns the network tag mixed into domain-separated hashes.
    pub fn domain_tag(self) -> &'static str {
        match self {
            Self::Mainnet => "mainnet",
            Self::Testnet => "testnet",
            Self::Regnet => "regnet",
            Self::Prunetest => "prunetest",
        }
    }

    /// Returns the canonical CLI selector.
    pub fn cli_arg(self) -> &'static str {
        self.domain_tag()
    }

    /// Returns the default TCP P2P port for the network.
    pub fn p2p_port(self) -> u16 {
        match self {
            Self::Mainnet => 56_000,
            Self::Testnet => 9_100,
            Self::Regnet => 9_200,
            Self::Prunetest => 9_300,
        }
    }

    /// Returns the default loopback RPC port for the network.
    pub fn rpc_port(self) -> u16 {
        match self {
            Self::Mainnet => 9_010,
            Self::Testnet => 9_110,
            Self::Regnet => 9_210,
            Self::Prunetest => 9_310,
        }
    }

    /// Returns the 4-byte network magic used in framed disk and P2P records.
    ///
    /// INVARIANT: These values must remain unique per network so raw block
    /// archives and P2P traffic cannot be parsed across network boundaries.
    pub fn p2p_magic(self) -> [u8; 4] {
        match self {
            Self::Mainnet => [0xa7, 0x54, 0x48, 0x01],
            Self::Testnet => [0xa7, 0x54, 0x48, 0x02],
            Self::Regnet => [0xa7, 0x54, 0x48, 0x03],
            Self::Prunetest => [0xa7, 0x54, 0x48, 0x04],
        }
    }

    /// Decodes a 4-byte network magic into the corresponding network.
    pub fn from_p2p_magic(magic: [u8; 4]) -> Option<Self> {
        [Self::Mainnet, Self::Testnet, Self::Regnet, Self::Prunetest]
            .into_iter()
            .find(|network| network.p2p_magic() == magic)
    }

    /// Returns the first visible character of a base56 user-facing address.
    pub fn visible_prefix(self) -> char {
        match self {
            Self::Mainnet => 'A',
            Self::Testnet => 'T',
            Self::Regnet => 'R',
            Self::Prunetest => 'P',
        }
    }

    /// Returns the internal hashed-public-key prefix for the network.
    pub fn internal_hpk_prefix(self) -> &'static str {
        match self {
            Self::Mainnet => "ATHO",
            Self::Testnet | Self::Regnet => "ATHT",
            Self::Prunetest => "ATHP",
        }
    }

    /// Returns the storage flag embedded into genesis-derived UTXO state.
    pub fn utxo_flag(self) -> &'static str {
        match self {
            Self::Mainnet => "",
            Self::Testnet => "TEST-UTXO",
            Self::Regnet => "REG-UTXO",
            Self::Prunetest => "PRUNE-UTXO",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Network;

    #[test]
    fn network_identity_matches_reference() {
        assert_eq!(Network::Mainnet.id(), "atho-mainnet");
        assert_eq!(Network::Mainnet.cli_arg(), "mainnet");
        assert_eq!(Network::Mainnet.consensus_id(), 1);
        assert_eq!(Network::from_consensus_id(2), Some(Network::Testnet));
        assert_eq!(Network::parse("mainnet"), Some(Network::Mainnet));
        assert_eq!(Network::parse("atho-mainnet"), Some(Network::Mainnet));
        assert_eq!(Network::parse("atho-testnet"), Some(Network::Testnet));
        assert_eq!(Network::parse("atho-regnet"), Some(Network::Regnet));
        assert_eq!(Network::parse("prune-test"), Some(Network::Prunetest));
        assert_eq!(Network::from_consensus_id(4), Some(Network::Prunetest));
        assert_eq!(Network::Mainnet.p2p_port(), 56_000);
        assert_eq!(Network::Testnet.rpc_port(), 9_110);
        assert_eq!(Network::Regnet.visible_prefix(), 'R');
        assert_eq!(Network::Prunetest.visible_prefix(), 'P');
        assert_eq!(Network::Mainnet.p2p_magic(), [0xa7, 0x54, 0x48, 0x01]);
        assert_eq!(
            Network::from_p2p_magic([0xa7, 0x54, 0x48, 0x04]),
            Some(Network::Prunetest)
        );
        assert_eq!(Network::Mainnet.utxo_flag(), "");
        assert_eq!(Network::Testnet.utxo_flag(), "TEST-UTXO");
        assert_eq!(Network::Regnet.utxo_flag(), "REG-UTXO");
        assert_eq!(Network::Prunetest.utxo_flag(), "PRUNE-UTXO");
    }
}
