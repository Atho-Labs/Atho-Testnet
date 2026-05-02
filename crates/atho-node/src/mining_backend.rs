//! Mining backend selection and CPU/GPU fallback policy.
use crate::miner::{Miner, MiningInterrupted};
use atho_core::block::Block;
#[cfg(not(feature = "gpu-native"))]
use atho_errors::MINE_GPU_FEATURE_DISABLED;
use atho_errors::{
    registry_descriptor, AthoError, AthoErrorDescriptor, AthoErrorMeta, MINE_BACKEND_FAILURE,
    MINE_CANCELLED, MINE_GPU_UNKNOWN,
};
#[cfg(feature = "gpu-native")]
use atho_errors::{
    MINE_GPU_INVALID_ARGUMENT, MINE_GPU_KERNEL_MISSING, MINE_GPU_NONCE_OVERFLOW,
    MINE_GPU_SOLUTION_MISMATCH,
};
use atho_rpc::response::BlockTemplate;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
#[cfg(feature = "gpu-native")]
use std::sync::atomic::Ordering;
use std::sync::Arc;
use thiserror::Error;

const DEFAULT_GPU_BATCH_SIZE: u64 = 500_000;
const MAX_GPU_BATCH_SIZE: u64 = 2_000_000;
#[cfg(test)]
const HASH_BYTES: usize = 48;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MiningBackendKind {
    Cpu,
    Gpu,
    Auto,
}

impl MiningBackendKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "cpu" => Some(Self::Cpu),
            "gpu" | "opencl" => Some(Self::Gpu),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }

    pub fn from_env() -> Option<Self> {
        std::env::var("ATHO_MINING_BACKEND")
            .ok()
            .and_then(|value| Self::parse(&value))
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Gpu => "gpu",
            Self::Auto => "auto",
        }
    }

    pub fn variants() -> [Self; 3] {
        [Self::Auto, Self::Gpu, Self::Cpu]
    }
}

impl Default for MiningBackendKind {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MiningDeviceType {
    Gpu,
    Cpu,
    Unknown,
}

impl MiningDeviceType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Gpu => "gpu",
            Self::Cpu => "cpu",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MiningAcceleratorInfo {
    pub backend: String,
    pub device_type: MiningDeviceType,
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
    pub reason_code: Option<String>,
    pub reason_if_not: Option<String>,
}

impl MiningAcceleratorInfo {
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self::unavailable_with_code(MINE_GPU_UNKNOWN.code.as_str(), reason)
    }

    pub fn unavailable_with_code(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            backend: String::from("opencl"),
            device_type: MiningDeviceType::Unknown,
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
            reason_code: Some(code.into()),
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

    pub fn runtime_label(&self) -> Option<String> {
        self.device_label()
            .map(|device| format!("{device} via {}", self.backend))
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
            if let Some(code) = self.reason_code.as_deref() {
                format!("GPU unavailable ({code}): {reason}")
            } else {
                format!("GPU unavailable: {reason}")
            }
        }
    }

    pub fn fallback_reason(&self) -> String {
        let reason = self
            .reason_if_not
            .clone()
            .unwrap_or_else(|| String::from("no real OpenCL GPU detected"));
        if let Some(code) = self.reason_code.as_deref() {
            format!("{code}: {reason}")
        } else {
            reason
        }
    }
}

#[derive(Debug, Error)]
pub enum MiningBackendError {
    #[error("mining was cancelled")]
    Cancelled,
    #[error("{0}")]
    Message(String),
}

impl AthoErrorMeta for MiningBackendError {
    fn descriptor(&self) -> &'static AthoErrorDescriptor {
        match self {
            Self::Cancelled => &MINE_CANCELLED,
            Self::Message(message) => message
                .split(':')
                .next()
                .and_then(|code| registry_descriptor(code.trim()))
                .unwrap_or(&MINE_BACKEND_FAILURE),
        }
    }

    fn source_module(&self) -> &'static str {
        "atho-node::mining_backend"
    }

    fn safe_details(&self) -> Option<String> {
        match self {
            Self::Cancelled => None,
            Self::Message(message) => Some(message.clone()),
        }
    }

    fn to_atho_error(&self) -> AthoError {
        match self {
            Self::Cancelled => AthoErrorMeta::to_atho_error(self),
            Self::Message(message) => {
                let descriptor = self.descriptor();
                AthoError::new(descriptor, self.source_module(), descriptor.explanation)
                    .with_safe_details(message.clone())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MiningBackendUsed {
    Cpu,
    Gpu,
}

impl MiningBackendUsed {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Gpu => "gpu",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MiningReport {
    pub block: Block,
    pub backend_used: MiningBackendUsed,
    pub fallback_reason: Option<String>,
    pub accelerator: Option<MiningAcceleratorInfo>,
}

#[cfg(feature = "gpu-native")]
impl From<atho_gpu_native::GpuProbeInfo> for MiningAcceleratorInfo {
    fn from(value: atho_gpu_native::GpuProbeInfo) -> Self {
        let device_type = match value.device_type {
            atho_gpu_native::GpuDeviceType::Gpu => MiningDeviceType::Gpu,
            atho_gpu_native::GpuDeviceType::Cpu => MiningDeviceType::Cpu,
            atho_gpu_native::GpuDeviceType::Unknown => MiningDeviceType::Unknown,
        };
        Self {
            backend: value.backend,
            device_type,
            device_name: value.device_name,
            vendor: value.vendor,
            driver: value.driver,
            compute_units: value.compute_units,
            global_mem_mb: value.global_mem_mb,
            local_mem_kb: value.local_mem_kb,
            clock_mhz: value.clock_mhz,
            kernel_path: value.kernel_path,
            supports_fixed: value.supports_fixed,
            supports_template: value.supports_template,
            max_batch: value.max_batch,
            template_max_bytes: value.template_max_bytes,
            usable: value.usable,
            reason_code: value.reason_code.map(|code| code.as_str().to_string()),
            reason_if_not: value.reason_if_not,
        }
    }
}

impl From<MiningInterrupted> for MiningBackendError {
    fn from(_: MiningInterrupted) -> Self {
        Self::Cancelled
    }
}

impl MiningBackendError {
    fn coded(code: &str, message: impl Into<String>) -> Self {
        Self::Message(format!("{code}: {}", message.into()))
    }
}

#[cfg_attr(not(feature = "gpu-native"), allow(dead_code))]
#[derive(Debug, Clone)]
struct NativeGpuConfig {
    kernel_path: PathBuf,
    batch_size: u64,
}

#[derive(Debug, Clone)]
pub struct MiningController {
    backend: MiningBackendKind,
    cores: u32,
    gpu: NativeGpuConfig,
}

impl MiningController {
    pub fn new(backend: MiningBackendKind, cores: u32) -> Self {
        Self {
            backend,
            cores: cores.max(1),
            gpu: NativeGpuConfig::from_env(),
        }
    }

    pub fn from_env(cores: u32) -> Self {
        Self::new(
            MiningBackendKind::from_env().unwrap_or(MiningBackendKind::Auto),
            cores,
        )
    }

    pub fn backend(&self) -> MiningBackendKind {
        self.backend
    }

    pub fn gpu_probe_info(&self) -> MiningAcceleratorInfo {
        self.gpu.probe_info()
    }

    pub fn mine_block(
        &self,
        template: BlockTemplate,
        stop_requested: Arc<AtomicBool>,
    ) -> Result<Block, MiningBackendError> {
        self.mine_block_reported(template, stop_requested)
            .map(|report| report.block)
    }

    pub fn mine_block_reported(
        &self,
        template: BlockTemplate,
        stop_requested: Arc<AtomicBool>,
    ) -> Result<MiningReport, MiningBackendError> {
        match self.backend {
            MiningBackendKind::Cpu => self.mine_cpu_reported(template, stop_requested, None),
            MiningBackendKind::Gpu => self.mine_gpu_preferred(template, stop_requested),
            MiningBackendKind::Auto => self.mine_gpu_or_cpu(template, stop_requested),
        }
    }

    fn mine_cpu_reported(
        &self,
        template: BlockTemplate,
        stop_requested: Arc<AtomicBool>,
        fallback_reason: Option<String>,
    ) -> Result<MiningReport, MiningBackendError> {
        let miner = Miner::new(self.cores);
        let block = miner
            .solve_block_with_cancel(template.block, stop_requested)
            .map_err(MiningBackendError::from)?;
        Ok(MiningReport {
            block,
            backend_used: MiningBackendUsed::Cpu,
            fallback_reason,
            accelerator: None,
        })
    }

    fn mine_gpu_preferred(
        &self,
        template: BlockTemplate,
        stop_requested: Arc<AtomicBool>,
    ) -> Result<MiningReport, MiningBackendError> {
        let accelerator = self.gpu.probe_info();
        if !accelerator.usable {
            return self
                .mine_cpu_reported(
                    template,
                    stop_requested,
                    Some(format!(
                        "requested GPU backend but {}; using CPU",
                        accelerator.fallback_reason()
                    )),
                )
                .map(|mut report| {
                    report.accelerator = Some(accelerator);
                    report
                });
        }

        match self.gpu.mine(template.clone(), Arc::clone(&stop_requested)) {
            Ok(block) => Ok(MiningReport {
                block,
                backend_used: MiningBackendUsed::Gpu,
                fallback_reason: None,
                accelerator: Some(accelerator),
            }),
            Err(MiningBackendError::Cancelled) => Err(MiningBackendError::Cancelled),
            Err(err) => {
                let fallback_reason =
                    format!("requested GPU backend but GPU execution failed: {err}; using CPU");
                match self.mine_cpu_reported(
                    template,
                    stop_requested,
                    Some(fallback_reason.clone()),
                ) {
                    Ok(mut report) => {
                        report.fallback_reason = Some(fallback_reason);
                        report.accelerator = Some(accelerator);
                        Ok(report)
                    }
                    Err(cpu_err) => Err(MiningBackendError::Message(format!(
                        "{fallback_reason}; CPU fallback failed: {cpu_err}"
                    ))),
                }
            }
        }
    }

    fn mine_gpu_or_cpu(
        &self,
        template: BlockTemplate,
        stop_requested: Arc<AtomicBool>,
    ) -> Result<MiningReport, MiningBackendError> {
        let accelerator = self.gpu.probe_info();
        if !accelerator.usable {
            return self.mine_cpu_reported(
                template,
                stop_requested,
                Some(format!("{}; using CPU", accelerator.fallback_reason())),
            );
        }

        match self.gpu.mine(template.clone(), Arc::clone(&stop_requested)) {
            Ok(block) => Ok(MiningReport {
                block,
                backend_used: MiningBackendUsed::Gpu,
                fallback_reason: None,
                accelerator: Some(accelerator),
            }),
            Err(MiningBackendError::Cancelled) => Err(MiningBackendError::Cancelled),
            Err(err) => {
                let fallback_reason = format!("GPU backend failed: {err}; using CPU");
                match self.mine_cpu_reported(
                    template,
                    stop_requested,
                    Some(fallback_reason.clone()),
                ) {
                    Ok(mut report) => {
                        report.fallback_reason = Some(fallback_reason);
                        report.accelerator = Some(accelerator);
                        Ok(report)
                    }
                    Err(cpu_err) => Err(MiningBackendError::Message(format!(
                        "{fallback_reason}; CPU fallback failed: {cpu_err}"
                    ))),
                }
            }
        }
    }
}

impl NativeGpuConfig {
    fn from_env() -> Self {
        let kernel_path = std::env::var("ATHO_GPU_KERNEL_PATH")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(Self::default_kernel_path);
        let batch_size = std::env::var("ATHO_GPU_BATCH_SIZE")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DEFAULT_GPU_BATCH_SIZE)
            .clamp(1, MAX_GPU_BATCH_SIZE);
        Self {
            kernel_path,
            batch_size,
        }
    }

    fn default_kernel_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("native/gpu/sha3_384.cl")
    }

    fn probe_info(&self) -> MiningAcceleratorInfo {
        #[cfg(feature = "gpu-native")]
        {
            atho_gpu_native::probe_info(&self.kernel_path).into()
        }
        #[cfg(not(feature = "gpu-native"))]
        {
            let _ = self;
            MiningAcceleratorInfo::unavailable_with_code(
                MINE_GPU_FEATURE_DISABLED.code.as_str(),
                "gpu-native feature is not enabled",
            )
        }
    }

    fn mine(
        &self,
        template: BlockTemplate,
        stop_requested: Arc<AtomicBool>,
    ) -> Result<Block, MiningBackendError> {
        #[cfg(feature = "gpu-native")]
        {
            if !self.kernel_path.exists() {
                return Err(MiningBackendError::coded(
                    MINE_GPU_KERNEL_MISSING.code.as_str(),
                    format!(
                        "gpu kernel file not found at {}",
                        self.kernel_path.display()
                    ),
                ));
            }

            let header = template.header_bytes_without_nonce();
            let target = template.target;
            let nonce_offset = u32::try_from(template.nonce_offset_bytes()).map_err(|_| {
                MiningBackendError::coded(
                    MINE_GPU_INVALID_ARGUMENT.code.as_str(),
                    "nonce offset does not fit in u32",
                )
            })?;
            let mut start_nonce = 0u64;

            loop {
                if stop_requested.load(Ordering::Acquire) {
                    return Err(MiningBackendError::Cancelled);
                }

                match atho_gpu_native::mine_batch(
                    &header,
                    nonce_offset,
                    start_nonce,
                    self.batch_size,
                    &target,
                    &self.kernel_path,
                ) {
                    Ok(Some(solution)) => {
                        let mut block = template.block.clone();
                        block.header.nonce = solution.nonce;
                        let block_hash = block.header.block_hash();
                        if block_hash != solution.hash {
                            return Err(MiningBackendError::coded(
                                MINE_GPU_SOLUTION_MISMATCH.code.as_str(),
                                "gpu helper returned a hash that does not match the reconstructed block",
                            ));
                        }
                        return Ok(block);
                    }
                    Ok(None) => {
                        start_nonce =
                            start_nonce.checked_add(self.batch_size).ok_or_else(|| {
                                MiningBackendError::coded(
                                    MINE_GPU_NONCE_OVERFLOW.code.as_str(),
                                    "nonce range overflowed while advancing gpu batches",
                                )
                            })?;
                    }
                    Err(err) => {
                        return Err(MiningBackendError::coded(err.code.as_str(), err.message));
                    }
                }
            }
        }

        #[cfg(not(feature = "gpu-native"))]
        {
            let _ = template;
            let _ = stop_requested;
            Err(MiningBackendError::coded(
                MINE_GPU_FEATURE_DISABLED.code.as_str(),
                "gpu-native feature is not enabled",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atho_core::block::Block;
    use atho_core::consensus::pow;
    use atho_core::network::Network;
    use std::ffi::OsString;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    static TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.take() {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn easy_template() -> BlockTemplate {
        let mut block = Block::default();
        block.header.difficulty_target_or_bits = [0xff; HASH_BYTES];
        BlockTemplate {
            network: Network::Regnet,
            height: 0,
            previous_block_hash: [0; HASH_BYTES],
            target: [0xff; HASH_BYTES],
            transaction_count: 0,
            fees_atoms: 0,
            block,
        }
    }

    #[test]
    fn backend_kind_parses_expected_values() {
        assert_eq!(
            MiningBackendKind::parse("cpu"),
            Some(MiningBackendKind::Cpu)
        );
        assert_eq!(
            MiningBackendKind::parse("gpu"),
            Some(MiningBackendKind::Gpu)
        );
        assert_eq!(
            MiningBackendKind::parse("opencl"),
            Some(MiningBackendKind::Gpu)
        );
        assert_eq!(
            MiningBackendKind::parse("auto"),
            Some(MiningBackendKind::Auto)
        );
        assert_eq!(MiningBackendKind::parse("bogus"), None);
    }

    #[test]
    fn cpu_backend_succeeds_without_gpu_native_feature() {
        let controller = MiningController::new(MiningBackendKind::Cpu, 1);
        let template = easy_template();
        let mined = controller
            .mine_block(template, Arc::new(AtomicBool::new(false)))
            .expect("cpu mining");
        assert!(pow::meets_target(
            &mined.header.block_hash(),
            &mined.header.difficulty_target_or_bits
        ));
    }

    #[test]
    fn cpu_backend_reports_cancelled_when_stop_is_pre_requested() {
        let controller = MiningController::new(MiningBackendKind::Cpu, 1);
        let stop_requested = Arc::new(AtomicBool::new(true));
        let err = controller
            .mine_block(easy_template(), stop_requested)
            .expect_err("cancelled mining should error");
        assert!(matches!(err, MiningBackendError::Cancelled));
    }

    #[test]
    fn auto_backend_falls_back_to_cpu_when_gpu_is_unavailable() {
        let _lock = TEST_ENV_LOCK.lock().unwrap();
        let _kernel = EnvGuard::set(
            "ATHO_GPU_KERNEL_PATH",
            "/definitely/not/present/sha3_384.cl",
        );
        let controller = MiningController::new(MiningBackendKind::Auto, 1);
        let report = controller
            .mine_block_reported(easy_template(), Arc::new(AtomicBool::new(false)))
            .expect("auto fallback mining");
        assert_eq!(report.backend_used, MiningBackendUsed::Cpu);
        let fallback_reason = report.fallback_reason.as_deref().unwrap_or_default();
        #[cfg(feature = "gpu-native")]
        assert!(fallback_reason.contains("ATHO-MINE-103: gpu kernel file not found"));
        #[cfg(not(feature = "gpu-native"))]
        assert_eq!(
            fallback_reason,
            "ATHO-MINE-101: gpu-native feature is not enabled; using CPU"
        );
        assert!(fallback_reason.ends_with("; using CPU"));
        assert!(pow::meets_target(
            &report.block.header.block_hash(),
            &report.block.header.difficulty_target_or_bits
        ));
    }

    #[test]
    fn gpu_requested_falls_back_to_cpu_when_real_gpu_or_kernel_is_unavailable() {
        let _lock = TEST_ENV_LOCK.lock().unwrap();
        let _kernel = EnvGuard::set(
            "ATHO_GPU_KERNEL_PATH",
            "/definitely/not/present/sha3_384.cl",
        );
        let controller = MiningController::new(MiningBackendKind::Gpu, 1);
        let report = controller
            .mine_block_reported(easy_template(), Arc::new(AtomicBool::new(false)))
            .expect("explicit gpu should recover to cpu");
        assert_eq!(report.backend_used, MiningBackendUsed::Cpu);
        let fallback_reason = report.fallback_reason.as_deref().unwrap_or_default();
        #[cfg(feature = "gpu-native")]
        assert!(fallback_reason.contains("ATHO-MINE-103: gpu kernel file not found"));
        #[cfg(not(feature = "gpu-native"))]
        assert_eq!(
            fallback_reason,
            "requested GPU backend but ATHO-MINE-101: gpu-native feature is not enabled; using CPU"
        );
        assert!(fallback_reason.ends_with("; using CPU"));
    }

    #[test]
    fn auto_backend_preserves_cancellation_before_cpu_fallback() {
        let _lock = TEST_ENV_LOCK.lock().unwrap();
        let _kernel = EnvGuard::set(
            "ATHO_GPU_KERNEL_PATH",
            "/definitely/not/present/sha3_384.cl",
        );
        let controller = MiningController::new(MiningBackendKind::Auto, 1);
        let stop_requested = Arc::new(AtomicBool::new(false));
        stop_requested.store(true, Ordering::Release);
        let err = controller
            .mine_block_reported(easy_template(), stop_requested)
            .expect_err("cancelled mining should error");
        assert!(matches!(err, MiningBackendError::Cancelled));
    }

    #[cfg(feature = "gpu-native")]
    #[test]
    fn gpu_backend_smoke_batch_returns_nonce_zero_for_easy_target() {
        let controller = MiningController::new(MiningBackendKind::Gpu, 1);
        if !controller.gpu_probe_info().usable {
            return;
        }

        let mined = controller
            .mine_block_reported(easy_template(), Arc::new(AtomicBool::new(false)))
            .expect("gpu mining");
        assert_eq!(mined.backend_used, MiningBackendUsed::Gpu);
        assert_eq!(mined.block.header.nonce, 0);
        assert!(pow::meets_target(
            &mined.block.header.block_hash(),
            &mined.block.header.difficulty_target_or_bits
        ));
    }
}
