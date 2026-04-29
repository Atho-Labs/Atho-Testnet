#![forbid(unsafe_code)]

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
