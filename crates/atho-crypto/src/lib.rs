//! Atho cryptographic backends and secret-handling primitives.
//!
//! The current production path uses Falcon-512 for transaction signatures and
//! SHA3-384 for hashing. This crate keeps those implementations and their
//! secret-bearing wrappers isolated from higher-level runtime code.
#![deny(unsafe_code)]

pub mod error;
pub mod falcon;
pub mod secret;
