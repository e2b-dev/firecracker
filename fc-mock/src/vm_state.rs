//! In-memory VM state — fields used by the e2b orchestrator + guest memory.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{Mutex, Notify};

use crate::api_types::*;
use crate::guest_mem::GuestMemory;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    NotStarted,
    Running,
    Paused,
}

pub struct VmState {
    #[allow(dead_code)]
    pub id: String,
    pub lifecycle: Lifecycle,

    pub boot_source: Option<BootSourceConfig>,
    pub machine_config: MachineConfig,
    pub drives: HashMap<String, BlockDeviceConfig>,
    pub network_interfaces: HashMap<String, NetworkInterfaceConfig>,
    pub metrics: Option<MetricsConfig>,
    pub entropy: Option<EntropyDeviceConfig>,
    pub mmds_config: Option<MmdsConfig>,
    pub mmds_data: serde_json::Value,

    /// Allocated guest memory — readable via ProcessVMReadv by orchestrator.
    pub guest_mem: Option<GuestMemory>,

    pub workload: Option<WorkloadConfig>,
    pub started_at: Option<Instant>,
    pub shutdown: Arc<Notify>,
}

impl VmState {
    pub fn new(id: String) -> Self {
        Self {
            id,
            lifecycle: Lifecycle::NotStarted,
            boot_source: None,
            machine_config: MachineConfig::default(),
            drives: HashMap::new(),
            network_interfaces: HashMap::new(),
            metrics: None,
            entropy: None,
            mmds_config: None,
            mmds_data: serde_json::Value::Null,
            guest_mem: None,
            workload: None,
            started_at: None,
            shutdown: Arc::new(Notify::new()),
        }
    }

    /// Allocate guest memory matching machine_config.mem_size_mib.
    pub fn allocate_guest_memory(&mut self) -> Result<(), String> {
        let mem = GuestMemory::allocate(self.machine_config.mem_size_mib)?;
        self.guest_mem = Some(mem);
        Ok(())
    }

    pub fn workload_status(&self) -> WorkloadStatus {
        let uptime = self.started_at.map(|s| s.elapsed().as_millis() as u64).unwrap_or(0);
        match &self.workload {
            Some(w) => WorkloadStatus {
                active: true,
                cpu_load_percent: w.cpu_load_percent,
                memory_usage_mib: w.memory_usage_mib,
                io_ops_per_sec: w.io_ops_per_sec,
                uptime_ms: uptime,
                failure_mode: w.failure_mode.as_ref().map(|f| format!("{f:?}")),
            },
            None => WorkloadStatus {
                active: false,
                cpu_load_percent: 0.0,
                memory_usage_mib: 0,
                io_ops_per_sec: 0,
                uptime_ms: uptime,
                failure_mode: None,
            },
        }
    }
}

pub type Shared = Arc<Mutex<VmState>>;
