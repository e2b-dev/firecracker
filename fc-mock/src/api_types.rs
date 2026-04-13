//! Firecracker API types — mirrors vmm::vmm_config serde DTOs.
//!
//! Only includes types actually used by the e2b-dev/infra orchestrator.
//! Each type matches the corresponding struct in src/vmm/src/vmm_config/
//! field-for-field, including serde attributes.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// vmm_config::boot_source
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootSourceConfig {
    pub kernel_image_path: String,
    pub initrd_path: Option<String>,
    pub boot_args: Option<String>,
}

// ---------------------------------------------------------------------------
// vmm_config (mod.rs) — rate limiter
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenBucketConfig {
    pub size: u64,
    pub one_time_burst: Option<u64>,
    pub refill_time: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateLimiterConfig {
    pub bandwidth: Option<TokenBucketConfig>,
    pub ops: Option<TokenBucketConfig>,
}

// ---------------------------------------------------------------------------
// vmm_config::drive
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileEngineType {
    Async,
    #[default]
    Sync,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockDeviceConfig {
    pub drive_id: String,
    pub partuuid: Option<String>,
    pub is_root_device: bool,
    #[serde(default)]
    pub cache_type: Option<String>,
    pub is_read_only: Option<bool>,
    pub path_on_host: Option<String>,
    pub rate_limiter: Option<RateLimiterConfig>,
    #[serde(rename = "io_engine")]
    pub file_engine_type: Option<FileEngineType>,
    pub socket: Option<String>,
}

/// PATCH /drives/{id} — only path_on_host and rate_limiter can be updated.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockDeviceUpdateConfig {
    pub drive_id: String,
    pub path_on_host: Option<String>,
    pub rate_limiter: Option<RateLimiterConfig>,
}

// ---------------------------------------------------------------------------
// vmm_config::machine_config
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum HugePageConfig {
    #[default]
    None,
    #[serde(rename = "2M")]
    Hugetlbfs2M,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineConfig {
    pub vcpu_count: u8,
    pub mem_size_mib: usize,
    #[serde(default)]
    pub smt: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_template: Option<serde_json::Value>,
    #[serde(default)]
    pub track_dirty_pages: bool,
    #[serde(default)]
    pub huge_pages: HugePageConfig,
}

impl Default for MachineConfig {
    fn default() -> Self {
        Self {
            vcpu_count: 1,
            mem_size_mib: 128,
            smt: false,
            cpu_template: None,
            track_dirty_pages: false,
            huge_pages: HugePageConfig::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// vmm_config::net
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkInterfaceConfig {
    pub iface_id: String,
    pub host_dev_name: String,
    pub guest_mac: Option<String>,
    pub rx_rate_limiter: Option<RateLimiterConfig>,
    pub tx_rate_limiter: Option<RateLimiterConfig>,
}

/// PATCH /network-interfaces/{id} — rate limiter updates only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkInterfaceUpdateConfig {
    pub iface_id: String,
    pub rx_rate_limiter: Option<RateLimiterConfig>,
    pub tx_rate_limiter: Option<RateLimiterConfig>,
}

// ---------------------------------------------------------------------------
// vmm_config::entropy
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntropyDeviceConfig {
    pub rate_limiter: Option<RateLimiterConfig>,
}

// ---------------------------------------------------------------------------
// vmm_config::metrics
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricsConfig {
    pub metrics_path: PathBuf,
}

// ---------------------------------------------------------------------------
// vmm_config::mmds
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MmdsConfig {
    #[serde(default)]
    pub version: Option<String>,
    pub network_interfaces: Vec<String>,
    pub ipv4_address: Option<String>,
    #[serde(default)]
    pub imds_compat: bool,
}

// ---------------------------------------------------------------------------
// vmm_config::snapshot
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotType {
    Diff,
    #[default]
    Full,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemBackendType {
    File,
    Uffd,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateSnapshotParams {
    #[serde(default)]
    pub snapshot_type: SnapshotType,
    pub snapshot_path: PathBuf,
    pub mem_file_path: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkOverride {
    pub iface_id: String,
    pub host_dev_name: String,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemBackendConfig {
    pub backend_path: PathBuf,
    pub backend_type: MemBackendType,
}

#[allow(deprecated)]
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadSnapshotConfig {
    pub snapshot_path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mem_file_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mem_backend: Option<MemBackendConfig>,
    #[serde(default)]
    #[deprecated]
    pub enable_diff_snapshots: bool,
    #[serde(default)]
    pub track_dirty_pages: bool,
    #[serde(default)]
    pub resume_vm: bool,
    #[serde(default)]
    pub network_overrides: Vec<NetworkOverride>,
}

/// PATCH /vm — Paused | Resumed
#[derive(Debug, Serialize, Deserialize)]
pub enum SnapshotVmState {
    Paused,
    Resumed,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vm {
    pub state: SnapshotVmState,
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct InstanceActionInfo {
    pub action_type: String,
}

// ---------------------------------------------------------------------------
// Error response
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct Error {
    pub fault_message: String,
}

// ---------------------------------------------------------------------------
// UFFD handshake (matches src/vmm/src/persist.rs GuestRegionUffdMapping)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuestRegionUffdMapping {
    pub base_host_virt_addr: u64,
    pub size: usize,
    pub offset: u64,
    pub page_size: usize,
}

// ---------------------------------------------------------------------------
// Mock-specific: workload simulation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkloadConfig {
    #[serde(default)]
    pub cpu_load_percent: f64,
    #[serde(default)]
    pub memory_usage_mib: u64,
    #[serde(default)]
    pub io_ops_per_sec: u64,
    #[serde(default)]
    pub response_delay_ms: u64,
    #[serde(default)]
    pub failure_mode: Option<FailureMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FailureMode {
    #[serde(rename = "crash")]
    Crash { after_ms: u64 },
    #[serde(rename = "hang")]
    Hang { after_ms: u64 },
    #[serde(rename = "oom")]
    Oom { after_ms: u64 },
    #[serde(rename = "io_error")]
    IoError { probability: f64 },
    #[serde(rename = "random_exit")]
    RandomExit { min_ms: u64, max_ms: u64, exit_code: i32 },
    #[serde(rename = "kernel_panic")]
    KernelPanic { after_ms: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkloadStatus {
    pub active: bool,
    pub cpu_load_percent: f64,
    pub memory_usage_mib: u64,
    pub io_ops_per_sec: u64,
    pub uptime_ms: u64,
    pub failure_mode: Option<String>,
}
