//! Atho Qt-style desktop client built on `eframe`/`egui`.
//!
//! The GUI crate provides the wallet operator experience, node diagnostics,
//! debug console, and local RPC integration while keeping node validation in
//! the backend crates.
//!
//! SECURITY: GUI actions may display or copy sensitive wallet data. The UI
//! layer must keep those flows explicit and avoid accidental leakage in logs.
#![forbid(unsafe_code)]

pub mod app;
pub mod connection;
pub mod error;
pub mod resources;
pub mod state;
pub mod view;

#[cfg(test)]
pub(crate) mod test_support {
    use std::cell::RefCell;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    thread_local! {
        static TEST_LOCK_STATE: RefCell<TestLockState> = const { RefCell::new(TestLockState {
            depth: 0,
            guard: None,
        }) };
    }

    #[derive(Debug)]
    struct TestLockState {
        depth: usize,
        guard: Option<MutexGuard<'static, ()>>,
    }

    #[derive(Debug)]
    pub(crate) struct TestLockGuard;

    pub(crate) fn acquire_global_test_lock() -> TestLockGuard {
        TEST_LOCK_STATE.with(|state| {
            let mut state = state.borrow_mut();
            if state.depth == 0 {
                static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
                let guard = LOCK
                    .get_or_init(|| Mutex::new(()))
                    .lock()
                    .unwrap_or_else(|err| err.into_inner());
                state.guard = Some(guard);
            }
            state.depth = state.depth.saturating_add(1);
        });
        TestLockGuard
    }

    impl Drop for TestLockGuard {
        fn drop(&mut self) {
            TEST_LOCK_STATE.with(|state| {
                let mut state = state.borrow_mut();
                state.depth = state.depth.saturating_sub(1);
                if state.depth == 0 {
                    let _ = state.guard.take();
                }
            });
        }
    }
}
