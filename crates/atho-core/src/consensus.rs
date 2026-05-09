//! Consensus configuration and deterministic validation helpers.
//!
//! Submodules in this namespace define protocol versions, proof-of-work rules,
//! subsidy schedules, and signing digests that all nodes must interpret
//! identically.
//!
//! CONSENSUS: Any change in this module can split the network if historical
//! blocks or transactions are re-evaluated differently.
pub mod params;
pub mod pow;
pub mod rules;
pub mod signatures;
pub mod subsidy;
pub mod tx_policy;
