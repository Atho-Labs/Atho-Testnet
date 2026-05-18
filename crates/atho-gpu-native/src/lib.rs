// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Native GPU mining bridge types and error mapping helpers.

use atho_errors::{
    MINE_GPU_BATCH_TOO_LARGE, MINE_GPU_BUFFER_ALLOC_FAILED, MINE_GPU_BUFFER_IO_FAILED,
    MINE_GPU_CONTEXT_CREATE_FAILED, MINE_GPU_FEATURE_DISABLED, MINE_GPU_INVALID_ARGUMENT,
    MINE_GPU_KERNEL_BUILD_FAILED, MINE_GPU_KERNEL_CREATE_FAILED, MINE_GPU_KERNEL_EXEC_FAILED,
    MINE_GPU_KERNEL_LOAD_FAILED, MINE_GPU_KERNEL_MISSING, MINE_GPU_NONCE_OVERFLOW,
    MINE_GPU_NOT_FOUND, MINE_GPU_PROBE_FAILED, MINE_GPU_QUEUE_CREATE_FAILED, MINE_GPU_UNKNOWN,
};
use std::path::{Path, PathBuf};

#[cfg(feature = "gpu-native")]
use std::ffi::{CStr, CString};

/// Canonical serialized header length expected by the native mining bridge.
pub const HEADER_BYTES: usize = 211;
/// Target byte width used by Atho proof-of-work comparisons.
pub const TARGET_BYTES: usize = 48;
/// Hash byte width returned by the native GPU backend.
pub const HASH_BYTES: usize = 48;

/// Normalized GPU backend error codes exposed to Rust callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuErrorCode {
    FeatureDisabled,
    NotFound,
    KernelMissing,
    KernelLoadFailed,
    KernelBuildFailed,
    ContextCreateFailed,
    QueueCreateFailed,
    KernelCreateFailed,
    BufferAllocFailed,
    InvalidArgument,
    BatchTooLarge,
    NonceOverflow,
    KernelExecutionFailed,
    BufferIoFailed,
    ProbeFailed,
    Unknown,
}

impl GpuErrorCode {
    #[cfg(any(test, feature = "gpu-native"))]
    fn parse(value: &str) -> Self {
        match value.trim() {
            value
                if value == MINE_GPU_FEATURE_DISABLED.code.as_str()
                    || value == "GPU_FEATURE_DISABLED" =>
            {
                Self::FeatureDisabled
            }
            value if value == MINE_GPU_NOT_FOUND.code.as_str() || value == "GPU_NOT_FOUND" => {
                Self::NotFound
            }
            value
                if value == MINE_GPU_KERNEL_MISSING.code.as_str()
                    || value == "GPU_KERNEL_MISSING" =>
            {
                Self::KernelMissing
            }
            value
                if value == MINE_GPU_KERNEL_LOAD_FAILED.code.as_str()
                    || value == "GPU_KERNEL_LOAD_FAILED" =>
            {
                Self::KernelLoadFailed
            }
            value
                if value == MINE_GPU_KERNEL_BUILD_FAILED.code.as_str()
                    || value == "GPU_KERNEL_BUILD_FAILED" =>
            {
                Self::KernelBuildFailed
            }
            value
                if value == MINE_GPU_CONTEXT_CREATE_FAILED.code.as_str()
                    || value == "GPU_CONTEXT_CREATE_FAILED" =>
            {
                Self::ContextCreateFailed
            }
            value
                if value == MINE_GPU_QUEUE_CREATE_FAILED.code.as_str()
                    || value == "GPU_QUEUE_CREATE_FAILED" =>
            {
                Self::QueueCreateFailed
            }
            value
                if value == MINE_GPU_KERNEL_CREATE_FAILED.code.as_str()
                    || value == "GPU_KERNEL_CREATE_FAILED" =>
            {
                Self::KernelCreateFailed
            }
            value
                if value == MINE_GPU_BUFFER_ALLOC_FAILED.code.as_str()
                    || value == "GPU_BUFFER_ALLOC_FAILED" =>
            {
                Self::BufferAllocFailed
            }
            value
                if value == MINE_GPU_INVALID_ARGUMENT.code.as_str()
                    || value == "GPU_INVALID_ARGUMENT" =>
            {
                Self::InvalidArgument
            }
            value
                if value == MINE_GPU_BATCH_TOO_LARGE.code.as_str()
                    || value == "GPU_BATCH_TOO_LARGE" =>
            {
                Self::BatchTooLarge
            }
            value
                if value == MINE_GPU_NONCE_OVERFLOW.code.as_str()
                    || value == "GPU_NONCE_OVERFLOW" =>
            {
                Self::NonceOverflow
            }
            value
                if value == MINE_GPU_KERNEL_EXEC_FAILED.code.as_str()
                    || value == "GPU_KERNEL_EXECUTION_FAILED" =>
            {
                Self::KernelExecutionFailed
            }
            value
                if value == MINE_GPU_BUFFER_IO_FAILED.code.as_str()
                    || value == "GPU_BUFFER_IO_FAILED" =>
            {
                Self::BufferIoFailed
            }
            value
                if value == MINE_GPU_PROBE_FAILED.code.as_str() || value == "GPU_PROBE_FAILED" =>
            {
                Self::ProbeFailed
            }
            _ => Self::Unknown,
        }
    }

    /// Returns the stable Atho error-code string for this GPU failure.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FeatureDisabled => MINE_GPU_FEATURE_DISABLED.code.as_str(),
            Self::NotFound => MINE_GPU_NOT_FOUND.code.as_str(),
            Self::KernelMissing => MINE_GPU_KERNEL_MISSING.code.as_str(),
            Self::KernelLoadFailed => MINE_GPU_KERNEL_LOAD_FAILED.code.as_str(),
            Self::KernelBuildFailed => MINE_GPU_KERNEL_BUILD_FAILED.code.as_str(),
            Self::ContextCreateFailed => MINE_GPU_CONTEXT_CREATE_FAILED.code.as_str(),
            Self::QueueCreateFailed => MINE_GPU_QUEUE_CREATE_FAILED.code.as_str(),
            Self::KernelCreateFailed => MINE_GPU_KERNEL_CREATE_FAILED.code.as_str(),
            Self::BufferAllocFailed => MINE_GPU_BUFFER_ALLOC_FAILED.code.as_str(),
            Self::InvalidArgument => MINE_GPU_INVALID_ARGUMENT.code.as_str(),
            Self::BatchTooLarge => MINE_GPU_BATCH_TOO_LARGE.code.as_str(),
            Self::NonceOverflow => MINE_GPU_NONCE_OVERFLOW.code.as_str(),
            Self::KernelExecutionFailed => MINE_GPU_KERNEL_EXEC_FAILED.code.as_str(),
            Self::BufferIoFailed => MINE_GPU_BUFFER_IO_FAILED.code.as_str(),
            Self::ProbeFailed => MINE_GPU_PROBE_FAILED.code.as_str(),
            Self::Unknown => MINE_GPU_UNKNOWN.code.as_str(),
        }
    }
}

/// Successful GPU mining result returned by the native bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuSolution {
    pub nonce: u64,
    pub hash: [u8; HASH_BYTES],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuDeviceType {
    Gpu,
    Cpu,
    Unknown,
}

impl GpuDeviceType {
    #[cfg(any(test, feature = "gpu-native"))]
    fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "gpu" => Self::Gpu,
            "cpu" => Self::Cpu,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Gpu => "gpu",
            Self::Cpu => "cpu",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuProbeInfo {
    pub backend: String,
    pub device_type: GpuDeviceType,
    pub device_name: Option<String>,
    pub vendor: Option<String>,
    pub driver: Option<String>,
    pub compute_units: Option<u32>,
    pub global_mem_mb: Option<u64>,
    pub local_mem_kb: Option<u64>,
    pub clock_mhz: Option<u32>,
    pub kernel_path: Option<PathBuf>,
    pub supports_fixed: bool,
    pub supports_template: bool,
    pub max_batch: Option<u64>,
    pub template_max_bytes: Option<u64>,
    pub usable: bool,
    pub reason_code: Option<GpuErrorCode>,
    pub reason_if_not: Option<String>,
}

impl GpuProbeInfo {
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self::unavailable_with_code(GpuErrorCode::Unknown, reason)
    }

    pub fn unavailable_with_code(code: GpuErrorCode, reason: impl Into<String>) -> Self {
        Self {
            backend: String::from("opencl"),
            device_type: GpuDeviceType::Unknown,
            device_name: None,
            vendor: None,
            driver: None,
            compute_units: None,
            global_mem_mb: None,
            local_mem_kb: None,
            clock_mhz: None,
            kernel_path: None,
            supports_fixed: false,
            supports_template: false,
            max_batch: None,
            template_max_bytes: None,
            usable: false,
            reason_code: Some(code),
            reason_if_not: Some(reason.into()),
        }
    }

    pub fn device_label(&self) -> Option<String> {
        match (self.device_name.as_deref(), self.vendor.as_deref()) {
            (Some(name), Some(vendor)) if !vendor.is_empty() && !name.contains(vendor) => {
                Some(format!("{name} ({vendor})"))
            }
            (Some(name), _) => Some(name.to_string()),
            (None, Some(vendor)) if !vendor.is_empty() => Some(vendor.to_string()),
            _ => None,
        }
    }

    pub fn summary(&self) -> String {
        if self.usable {
            let device = self
                .device_label()
                .unwrap_or_else(|| String::from("OpenCL GPU"));
            format!("GPU detected: {device} via {}", self.backend)
        } else {
            let reason = self
                .reason_if_not
                .clone()
                .unwrap_or_else(|| String::from("unknown reason"));
            if let Some(code) = self.reason_code {
                format!("GPU unavailable ({}): {reason}", code.as_str())
            } else {
                format!("GPU unavailable: {reason}")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuError {
    pub code: GpuErrorCode,
    pub message: String,
}

impl GpuError {
    pub fn new(code: GpuErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for GpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for GpuError {}

#[cfg(feature = "gpu-native")]
mod ffi {
    use std::os::raw::{c_char, c_int};

    unsafe extern "C" {
        pub(super) fn atho_gpu_probe() -> c_int;
        pub(super) fn atho_gpu_probe_info(
            kernel_path: *const c_char,
            out_buf: *mut c_char,
            out_buf_len: usize,
            error_buf: *mut c_char,
            error_buf_len: usize,
        ) -> c_int;
        pub(super) fn atho_gpu_mine_batch(
            header_bytes: *const u8,
            header_len: usize,
            nonce_offset: u32,
            start_nonce: u64,
            batch_size: u64,
            target_bytes: *const u8,
            target_len: usize,
            kernel_path: *const c_char,
            out_nonce: *mut u64,
            out_hash: *mut u8,
            out_hash_len: usize,
            error_buf: *mut c_char,
            error_buf_len: usize,
        ) -> c_int;
    }
}

pub fn probe() -> bool {
    #[cfg(feature = "gpu-native")]
    {
        unsafe { ffi::atho_gpu_probe() > 0 }
    }

    #[cfg(not(feature = "gpu-native"))]
    {
        false
    }
}

pub fn probe_info(kernel_path: &Path) -> GpuProbeInfo {
    #[cfg(feature = "gpu-native")]
    {
        const OUTPUT_BUFFER_LEN: usize = 4096;
        const ERROR_BUFFER_LEN: usize = 512;

        let kernel_path = match CString::new(kernel_path.to_string_lossy().into_owned()) {
            Ok(path) => path,
            Err(_) => {
                return GpuProbeInfo::unavailable_with_code(
                    GpuErrorCode::InvalidArgument,
                    "kernel path contains NUL",
                )
            }
        };
        let mut output_buf = [0 as std::os::raw::c_char; OUTPUT_BUFFER_LEN];
        let mut error_buf = [0 as std::os::raw::c_char; ERROR_BUFFER_LEN];
        let rc = unsafe {
            ffi::atho_gpu_probe_info(
                kernel_path.as_ptr(),
                output_buf.as_mut_ptr(),
                output_buf.len(),
                error_buf.as_mut_ptr(),
                error_buf.len(),
            )
        };
        if rc < 0 {
            let raw = unsafe {
                CStr::from_ptr(error_buf.as_ptr())
                    .to_string_lossy()
                    .trim()
                    .to_string()
            };
            let err = decode_ffi_error(&raw, GpuErrorCode::ProbeFailed, "gpu probe failed");
            return GpuProbeInfo::unavailable_with_code(err.code, err.message);
        }

        let output = unsafe { CStr::from_ptr(output_buf.as_ptr()) }
            .to_string_lossy()
            .trim()
            .to_string();
        parse_probe_output(&output)
    }

    #[cfg(not(feature = "gpu-native"))]
    {
        let _ = kernel_path;
        GpuProbeInfo::unavailable_with_code(
            GpuErrorCode::FeatureDisabled,
            "gpu-native feature is not enabled",
        )
    }
}

pub fn mine_batch(
    header: &[u8],
    nonce_offset: u32,
    start_nonce: u64,
    batch_size: u64,
    target: &[u8],
    kernel_path: &Path,
) -> Result<Option<GpuSolution>, GpuError> {
    if header.len() != HEADER_BYTES {
        return Err(GpuError::new(
            GpuErrorCode::InvalidArgument,
            format!("header must be exactly {HEADER_BYTES} bytes"),
        ));
    }
    if target.len() != TARGET_BYTES {
        return Err(GpuError::new(
            GpuErrorCode::InvalidArgument,
            format!("target must be exactly {TARGET_BYTES} bytes"),
        ));
    }

    #[cfg(feature = "gpu-native")]
    {
        const ERROR_BUFFER_LEN: usize = 512;
        let kernel_path =
            CString::new(kernel_path.to_string_lossy().into_owned()).map_err(|_| {
                GpuError::new(GpuErrorCode::InvalidArgument, "kernel path contains NUL")
            })?;
        let mut out_nonce = 0u64;
        let mut out_hash = [0u8; HASH_BYTES];
        let mut error_buf = [0 as std::os::raw::c_char; ERROR_BUFFER_LEN];
        let rc = unsafe {
            ffi::atho_gpu_mine_batch(
                header.as_ptr(),
                header.len(),
                nonce_offset,
                start_nonce,
                batch_size,
                target.as_ptr(),
                target.len(),
                kernel_path.as_ptr(),
                &mut out_nonce,
                out_hash.as_mut_ptr(),
                out_hash.len(),
                error_buf.as_mut_ptr(),
                error_buf.len(),
            )
        };
        match rc {
            0 => Ok(Some(GpuSolution {
                nonce: out_nonce,
                hash: out_hash,
            })),
            1 => Ok(None),
            _ => {
                let raw = unsafe {
                    CStr::from_ptr(error_buf.as_ptr())
                        .to_string_lossy()
                        .trim()
                        .to_string()
                };
                Err(decode_ffi_error(
                    &raw,
                    GpuErrorCode::Unknown,
                    "gpu ffi call failed",
                ))
            }
        }
    }

    #[cfg(not(feature = "gpu-native"))]
    {
        let _ = (
            header,
            nonce_offset,
            start_nonce,
            batch_size,
            target,
            kernel_path,
        );
        Err(GpuError::new(
            GpuErrorCode::FeatureDisabled,
            "gpu-native feature is not enabled",
        ))
    }
}

pub fn default_kernel_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../atho-node/native/gpu/sha3_384.cl")
}

#[cfg(any(test, feature = "gpu-native"))]
fn parse_probe_output(output: &str) -> GpuProbeInfo {
    if output.trim().is_empty() {
        return GpuProbeInfo::unavailable_with_code(
            GpuErrorCode::ProbeFailed,
            "gpu probe returned no data",
        );
    }

    let mut info =
        GpuProbeInfo::unavailable_with_code(GpuErrorCode::NotFound, "no real OpenCL GPU detected");
    for line in output.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "backend" => info.backend = value.to_string(),
            "usable" => info.usable = value == "1" || value.eq_ignore_ascii_case("true"),
            "device_type" => info.device_type = GpuDeviceType::parse(value),
            "device_name" if !value.is_empty() => info.device_name = Some(value.to_string()),
            "device_vendor" if !value.is_empty() => info.vendor = Some(value.to_string()),
            "device_driver" if !value.is_empty() => info.driver = Some(value.to_string()),
            "compute_units" => info.compute_units = value.parse::<u32>().ok(),
            "global_mem_mb" => info.global_mem_mb = value.parse::<u64>().ok(),
            "local_mem_kb" => info.local_mem_kb = value.parse::<u64>().ok(),
            "clock_mhz" => info.clock_mhz = value.parse::<u32>().ok(),
            "kernel_path" if !value.is_empty() => info.kernel_path = Some(PathBuf::from(value)),
            "supports_fixed" => {
                info.supports_fixed = value == "1" || value.eq_ignore_ascii_case("true")
            }
            "supports_template" => {
                info.supports_template = value == "1" || value.eq_ignore_ascii_case("true")
            }
            "max_batch" => info.max_batch = value.parse::<u64>().ok(),
            "template_max_bytes" => info.template_max_bytes = value.parse::<u64>().ok(),
            "reason_code" if !value.is_empty() => {
                info.reason_code = Some(GpuErrorCode::parse(value))
            }
            "reason" if !value.is_empty() => info.reason_if_not = Some(value.to_string()),
            _ => {}
        }
    }

    if info.usable {
        info.reason_code = None;
        info.reason_if_not = None;
        if matches!(info.device_type, GpuDeviceType::Unknown) {
            info.device_type = GpuDeviceType::Gpu;
        }
        if info.backend.is_empty() {
            info.backend = String::from("opencl");
        }
    } else if info.backend.is_empty() {
        info.backend = String::from("opencl");
    }

    info
}

#[cfg(feature = "gpu-native")]
fn decode_ffi_error(raw: &str, default_code: GpuErrorCode, default_message: &str) -> GpuError {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return GpuError::new(default_code, default_message);
    }
    if let Some((code, message)) = trimmed.split_once('|') {
        let message = if message.trim().is_empty() {
            default_message
        } else {
            message.trim()
        };
        return GpuError::new(GpuErrorCode::parse(code), message);
    }
    GpuError::new(default_code, trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "gpu-native")]
    use atho_core::block::Block;
    #[cfg(feature = "gpu-native")]
    use atho_core::crypto::hash::sha3_384;

    #[cfg(feature = "gpu-native")]
    fn sample_header_without_nonce() -> Vec<u8> {
        let bytes = Block::default().header.canonical_bytes_without_nonce();
        assert_eq!(bytes.len(), HEADER_BYTES);
        bytes
    }

    #[cfg(feature = "gpu-native")]
    fn cpu_hash_for_nonce(header_without_nonce: &[u8], nonce: u64) -> [u8; HASH_BYTES] {
        let mut bytes = header_without_nonce.to_vec();
        bytes.extend_from_slice(&nonce.to_le_bytes());
        sha3_384(&bytes)
    }

    #[test]
    fn defaults_point_at_workspace_kernel() {
        let path = default_kernel_path();
        assert!(path.ends_with("atho-node/native/gpu/sha3_384.cl"));
    }

    #[test]
    fn parse_probe_output_reports_detected_gpu() {
        let info = parse_probe_output(
            "status=ok\nbackend=opencl\nusable=1\ndevice_type=gpu\ndevice_name=Apple M4\ndevice_vendor=Apple\ndevice_driver=1.2\ncompute_units=10\nglobal_mem_mb=8192\nlocal_mem_kb=64\nclock_mhz=1500\nkernel_path=/tmp/sha3_384.cl\nsupports_fixed=1\nsupports_template=1\nmax_batch=2000000\ntemplate_max_bytes=4096\n",
        );
        assert!(info.usable);
        assert_eq!(info.backend, "opencl");
        assert_eq!(info.device_type, GpuDeviceType::Gpu);
        assert_eq!(info.device_name.as_deref(), Some("Apple M4"));
        assert_eq!(info.vendor.as_deref(), Some("Apple"));
        assert_eq!(info.max_batch, Some(2_000_000));
        assert_eq!(info.reason_code, None);
    }

    #[test]
    fn parse_probe_output_reports_unavailable_reason() {
        let info = parse_probe_output(
            "status=unavailable\nbackend=opencl\nusable=0\nreason_code=ATHO-MINE-102\nreason=no real OpenCL GPU detected\n",
        );
        assert!(!info.usable);
        assert_eq!(info.reason_code, Some(GpuErrorCode::NotFound));
        assert_eq!(
            info.reason_if_not.as_deref(),
            Some("no real OpenCL GPU detected")
        );
    }

    #[test]
    fn mine_batch_rejects_wrong_header_length_before_ffi() {
        let err = mine_batch(
            &[0u8; HEADER_BYTES - 1],
            0,
            0,
            1,
            &[0u8; TARGET_BYTES],
            Path::new("sha3_384.cl"),
        )
        .expect_err("invalid header length should fail");
        assert_eq!(err.code, GpuErrorCode::InvalidArgument);
        assert_eq!(
            err.message,
            format!("header must be exactly {HEADER_BYTES} bytes")
        );
    }

    #[test]
    fn mine_batch_rejects_wrong_target_length_before_ffi() {
        let err = mine_batch(
            &[0u8; HEADER_BYTES],
            0,
            0,
            1,
            &[0u8; TARGET_BYTES - 1],
            Path::new("sha3_384.cl"),
        )
        .expect_err("invalid target length should fail");
        assert_eq!(err.code, GpuErrorCode::InvalidArgument);
        assert_eq!(
            err.message,
            format!("target must be exactly {TARGET_BYTES} bytes")
        );
    }

    #[cfg(feature = "gpu-native")]
    #[test]
    fn gpu_hash_matches_cpu_for_non_zero_nonce() {
        if !probe() {
            return;
        }

        let header = sample_header_without_nonce();
        let solution = mine_batch(
            &header,
            HEADER_BYTES as u32,
            1,
            1,
            &[0xff; TARGET_BYTES],
            &default_kernel_path(),
        )
        .expect("gpu batch")
        .expect("solution for all-ones target");

        assert_eq!(solution.nonce, 1);
        assert_eq!(solution.hash, cpu_hash_for_nonce(&header, 1));
    }

    #[cfg(feature = "gpu-native")]
    #[test]
    fn gpu_accepts_hash_equal_to_target() {
        if !probe() {
            return;
        }

        let header = sample_header_without_nonce();
        let expected_hash = cpu_hash_for_nonce(&header, 1);
        let solution = mine_batch(
            &header,
            HEADER_BYTES as u32,
            1,
            1,
            &expected_hash,
            &default_kernel_path(),
        )
        .expect("gpu batch");

        let solution = solution.expect("target equality should be accepted");
        assert_eq!(solution.nonce, 1);
        assert_eq!(solution.hash, expected_hash);
    }

    #[cfg(not(feature = "gpu-native"))]
    #[test]
    fn probe_returns_false_without_native_feature() {
        assert!(!probe());
    }
}
