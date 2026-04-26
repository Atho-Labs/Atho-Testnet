use atho_core::network::Network;
use atho_node::system::AthoSystem;
use atho_rpc::request::RpcRequest;
use atho_rpc::response::RpcResponse;

#[derive(Debug)]
pub struct ReadOnlyNodeConnection {
    system: AthoSystem,
}

impl ReadOnlyNodeConnection {
    pub fn new(network: Network) -> Self {
        let mut system = AthoSystem::new(atho_node::config::NodeConfig::new(network));
        system.start();
        Self { system }
    }

    pub fn request(&self, request: RpcRequest) -> RpcResponse {
        self.system.handle(request)
    }

    pub fn status(&self) -> atho_node::system::SystemStatus {
        self.system.status()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atho_rpc::request::RpcRequest;
    use atho_rpc::response::RpcResponse;

    #[test]
    fn read_only_connection_forwards_rpc_requests() {
        let conn = ReadOnlyNodeConnection::new(Network::Mainnet);
        assert_eq!(conn.request(RpcRequest::GetNetwork), RpcResponse::Network("atho-mainnet".into()));
    }
}
