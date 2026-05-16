// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

#define CL_TARGET_OPENCL_VERSION 120
#if defined(__APPLE__)
#include <OpenCL/opencl.h>
#else
#include <CL/cl.h>
#endif

#include <algorithm>
#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <cctype>
#include <cstdio>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <limits>
#include <memory>
#include <sstream>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>

namespace {

constexpr size_t HEADER_SIZE = 211;
constexpr size_t NONCE_SIZE = 8;
constexpr size_t BLOCK_SIZE = HEADER_SIZE + NONCE_SIZE;
constexpr size_t HASH_SIZE = 48;
constexpr size_t TARGET_SIZE = 48;
constexpr uint64_t DEFAULT_MAX_BATCH = 2'000'000ULL;
constexpr uint64_t DEFAULT_MAX_TEMPLATE_BYTES = 4096ULL;

constexpr const char* GPU_ERR_FEATURE_DISABLED = "ATHO-MINE-101";
constexpr const char* GPU_ERR_NOT_FOUND = "ATHO-MINE-102";
constexpr const char* GPU_ERR_KERNEL_MISSING = "ATHO-MINE-103";
constexpr const char* GPU_ERR_KERNEL_LOAD_FAILED = "ATHO-MINE-104";
constexpr const char* GPU_ERR_KERNEL_BUILD_FAILED = "ATHO-MINE-105";
constexpr const char* GPU_ERR_CONTEXT_CREATE_FAILED = "ATHO-MINE-106";
constexpr const char* GPU_ERR_QUEUE_CREATE_FAILED = "ATHO-MINE-107";
constexpr const char* GPU_ERR_KERNEL_CREATE_FAILED = "ATHO-MINE-108";
constexpr const char* GPU_ERR_BUFFER_ALLOC_FAILED = "ATHO-MINE-109";
constexpr const char* GPU_ERR_INVALID_ARGUMENT = "ATHO-MINE-110";
constexpr const char* GPU_ERR_BATCH_TOO_LARGE = "ATHO-MINE-111";
constexpr const char* GPU_ERR_NONCE_OVERFLOW = "ATHO-MINE-112";
constexpr const char* GPU_ERR_KERNEL_EXECUTION_FAILED = "ATHO-MINE-113";
constexpr const char* GPU_ERR_BUFFER_IO_FAILED = "ATHO-MINE-114";
constexpr const char* GPU_ERR_PROBE_FAILED = "ATHO-MINE-115";
constexpr const char* GPU_ERR_UNKNOWN = "ATHO-MINE-116";

struct DeviceInfo {
    std::string name;
    std::string vendor;
    std::string driver;
    cl_uint compute_units = 0;
    cl_ulong global_mem = 0;
    cl_ulong local_mem = 0;
    cl_uint clock_mhz = 0;
};

struct ProbeInfoResult {
    std::string backend = "opencl";
    std::string device_type = "unknown";
    std::string device_name;
    std::string device_vendor;
    std::string device_driver;
    cl_uint compute_units = 0;
    cl_ulong global_mem_mb = 0;
    cl_ulong local_mem_kb = 0;
    cl_uint clock_mhz = 0;
    std::string kernel_path;
    bool supports_fixed = true;
    bool supports_template = true;
    uint64_t max_batch = DEFAULT_MAX_BATCH;
    uint64_t template_max_bytes = DEFAULT_MAX_TEMPLATE_BYTES;
    bool usable = false;
    std::string reason_code;
    std::string reason;
};

class GpuFailure : public std::runtime_error {
public:
    GpuFailure(std::string code, std::string message)
        : std::runtime_error(std::move(message)), code_(std::move(code)) {}

    const std::string& code() const noexcept {
        return code_;
    }

private:
    std::string code_;
};

struct CLHandles {
    cl_context context = nullptr;
    cl_command_queue queue = nullptr;
    cl_program program = nullptr;
    cl_kernel kernel = nullptr;
    cl_mem header_buf = nullptr;
    cl_mem target_buf = nullptr;
    cl_mem aux_buf = nullptr;
    cl_mem output_buf = nullptr;
    cl_mem flag_buf = nullptr;
    cl_mem nonce_buf = nullptr;
    cl_event kernel_event = nullptr;
};

void release_handles(CLHandles& h);

struct GpuResultCapture {
    bool found = false;
    uint64_t nonce = 0;
    unsigned char hash[HASH_SIZE] = {0};
};

thread_local GpuResultCapture* g_gpu_result_capture = nullptr;

struct GpuResultCaptureScope {
    GpuResultCapture* previous;

    explicit GpuResultCaptureScope(GpuResultCapture* current)
        : previous(g_gpu_result_capture) {
        g_gpu_result_capture = current;
    }

    ~GpuResultCaptureScope() {
        g_gpu_result_capture = previous;
    }
};

struct FixedKernelSession {
    CLHandles handles;
    std::string kernel_path;
    cl_device_id device = nullptr;
    bool profiling_enabled = false;
};

thread_local std::unique_ptr<FixedKernelSession> g_fixed_kernel_session;

DeviceInfo get_device_info(cl_device_id device);

[[noreturn]] void throw_gpu_failure(const char* code, std::string message) {
    throw GpuFailure(code, std::move(message));
}

std::string encode_error(const std::string& code, const std::string& message) {
    return code + "|" + message;
}

void reset_fixed_kernel_session() {
    if (!g_fixed_kernel_session) {
        return;
    }
    release_handles(g_fixed_kernel_session->handles);
    g_fixed_kernel_session.reset();
}

void release_handles(CLHandles& h) {
    if (h.kernel_event) clReleaseEvent(h.kernel_event);
    if (h.nonce_buf) clReleaseMemObject(h.nonce_buf);
    if (h.flag_buf) clReleaseMemObject(h.flag_buf);
    if (h.output_buf) clReleaseMemObject(h.output_buf);
    if (h.aux_buf) clReleaseMemObject(h.aux_buf);
    if (h.target_buf) clReleaseMemObject(h.target_buf);
    if (h.header_buf) clReleaseMemObject(h.header_buf);
    if (h.kernel) clReleaseKernel(h.kernel);
    if (h.program) clReleaseProgram(h.program);
    if (h.queue) clReleaseCommandQueue(h.queue);
    if (h.context) clReleaseContext(h.context);
    h = CLHandles{};
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

bool env_bool(const char* key, bool fallback) {
    const char* raw = std::getenv(key);
    if (!raw || !*raw) return fallback;
    std::string s(raw);
    std::transform(s.begin(), s.end(), s.begin(), [](unsigned char c) {
        return static_cast<char>(std::tolower(c));
    });
    if (s == "1" || s == "true" || s == "yes" || s == "on") return true;
    if (s == "0" || s == "false" || s == "no" || s == "off") return false;
    return fallback;
}

std::string dirname_of(std::string path) {
    if (path.empty()) return ".";
    for (char& c : path) {
        if (c == '\\') c = '/';
    }
    const size_t pos = path.find_last_of('/');
    if (pos == std::string::npos) return ".";
    if (pos == 0) return "/";
    return path.substr(0, pos);
}

std::string load_file(const std::string& file_path) {
    std::ifstream file(file_path, std::ios::binary);
    if (!file.is_open()) {
        throw_gpu_failure(GPU_ERR_KERNEL_LOAD_FAILED, "failed to open file: " + file_path);
    }
    std::string content((std::istreambuf_iterator<char>(file)), std::istreambuf_iterator<char>());
    if (content.empty()) {
        throw_gpu_failure(GPU_ERR_KERNEL_LOAD_FAILED, "file is empty: " + file_path);
    }
    return content;
}

uint64_t fnv1a64_update(uint64_t hash, const unsigned char* data, size_t len) {
    constexpr uint64_t kPrime = 1099511628211ULL;
    for (size_t i = 0; i < len; ++i) {
        hash ^= static_cast<uint64_t>(data[i]);
        hash *= kPrime;
    }
    return hash;
}

uint64_t fnv1a64(const std::string& value, uint64_t seed = 1469598103934665603ULL) {
    return fnv1a64_update(seed, reinterpret_cast<const unsigned char*>(value.data()), value.size());
}

std::string u64_hex(uint64_t value) {
    std::ostringstream ss;
    ss << std::hex << std::setfill('0') << std::setw(16) << value;
    return ss.str();
}

std::string opencl_cache_dir(const std::string& kernel_path) {
    const char* env_dir = std::getenv("ATHO_OPENCL_CACHE_DIR");
    if (env_dir && *env_dir) {
        return std::string(env_dir);
    }
    return dirname_of(kernel_path) + "/.opencl_cache";
}

std::string build_log_for(cl_program program, cl_device_id device) {
    size_t log_size = 0;
    if (clGetProgramBuildInfo(program, device, CL_PROGRAM_BUILD_LOG, 0, nullptr, &log_size) != CL_SUCCESS) {
        return "unknown build error (unable to read OpenCL build log)";
    }
    std::vector<char> log(log_size + 1, '\0');
    clGetProgramBuildInfo(program, device, CL_PROGRAM_BUILD_LOG, log_size, log.data(), nullptr);
    return std::string(log.data());
}

bool read_binary_file(const std::string& path, std::vector<unsigned char>& out) {
    std::ifstream in(path, std::ios::binary);
    if (!in.is_open()) return false;
    in.seekg(0, std::ios::end);
    const std::streamoff size = in.tellg();
    if (size <= 0) return false;
    in.seekg(0, std::ios::beg);
    out.assign(static_cast<size_t>(size), 0);
    in.read(reinterpret_cast<char*>(out.data()), size);
    return static_cast<std::streamoff>(in.gcount()) == size;
}

void write_binary_file_atomic(const std::string& path, const std::vector<unsigned char>& data) {
    if (data.empty()) return;
    std::error_code ec;
    const auto target = std::filesystem::path(path);
    std::filesystem::create_directories(target.parent_path(), ec);
    const std::string tmp = path + ".tmp." +
                            std::to_string(static_cast<unsigned long long>(
                                std::chrono::steady_clock::now().time_since_epoch().count()));
    {
        std::ofstream out(tmp, std::ios::binary | std::ios::trunc);
        if (!out.is_open()) return;
        out.write(reinterpret_cast<const char*>(data.data()), static_cast<std::streamsize>(data.size()));
        if (!out.good()) return;
    }
    std::filesystem::rename(std::filesystem::path(tmp), target, ec);
    if (ec) {
        std::filesystem::remove(std::filesystem::path(tmp), ec);
    }
}

std::string opencl_binary_cache_path(
    cl_device_id device,
    const std::string& kernel_path,
    const std::string& kernel_src,
    const std::string& build_opts
) {
    const DeviceInfo info = get_device_info(device);
    uint64_t hash = 1469598103934665603ULL;
    hash = fnv1a64(kernel_src, hash);
    hash = fnv1a64(build_opts, hash);
    hash = fnv1a64(info.vendor, hash);
    hash = fnv1a64(info.name, hash);
    hash = fnv1a64(info.driver, hash);
    return opencl_cache_dir(kernel_path) + "/sha3_384_" + u64_hex(hash) + ".bin";
}

bool try_load_cached_program(
    CLHandles& h,
    cl_device_id device,
    const std::string& cache_path,
    const std::string& build_opts
) {
    std::vector<unsigned char> binary;
    if (!read_binary_file(cache_path, binary)) return false;
    const unsigned char* ptr = binary.data();
    const size_t size = binary.size();
    cl_int binary_status = CL_SUCCESS;
    cl_int err = CL_SUCCESS;
    h.program = clCreateProgramWithBinary(h.context, 1, &device, &size, &ptr, &binary_status, &err);
    if (!h.program || err != CL_SUCCESS || binary_status != CL_SUCCESS) {
        if (h.program) {
            clReleaseProgram(h.program);
            h.program = nullptr;
        }
        return false;
    }
    err = clBuildProgram(h.program, 1, &device, build_opts.c_str(), nullptr, nullptr);
    if (err != CL_SUCCESS) {
        clReleaseProgram(h.program);
        h.program = nullptr;
        return false;
    }
    return true;
}

void persist_program_binary(cl_program program, const std::string& cache_path) {
    size_t binary_size = 0;
    if (clGetProgramInfo(program, CL_PROGRAM_BINARY_SIZES, sizeof(size_t), &binary_size, nullptr) != CL_SUCCESS) {
        return;
    }
    if (binary_size == 0) return;
    std::vector<unsigned char> binary(binary_size, 0);
    unsigned char* binary_ptr = binary.data();
    if (clGetProgramInfo(program, CL_PROGRAM_BINARIES, sizeof(unsigned char*), &binary_ptr, nullptr) != CL_SUCCESS) {
        return;
    }
    write_binary_file_atomic(cache_path, binary);
}

void build_opencl_program(
    CLHandles& h,
    cl_device_id device,
    const std::string& kernel_src,
    const std::string& kernel_path
) {
    const char* raw_build_opts = std::getenv("ATHO_OPENCL_BUILD_OPTS");
    const std::string build_opts = raw_build_opts ? std::string(raw_build_opts) : std::string("-cl-std=CL1.2");
    const bool disable_cache = env_bool("ATHO_OPENCL_DISABLE_CACHE", false);
    const std::string cache_path = disable_cache
        ? std::string()
        : opencl_binary_cache_path(device, kernel_path, kernel_src, build_opts);
    if (!cache_path.empty() && try_load_cached_program(h, device, cache_path, build_opts)) {
        return;
    }

    cl_int err = CL_SUCCESS;
    const char* src_ptr = kernel_src.c_str();
    h.program = clCreateProgramWithSource(h.context, 1, &src_ptr, nullptr, &err);
    if (!h.program || err != CL_SUCCESS) {
        throw_gpu_failure(GPU_ERR_KERNEL_BUILD_FAILED, "failed to create OpenCL program");
    }
    err = clBuildProgram(h.program, 1, &device, build_opts.c_str(), nullptr, nullptr);
    if (err != CL_SUCCESS) {
        const std::string log = build_log_for(h.program, device);
        throw_gpu_failure(GPU_ERR_KERNEL_BUILD_FAILED, "kernel build failed:\n" + log);
    }
    if (!cache_path.empty()) {
        persist_program_binary(h.program, cache_path);
    }
}

size_t choose_local_work_size(cl_kernel kernel, cl_device_id device, uint64_t batch_size) {
    size_t kernel_wg_max = 0;
    size_t preferred_multiple = 0;
    clGetKernelWorkGroupInfo(kernel, device, CL_KERNEL_WORK_GROUP_SIZE, sizeof(kernel_wg_max), &kernel_wg_max, nullptr);
    clGetKernelWorkGroupInfo(
        kernel,
        device,
        CL_KERNEL_PREFERRED_WORK_GROUP_SIZE_MULTIPLE,
        sizeof(preferred_multiple),
        &preferred_multiple,
        nullptr
    );
    const uint64_t env_local = env_u64("ATHO_OPENCL_LOCAL_WORK_SIZE", 0);
    size_t local_work = env_local > 0 ? static_cast<size_t>(env_local) : (preferred_multiple ? preferred_multiple : 64);
    if (kernel_wg_max > 0) {
        local_work = std::min(local_work, kernel_wg_max);
    }
    if (preferred_multiple > 0 && local_work >= preferred_multiple) {
        local_work = (local_work / preferred_multiple) * preferred_multiple;
    }
    if (local_work == 0) local_work = 1;
    if (batch_size > 0) {
        local_work = std::min(local_work, static_cast<size_t>(batch_size));
        if (local_work == 0) local_work = 1;
    }
    return local_work;
}

std::string resolve_kernel_path(const std::string& argv0) {
    if (const char* explicit_path = std::getenv("ATHO_GPU_KERNEL_PATH")) {
        std::string p(explicit_path);
        std::ifstream f(p);
        if (f.good()) return p;
    }
    std::vector<std::string> candidates = {
        dirname_of(argv0) + "/sha3_384.cl",
        "sha3_384.cl",
    };
    for (const auto& path : candidates) {
        std::ifstream f(path);
        if (f.good()) return path;
    }
    throw_gpu_failure(
        GPU_ERR_KERNEL_MISSING,
        "unable to locate sha3_384.cl (set ATHO_GPU_KERNEL_PATH)"
    );
}

std::vector<unsigned char> hex_to_bytes(const std::string& hex) {
    if (hex.size() % 2 != 0) {
        throw_gpu_failure(GPU_ERR_INVALID_ARGUMENT, "hex string length must be even");
    }
    std::vector<unsigned char> out;
    out.reserve(hex.size() / 2);
    for (size_t i = 0; i < hex.size(); i += 2) {
        const std::string piece = hex.substr(i, 2);
        unsigned int value = 0;
        std::stringstream ss;
        ss << std::hex << piece;
        if (!(ss >> value)) {
            throw_gpu_failure(
                GPU_ERR_INVALID_ARGUMENT,
                "invalid hex at offset " + std::to_string(i)
            );
        }
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
    for (const char c : target_hex) {
        const bool ok = (c >= '0' && c <= '9') ||
                        (c >= 'a' && c <= 'f') ||
                        (c >= 'A' && c <= 'F');
        if (!ok) {
            throw_gpu_failure(
                GPU_ERR_INVALID_ARGUMENT,
                "target contains non-hex characters"
            );
        }
    }
    if (target_hex.size() < TARGET_SIZE * 2) {
        target_hex = std::string(TARGET_SIZE * 2 - target_hex.size(), '0') + target_hex;
    } else if (target_hex.size() > TARGET_SIZE * 2) {
        target_hex = target_hex.substr(target_hex.size() - TARGET_SIZE * 2);
    }
    return target_hex;
}

DeviceInfo get_device_info(cl_device_id device) {
    char name[256] = {0};
    char vendor[256] = {0};
    char version[256] = {0};
    DeviceInfo info;

    clGetDeviceInfo(device, CL_DEVICE_NAME, sizeof(name), name, nullptr);
    clGetDeviceInfo(device, CL_DEVICE_VENDOR, sizeof(vendor), vendor, nullptr);
    clGetDeviceInfo(device, CL_DRIVER_VERSION, sizeof(version), version, nullptr);
    clGetDeviceInfo(device, CL_DEVICE_MAX_COMPUTE_UNITS, sizeof(info.compute_units), &info.compute_units, nullptr);
    clGetDeviceInfo(device, CL_DEVICE_GLOBAL_MEM_SIZE, sizeof(info.global_mem), &info.global_mem, nullptr);
    clGetDeviceInfo(device, CL_DEVICE_LOCAL_MEM_SIZE, sizeof(info.local_mem), &info.local_mem, nullptr);
    clGetDeviceInfo(device, CL_DEVICE_MAX_CLOCK_FREQUENCY, sizeof(info.clock_mhz), &info.clock_mhz, nullptr);
    info.name = name;
    info.vendor = vendor;
    info.driver = version;
    return info;
}

void print_device_info(cl_device_id device) {
    const DeviceInfo info = get_device_info(device);
    std::cout << "Device: " << info.name
              << " | Vendor: " << info.vendor
              << " | Driver: " << info.driver
              << " | CUs: " << info.compute_units
              << " | GlobalMemMB: " << (info.global_mem / 1024 / 1024)
              << " | LocalMemKB: " << (info.local_mem / 1024)
              << " | ClockMHz: " << info.clock_mhz << "\n";
}

bool pick_gpu_device(cl_platform_id& out_platform, cl_device_id& out_device) {
    cl_uint platform_count = 0;
    if (clGetPlatformIDs(0, nullptr, &platform_count) != CL_SUCCESS || platform_count == 0) {
        return false;
    }
    std::vector<cl_platform_id> platforms(platform_count);
    if (clGetPlatformIDs(platform_count, platforms.data(), nullptr) != CL_SUCCESS) {
        return false;
    }

    for (cl_platform_id p : platforms) {
        cl_device_id dev = nullptr;
        if (clGetDeviceIDs(p, CL_DEVICE_TYPE_GPU, 1, &dev, nullptr) == CL_SUCCESS && dev) {
            out_platform = p;
            out_device = dev;
            return true;
        }
    }
    return false;
}

ProbeInfoResult probe_opencl_info(const std::string& kernel_path) {
    ProbeInfoResult info;
    info.kernel_path = kernel_path;
    info.max_batch = env_u64("ATHO_GPU_MAX_BATCH", DEFAULT_MAX_BATCH);
    info.template_max_bytes =
        env_u64("ATHO_GPU_TEMPLATE_MAX_BYTES", DEFAULT_MAX_TEMPLATE_BYTES);

    cl_platform_id platform = nullptr;
    cl_device_id device = nullptr;
    if (!pick_gpu_device(platform, device)) {
        info.reason_code = GPU_ERR_NOT_FOUND;
        info.reason = "no real OpenCL GPU detected";
        return info;
    }

    const DeviceInfo device_info = get_device_info(device);
    info.device_type = "gpu";
    info.device_name = device_info.name;
    info.device_vendor = device_info.vendor;
    info.device_driver = device_info.driver;
    info.compute_units = device_info.compute_units;
    info.global_mem_mb = device_info.global_mem / 1024 / 1024;
    info.local_mem_kb = device_info.local_mem / 1024;
    info.clock_mhz = device_info.clock_mhz;

    if (kernel_path.empty()) {
        info.reason_code = GPU_ERR_KERNEL_MISSING;
        info.reason = "gpu kernel file could not be located";
        return info;
    }
    if (!std::filesystem::exists(std::filesystem::path(kernel_path))) {
        info.reason_code = GPU_ERR_KERNEL_MISSING;
        info.reason = "gpu kernel file not found at " + kernel_path;
        return info;
    }

    info.usable = true;
    return info;
}

std::string format_probe_info(const ProbeInfoResult& info) {
    std::ostringstream out;
    out << "status=" << (info.usable ? "ok" : "unavailable") << "\n";
    out << "backend=" << info.backend << "\n";
    out << "usable=" << (info.usable ? 1 : 0) << "\n";
    out << "device_type=" << info.device_type << "\n";
    if (!info.device_name.empty()) out << "device_name=" << info.device_name << "\n";
    if (!info.device_vendor.empty()) out << "device_vendor=" << info.device_vendor << "\n";
    if (!info.device_driver.empty()) out << "device_driver=" << info.device_driver << "\n";
    if (info.compute_units > 0) out << "compute_units=" << info.compute_units << "\n";
    if (info.global_mem_mb > 0) out << "global_mem_mb=" << info.global_mem_mb << "\n";
    if (info.local_mem_kb > 0) out << "local_mem_kb=" << info.local_mem_kb << "\n";
    if (info.clock_mhz > 0) out << "clock_mhz=" << info.clock_mhz << "\n";
    if (!info.kernel_path.empty()) out << "kernel_path=" << info.kernel_path << "\n";
    out << "supports_fixed=" << (info.supports_fixed ? 1 : 0) << "\n";
    out << "supports_template=" << (info.supports_template ? 1 : 0) << "\n";
    out << "max_batch=" << info.max_batch << "\n";
    out << "template_max_bytes=" << info.template_max_bytes << "\n";
    if (!info.reason_code.empty()) out << "reason_code=" << info.reason_code << "\n";
    if (!info.reason.empty()) out << "reason=" << info.reason << "\n";
    return out.str();
}

FixedKernelSession& prepare_fixed_kernel_session(
    const std::string& kernel_path,
    bool profiling_enabled,
    bool print_device
) {
    if (!g_fixed_kernel_session ||
        g_fixed_kernel_session->kernel_path != kernel_path ||
        g_fixed_kernel_session->profiling_enabled != profiling_enabled) {
        reset_fixed_kernel_session();

        cl_platform_id platform = nullptr;
        cl_device_id device = nullptr;
        if (!pick_gpu_device(platform, device)) {
            throw_gpu_failure(GPU_ERR_NOT_FOUND, "no real OpenCL GPU detected");
        }

        auto session = std::make_unique<FixedKernelSession>();
        session->kernel_path = kernel_path;
        session->device = device;
        session->profiling_enabled = profiling_enabled;

        try {
            cl_int err = CL_SUCCESS;
            session->handles.context = clCreateContext(nullptr, 1, &device, nullptr, nullptr, &err);
            if (!session->handles.context || err != CL_SUCCESS) {
                throw_gpu_failure(GPU_ERR_CONTEXT_CREATE_FAILED, "failed to create OpenCL context");
            }

            const cl_command_queue_properties queue_props =
                profiling_enabled ? CL_QUEUE_PROFILING_ENABLE : 0;
            session->handles.queue =
                clCreateCommandQueue(session->handles.context, device, queue_props, &err);
            if (!session->handles.queue || err != CL_SUCCESS) {
                throw_gpu_failure(
                    GPU_ERR_QUEUE_CREATE_FAILED,
                    "failed to create OpenCL command queue"
                );
            }

            const std::string kernel_src = load_file(kernel_path);
            build_opencl_program(session->handles, device, kernel_src, kernel_path);

            session->handles.kernel =
                clCreateKernel(session->handles.program, "sha3_384_mining", &err);
            if (!session->handles.kernel || err != CL_SUCCESS) {
                throw_gpu_failure(GPU_ERR_KERNEL_CREATE_FAILED, "failed to create kernel");
            }

            session->handles.header_buf = clCreateBuffer(
                session->handles.context,
                CL_MEM_READ_ONLY,
                HEADER_SIZE,
                nullptr,
                &err
            );
            if (!session->handles.header_buf || err != CL_SUCCESS) {
                throw_gpu_failure(
                    GPU_ERR_BUFFER_ALLOC_FAILED,
                    "failed to allocate header buffer"
                );
            }
            session->handles.target_buf = clCreateBuffer(
                session->handles.context,
                CL_MEM_READ_ONLY,
                TARGET_SIZE,
                nullptr,
                &err
            );
            if (!session->handles.target_buf || err != CL_SUCCESS) {
                throw_gpu_failure(
                    GPU_ERR_BUFFER_ALLOC_FAILED,
                    "failed to allocate target buffer"
                );
            }
            session->handles.flag_buf = clCreateBuffer(
                session->handles.context,
                CL_MEM_READ_WRITE,
                sizeof(cl_int),
                nullptr,
                &err
            );
            if (!session->handles.flag_buf || err != CL_SUCCESS) {
                throw_gpu_failure(
                    GPU_ERR_BUFFER_ALLOC_FAILED,
                    "failed to allocate found-flag buffer"
                );
            }
            session->handles.nonce_buf = clCreateBuffer(
                session->handles.context,
                CL_MEM_READ_WRITE,
                sizeof(cl_ulong),
                nullptr,
                &err
            );
            if (!session->handles.nonce_buf || err != CL_SUCCESS) {
                throw_gpu_failure(
                    GPU_ERR_BUFFER_ALLOC_FAILED,
                    "failed to allocate winner-nonce buffer"
                );
            }
            session->handles.output_buf = clCreateBuffer(
                session->handles.context,
                CL_MEM_WRITE_ONLY,
                HASH_SIZE,
                nullptr,
                &err
            );
            if (!session->handles.output_buf || err != CL_SUCCESS) {
                throw_gpu_failure(
                    GPU_ERR_BUFFER_ALLOC_FAILED,
                    "failed to allocate winner-hash buffer"
                );
            }
        } catch (...) {
            release_handles(session->handles);
            throw;
        }

        g_fixed_kernel_session = std::move(session);
    }

    if (print_device) {
        print_device_info(g_fixed_kernel_session->device);
    }
    return *g_fixed_kernel_session;
}

void mine_gpu_bytes(
    const std::vector<unsigned char>& header,
    uint32_t nonce_offset,
    uint64_t start_nonce,
    uint64_t batch_size,
    const std::vector<unsigned char>& target,
    const std::string& kernel_path
) {
    const bool capture_output = g_gpu_result_capture != nullptr;
    const bool machine_output = capture_output || env_bool("ATHO_GPU_MACHINE_OUTPUT", false);
    const bool quiet = capture_output || env_bool("ATHO_GPU_QUIET", machine_output);
    const bool print_device = env_bool("ATHO_GPU_PRINT_DEVICE", !quiet && !machine_output);
    const bool print_perf = env_bool("ATHO_GPU_PRINT_PERF", !machine_output);
    const uint64_t sample_cfg = env_u64(
        "ATHO_GPU_RESULT_SAMPLE_COUNT",
        (quiet || machine_output) ? 0ULL : 3ULL
    );

    if (header.size() != HEADER_SIZE) {
        throw_gpu_failure(GPU_ERR_INVALID_ARGUMENT, "header must be exactly 211 bytes");
    }
    if (nonce_offset != HEADER_SIZE) {
        throw_gpu_failure(
            GPU_ERR_INVALID_ARGUMENT,
            "nonce_offset must be 211 for this kernel"
        );
    }
    if (target.size() != TARGET_SIZE) {
        throw_gpu_failure(GPU_ERR_INVALID_ARGUMENT, "target must be exactly 48 bytes");
    }
    const uint64_t max_batch = env_u64("ATHO_GPU_MAX_BATCH", DEFAULT_MAX_BATCH);
    if (batch_size == 0 || batch_size > max_batch) {
        throw_gpu_failure(
            GPU_ERR_BATCH_TOO_LARGE,
            "batch_size out of allowed range (1.." + std::to_string(max_batch) + ")"
        );
    }
    if (start_nonce > (std::numeric_limits<uint64_t>::max() - (batch_size - 1))) {
        throw_gpu_failure(GPU_ERR_NONCE_OVERFLOW, "nonce range overflows uint64");
    }

    FixedKernelSession& session =
        prepare_fixed_kernel_session(kernel_path, print_perf, print_device);
    CLHandles& h = session.handles;

    try {
        cl_int err = clEnqueueWriteBuffer(
            h.queue,
            h.header_buf,
            CL_TRUE,
            0,
            HEADER_SIZE,
            header.data(),
            0,
            nullptr,
            nullptr
        );
        if (err != CL_SUCCESS) {
            reset_fixed_kernel_session();
            throw_gpu_failure(GPU_ERR_BUFFER_IO_FAILED, "failed to upload header buffer");
        }
        err = clEnqueueWriteBuffer(
            h.queue,
            h.target_buf,
            CL_TRUE,
            0,
            TARGET_SIZE,
            target.data(),
            0,
            nullptr,
            nullptr
        );
        if (err != CL_SUCCESS) {
            reset_fixed_kernel_session();
            throw_gpu_failure(GPU_ERR_BUFFER_IO_FAILED, "failed to upload target buffer");
        }
        cl_int found_init = 0;
        err = clEnqueueWriteBuffer(
            h.queue,
            h.flag_buf,
            CL_TRUE,
            0,
            sizeof(cl_int),
            &found_init,
            0,
            nullptr,
            nullptr
        );
        if (err != CL_SUCCESS) {
            reset_fixed_kernel_session();
            throw_gpu_failure(GPU_ERR_BUFFER_IO_FAILED, "failed to reset found-flag buffer");
        }
        cl_ulong nonce_init = 0;
        err = clEnqueueWriteBuffer(
            h.queue,
            h.nonce_buf,
            CL_TRUE,
            0,
            sizeof(cl_ulong),
            &nonce_init,
            0,
            nullptr,
            nullptr
        );
        if (err != CL_SUCCESS) {
            reset_fixed_kernel_session();
            throw_gpu_failure(
                GPU_ERR_BUFFER_IO_FAILED,
                "failed to reset winner-nonce buffer"
            );
        }
        const unsigned char zero_hash[HASH_SIZE] = {0};
        err = clEnqueueWriteBuffer(
            h.queue,
            h.output_buf,
            CL_TRUE,
            0,
            HASH_SIZE,
            zero_hash,
            0,
            nullptr,
            nullptr
        );
        if (err != CL_SUCCESS) {
            reset_fixed_kernel_session();
            throw_gpu_failure(
                GPU_ERR_BUFFER_IO_FAILED,
                "failed to reset winner-hash buffer"
            );
        }

        err  = clSetKernelArg(h.kernel, 0, sizeof(cl_mem), &h.header_buf);
        err |= clSetKernelArg(h.kernel, 1, sizeof(cl_mem), &h.target_buf);
        err |= clSetKernelArg(h.kernel, 2, sizeof(cl_mem), &h.flag_buf);
        err |= clSetKernelArg(h.kernel, 3, sizeof(cl_mem), &h.nonce_buf);
        err |= clSetKernelArg(h.kernel, 4, sizeof(cl_mem), &h.output_buf);
        err |= clSetKernelArg(h.kernel, 5, sizeof(cl_uint), &nonce_offset);
        err |= clSetKernelArg(h.kernel, 6, sizeof(cl_ulong), &start_nonce);
        err |= clSetKernelArg(h.kernel, 7, sizeof(cl_ulong), &batch_size);
        if (err != CL_SUCCESS) {
            reset_fixed_kernel_session();
            throw_gpu_failure(GPU_ERR_INVALID_ARGUMENT, "failed to set kernel arguments");
        }

        const size_t local_work = choose_local_work_size(h.kernel, session.device, batch_size);
        size_t global_work = static_cast<size_t>(batch_size);
        if (global_work % local_work != 0) {
            global_work = ((global_work + local_work - 1) / local_work) * local_work;
        }

        if (h.kernel_event) {
            clReleaseEvent(h.kernel_event);
            h.kernel_event = nullptr;
        }
        err = clEnqueueNDRangeKernel(h.queue, h.kernel, 1, nullptr, &global_work, &local_work, 0, nullptr, &h.kernel_event);
        if (err != CL_SUCCESS) {
            reset_fixed_kernel_session();
            throw_gpu_failure(
                GPU_ERR_KERNEL_EXECUTION_FAILED,
                "kernel execution failed with error " + std::to_string(err)
            );
        }
        clWaitForEvents(1, &h.kernel_event);

        double elapsed_ms = 0.0;
        double throughput = 0.0;
        if (print_perf) {
            cl_ulong t0 = 0, t1 = 0;
            clGetEventProfilingInfo(h.kernel_event, CL_PROFILING_COMMAND_START, sizeof(t0), &t0, nullptr);
            clGetEventProfilingInfo(h.kernel_event, CL_PROFILING_COMMAND_END, sizeof(t1), &t1, nullptr);
            elapsed_ms = (t1 - t0) * 1e-6;
            throughput = elapsed_ms > 0.0 ? (static_cast<double>(batch_size) * 1000.0 / elapsed_ms) : 0.0;
        }

        cl_int found_flag = 0;
        err = clEnqueueReadBuffer(
            h.queue,
            h.flag_buf,
            CL_TRUE,
            0,
            sizeof(cl_int),
            &found_flag,
            0,
            nullptr,
            nullptr
        );
        if (err != CL_SUCCESS) {
            reset_fixed_kernel_session();
            throw_gpu_failure(GPU_ERR_BUFFER_IO_FAILED, "failed to read found-flag buffer");
        }

        uint64_t win_nonce = 0;
        unsigned char win_hash_bytes[HASH_SIZE] = {0};
        std::string win_hash;
        const bool found = (found_flag != 0);
        if (found) {
            cl_ulong winner_nonce_raw = 0;
            err = clEnqueueReadBuffer(
                h.queue,
                h.nonce_buf,
                CL_TRUE,
                0,
                sizeof(cl_ulong),
                &winner_nonce_raw,
                0,
                nullptr,
                nullptr
            );
            if (err != CL_SUCCESS) {
                reset_fixed_kernel_session();
                throw_gpu_failure(GPU_ERR_BUFFER_IO_FAILED, "failed to read winner nonce buffer");
            }
            err = clEnqueueReadBuffer(
                h.queue,
                h.output_buf,
                CL_TRUE,
                0,
                HASH_SIZE,
                win_hash_bytes,
                0,
                nullptr,
                nullptr
            );
            if (err != CL_SUCCESS) {
                reset_fixed_kernel_session();
                throw_gpu_failure(GPU_ERR_BUFFER_IO_FAILED, "failed to read winner hash buffer");
            }
            win_nonce = static_cast<uint64_t>(winner_nonce_raw);
            win_hash = bytes_to_hex(win_hash_bytes, HASH_SIZE);
        }

        if (print_perf) {
            std::cout << std::fixed << std::setprecision(3);
            std::cout << "[PERF] Kernel execution time: " << elapsed_ms << " ms\n";
            std::cout << "[PERF] Throughput: " << throughput << " H/s\n";
        }

        if (machine_output) {
            if (found) {
                if (g_gpu_result_capture) {
                    g_gpu_result_capture->found = true;
                    g_gpu_result_capture->nonce = win_nonce;
                    std::memcpy(g_gpu_result_capture->hash, win_hash_bytes, HASH_SIZE);
                } else {
                    std::cout << "FOUND nonce=" << win_nonce << " hash=" << win_hash << "\n";
                }
            } else if (!g_gpu_result_capture) {
                std::cout << "NO_SOLUTION\n";
            }
        } else if (found) {
            std::cout << "🎉 SOLUTION FOUND!\n";
            std::cout << "Nonce: " << win_nonce << "\n";
            std::cout << "Hash: " << win_hash << "\n";
            std::cout << "Kernel Time: " << elapsed_ms << " ms\n";
        } else {
            std::cout << "[RESULT] No valid nonce found in batch\n";
        }
        if (sample_cfg > 0 && !machine_output) {
            std::cout << "[RESULT] Winner-only fixed-header kernel active\n";
        }
    } catch (const GpuFailure&) {
        throw;
    } catch (const std::exception& e) {
        reset_fixed_kernel_session();
        throw_gpu_failure(GPU_ERR_UNKNOWN, e.what());
    }
}

void mine_gpu(
    const std::string& header_hex,
    uint32_t nonce_offset,
    uint64_t start_nonce,
    uint64_t batch_size,
    const std::string& target_hex,
    const std::string& kernel_path
) {
    if (header_hex.size() != HEADER_SIZE * 2) {
        throw_gpu_failure(GPU_ERR_INVALID_ARGUMENT, "header must be exactly 422 hex chars");
    }
    const std::vector<unsigned char> header = hex_to_bytes(header_hex);
    const std::string normalized_target_hex = normalize_target_hex(target_hex);
    const std::vector<unsigned char> target = hex_to_bytes(normalized_target_hex);
    mine_gpu_bytes(header, nonce_offset, start_nonce, batch_size, target, kernel_path);
}

void mine_gpu_template(
    const std::string& prefix_hex,
    const std::string& suffix_hex,
    uint64_t start_nonce,
    uint64_t batch_size,
    const std::string& target_hex,
    const std::string& kernel_path
) {
    const bool machine_output = env_bool("ATHO_GPU_MACHINE_OUTPUT", false);
    const bool quiet = env_bool("ATHO_GPU_QUIET", machine_output);
    const bool print_device = env_bool("ATHO_GPU_PRINT_DEVICE", !quiet && !machine_output);
    const bool print_perf = env_bool("ATHO_GPU_PRINT_PERF", !machine_output);

    const uint64_t max_batch = env_u64("ATHO_GPU_MAX_BATCH", DEFAULT_MAX_BATCH);
    if (batch_size == 0 || batch_size > max_batch) {
        throw_gpu_failure(
            GPU_ERR_BATCH_TOO_LARGE,
            "batch_size out of allowed range (1.." + std::to_string(max_batch) + ")"
        );
    }
    if (start_nonce > (std::numeric_limits<uint64_t>::max() - (batch_size - 1))) {
        throw_gpu_failure(GPU_ERR_NONCE_OVERFLOW, "nonce range overflows uint64");
    }
    if (batch_size > (std::numeric_limits<size_t>::max() / HASH_SIZE)) {
        throw_gpu_failure(
            GPU_ERR_BATCH_TOO_LARGE,
            "output buffer would overflow address space"
        );
    }

    const std::vector<unsigned char> prefix = hex_to_bytes(prefix_hex);
    const std::vector<unsigned char> suffix = hex_to_bytes(suffix_hex);
    const uint64_t template_min_len = static_cast<uint64_t>(prefix.size()) + static_cast<uint64_t>(suffix.size()) + 1ULL;
    const uint64_t template_max = env_u64("ATHO_GPU_TEMPLATE_MAX_BYTES", DEFAULT_MAX_TEMPLATE_BYTES);
    if (template_min_len > template_max) {
        throw_gpu_failure(
            GPU_ERR_INVALID_ARGUMENT,
            "template length exceeds ATHO_GPU_TEMPLATE_MAX_BYTES (" + std::to_string(template_min_len) +
            " > " + std::to_string(template_max) + ")"
        );
    }
    if (prefix.size() > std::numeric_limits<cl_uint>::max() || suffix.size() > std::numeric_limits<cl_uint>::max()) {
        throw_gpu_failure(
            GPU_ERR_INVALID_ARGUMENT,
            "template prefix/suffix too large for OpenCL kernel argument width"
        );
    }

    const std::string normalized_target_hex = normalize_target_hex(target_hex);
    const std::vector<unsigned char> target = hex_to_bytes(normalized_target_hex);

    cl_platform_id platform = nullptr;
    cl_device_id device = nullptr;
    if (!pick_gpu_device(platform, device)) {
        throw_gpu_failure(GPU_ERR_NOT_FOUND, "no OpenCL GPU found");
    }
    if (print_device) {
        print_device_info(device);
    }

    CLHandles h;
    cl_int err = CL_SUCCESS;
    try {
        h.context = clCreateContext(nullptr, 1, &device, nullptr, nullptr, &err);
        if (!h.context || err != CL_SUCCESS) {
            throw_gpu_failure(GPU_ERR_CONTEXT_CREATE_FAILED, "failed to create OpenCL context");
        }
        const cl_command_queue_properties queue_props = print_perf ? CL_QUEUE_PROFILING_ENABLE : 0;
        h.queue = clCreateCommandQueue(h.context, device, queue_props, &err);
        if (!h.queue || err != CL_SUCCESS) {
            throw_gpu_failure(GPU_ERR_QUEUE_CREATE_FAILED, "failed to create OpenCL command queue");
        }

        const std::string kernel_src = load_file(kernel_path);
        build_opencl_program(h, device, kernel_src, kernel_path);

        h.kernel = clCreateKernel(h.program, "sha3_384_mining_template", &err);
        if (!h.kernel || err != CL_SUCCESS) {
            throw_gpu_failure(GPU_ERR_KERNEL_CREATE_FAILED, "failed to create template kernel");
        }

        const std::vector<unsigned char> prefix_alloc = prefix.empty() ? std::vector<unsigned char>{0} : prefix;
        const std::vector<unsigned char> suffix_alloc = suffix.empty() ? std::vector<unsigned char>{0} : suffix;
        const cl_uint prefix_len = static_cast<cl_uint>(prefix.size());
        const cl_uint suffix_len = static_cast<cl_uint>(suffix.size());

        h.header_buf = clCreateBuffer(
            h.context, CL_MEM_READ_ONLY | CL_MEM_COPY_HOST_PTR,
            prefix_alloc.size(), const_cast<unsigned char*>(prefix_alloc.data()), &err
        );
        if (!h.header_buf || err != CL_SUCCESS) {
            throw_gpu_failure(GPU_ERR_BUFFER_ALLOC_FAILED, "failed to allocate prefix buffer");
        }
        h.target_buf = clCreateBuffer(
            h.context, CL_MEM_READ_ONLY | CL_MEM_COPY_HOST_PTR,
            suffix_alloc.size(), const_cast<unsigned char*>(suffix_alloc.data()), &err
        );
        if (!h.target_buf || err != CL_SUCCESS) {
            throw_gpu_failure(GPU_ERR_BUFFER_ALLOC_FAILED, "failed to allocate suffix buffer");
        }
        h.aux_buf = clCreateBuffer(
            h.context, CL_MEM_READ_ONLY | CL_MEM_COPY_HOST_PTR,
            TARGET_SIZE, const_cast<unsigned char*>(target.data()), &err
        );
        if (!h.aux_buf || err != CL_SUCCESS) {
            throw_gpu_failure(GPU_ERR_BUFFER_ALLOC_FAILED, "failed to allocate target buffer");
        }

        h.output_buf = clCreateBuffer(h.context, CL_MEM_WRITE_ONLY, HASH_SIZE, nullptr, &err);
        if (!h.output_buf || err != CL_SUCCESS) {
            throw_gpu_failure(
                GPU_ERR_BUFFER_ALLOC_FAILED,
                "failed to allocate winner-hash buffer"
            );
        }
        cl_int found_init = 0;
        h.flag_buf = clCreateBuffer(
            h.context, CL_MEM_READ_WRITE | CL_MEM_COPY_HOST_PTR, sizeof(cl_int), &found_init, &err
        );
        if (!h.flag_buf || err != CL_SUCCESS) {
            throw_gpu_failure(
                GPU_ERR_BUFFER_ALLOC_FAILED,
                "failed to allocate found-flag buffer"
            );
        }
        cl_ulong nonce_init = 0;
        h.nonce_buf = clCreateBuffer(
            h.context, CL_MEM_READ_WRITE | CL_MEM_COPY_HOST_PTR, sizeof(cl_ulong), &nonce_init, &err
        );
        if (!h.nonce_buf || err != CL_SUCCESS) {
            throw_gpu_failure(
                GPU_ERR_BUFFER_ALLOC_FAILED,
                "failed to allocate winner-nonce buffer"
            );
        }

        err  = clSetKernelArg(h.kernel, 0, sizeof(cl_mem), &h.header_buf);
        err |= clSetKernelArg(h.kernel, 1, sizeof(cl_uint), &prefix_len);
        err |= clSetKernelArg(h.kernel, 2, sizeof(cl_mem), &h.target_buf);
        err |= clSetKernelArg(h.kernel, 3, sizeof(cl_uint), &suffix_len);
        err |= clSetKernelArg(h.kernel, 4, sizeof(cl_mem), &h.aux_buf);
        err |= clSetKernelArg(h.kernel, 5, sizeof(cl_mem), &h.flag_buf);
        err |= clSetKernelArg(h.kernel, 6, sizeof(cl_mem), &h.nonce_buf);
        err |= clSetKernelArg(h.kernel, 7, sizeof(cl_mem), &h.output_buf);
        err |= clSetKernelArg(h.kernel, 8, sizeof(cl_ulong), &start_nonce);
        err |= clSetKernelArg(h.kernel, 9, sizeof(cl_ulong), &batch_size);
        if (err != CL_SUCCESS) {
            throw_gpu_failure(GPU_ERR_INVALID_ARGUMENT, "failed to set template kernel arguments");
        }

        const size_t local_work = choose_local_work_size(h.kernel, device, batch_size);
        size_t global_work = static_cast<size_t>(batch_size);
        if (global_work % local_work != 0) {
            global_work = ((global_work + local_work - 1) / local_work) * local_work;
        }

        err = clEnqueueNDRangeKernel(h.queue, h.kernel, 1, nullptr, &global_work, &local_work, 0, nullptr, &h.kernel_event);
        if (err != CL_SUCCESS) {
            throw_gpu_failure(
                GPU_ERR_KERNEL_EXECUTION_FAILED,
                "template kernel execution failed with error " + std::to_string(err)
            );
        }
        clWaitForEvents(1, &h.kernel_event);

        double elapsed_ms = 0.0;
        double throughput = 0.0;
        if (print_perf) {
            cl_ulong t0 = 0, t1 = 0;
            clGetEventProfilingInfo(h.kernel_event, CL_PROFILING_COMMAND_START, sizeof(t0), &t0, nullptr);
            clGetEventProfilingInfo(h.kernel_event, CL_PROFILING_COMMAND_END, sizeof(t1), &t1, nullptr);
            elapsed_ms = (t1 - t0) * 1e-6;
            throughput = elapsed_ms > 0.0 ? (static_cast<double>(batch_size) * 1000.0 / elapsed_ms) : 0.0;
        }

        cl_int found_flag = 0;
        err = clEnqueueReadBuffer(h.queue, h.flag_buf, CL_TRUE, 0, sizeof(cl_int), &found_flag, 0, nullptr, nullptr);
        if (err != CL_SUCCESS) {
            throw_gpu_failure(GPU_ERR_BUFFER_IO_FAILED, "failed to read found-flag buffer");
        }
        bool found = (found_flag != 0);
        uint64_t win_nonce = 0;
        std::string win_hash;
        if (found) {
            cl_ulong win_nonce_raw = 0;
            std::vector<unsigned char> winner_hash(HASH_SIZE, 0);
            err = clEnqueueReadBuffer(h.queue, h.nonce_buf, CL_TRUE, 0, sizeof(cl_ulong), &win_nonce_raw, 0, nullptr, nullptr);
            if (err != CL_SUCCESS) {
                throw_gpu_failure(GPU_ERR_BUFFER_IO_FAILED, "failed to read winner nonce buffer");
            }
            err = clEnqueueReadBuffer(h.queue, h.output_buf, CL_TRUE, 0, HASH_SIZE, winner_hash.data(), 0, nullptr, nullptr);
            if (err != CL_SUCCESS) {
                throw_gpu_failure(GPU_ERR_BUFFER_IO_FAILED, "failed to read winner hash buffer");
            }
            win_nonce = static_cast<uint64_t>(win_nonce_raw);
            win_hash = bytes_to_hex(winner_hash.data(), HASH_SIZE);
        }

        if (print_perf) {
            std::cout << std::fixed << std::setprecision(3);
            std::cout << "[PERF] Kernel execution time: " << elapsed_ms << " ms\n";
            std::cout << "[PERF] Throughput: " << throughput << " H/s\n";
        }

        if (machine_output) {
            if (found) {
                std::cout << "FOUND nonce=" << win_nonce << " hash=" << win_hash << "\n";
            } else {
                std::cout << "NO_SOLUTION\n";
            }
        } else if (found) {
            std::cout << "🎉 SOLUTION FOUND!\n";
            std::cout << "Nonce: " << win_nonce << "\n";
            std::cout << "Hash: " << win_hash << "\n";
            if (print_perf) {
                std::cout << "Kernel Time: " << elapsed_ms << " ms\n";
            }
        } else {
            std::cout << "[RESULT] No valid nonce found in batch\n";
        }
    } catch (...) {
        release_handles(h);
        throw;
    }

    release_handles(h);
}

int probe_opencl(const std::string& argv0) {
    try {
        std::string kernel_path;
        try {
            kernel_path = resolve_kernel_path(argv0);
        } catch (...) {
            kernel_path.clear();
        }
        const ProbeInfoResult info = probe_opencl_info(kernel_path);
        std::istringstream lines(format_probe_info(info));
        std::string line;
        while (std::getline(lines, line)) {
            if (!line.empty()) {
                std::cout << "[PROBE] " << line << "\n";
            }
        }
        return info.usable ? 0 : 2;
    } catch (const GpuFailure& e) {
        std::cout << "[PROBE] status=error\n";
        std::cout << "[PROBE] backend=opencl\n";
        std::cout << "[PROBE] error_code=" << e.code() << "\n";
        std::cout << "[PROBE] error=" << e.what() << "\n";
        return 2;
    } catch (const std::exception& e) {
        std::cout << "[PROBE] status=error\n";
        std::cout << "[PROBE] backend=opencl\n";
        std::cout << "[PROBE] error_code=" << GPU_ERR_PROBE_FAILED << "\n";
        std::cout << "[PROBE] error=" << e.what() << "\n";
        return 2;
    }
}

}  // namespace

#ifndef ATHO_GPU_BUILD_LIBRARY

int main(int argc, char* argv[]) {
    try {
        const bool machine_output = env_bool("ATHO_GPU_MACHINE_OUTPUT", false);
        const bool quiet = env_bool("ATHO_GPU_QUIET", machine_output);
        if (argc == 2 && std::string(argv[1]) == "--probe") {
            return probe_opencl(argv[0]);
        }
        if (argc == 7 && std::string(argv[1]) == "--template") {
            const std::string prefix_hex(argv[2]);
            const std::string suffix_hex(argv[3]);
            const uint64_t start_nonce = static_cast<uint64_t>(std::stoull(argv[4]));
            const uint64_t batch_size = static_cast<uint64_t>(std::stoull(argv[5]));
            const std::string target_hex(argv[6]);
            const std::string kernel_path = resolve_kernel_path(argv[0]);
            if (!quiet) {
                std::cout << "GPU backend: OpenCL\n";
                std::cout << "Kernel path: " << kernel_path << "\n";
                std::cout << "Mode: template\n";
            }
            mine_gpu_template(prefix_hex, suffix_hex, start_nonce, batch_size, target_hex, kernel_path);
            return 0;
        }
        if (argc != 6) {
            std::cerr << "Usage: " << argv[0]
                      << " <header_hex> <nonce_offset> <start_nonce> <batch_size> <target_hex>\n";
            std::cerr << "   or: " << argv[0]
                      << " --template <prefix_hex> <suffix_hex> <start_nonce> <batch_size> <target_hex>\n";
            return 1;
        }

        const std::string header_hex(argv[1]);
        const uint32_t nonce_offset = static_cast<uint32_t>(std::stoul(argv[2]));
        const uint64_t start_nonce = static_cast<uint64_t>(std::stoull(argv[3]));
        const uint64_t batch_size = static_cast<uint64_t>(std::stoull(argv[4]));
        const std::string target_hex(argv[5]);
        const std::string kernel_path = resolve_kernel_path(argv[0]);

        if (!quiet) {
            std::cout << "GPU backend: OpenCL\n";
            std::cout << "Kernel path: " << kernel_path << "\n";
        }
        mine_gpu(header_hex, nonce_offset, start_nonce, batch_size, target_hex, kernel_path);
        return 0;
    } catch (const GpuFailure& e) {
        std::cerr << "❌ Fatal Error [" << e.code() << "]: " << e.what() << "\n";
        return 1;
    } catch (const std::exception& e) {
        std::cerr << "❌ Fatal Error: " << e.what() << "\n";
        return 1;
    }
}

#else

extern "C" int atho_gpu_probe() noexcept {
    try {
        cl_platform_id platform = nullptr;
        cl_device_id device = nullptr;
        return pick_gpu_device(platform, device) ? 1 : 0;
    } catch (...) {
        return 0;
    }
}

extern "C" int atho_gpu_probe_info(
    const char* kernel_path,
    char* out_buf,
    size_t out_buf_len,
    char* error_buf,
    size_t error_buf_len
) noexcept {
    auto write_error = [&](const std::string& code, const std::string& message) -> int {
        if (error_buf && error_buf_len > 0) {
            const std::string encoded = encode_error(code, message);
            std::snprintf(error_buf, error_buf_len, "%s", encoded.c_str());
        }
        return -1;
    };

    if (!out_buf || out_buf_len == 0) {
        return write_error(GPU_ERR_INVALID_ARGUMENT, "probe output buffer is missing");
    }

    try {
        const std::string resolved_kernel_path = kernel_path ? std::string(kernel_path) : std::string();
        const std::string output = format_probe_info(probe_opencl_info(resolved_kernel_path));
        std::snprintf(out_buf, out_buf_len, "%s", output.c_str());
        return 0;
    } catch (const GpuFailure& e) {
        return write_error(e.code(), e.what());
    } catch (const std::exception& e) {
        return write_error(GPU_ERR_PROBE_FAILED, e.what());
    } catch (...) {
        return write_error(GPU_ERR_PROBE_FAILED, "unknown gpu probe error");
    }
}

extern "C" int atho_gpu_mine_batch(
    const unsigned char* header_bytes,
    size_t header_len,
    uint32_t nonce_offset,
    uint64_t start_nonce,
    uint64_t batch_size,
    const unsigned char* target_bytes,
    size_t target_len,
    const char* kernel_path,
    uint64_t* out_nonce,
    unsigned char* out_hash,
    size_t out_hash_len,
    char* error_buf,
    size_t error_buf_len
) noexcept {
    auto write_error = [&](const std::string& code, const std::string& message) -> int {
        if (error_buf && error_buf_len > 0) {
            const std::string encoded = encode_error(code, message);
            std::snprintf(error_buf, error_buf_len, "%s", encoded.c_str());
        }
        return -1;
    };

    if (!header_bytes || !target_bytes || !kernel_path || !out_nonce || !out_hash) {
        return write_error(GPU_ERR_INVALID_ARGUMENT, "invalid null pointer in gpu ffi call");
    }
    if (header_len != HEADER_SIZE) {
        return write_error(GPU_ERR_INVALID_ARGUMENT, "header must be exactly 211 bytes");
    }
    if (target_len != TARGET_SIZE) {
        return write_error(GPU_ERR_INVALID_ARGUMENT, "target must be exactly 48 bytes");
    }
    if (out_hash_len < HASH_SIZE) {
        return write_error(GPU_ERR_INVALID_ARGUMENT, "output hash buffer is too small");
    }

    try {
        std::vector<unsigned char> header(header_bytes, header_bytes + header_len);
        std::vector<unsigned char> target(target_bytes, target_bytes + target_len);
        GpuResultCapture capture;
        GpuResultCaptureScope scope(&capture);
        mine_gpu_bytes(header, nonce_offset, start_nonce, batch_size, target, std::string(kernel_path));
        if (!capture.found) {
            return 1;
        }
        *out_nonce = capture.nonce;
        std::memcpy(out_hash, capture.hash, HASH_SIZE);
        return 0;
    } catch (const GpuFailure& e) {
        return write_error(e.code(), e.what());
    } catch (const std::exception& e) {
        return write_error(GPU_ERR_UNKNOWN, e.what());
    } catch (...) {
        return write_error(GPU_ERR_UNKNOWN, "unknown gpu ffi error");
    }
}

#endif
