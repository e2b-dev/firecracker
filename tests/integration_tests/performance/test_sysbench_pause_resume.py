# Copyright 2025 Amazon.com, Inc. or its affiliates. All Rights Reserved.
# SPDX-License-Identifier: Apache-2.0
"""Benchmark: memory workload time after pause/resume or UFFD restore.

Uses dd to /dev/shm (1 GiB) so no apt/network; after UFFD restore this triggers
page faults. Scenarios: normal no UFFD (pause/resume), normal UFFD, hugepages UFFD.
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
        ("normal_no_uffd", HugePagesConfig.NONE, False),
        ("normal_uffd", HugePagesConfig.NONE, True),
        ("hugepages_uffd", HugePagesConfig.HUGETLBFS_2MB, True),
    ],
    ids=["normal_pages_no_uffd", "normal_pages_uffd", "hugepages_uffd"],
)
def test_sysbench_after_pause_resume_or_uffd_restore(
    microvm_factory, guest_kernel_linux_6_1, rootfs, scenario
):
    """Run 1 GiB memory workload after pause/resume or UFFD restore; report duration."""
    scenario_name, huge_pages, use_uffd = scenario

    vm = microvm_factory.build(guest_kernel_linux_6_1, rootfs)
    vm.memory_monitor = None
    vm.spawn()
    vm.basic_config(vcpu_count=2, mem_size_mib=MEM_SIZE_MIB, huge_pages=huge_pages)
    vm.add_net_iface()
    vm.start()

    if use_uffd:
        snapshot = vm.snapshot_full()
        vm.kill()
        vm = microvm_factory.build(guest_kernel_linux_6_1, rootfs)
        vm.memory_monitor = None
        vm.spawn()
        spawn_pf_handler(vm, uffd_handler("on_demand"), snapshot)
        vm.restore_from_snapshot(resume=True)

    else:
        vm.pause()
        vm.resume()

    duration = _run_workload(vm, scenario_name)
    vm.kill()
    print(f"[{scenario_name}] benchmark duration: {duration:.3f}s\n")
