use atho_core::network::Network;

#[derive(Debug, Clone)]
pub struct RelayLoop {
    network: Network,
}

impl RelayLoop {
    pub fn new(network: Network) -> Self {
        Self { network }
    }

    pub fn prime(&self) {
        crate::audit::append_log(
            "p2p",
            &format!("relay primed network={}", self.network.id()),
        );
    }

    pub fn sync_headers(&self, best_height: u64) {
        crate::audit::append_log(
            "p2p",
            &format!(
                "relay headers placeholder network={} best_height={}",
                self.network.id(),
                best_height
            ),
        );
    }

    pub fn relay_block(&self, block_hash: &[u8; 48], tx_count: usize) {
        crate::audit::append_log(
            "p2p",
            &format!(
                "relay block placeholder network={} block={} txs={}",
                self.network.id(),
                hex::encode(block_hash),
                tx_count
            ),
        );
    }

    pub fn relay_transaction(&self, txid: &[u8; 48]) {
        crate::audit::append_log(
            "p2p",
            &format!(
                "relay tx placeholder network={} txid={}",
                self.network.id(),
                hex::encode(txid)
            ),
        );
    }
}
