// Copyright 2025 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use kvm_bindings::{kvm_mp_state, kvm_vcpu_init};
use serde::{Deserialize, Serialize};

use crate::convert::{ConvertError, irq_to_gsi};
use crate::v1_12;

use super::{
    ACPIDeviceManagerState, GuestMemoryState, MMIODeviceInfo, ResourceAllocator, VMGenIDState,
};

// ───────────────────────────────────────────────────────────────────
// StaticCpuTemplate — canonical definition (identical in v1.10, v1.12, v1.14)
// ───────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StaticCpuTemplate {
    V1N1,
    #[default]
    None,
}

// ───────────────────────────────────────────────────────────────────
// aarch64 legacy device types — canonical definitions
// ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeviceType {
    Virtio(u32),
    Serial,
    Rtc,
}

// ───────────────────────────────────────────────────────────────────
// GIC helper types — canonical definitions (unchanged since v1.10)
// ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(serialize = "T: Serialize", deserialize = "T: for<'a> Deserialize<'a>"))]
pub struct GicRegState<T: Serialize + for<'a> Deserialize<'a>> {
    pub chunks: Vec<T>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VgicSysRegsState {
    pub main_icc_regs: Vec<GicRegState<u64>>,
    pub ap_icc_regs: Vec<Option<GicRegState<u64>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GicVcpuState {
    pub rdist: Vec<GicRegState<u32>>,
    pub icc: VgicSysRegsState,
}

// ───────────────────────────────────────────────────────────────────
// aarch64 register vector — canonical definition (unchanged since v1.10)
// ───────────────────────────────────────────────────────────────────

/// aarch64 register vector with custom serde: serialized as (Vec<u64>, Vec<u8>)
#[derive(Debug, Clone)]
pub struct Aarch64RegisterVec {
    pub ids: Vec<u64>,
    pub data: Vec<u8>,
}

impl Serialize for Aarch64RegisterVec {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (&self.ids, &self.data).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Aarch64RegisterVec {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (ids, data) = <(Vec<u64>, Vec<u8>)>::deserialize(deserializer)?;
        Ok(Aarch64RegisterVec { ids, data })
    }
}

// ───────────────────────────────────────────────────────────────────
// aarch64 ConnectedLegacyState (uses updated MMIODeviceInfo with gsi)
// ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectedLegacyState {
    pub type_: DeviceType,
    pub device_info: MMIODeviceInfo,
}

impl From<v1_12::ConnectedLegacyState> for ConnectedLegacyState {
    fn from(s: v1_12::ConnectedLegacyState) -> Self {
        ConnectedLegacyState {
            type_: s.type_,
            device_info: MMIODeviceInfo::from(s.device_info),
        }
    }
}

// ───────────────────────────────────────────────────────────────────
// aarch64 GIC state (v1.14: adds its_state)
// GicRegState, VgicSysRegsState, GicVcpuState are defined above
// ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItsRegisterState {
    pub iidr: u64,
    pub cbaser: u64,
    pub creadr: u64,
    pub cwriter: u64,
    pub baser: [u64; 8],
    pub ctlr: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GicState {
    pub dist: Vec<GicRegState<u32>>,
    pub gic_vcpu_states: Vec<GicVcpuState>,
    /// ITS state (GICv3 only). None for GICv2 or when converted from v1.12.
    pub its_state: Option<ItsRegisterState>,
}

impl GicState {
    pub(crate) fn from(old_state: v1_12::GicState) -> GicState {
        GicState {
            dist: old_state.dist,
            gic_vcpu_states: old_state.gic_vcpu_states,
            its_state: None, // v1.12 had no ITS support
        }
    }
}

// ───────────────────────────────────────────────────────────────────
// vCPU state (aarch64, v1.14: gains pvtime_ipa)
// ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VcpuState {
    pub mp_state: kvm_mp_state,
    pub regs: Aarch64RegisterVec,
    pub mpidr: u64,
    pub kvi: kvm_vcpu_init,
    pub pvtime_ipa: Option<u64>,
}

impl VcpuState {
    pub(crate) fn from(old_state: v1_12::VcpuState) -> VcpuState {
        VcpuState {
            mp_state: old_state.mp_state,
            regs: old_state.regs,
            mpidr: old_state.mpidr,
            kvi: old_state.kvi,
            pvtime_ipa: None, // new in v1.14; default to None (not configured)
        }
    }
}

// ───────────────────────────────────────────────────────────────────
// ACPI device state impl (aarch64: no vmclock)
// ───────────────────────────────────────────────────────────────────

impl ACPIDeviceManagerState {
    pub(crate) fn from(
        s: v1_12::ACPIDeviceManagerState,
        _resource_allocator: &mut ResourceAllocator,
    ) -> Result<ACPIDeviceManagerState, ConvertError> {
        let vmgenid = s.vmgenid.ok_or(ConvertError::MissingVmGenId)?;
        Ok(ACPIDeviceManagerState {
            vmgenid: VMGenIDState {
                // v1.12 aarch64 uses IRQ_BASE=32-based numbers; v1.14 uses 0-based GSIs
                gsi: irq_to_gsi(vmgenid.gsi),
                addr: vmgenid.addr,
            },
        })
    }
}

// ───────────────────────────────────────────────────────────────────
// VM state (aarch64, v1.14: adds resource_allocator)
// ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmState {
    pub memory: GuestMemoryState,
    pub gic: GicState,
    pub resource_allocator: ResourceAllocator,
}

impl VmState {
    pub(crate) fn from(
        old_state: v1_12::VmState,
        resource_allocator: ResourceAllocator,
    ) -> VmState {
        VmState {
            memory: GuestMemoryState::from(old_state.memory),
            gic: GicState::from(old_state.gic),
            resource_allocator,
        }
    }
}
