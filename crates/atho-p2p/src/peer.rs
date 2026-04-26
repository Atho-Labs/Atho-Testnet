use crate::protocol::{validate_handshake, Handshake, ProtocolError};
use atho_core::network::Network;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub network: Network,
    pub remote_addr: String,
}

impl Peer {
    pub fn new(network: Network, remote_addr: impl Into<String>) -> Self {
        Self {
            network,
            remote_addr: remote_addr.into(),
        }
    }

    pub fn accept_handshake(&self, handshake: &Handshake) -> Result<(), ProtocolError> {
        validate_handshake(handshake)?;
        if handshake.network != self.network {
            crate::audit::append_log(
                "p2p",
                &format!(
                    "rejected handshake network={} peer_network={}",
                    handshake.network.id(),
                    self.network.id()
                ),
            );
            return Err(ProtocolError::UnsupportedNetwork);
        }
        crate::audit::append_log(
            "p2p",
            &format!(
                "accepted handshake network={} peer={}",
                handshake.network.id(),
                self.remote_addr
            ),
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Handshake;

    #[test]
    fn peer_rejects_cross_network_handshakes() {
        let peer = Peer::new(Network::Mainnet, "127.0.0.1:56000");
        let handshake = Handshake {
            network: Network::Testnet,
            protocol_version: 1,
        };
        assert!(peer.accept_handshake(&handshake).is_err());
    }
}
