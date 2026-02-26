// Copyright 2025 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Serializable state types for Firecracker v1.12 (snapshot format version 6.0.0).
//!
//! Types that are structurally identical to v1.14 are imported from that module.
//! Types that are the same in v1.10 and v1.12 (but different from v1.14) are defined
//! here as the canonical source; v1.10 imports them from this module.
//! Only types that are truly v1.12-specific are also defined here.
//!
//! Changes from v1.10:
//! - `MMIODeviceInfo`: `irqs: Vec<u32>` → `irq: Option<u32>` (v1.11)
//! - `GuestMemoryRegionState`: `offset` field removed (v1.11)
//! - `VmState`: memory moved here from `MicrovmState`, `kvm_cap_modifiers` moved to `KvmState`
//! - x86_64 `VcpuState.xsave`: `kvm_xsave` → `Xsave` (v1.12)
//! - `KvmState`: new wrapper for `kvm_cap_modifiers`
//! - `MicrovmState`: adds `kvm_state`, removes `memory_state`

use serde::{Deserialize, Serialize};

use super::v1_10;
use crate::arch::VcpuState;
use crate::devices::acpi::vmgenid::VMGenIDState;
use crate::devices::virtio::balloon::persist::BalloonConfigSpaceState;
use crate::devices::virtio::block::CacheType;
use crate::devices::virtio::block::virtio::persist::FileEngineTypeState;
use crate::devices::virtio::net::persist::{NetConfigSpaceState, RxBufferState};
use crate::devices::virtio::persist::QueueState;
use crate::devices::virtio::vsock::persist::VsockBackendState;
use crate::mmds::persist::MmdsNetworkStackState;
use crate::persist::VmInfo;
use crate::rate_limiter::persist::RateLimiterState;
use crate::vstate::kvm::KvmState;

#[cfg(target_arch = "x86_64")]
pub(crate) mod x86_64;
#[cfg(target_arch = "x86_64")]
pub use x86_64::*;

#[cfg(target_arch = "aarch64")]
pub(crate) mod aarch64;
#[cfg(target_arch = "aarch64")]
pub use aarch64::*;

// ───────────────────────────────────────────────────────────────────
// Shared simple types — same in v1.10 and v1.12; differs in v1.14
// Canonical definitions are here; v1.10 imports from this module.
// ───────────────────────────────────────────────────────────────────
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtioDeviceState {
    pub device_type: u32,
    pub avail_features: u64,
    pub acked_features: u64,
    pub queues: Vec<QueueState>,
    pub interrupt_status: u32,
    pub activated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MmioTransportState {
    pub features_select: u32,
    pub acked_features_select: u32,
    pub queue_select: u32,
    pub device_status: u32,
    pub config_generation: u32,
}

// ───────────────────────────────────────────────────────────────────
// Block device
// ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtioBlockState {
    pub id: String,
    pub partuuid: Option<String>,
    pub cache_type: CacheType,
    pub root_device: bool,
    pub disk_path: String,
    pub virtio_state: VirtioDeviceState,
    pub rate_limiter_state: RateLimiterState,
    pub file_engine_type: FileEngineTypeState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VhostUserBlockState {
    pub id: String,
    pub partuuid: Option<String>,
    pub cache_type: CacheType,
    pub root_device: bool,
    pub socket_path: String,
    pub vu_acked_protocol_features: u64,
    pub config_space: Vec<u8>,
    pub virtio_state: VirtioDeviceState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BlockState {
    Virtio(VirtioBlockState),
    VhostUser(VhostUserBlockState),
}

// ───────────────────────────────────────────────────────────────────
// Net device
// ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetState {
    pub id: String,
    pub tap_if_name: String,
    pub rx_rate_limiter_state: RateLimiterState,
    pub tx_rate_limiter_state: RateLimiterState,
    pub mmds_ns: Option<MmdsNetworkStackState>,
    pub config_space: NetConfigSpaceState,
    pub virtio_state: VirtioDeviceState,
    pub rx_buffers_state: RxBufferState,
}

// ───────────────────────────────────────────────────────────────────
// Vsock device
// ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VsockFrontendState {
    pub cid: u64,
    pub virtio_state: VirtioDeviceState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VsockState {
    pub backend: VsockBackendState,
    pub frontend: VsockFrontendState,
}

// ───────────────────────────────────────────────────────────────────
// Balloon device
// ───────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct BalloonStatsState {
    pub swap_in: Option<u64>,
    pub swap_out: Option<u64>,
    pub major_faults: Option<u64>,
    pub minor_faults: Option<u64>,
    pub free_memory: Option<u64>,
    pub total_memory: Option<u64>,
    pub available_memory: Option<u64>,
    pub disk_caches: Option<u64>,
    pub hugetlb_allocations: Option<u64>,
    pub hugetlb_failures: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalloonState {
    pub stats_polling_interval_s: u16,
    pub stats_desc_index: Option<u16>,
    pub latest_stats: BalloonStatsState,
    pub config_space: BalloonConfigSpaceState,
    pub virtio_state: VirtioDeviceState,
}

// ───────────────────────────────────────────────────────────────────
// Entropy device
// ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntropyState {
    pub virtio_state: VirtioDeviceState,
    pub rate_limiter_state: RateLimiterState,
}

// ───────────────────────────────────────────────────────────────────
// MMDS
// ───────────────────────────────────────────────────────────────────

/// MMDS version (renamed to `MmdsVersion` and restructured in v1.14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MmdsVersionState {
    V1,
    V2,
}

// ───────────────────────────────────────────────────────────────────
// ACPI devices state (same as v1.10; vmgenid becomes mandatory in v1.14)
// ───────────────────────────────────────────────────────────────────

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct ACPIDeviceManagerState {
    pub vmgenid: Option<VMGenIDState>,
}

// ───────────────────────────────────────────────────────────────────
// Changed in v1.11: irqs: Vec<u32> → irq: Option<u32>
// ───────────────────────────────────────────────────────────────────

/// MMIO device info.
///
/// Note: stored as `Option<NonZeroU32>` in Firecracker source, but `NonZeroU32` has
/// the same bincode wire format as `u32`, so we use `Option<u32>` here.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MMIODeviceInfo {
    pub addr: u64,
    pub len: u64,
    pub irq: Option<u32>,
}

impl MMIODeviceInfo {
    pub(crate) fn from(old: v1_10::MMIODeviceInfo) -> MMIODeviceInfo {
        MMIODeviceInfo {
            addr: old.addr,
            len: old.len,
            // v1.10 stored a Vec of IRQs; v1.11+ uses a single optional IRQ.
            // In practice exactly one IRQ was always present for devices that have one.
            irq: old.irqs.into_iter().next(),
        }
    }
}

// ───────────────────────────────────────────────────────────────────
// Changed in v1.11: `offset` field removed from GuestMemoryRegionState
// ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestMemoryRegionState {
    pub base_address: u64,
    pub size: usize,
}

impl From<v1_10::GuestMemoryRegionState> for GuestMemoryRegionState {
    fn from(old: v1_10::GuestMemoryRegionState) -> Self {
        // Drop the `offset` field which was removed in v1.11.
        GuestMemoryRegionState {
            base_address: old.base_address,
            size: old.size,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestMemoryState {
    pub regions: Vec<GuestMemoryRegionState>,
}

impl From<v1_10::GuestMemoryState> for GuestMemoryState {
    fn from(old: v1_10::GuestMemoryState) -> Self {
        GuestMemoryState {
            regions: old
                .regions
                .into_iter()
                .map(GuestMemoryRegionState::from)
                .collect(),
        }
    }
}

// ───────────────────────────────────────────────────────────────────
// Connected device state wrappers — redefined because MMIODeviceInfo changed.
// ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectedBlockState {
    pub device_id: String,
    pub device_state: BlockState,
    pub transport_state: MmioTransportState,
    pub device_info: MMIODeviceInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectedNetState {
    pub device_id: String,
    pub device_state: NetState,
    pub transport_state: MmioTransportState,
    pub device_info: MMIODeviceInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectedVsockState {
    pub device_id: String,
    pub device_state: VsockState,
    pub transport_state: MmioTransportState,
    pub device_info: MMIODeviceInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectedBalloonState {
    pub device_id: String,
    pub device_state: BalloonState,
    pub transport_state: MmioTransportState,
    pub device_info: MMIODeviceInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectedEntropyState {
    pub device_id: String,
    pub device_state: EntropyState,
    pub transport_state: MmioTransportState,
    pub device_info: MMIODeviceInfo,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct DeviceStates {
    #[cfg(target_arch = "aarch64")]
    pub legacy_devices: Vec<ConnectedLegacyState>,
    pub block_devices: Vec<ConnectedBlockState>,
    pub net_devices: Vec<ConnectedNetState>,
    pub vsock_device: Option<ConnectedVsockState>,
    pub balloon_device: Option<ConnectedBalloonState>,
    pub mmds_version: Option<MmdsVersionState>,
    pub entropy_device: Option<ConnectedEntropyState>,
}

impl From<v1_10::DeviceStates> for DeviceStates {
    fn from(old: v1_10::DeviceStates) -> Self {
        DeviceStates {
            #[cfg(target_arch = "aarch64")]
            legacy_devices: old
                .legacy_devices
                .into_iter()
                .map(|ld| ConnectedLegacyState {
                    type_: ld.type_,
                    device_info: MMIODeviceInfo::from(ld.device_info),
                })
                .collect(),
            block_devices: old
                .block_devices
                .into_iter()
                .map(|d| ConnectedBlockState {
                    device_id: d.device_id,
                    device_state: d.device_state,
                    transport_state: d.transport_state,
                    device_info: MMIODeviceInfo::from(d.device_info),
                })
                .collect(),
            net_devices: old
                .net_devices
                .into_iter()
                .map(|d| ConnectedNetState {
                    device_id: d.device_id,
                    device_state: d.device_state,
                    transport_state: d.transport_state,
                    device_info: MMIODeviceInfo::from(d.device_info),
                })
                .collect(),
            vsock_device: old.vsock_device.map(|d| ConnectedVsockState {
                device_id: d.device_id,
                device_state: d.device_state,
                transport_state: d.transport_state,
                device_info: MMIODeviceInfo::from(d.device_info),
            }),
            balloon_device: old.balloon_device.map(|d| ConnectedBalloonState {
                device_id: d.device_id,
                device_state: d.device_state,
                transport_state: d.transport_state,
                device_info: MMIODeviceInfo::from(d.device_info),
            }),
            mmds_version: old.mmds_version,
            entropy_device: old.entropy_device.map(|d| ConnectedEntropyState {
                device_id: d.device_id,
                device_state: d.device_state,
                transport_state: d.transport_state,
                device_info: MMIODeviceInfo::from(d.device_info),
            }),
        }
    }
}

// ───────────────────────────────────────────────────────────────────
// Top-level MicrovmState (v1.12)
// ───────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct MicrovmState {
    /// Imported from v1_14; unchanged through all versions.
    pub vm_info: VmInfo,
    /// Imported from v1_14; wraps `kvm_cap_modifiers`, extracted from v1.10's `VmState`.
    pub kvm_state: KvmState,
    /// Redefined in v1.12: `memory` moved in from top-level `MicrovmState.memory_state`,
    /// `kvm_cap_modifiers` moved out to `KvmState`. Redefined again in v1.14: gains
    /// `resource_allocator`; `GuestMemoryRegionState` gains `region_type` and `plugged`.
    pub vm_state: VmState,
    /// x86_64: redefined here (`xsave` type changed from `kvm_xsave` to `Xsave`);
    ///   imported into v1.14 (same type).
    /// aarch64: canonical definition here (same as v1.10; gains `pvtime_ipa` in v1.14).
    pub vcpu_states: Vec<VcpuState>,
    /// Redefined here: all `ConnectedXxxState` wrappers rebuilt because `MMIODeviceInfo`
    /// changed (`irqs: Vec<u32>` → `irq: Option<u32>`). Inner device states (BlockState,
    /// NetState, etc.) are defined in this module as the v1.10/v1.12 canonical source.
    pub device_states: DeviceStates,
    /// Defined in this module as the v1.10/v1.12 canonical source. Redefined in v1.14:
    /// `vmgenid` becomes mandatory, x86_64 gains `vmclock`; moved inside
    /// `DevicesState.acpi_state` (no longer top-level).
    pub acpi_dev_state: ACPIDeviceManagerState,
}

impl From<v1_10::MicrovmState> for MicrovmState {
    fn from(old: v1_10::MicrovmState) -> Self {
        // In v1.10, kvm_cap_modifiers lives in VmState; in v1.12 it moves to KvmState.
        // KvmCapability is the same type in all versions (imported from v1_14).
        let kvm_cap_modifiers = old.vm_state.kvm_cap_modifiers;

        let memory = GuestMemoryState::from(old.memory_state);

        #[cfg(target_arch = "x86_64")]
        let vm_state = VmState {
            memory,
            pitstate: old.vm_state.pitstate,
            clock: old.vm_state.clock,
            pic_master: old.vm_state.pic_master,
            pic_slave: old.vm_state.pic_slave,
            ioapic: old.vm_state.ioapic,
        };

        #[cfg(target_arch = "aarch64")]
        let vm_state = VmState {
            memory,
            gic: old.vm_state.gic,
        };

        // x86_64: xsave type changed from kvm_xsave → Xsave, needs conversion.
        // aarch64: VcpuState is identical in v1.10 and v1.12 (v1_12 is canonical source).
        #[cfg(target_arch = "x86_64")]
        let vcpu_states: Vec<VcpuState> =
            old.vcpu_states.into_iter().map(VcpuState::from).collect();
        #[cfg(target_arch = "aarch64")]
        let vcpu_states = old.vcpu_states;

        MicrovmState {
            vm_info: old.vm_info,
            kvm_state: KvmState { kvm_cap_modifiers },
            vm_state,
            vcpu_states,
            device_states: DeviceStates::from(old.device_states),
            acpi_dev_state: old.acpi_dev_state,
        }
    }
}
