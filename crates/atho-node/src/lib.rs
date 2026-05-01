#![forbid(unsafe_code)]

pub mod chainstate;
pub mod config;
pub mod dev;
pub mod error;
pub mod logging;
pub mod mempool;
pub mod miner;
pub mod mining;
pub mod mining_backend;
pub mod node;
pub mod orchestrator;
pub mod runtime;
pub mod service;
pub mod sync;
pub mod system;
pub mod tcp_p2p;
pub mod validation;
pub mod wallet_history;

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

    #[allow(dead_code)]
    fn _lock_depth() -> usize {
        TEST_LOCK_STATE.with(|state| state.borrow().depth)
    }
}
