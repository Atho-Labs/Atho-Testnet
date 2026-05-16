// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

#include <cuda_runtime.h>

#include <algorithm>
#include <cstdint>
#include <cstdlib>
#include <iomanip>
#include <iostream>
#include <limits>
#include <sstream>
#include <stdexcept>
#include <string>
#include <vector>

namespace {

constexpr size_t HEADER_SIZE = 211;
constexpr size_t NONCE_SIZE = 8;
constexpr size_t BLOCK_SIZE = HEADER_SIZE + NONCE_SIZE;
constexpr size_t HASH_SIZE = 48;
constexpr size_t TARGET_SIZE = 48;
constexpr uint64_t DEFAULT_MAX_BATCH = 2'000'000ULL;

#define CHECK_CUDA(expr)                                                                  \
    do {                                                                                  \
        cudaError_t _err = (expr);                                                        \
        if (_err != cudaSuccess) {                                                        \
            throw std::runtime_error(std::string("CUDA error: ") + cudaGetErrorString(_err)); \
        }                                                                                 \
    } while (0)

__device__ __constant__ unsigned long long RC[24] = {
    0x0000000000000001ULL, 0x0000000000008082ULL, 0x800000000000808aULL,
    0x8000000080008000ULL, 0x000000000000808bULL, 0x0000000080000001ULL,
    0x8000000080008081ULL, 0x8000000000008009ULL, 0x000000000000008aULL,
    0x0000000000000088ULL, 0x0000000080008009ULL, 0x000000008000000aULL,
    0x000000008000808bULL, 0x800000000000008bULL, 0x8000000000008089ULL,
    0x8000000000008003ULL, 0x8000000000008002ULL, 0x8000000000000080ULL,
    0x000000000000800aULL, 0x800000008000000aULL, 0x8000000080008081ULL,
    0x8000000000008080ULL, 0x0000000080000001ULL, 0x8000000080008008ULL
};

__device__ __constant__ int RHO[25] = {
     0,  1, 62, 28, 27,
    36, 44,  6, 55, 20,
     3, 10, 43, 25, 39,
    41, 45, 15, 21,  8,
    18,  2, 61, 56, 14
};

__device__ __constant__ int PI[25] = {
     0, 10, 20,  5, 15,
    16,  1, 11, 21,  6,
     7, 17,  2, 12, 22,
    23,  8, 18,  3, 13,
    14, 24,  9, 19,  4
};

__device__ inline unsigned long long rotl64(unsigned long long x, int n) {
    n &= 63;
    if (n == 0) return x;
    return (x << n) | (x >> (64 - n));
}

__device__ void keccak_f1600(unsigned long long state[25]) {
    unsigned long long C[5], D[5], B[25];
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

__device__ inline void absorb_rate_block(unsigned long long state[25], const unsigned char block[104]) {
    for (unsigned int i = 0; i < 104; i += 8) {
        unsigned long long lane = 0ULL;
        for (int j = 0; j < 8; ++j) {
            lane |= ((unsigned long long)block[i + (unsigned int)j]) << (8 * j);
        }
        state[i >> 3] ^= lane;
    }
}

__device__ inline unsigned char squeeze_digest_byte(const unsigned long long state[25], int idx) {
    int lane = idx >> 3;
    int shift = (idx & 7) * 8;
    return (unsigned char)((state[lane] >> shift) & 0xFFULL);
}

__device__ inline bool digest_le_target(
    const unsigned char digest[HASH_SIZE],
    const unsigned char* target_bytes
) {
    for (int i = 0; i < (int)HASH_SIZE; ++i) {
        const unsigned char d = digest[i];
        const unsigned char t = target_bytes[i];
        if (d < t) return true;
        if (d > t) return false;
    }
    return true;
}

__global__ void sha3_384_cuda(
    const unsigned char* header,
    const unsigned char* target_bytes,
    int* found_flag,
    unsigned long long* found_nonce,
    unsigned char* found_hash,
    unsigned int nonce_offset,
    unsigned long long start_nonce,
    unsigned long long batch_size
) {
    const unsigned long long gid = (unsigned long long)blockIdx.x * blockDim.x + threadIdx.x;
    if (gid >= batch_size) return;

    const unsigned long long nonce = start_nonce + gid;
    unsigned long long state[25] = {0};
    unsigned char msg[BLOCK_SIZE];
    unsigned char rate_block[104];

    for (unsigned int i = 0; i < HEADER_SIZE; ++i) {
        msg[i] = header[i];
    }
    for (int i = 0; i < 8; ++i) {
        msg[nonce_offset + (unsigned int)i] = (unsigned char)((nonce >> (i * 8)) & 0xFFULL);
    }

    const unsigned int total_len = BLOCK_SIZE;
    unsigned int offset = 0u;

    while ((total_len - offset) >= 104u) {
        for (unsigned int i = 0; i < 104u; ++i) {
            rate_block[i] = msg[offset + i];
        }
        absorb_rate_block(state, rate_block);
        keccak_f1600(state);
        offset += 104u;
    }

    for (unsigned int i = 0; i < 104u; ++i) {
        rate_block[i] = 0;
    }
    const unsigned int remaining = total_len - offset;
    for (unsigned int i = 0; i < remaining; ++i) {
        rate_block[i] = msg[offset + i];
    }
    rate_block[remaining] ^= 0x06;
    rate_block[103] ^= 0x80;
    absorb_rate_block(state, rate_block);
    keccak_f1600(state);

    unsigned char digest[HASH_SIZE];
    for (int i = 0; i < (int)HASH_SIZE; ++i) {
        digest[i] = squeeze_digest_byte(state, i);
    }

    if (digest_le_target(digest, target_bytes)) {
        if (atomicCAS(found_flag, 0, 1) == 0) {
            found_nonce[0] = nonce;
            for (int i = 0; i < (int)HASH_SIZE; ++i) {
                found_hash[i] = digest[i];
            }
        }
    }
}

uint64_t env_u64(const char* key, uint64_t fallback) {
    const char* raw = std::getenv(key);
    if (!raw || !*raw) return fallback;
    try {
        return std::stoull(raw);
    } catch (...) {
        return fallback;
    }
}

std::vector<unsigned char> hex_to_bytes(const std::string& hex) {
    if (hex.size() % 2 != 0) throw std::runtime_error("hex string length must be even");
    std::vector<unsigned char> out;
    out.reserve(hex.size() / 2);
    for (size_t i = 0; i < hex.size(); i += 2) {
        const std::string piece = hex.substr(i, 2);
        unsigned int value = 0;
        std::stringstream ss;
        ss << std::hex << piece;
        if (!(ss >> value)) throw std::runtime_error("invalid hex at offset " + std::to_string(i));
        out.push_back(static_cast<unsigned char>(value));
    }
    return out;
}

std::string bytes_to_hex(const unsigned char* data, size_t len) {
    std::ostringstream ss;
    ss << std::hex << std::setfill('0');
    for (size_t i = 0; i < len; ++i) {
        ss << std::setw(2) << static_cast<unsigned int>(data[i]);
    }
    return ss.str();
}

std::string normalize_target_hex(std::string target_hex) {
    if (target_hex.rfind("0x", 0) == 0 || target_hex.rfind("0X", 0) == 0) {
        target_hex = target_hex.substr(2);
    }
    if (target_hex.empty()) target_hex = "0";
    for (char c : target_hex) {
        const bool ok = (c >= '0' && c <= '9') ||
                        (c >= 'a' && c <= 'f') ||
                        (c >= 'A' && c <= 'F');
        if (!ok) throw std::runtime_error("target contains non-hex characters");
    }
    if (target_hex.size() < TARGET_SIZE * 2) {
        target_hex = std::string(TARGET_SIZE * 2 - target_hex.size(), '0') + target_hex;
    } else if (target_hex.size() > TARGET_SIZE * 2) {
        target_hex = target_hex.substr(target_hex.size() - TARGET_SIZE * 2);
    }
    return target_hex;
}

void run_cuda(
    const std::string& header_hex,
    uint32_t nonce_offset,
    uint64_t start_nonce,
    uint64_t batch_size,
    const std::string& target_hex
) {
    if (header_hex.size() != HEADER_SIZE * 2) {
        throw std::runtime_error("header must be exactly 422 hex chars");
    }
    if (nonce_offset != HEADER_SIZE) {
        throw std::runtime_error("nonce_offset must be 211 for this kernel");
    }
    const uint64_t max_batch = env_u64("ATHO_GPU_MAX_BATCH", DEFAULT_MAX_BATCH);
    if (batch_size == 0 || batch_size > max_batch) {
        throw std::runtime_error("batch_size out of allowed range (1.." + std::to_string(max_batch) + ")");
    }
    if (start_nonce > (std::numeric_limits<uint64_t>::max() - (batch_size - 1))) {
        throw std::runtime_error("nonce range overflows uint64");
    }
    if (batch_size > (std::numeric_limits<size_t>::max() / HASH_SIZE)) {
        throw std::runtime_error("output buffer would overflow address space");
    }

    const std::vector<unsigned char> header = hex_to_bytes(header_hex);
    const std::string norm_target_hex = normalize_target_hex(target_hex);
    const std::vector<unsigned char> target = hex_to_bytes(norm_target_hex);

    int dev_count = 0;
    CHECK_CUDA(cudaGetDeviceCount(&dev_count));
    if (dev_count <= 0) {
        throw std::runtime_error("no CUDA devices found");
    }
    CHECK_CUDA(cudaSetDevice(0));

    cudaDeviceProp prop{};
    CHECK_CUDA(cudaGetDeviceProperties(&prop, 0));
    std::cout << "GPU backend: CUDA\n";
    std::cout << "Device: " << prop.name
              << " | SMs: " << prop.multiProcessorCount
              << " | ClockKHz: " << prop.clockRate
              << " | GlobalMemMB: " << (prop.totalGlobalMem / 1024 / 1024) << "\n";

    unsigned char* d_header = nullptr;
    unsigned char* d_target = nullptr;
    int* d_found_flag = nullptr;
    unsigned long long* d_found_nonce = nullptr;
    unsigned char* d_found_hash = nullptr;

    cudaEvent_t ev_start = nullptr, ev_end = nullptr;
    try {
        CHECK_CUDA(cudaMalloc(&d_header, HEADER_SIZE));
        CHECK_CUDA(cudaMalloc(&d_target, TARGET_SIZE));
        CHECK_CUDA(cudaMalloc(&d_found_flag, sizeof(int)));
        CHECK_CUDA(cudaMalloc(&d_found_nonce, sizeof(unsigned long long)));
        CHECK_CUDA(cudaMalloc(&d_found_hash, HASH_SIZE));
        CHECK_CUDA(cudaMemcpy(d_header, header.data(), HEADER_SIZE, cudaMemcpyHostToDevice));
        CHECK_CUDA(cudaMemcpy(d_target, target.data(), TARGET_SIZE, cudaMemcpyHostToDevice));
        const int init_flag = 0;
        const unsigned long long init_nonce = 0ULL;
        CHECK_CUDA(cudaMemcpy(d_found_flag, &init_flag, sizeof(int), cudaMemcpyHostToDevice));
        CHECK_CUDA(cudaMemcpy(d_found_nonce, &init_nonce, sizeof(unsigned long long), cudaMemcpyHostToDevice));

        CHECK_CUDA(cudaEventCreate(&ev_start));
        CHECK_CUDA(cudaEventCreate(&ev_end));

        const uint64_t requested_block_size = std::max<uint64_t>(1ULL, env_u64("ATHO_CUDA_BLOCK_SIZE", 256));
        const uint32_t block_size = static_cast<uint32_t>(
            std::min<uint64_t>(requested_block_size, static_cast<uint64_t>(std::max(1, prop.maxThreadsPerBlock)))
        );
        const uint32_t grid_size = static_cast<uint32_t>((batch_size + block_size - 1ULL) / block_size);

        CHECK_CUDA(cudaEventRecord(ev_start));
        sha3_384_cuda<<<grid_size, block_size>>>(
            d_header,
            d_target,
            d_found_flag,
            d_found_nonce,
            d_found_hash,
            nonce_offset,
            start_nonce,
            batch_size
        );
        CHECK_CUDA(cudaGetLastError());
        CHECK_CUDA(cudaEventRecord(ev_end));
        CHECK_CUDA(cudaEventSynchronize(ev_end));

        float elapsed_ms = 0.0f;
        CHECK_CUDA(cudaEventElapsedTime(&elapsed_ms, ev_start, ev_end));

        int found_flag = 0;
        CHECK_CUDA(cudaMemcpy(&found_flag, d_found_flag, sizeof(int), cudaMemcpyDeviceToHost));
        const bool found = (found_flag != 0);
        unsigned long long win_nonce = 0ULL;
        std::string win_hash;
        if (found) {
            std::vector<unsigned char> winner_hash(HASH_SIZE, 0);
            CHECK_CUDA(cudaMemcpy(&win_nonce, d_found_nonce, sizeof(unsigned long long), cudaMemcpyDeviceToHost));
            CHECK_CUDA(cudaMemcpy(winner_hash.data(), d_found_hash, HASH_SIZE, cudaMemcpyDeviceToHost));
            win_hash = bytes_to_hex(winner_hash.data(), HASH_SIZE);
        }

        const double throughput = elapsed_ms > 0.0 ? (static_cast<double>(batch_size) * 1000.0 / elapsed_ms) : 0.0;
        std::cout << std::fixed << std::setprecision(3);
        std::cout << "[PERF] Kernel execution time: " << elapsed_ms << " ms\n";
        std::cout << "[PERF] Throughput: " << throughput << " H/s\n";

        if (found) {
            std::cout << "🎉 SOLUTION FOUND!\n";
            std::cout << "Nonce: " << static_cast<uint64_t>(win_nonce) << "\n";
            std::cout << "Hash: " << win_hash << "\n";
            std::cout << "Kernel Time: " << elapsed_ms << " ms\n";
        } else {
            std::cout << "[RESULT] No valid nonce found in batch\n";
        }
    } catch (...) {
        if (ev_end) cudaEventDestroy(ev_end);
        if (ev_start) cudaEventDestroy(ev_start);
        if (d_found_hash) cudaFree(d_found_hash);
        if (d_found_nonce) cudaFree(d_found_nonce);
        if (d_found_flag) cudaFree(d_found_flag);
        if (d_target) cudaFree(d_target);
        if (d_header) cudaFree(d_header);
        throw;
    }

    if (ev_end) cudaEventDestroy(ev_end);
    if (ev_start) cudaEventDestroy(ev_start);
    if (d_found_hash) cudaFree(d_found_hash);
    if (d_found_nonce) cudaFree(d_found_nonce);
    if (d_found_flag) cudaFree(d_found_flag);
    if (d_target) cudaFree(d_target);
    if (d_header) cudaFree(d_header);
}

int probe_cuda() {
    try {
        int dev_count = 0;
        CHECK_CUDA(cudaGetDeviceCount(&dev_count));
        if (dev_count <= 0) {
            std::cout << "[PROBE] status=error\n";
            std::cout << "[PROBE] backend=cuda\n";
            std::cout << "[PROBE] error=no_cuda_device_found\n";
            return 2;
        }
        CHECK_CUDA(cudaSetDevice(0));
        cudaDeviceProp prop{};
        CHECK_CUDA(cudaGetDeviceProperties(&prop, 0));
        std::cout << "[PROBE] status=ok\n";
        std::cout << "[PROBE] backend=cuda\n";
        std::cout << "[PROBE] device_name=" << prop.name << "\n";
        std::cout << "[PROBE] device_vendor=nvidia\n";
        std::cout << "[PROBE] compute_units=" << prop.multiProcessorCount << "\n";
        std::cout << "[PROBE] clock_khz=" << prop.clockRate << "\n";
        std::cout << "[PROBE] global_mem_mb=" << (prop.totalGlobalMem / 1024 / 1024) << "\n";
        std::cout << "[PROBE] supports_fixed=1\n";
        std::cout << "[PROBE] supports_template=0\n";
        std::cout << "[PROBE] max_batch=" << env_u64("ATHO_GPU_MAX_BATCH", DEFAULT_MAX_BATCH) << "\n";
        std::cout << "[PROBE] template_max_bytes=0\n";
        return 0;
    } catch (const std::exception& e) {
        std::cout << "[PROBE] status=error\n";
        std::cout << "[PROBE] backend=cuda\n";
        std::cout << "[PROBE] error=" << e.what() << "\n";
        return 2;
    }
}

}  // namespace

int main(int argc, char* argv[]) {
    try {
        if (argc == 2 && std::string(argv[1]) == "--probe") {
            return probe_cuda();
        }
        if (argc != 6) {
            std::cerr << "Usage: " << argv[0]
                      << " <header_hex> <nonce_offset> <start_nonce> <batch_size> <target_hex>\n";
            return 1;
        }
        const std::string header_hex(argv[1]);
        const uint32_t nonce_offset = static_cast<uint32_t>(std::stoul(argv[2]));
        const uint64_t start_nonce = static_cast<uint64_t>(std::stoull(argv[3]));
        const uint64_t batch_size = static_cast<uint64_t>(std::stoull(argv[4]));
        const std::string target_hex(argv[5]);
        run_cuda(header_hex, nonce_offset, start_nonce, batch_size, target_hex);
        return 0;
    } catch (const std::exception& e) {
        std::cerr << "❌ Fatal Error: " << e.what() << "\n";
        return 1;
    }
}
