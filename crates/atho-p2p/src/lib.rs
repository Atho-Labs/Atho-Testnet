//! Atho peer-to-peer networking and relay logic.
//!
//! This crate handles handshake validation, peer management, message framing,
//! relay, and sync coordination for the Atho wire protocol.
//!
//! SECURITY: Network identity checks in this crate prevent cross-network relay
//! and constrain how untrusted peers can influence local node behavior.
#![forbid(unsafe_code)]

pub mod address_manager;
pub mod audit;
pub mod banlist;
pub mod codec;
pub mod config;
pub mod connection;
pub mod downloader;
pub mod handshake;
pub mod peer;
pub mod protocol;
pub mod relay;
pub mod sync;
