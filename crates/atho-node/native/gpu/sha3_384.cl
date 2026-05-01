#ifndef DEBUG
#define DEBUG 0
#endif

// SHA3-384 parameters.
#define SHA3_RATE_BYTES 104u
#define HEADER_BYTES 211u
#define BLOCK_BYTES 219u
#define HASH_BYTES 48u
#define NONCE_DEC_MAX 20u

__constant static ulong RC[24] = {
    0x0000000000000001UL, 0x0000000000008082UL, 0x800000000000808aUL,
    0x8000000080008000UL, 0x000000000000808bUL, 0x0000000080000001UL,
    0x8000000080008081UL, 0x8000000000008009UL, 0x000000000000008aUL,
    0x0000000000000088UL, 0x0000000080008009UL, 0x000000008000000aUL,
    0x000000008000808bUL, 0x800000000000008bUL, 0x8000000000008089UL,
    0x8000000000008003UL, 0x8000000000008002UL, 0x8000000000000080UL,
    0x000000000000800aUL, 0x800000008000000aUL, 0x8000000080008081UL,
    0x8000000000008080UL, 0x0000000080000001UL, 0x8000000080008008UL
};

__constant static int RHO[25] = {
     0,  1, 62, 28, 27,
    36, 44,  6, 55, 20,
     3, 10, 43, 25, 39,
    41, 45, 15, 21,  8,
    18,  2, 61, 56, 14
};

__constant static int PI[25] = {
     0, 10, 20,  5, 15,
    16,  1, 11, 21,  6,
     7, 17,  2, 12, 22,
    23,  8, 18,  3, 13,
    14, 24,  9, 19,  4
};

inline ulong rotl64(ulong x, int n) {
    n &= 63;
    if (n == 0) return x;
    return (x << n) | (x >> (64 - n));
}

inline void keccak_f1600(ulong state[25]) {
    ulong C[5], D[5], B[25];
    for (int round = 0; round < 24; ++round) {
        for (int x = 0; x < 5; ++x) {
            C[x] = state[x] ^ state[x + 5] ^ state[x + 10] ^ state[x + 15] ^ state[x + 20];
        }
        for (int x = 0; x < 5; ++x) {
            D[x] = C[(x + 4) % 5] ^ rotl64(C[(x + 1) % 5], 1);
            for (int y = 0; y < 5; ++y) {
                state[x + 5 * y] ^= D[x];
            }
        }
        for (int x = 0; x < 25; ++x) {
            B[PI[x]] = rotl64(state[x], RHO[x]);
        }
        for (int y = 0; y < 5; ++y) {
            for (int x = 0; x < 5; ++x) {
                state[x + 5 * y] =
                    B[x + 5 * y] ^
                    ((~B[(x + 1) % 5 + 5 * y]) & B[(x + 2) % 5 + 5 * y]);
            }
        }
        state[0] ^= RC[round];
    }
}

inline void absorb_rate_block(ulong state[25], __private const uchar block[SHA3_RATE_BYTES]) {
    for (uint i = 0; i < SHA3_RATE_BYTES; i += 8u) {
        ulong lane = 0UL;
        for (int j = 0; j < 8; ++j) {
            lane |= ((ulong)block[i + (uint)j]) << (8 * j);
        }
        state[i >> 3] ^= lane;
    }
}

inline uchar squeeze_digest_byte(const ulong state[25], int idx) {
    // SHA3 output is lane-little-endian byte stream.
    int lane = idx >> 3;
    int shift = (idx & 7) * 8;
    return (uchar)((state[lane] >> shift) & 0xFFUL);
}

inline bool digest_le_target(__private const uchar digest[HASH_BYTES], __global const uchar* target_bytes) {
    for (int i = 0; i < HASH_BYTES; ++i) {
        const uchar d = digest[i];
        const uchar t = target_bytes[i];
        if (d < t) return true;
        if (d > t) return false;
    }
    return true;
}

inline void fill_rate_block_from_template(
    __global const uchar* prefix,
    const uint prefix_len,
    __private const uchar nonce_dec[NONCE_DEC_MAX],
    const uint nonce_len,
    __global const uchar* suffix,
    const uint suffix_len,
    const uint offset,
    __private uchar rate_block[SHA3_RATE_BYTES]
) {
    for (uint i = 0u; i < SHA3_RATE_BYTES; ++i) {
        rate_block[i] = 0;
    }

    const uint total_len = prefix_len + nonce_len + suffix_len;
    if (offset >= total_len) {
        return;
    }

    uint to_copy = total_len - offset;
    if (to_copy > SHA3_RATE_BYTES) {
        to_copy = SHA3_RATE_BYTES;
    }

    uint dst = 0u;
    uint src = offset;
    const uint nonce_start = prefix_len;
    const uint suffix_start = prefix_len + nonce_len;

    while (dst < to_copy) {
        if (src < prefix_len) {
            uint avail = prefix_len - src;
            uint take = to_copy - dst;
            if (take > avail) {
                take = avail;
            }
            for (uint j = 0u; j < take; ++j) {
                rate_block[dst + j] = prefix[src + j];
            }
            dst += take;
            src += take;
            continue;
        }

        if (src < suffix_start) {
            const uint nonce_idx = src - nonce_start;
            uint avail = nonce_len - nonce_idx;
            uint take = to_copy - dst;
            if (take > avail) {
                take = avail;
            }
            for (uint j = 0u; j < take; ++j) {
                rate_block[dst + j] = nonce_dec[nonce_idx + j];
            }
            dst += take;
            src += take;
            continue;
        }

        const uint suffix_idx = src - suffix_start;
        uint avail = suffix_len - suffix_idx;
        uint take = to_copy - dst;
        if (take > avail) {
            take = avail;
        }
        for (uint j = 0u; j < take; ++j) {
            rate_block[dst + j] = suffix[suffix_idx + j];
        }
        dst += take;
        src += take;
    }
}

inline void hash_template_sha3_384(
    __global const uchar* prefix,
    const uint prefix_len,
    __private const uchar nonce_dec[NONCE_DEC_MAX],
    const uint nonce_len,
    __global const uchar* suffix,
    const uint suffix_len,
    __private uchar digest[HASH_BYTES]
) {
    ulong state[25] = {0};
    uchar rate_block[SHA3_RATE_BYTES];
    const uint total_len = prefix_len + nonce_len + suffix_len;
    uint offset = 0u;

    while ((total_len - offset) >= SHA3_RATE_BYTES) {
        fill_rate_block_from_template(prefix, prefix_len, nonce_dec, nonce_len, suffix, suffix_len, offset, rate_block);
        absorb_rate_block(state, rate_block);
        keccak_f1600(state);
        offset += SHA3_RATE_BYTES;
    }

    fill_rate_block_from_template(prefix, prefix_len, nonce_dec, nonce_len, suffix, suffix_len, offset, rate_block);
    const uint rem = total_len - offset;
    rate_block[rem] ^= 0x06;
    rate_block[SHA3_RATE_BYTES - 1u] ^= 0x80;
    absorb_rate_block(state, rate_block);
    keccak_f1600(state);

    for (int i = 0; i < HASH_BYTES; ++i) {
        digest[i] = squeeze_digest_byte(state, i);
    }
}

__kernel void sha3_384_mining(
    __global const uchar* header,
    __global const uchar* target_bytes,
    volatile __global int* found_flag,
    __global ulong* found_nonce,
    __global uchar* found_hash,
    const uint nonce_offset,
    const ulong start_nonce,
    const ulong batch_size
) {
    const ulong gid = get_global_id(0);
    if (gid >= batch_size) {
        return;
    }

    const ulong nonce = start_nonce + gid;
    ulong state[25] = {0};
    uchar msg[BLOCK_BYTES];
    uchar rate_block[SHA3_RATE_BYTES];

    for (uint i = 0; i < HEADER_BYTES; ++i) {
        msg[i] = header[i];
    }
    for (int i = 0; i < 8; ++i) {
        msg[nonce_offset + (uint)i] = (uchar)((nonce >> (i * 8)) & 0xFFUL);
    }

    const uint total_len = BLOCK_BYTES;
    uint offset = 0u;

    while ((total_len - offset) >= SHA3_RATE_BYTES) {
        for (uint i = 0; i < SHA3_RATE_BYTES; ++i) {
            rate_block[i] = msg[offset + i];
        }
        absorb_rate_block(state, rate_block);
        keccak_f1600(state);
        offset += SHA3_RATE_BYTES;
    }

    for (uint i = 0; i < SHA3_RATE_BYTES; ++i) {
        rate_block[i] = 0;
    }
    const uint remaining = total_len - offset;
    for (uint i = 0; i < remaining; ++i) {
        rate_block[i] = msg[offset + i];
    }
    rate_block[remaining] ^= 0x06; // domain separation for SHA3
    rate_block[SHA3_RATE_BYTES - 1u] ^= 0x80; // final bit
    absorb_rate_block(state, rate_block);
    keccak_f1600(state);

    uchar digest[HASH_BYTES];
    for (int i = 0; i < HASH_BYTES; ++i) {
        digest[i] = squeeze_digest_byte(state, i);
    }

    const bool is_valid = digest_le_target(digest, target_bytes);
    if (is_valid) {
        if ((*found_flag == 0) && (atomic_cmpxchg(found_flag, 0, 1) == 0)) {
            found_nonce[0] = nonce;
            for (int i = 0; i < HASH_BYTES; ++i) {
                found_hash[i] = digest[i];
            }
        }
    }

    #if DEBUG
    if (gid == 0 || is_valid) {
        printf("[KERNEL] nonce=%lu valid=%d\n", nonce, (int)is_valid);
    }
    #endif
}

__kernel void sha3_384_mining_template(
    __global const uchar* prefix,
    const uint prefix_len,
    __global const uchar* suffix,
    const uint suffix_len,
    __global const uchar* target_bytes,
    volatile __global int* found_flag,
    __global ulong* found_nonce,
    __global uchar* found_hash,
    const ulong start_nonce,
    const ulong batch_size
) {
    const ulong gid = get_global_id(0);
    if (gid >= batch_size) {
        return;
    }

    const ulong nonce = start_nonce + gid;
    uchar nonce_dec[NONCE_DEC_MAX];
    uchar rev[NONCE_DEC_MAX];
    uint nonce_len = 0u;
    ulong tmp = nonce;

    if (tmp == 0UL) {
        nonce_dec[0] = (uchar)'0';
        nonce_len = 1u;
    } else {
        while (tmp > 0UL && nonce_len < NONCE_DEC_MAX) {
            rev[nonce_len] = (uchar)('0' + (tmp % 10UL));
            tmp /= 10UL;
            nonce_len++;
        }
        for (uint i = 0u; i < nonce_len; ++i) {
            nonce_dec[i] = rev[nonce_len - 1u - i];
        }
    }

    uchar digest[HASH_BYTES];
    hash_template_sha3_384(prefix, prefix_len, nonce_dec, nonce_len, suffix, suffix_len, digest);

    const bool is_valid = digest_le_target(digest, target_bytes);
    if (is_valid) {
        if ((*found_flag == 0) && (atomic_cmpxchg(found_flag, 0, 1) == 0)) {
            found_nonce[0] = nonce;
            for (int i = 0; i < HASH_BYTES; ++i) {
                found_hash[i] = digest[i];
            }
        }
    }

    #if DEBUG
    if (gid == 0 || is_valid) {
        printf("[KERNEL][TEMPLATE] nonce=%lu valid=%d\n", nonce, (int)is_valid);
    }
    #endif
}
