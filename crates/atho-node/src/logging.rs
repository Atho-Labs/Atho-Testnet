//! Lightweight logging configuration for node processes and local tooling.
use atho_core::network::Network;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoggingConfig {
    pub level: LogLevel,
}

impl LoggingConfig {
    pub fn new(level: LogLevel) -> Self {
        Self { level }
    }

    pub fn startup_line(&self, network: Network) -> String {
        format!("athod starting on {} at {:?}", network.id(), self.level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_line_is_simple_and_network_specific() {
        let config = LoggingConfig::new(LogLevel::Info);
        let line = config.startup_line(Network::Mainnet);
        assert!(line.contains("atho-mainnet"));
    }
}
