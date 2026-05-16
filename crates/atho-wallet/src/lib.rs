// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Atho wallet key management, address derivation, and encrypted persistence.
//!
//! The wallet crate is responsible for deterministic seed handling, HD address
//! generation, keypool management, and safe import/export flows.
//!
//! WALLET SECURITY: Secret-bearing types in this crate must never be logged,
//! serialized into unsafe debug output, or returned through public RPCs.
#![forbid(unsafe_code)]

pub mod address_book;
pub mod hd;
pub mod keypool;
pub mod mnemonic;
pub mod snapshot;
pub mod wallet;
