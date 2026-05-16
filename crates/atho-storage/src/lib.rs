// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Persistent chainstate, block archive, and UTXO storage for Atho.
//!
//! The storage crate wraps LMDB-backed persistence and exposes the canonical
//! on-disk view of blocks, transactions, UTXOs, metadata, and peer health.
//!
//! STORAGE: Changes here must preserve atomic updates between the chain tip
//! snapshot and the UTXO set or the node can restart into an inconsistent view.
#![forbid(unsafe_code)]

pub mod block_files;
pub mod chainstate;
pub mod config;
pub mod db;
pub mod error;
pub mod path;
pub mod utxo;
pub mod validation;

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    pub(crate) fn acquire_global_test_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }
}
