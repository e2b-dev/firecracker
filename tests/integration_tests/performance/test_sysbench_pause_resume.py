# Copyright 2025 Amazon.com, Inc. or its affiliates. All Rights Reserved.
# SPDX-License-Identifier: Apache-2.0
"""Benchmark: dd 1 GiB to /dev/shm after snapshot/restore."""

import time

import pytest

from framework.microvm import HugePagesConfig
from framework.utils_uffd import spawn_pf_handler, uffd_handler

VCPU_COUNT = 2
MEM_SIZE_MIB = 2048
WORKLOAD_SIZE_MIB = 1024
WORKLOAD_CMD = (
    f"dd if=/dev/zero of=/dev/shm/bench bs=1M count={WORKLOAD_SIZE_MIB} conv=fsync"
)


def _print_result(name, duration):
    throughput = WORKLOAD_SIZE_MIB / duration if duration > 0 else float("inf")
    print(
        f"\n=== {name} ===\n"
        f"  Duration   : {duration:.3f} s\n"
        f"  Throughput : {throughput:.1f} MiB/s\n"
    )


@pytest.mark.nonci
@pytest.mark.parametrize(
    "scenario",
    [
        ("normal_no_uffd", HugePagesConfig.NONE, False, False),
        # ("normal_no_uffd_dirty_tracking", HugePagesConfig.NONE, False, True),
        # ("normal_uffd", HugePagesConfig.NONE, True, False),
        # ("hugepages_uffd", HugePagesConfig.HUGETLBFS_2MB, True, False),
    ],
    ids=[
        "normal_pages_no_uffd",
        # "normal_pages_no_uffd_dirty_tracking",
        # "normal_pages_uffd",
        # "hugepages_uffd",
    ],
)
def test_sysbench_after_snapshot_restore(
    microvm_factory, guest_kernel_linux_6_1, rootfs, scenario
):
    """dd 1 GiB to /dev/shm after snapshot/restore; report duration."""
    scenario_name, huge_pages, use_uffd, track_dirty_pages = scenario

    vm = microvm_factory.build(guest_kernel_linux_6_1, rootfs)
    vm.memory_monitor = None
    vm.spawn()
    vm.basic_config(
        vcpu_count=VCPU_COUNT,
        mem_size_mib=MEM_SIZE_MIB,
        huge_pages=huge_pages,
        track_dirty_pages=track_dirty_pages,
    )
    vm.add_net_iface()
    vm.start()

    snapshot = vm.snapshot_full()
    vm.kill()

    vm = microvm_factory.build(guest_kernel_linux_6_1, rootfs)
    vm.memory_monitor = None
    vm.spawn()

    if use_uffd:
        spawn_pf_handler(vm, uffd_handler("on_demand"), snapshot)
        vm.restore_from_snapshot(resume=True)
    else:
        vm.restore_from_snapshot(snapshot, resume=True)

    start = time.perf_counter()
    vm.ssh.run(WORKLOAD_CMD, timeout=300)
    duration = time.perf_counter() - start

    _print_result(scenario_name, duration)
    vm.kill()
