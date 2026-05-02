//! Structured P2P audit logging used by diagnostics and tests.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

fn logs_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("dev")
        .join("logs")
}

pub fn append_log(component: &str, line: &str) {
    if fs::create_dir_all(logs_dir()).is_err() {
        return;
    }
    let path = logs_dir().join(format!("{component}.log"));
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}
