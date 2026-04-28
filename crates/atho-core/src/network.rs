use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Network {
    Mainnet,
    Testnet,
    Regnet,
}

impl Network {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "mainnet" | "atho-mainnet" => Some(Self::Mainnet),
            "testnet" | "atho-testnet" => Some(Self::Testnet),
            "regnet" | "regtest" | "atho-regnet" => Some(Self::Regnet),
            _ => None,
        }
    }

    pub fn consensus_id(self) -> u8 {
        match self {
            Self::Mainnet => 1,
            Self::Testnet => 2,
            Self::Regnet => 3,
        }
    }

    pub fn from_consensus_id(id: u8) -> Option<Self> {
        match id {
            1 => Some(Self::Mainnet),
            2 => Some(Self::Testnet),
            3 => Some(Self::Regnet),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Mainnet => "atho-mainnet",
            Self::Testnet => "atho-testnet",
            Self::Regnet => "atho-regnet",
        }
    }

    pub fn domain_tag(self) -> &'static str {
        match self {
            Self::Mainnet => "mainnet",
            Self::Testnet => "testnet",
            Self::Regnet => "regnet",
        }
    }

    pub fn cli_arg(self) -> &'static str {
        self.domain_tag()
    }

    pub fn p2p_port(self) -> u16 {
        match self {
            Self::Mainnet => 56_000,
            Self::Testnet => 9_100,
            Self::Regnet => 9_200,
        }
    }

    pub fn rpc_port(self) -> u16 {
        match self {
            Self::Mainnet => 9_010,
            Self::Testnet => 9_110,
            Self::Regnet => 9_210,
        }
    }

    pub fn visible_prefix(self) -> char {
        match self {
            Self::Mainnet => 'A',
            Self::Testnet => 'T',
            Self::Regnet => 'R',
        }
    }

    pub fn internal_hpk_prefix(self) -> &'static str {
        match self {
            Self::Mainnet => "ATHO",
            Self::Testnet | Self::Regnet => "ATHT",
        }
    }

    pub fn utxo_flag(self) -> &'static str {
        match self {
            Self::Mainnet => "",
            Self::Testnet => "TEST-UTXO",
            Self::Regnet => "REG-UTXO",
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
        assert_eq!(Network::Mainnet.p2p_port(), 56_000);
        assert_eq!(Network::Testnet.rpc_port(), 9_110);
        assert_eq!(Network::Regnet.visible_prefix(), 'R');
        assert_eq!(Network::Mainnet.utxo_flag(), "");
        assert_eq!(Network::Testnet.utxo_flag(), "TEST-UTXO");
        assert_eq!(Network::Regnet.utxo_flag(), "REG-UTXO");
    }
}
