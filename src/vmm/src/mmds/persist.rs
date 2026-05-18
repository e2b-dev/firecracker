// Copyright 2020 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Defines the structures needed for saving/restoring MmdsNetworkStack.

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::ns::MmdsNetworkStack;
use crate::mmds::data_store::Mmds;
use crate::snapshot::Persist;
use crate::utils::net::mac::{MAC_ADDR_LEN, MacAddr};

/// State of a MmdsNetworkStack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MmdsNetworkStackState {
    mac_addr: [u8; MAC_ADDR_LEN as usize],
    ipv4_addr: u32,
    tcp_port: u16,
}

impl Persist<'_> for MmdsNetworkStack {
    type State = MmdsNetworkStackState;
    type ConstructorArgs = Arc<Mutex<Mmds>>;
    type Error = ();

    fn save(&self) -> Self::State {
        let mut mac_addr = [0; MAC_ADDR_LEN as usize];
        mac_addr.copy_from_slice(self.mac_addr.get_bytes());

        MmdsNetworkStackState {
            mac_addr,
            ipv4_addr: self.ipv4_addr.into(),
            tcp_port: self.tcp_handler.local_port(),
        }
    }

    fn restore(mmds: Self::ConstructorArgs, state: &Self::State) -> Result<Self, Self::Error> {
        Ok(MmdsNetworkStack::new(
            MacAddr::from_bytes_unchecked(&state.mac_addr),
            Ipv4Addr::from(state.ipv4_addr),
            state.tcp_port,
            mmds,
        ))
    }
}

#[cfg(test)]
mod tests {

    use serde_json::json;

    use super::*;
    use crate::device_manager::persist::MmdsState;
    use crate::snapshot::Snapshot;

    // Reproducer for Finding P2-6 (docs/async-io-snapshot-analysis.md): the
    // device_manager only persists `{version, imds_compat}` into `MmdsState`;
    // the IMDS JSON datastore itself is dropped. On restore, the orchestrator
    // calls `setMmds` *after* `resumeVM`, so guests that read MMDS during the
    // first ticks of the resumed kernel observe `NotInitialized`.
    #[test]
    fn test_p2_6_mmds_data_store_not_persisted() {
        let mut mmds = Mmds::default();
        mmds.put_data(json!({ "instance-id": "i-test", "secret": "abc" }))
            .unwrap();
        assert_eq!(mmds.data_store_value()["instance-id"], json!("i-test"));

        // Verbatim of what device_manager::persist::save writes into the
        // snapshot for MMDS (src/vmm/src/device_manager/persist.rs:271-275).
        let saved = MmdsState {
            version: mmds.version(),
            imds_compat: mmds.imds_compat(),
        };

        // Restore the way builder.rs / NetPersist::restore do it: fresh
        // `Mmds::default()` then apply whatever the saved state carries.
        let mut restored = Mmds::default();
        restored.set_version(saved.version);
        restored.set_imds_compat(saved.imds_compat);

        // Bug: the data store is gone after restore.
        assert!(
            restored.data_store_value().is_null(),
            "P2-6: expected empty data store on restore"
        );
    }

    #[test]
    fn test_persistence() {
        let ns = MmdsNetworkStack::new_with_defaults(None, Arc::new(Mutex::new(Mmds::default())));

        let mut mem = vec![0; 4096];

        Snapshot::new(ns.save())
            .save(&mut mem.as_mut_slice())
            .unwrap();

        let restored_ns = MmdsNetworkStack::restore(
            Arc::new(Mutex::new(Mmds::default())),
            &Snapshot::load_without_crc_check(mem.as_slice())
                .unwrap()
                .data,
        )
        .unwrap();

        assert_eq!(restored_ns.mac_addr, ns.mac_addr);
        assert_eq!(restored_ns.ipv4_addr, ns.ipv4_addr);
        assert_eq!(
            restored_ns.tcp_handler.local_port(),
            ns.tcp_handler.local_port()
        );
    }
}
