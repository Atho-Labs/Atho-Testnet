// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

pub fn compact_size_len(value: usize) -> usize {
    match value as u64 {
        0..=0xfc => 1,
        0xfd..=0xffff => 3,
        0x1_0000..=0xffff_ffff => 5,
        _ => 9,
    }
}

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
