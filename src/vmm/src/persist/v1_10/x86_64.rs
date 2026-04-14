// Copyright 2025 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use kvm_bindings::{
    CpuId, Msrs, kvm_clock_data, kvm_debugregs, kvm_irqchip, kvm_lapic_state, kvm_mp_state,
    kvm_pit_state2, kvm_regs, kvm_sregs, kvm_vcpu_events, kvm_xcrs, kvm_xsave,
};
use serde::{Deserialize, Serialize};

use crate::cpu_config::templates::KvmCapability;

// ───────────────────────────────────────────────────────────────────
// VM state (x86_64, v1.10)
// In v1.10, VmState holds kvm_cap_modifiers; memory_state is at MicrovmState level.
// ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmState {
    pub pitstate: kvm_pit_state2,
    pub clock: kvm_clock_data,
    pub pic_master: kvm_irqchip,
    pub pic_slave: kvm_irqchip,
    pub ioapic: kvm_irqchip,
    pub kvm_cap_modifiers: Vec<KvmCapability>,
}

// ───────────────────────────────────────────────────────────────────
// vCPU state (x86_64, v1.10)
// xsave is kvm_xsave (not Xsave/FamStructWrapper<kvm_xsave2>)
// ───────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
pub struct VcpuState {
    pub cpuid: CpuId,
    pub saved_msrs: Vec<Msrs>,
    pub debug_regs: kvm_debugregs,
    pub lapic: kvm_lapic_state,
    pub mp_state: kvm_mp_state,
    pub regs: kvm_regs,
    pub sregs: kvm_sregs,
    pub vcpu_events: kvm_vcpu_events,
    pub xcrs: kvm_xcrs,
    /// In v1.10, xsave is stored as kvm_xsave (4096-byte opaque blob).
    /// In v1.12+, it became Xsave = FamStructWrapper<kvm_xsave2> to support Intel AMX.
    pub xsave: kvm_xsave,
    pub tsc_khz: Option<u32>,
}

impl std::fmt::Debug for VcpuState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VcpuState")
            .field("tsc_khz", &self.tsc_khz)
            .finish_non_exhaustive()
    }
}
