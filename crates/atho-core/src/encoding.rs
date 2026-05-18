// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Compact integer encoding helpers shared by Atho's wire and storage formats.
//!
//! The implementation mirrors Bitcoin-style compact size prefixes so callers can
//! produce canonical byte layouts for hashes, P2P payloads, and persisted data.

/// Returns the number of bytes needed to encode `value` as a compact size.
pub fn compact_size_len(value: usize) -> usize {
    match value as u64 {
        0..=0xfc => 1,
        0xfd..=0xffff => 3,
        0x1_0000..=0xffff_ffff => 5,
        _ => 9,
    }
}

/// Appends the canonical compact-size encoding for `value` to `out`.
pub fn write_compact_size(out: &mut Vec<u8>, value: usize) {
    let value = value as u64;
    match value {
        0..=0xfc => out.push(value as u8),
        0xfd..=0xffff => {
            out.push(0xfd);
            out.extend_from_slice(&(value as u16).to_le_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(0xfe);
            out.extend_from_slice(&(value as u32).to_le_bytes());
        }
        _ => {
            out.push(0xff);
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
}
