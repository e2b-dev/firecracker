# Copyright 2025 Amazon.com, Inc. or its affiliates. All Rights Reserved.
# SPDX-License-Identifier: Apache-2.0
"""Benchmark: memory workload time after snapshot/restore.

Uses dd to /dev/shm (1 GiB) so no apt/network; after restore this triggers
page faults. All scenarios do full snapshot/kill/restore cycle.
Scenarios: normal no UFFD, normal no UFFD + dirty tracking, normal UFFD, hugepages UFFD.
"""

import time

import pytest

from framework.microvm import HugePagesConfig
from framework.utils_uffd import spawn_pf_handler, uffd_handler

MEM_SIZE_MIB = 2048
WORKLOAD_CMD = "dd if=/dev/zero of=/dev/shm/bench bs=1M count=1024 conv=fsync"


def _run_workload(microvm, name):
    start = time.perf_counter()
    microvm.ssh.run(WORKLOAD_CMD, timeout=300)
    elapsed = time.perf_counter() - start
    print(f"[{name}] Workload duration: {elapsed:.3f}s")
    return elapsed


@pytest.mark.nonci
@pytest.mark.parametrize(
    "scenario",
    [
        ("normal_no_uffd", HugePagesConfig.NONE, False, False),
        ("normal_no_uffd_dirty_tracking", HugePagesConfig.NONE, False, True),
        ("normal_uffd", HugePagesConfig.NONE, True, False),
        ("hugepages_uffd", HugePagesConfig.HUGETLBFS_2MB, True, False),
    ],
    ids=[
        "normal_pages_no_uffd",
        "normal_pages_no_uffd_dirty_tracking",
        "normal_pages_uffd",
        "hugepages_uffd",
    ],
)
def test_sysbench_after_snapshot_restore(
    microvm_factory, guest_kernel_linux_6_1, rootfs, scenario
):
    """Run 1 GiB memory workload after snapshot/restore; report duration."""
    scenario_name, huge_pages, use_uffd, track_dirty_pages = scenario

    # Create and start the original VM
    vm = microvm_factory.build(guest_kernel_linux_6_1, rootfs)
    vm.memory_monitor = None
    vm.spawn()
    vm.basic_config(
        vcpu_count=2,
        mem_size_mib=MEM_SIZE_MIB,
        huge_pages=huge_pages,
        track_dirty_pages=track_dirty_pages,
    )
    vm.add_net_iface()
    vm.start()

    # Take snapshot and kill original VM
    snapshot = vm.snapshot_full()
    vm.kill()

    # Build new VM from snapshot
    vm = microvm_factory.build(guest_kernel_linux_6_1, rootfs)
    vm.memory_monitor = None
    vm.spawn()

    if use_uffd:
        # Restore with UFFD page fault handler (on-demand paging)
        spawn_pf_handler(vm, uffd_handler("on_demand"), snapshot)
        vm.restore_from_snapshot(resume=True)
    else:
        # Restore without UFFD (memory loaded upfront)
        vm.restore_from_snapshot(snapshot, resume=True)

    duration = _run_workload(vm, scenario_name)
    vm.kill()
    print(f"[{scenario_name}] benchmark duration: {duration:.3f}s\n")
