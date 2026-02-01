# Copyright 2025 Amazon.com, Inc. or its affiliates. All Rights Reserved.
# SPDX-License-Identifier: Apache-2.0
"""Benchmark: memory workload time after snapshot restore.

All tests follow the same pattern:
1. Start VM with anonymous memory
2. Snapshot VM to disk file
3. Kill VM
4. Create new VM and restore from snapshot
5. Run 1 GiB memory workload and time it
6. Kill VM

The ONLY difference between tests is HOW the memory is served during restore:
- shared mmap (MAP_SHARED)
- private mmap (MAP_PRIVATE)
- UFFD (userfaultfd page fault handling)
- block device (loop device over memory file)

And whether THP (transparent hugepages) is enabled.
"""

import subprocess
import time
from pathlib import Path

import pytest

from framework.microvm import HugePagesConfig
from framework.utils_uffd import spawn_pf_handler, uffd_handler


def _create_loop_device(file_path: Path) -> str:
    """Create a loop device over a file. Returns device path like /dev/loop0."""
    result = subprocess.run(
        ["losetup", "-f", "--show", str(file_path)],
        capture_output=True,
        text=True,
        check=True,
    )
    return result.stdout.strip()


def _detach_loop_device(loop_dev: str):
    """Detach a loop device."""
    subprocess.run(["losetup", "-d", loop_dev], check=False)


def _create_block_dev_in_chroot(loop_dev: str, chroot: Path) -> str:
    """Create a block device node inside the chroot.

    Block devices can't be hard linked, so we need to mknod.
    Returns the jailed path (relative to chroot root).
    """
    import os

    # Get major/minor numbers of the loop device
    dev_stat = os.stat(loop_dev)
    major = os.major(dev_stat.st_rdev)
    minor = os.minor(dev_stat.st_rdev)

    # Create device node inside chroot
    dev_name = Path(loop_dev).name  # e.g., "loop0"
    jailed_dev_path = chroot / dev_name

    # Use mknod to create the device node (runs as root in container)
    subprocess.run(
        ["mknod", str(jailed_dev_path), "b", str(major), str(minor)],
        check=True,
    )
    subprocess.run(["chmod", "666", str(jailed_dev_path)], check=True)

    # Return path relative to chroot root (for API call)
    return f"/{dev_name}"


MEM_SIZE_MIB = 2048
WORKLOAD_CMD = "dd if=/dev/zero of=/dev/shm/bench bs=1M count=1024 conv=fsync"


def _run_workload(microvm):
    """Run workload and return duration in seconds."""
    start = time.perf_counter()
    microvm.ssh.run(WORKLOAD_CMD, timeout=300)
    return time.perf_counter() - start


def _get_thp_stats():
    """Get THP statistics from /proc/meminfo."""
    with open("/proc/meminfo") as f:
        meminfo = f.read()
    stats = {}
    for line in meminfo.splitlines():
        if "Huge" in line or "Thp" in line.lower():
            parts = line.split()
            if len(parts) >= 2:
                stats[parts[0].rstrip(":")] = int(parts[1])
    return stats


@pytest.mark.nonci
@pytest.mark.parametrize(
    "scenario",
    [
        # (name, use_shared, use_thp, use_uffd, use_hugepages)
        # File mmap variants - restoring from snapshot file
        ("private_no_thp", False, False, False, False),
        ("private_with_thp", False, True, False, False),
        ("shared_no_thp", True, False, False, False),
        ("shared_with_thp", True, True, False, False),
        # UFFD variants (lazy page fault handling)
        ("uffd_normal", False, False, True, False),
        ("uffd_hugepages", False, False, True, True),
    ],
    ids=[
        "private_no_thp",
        "private_with_thp",
        "shared_no_thp",
        "shared_with_thp",
        "uffd_normal",
        "uffd_hugepages",
    ],
)
def test_snapshot_restore_workload(
    microvm_factory, guest_kernel_linux_6_1, rootfs, scenario
):
    """Benchmark memory workload after snapshot restore."""
    scenario_name, use_shared, use_thp, use_uffd, use_hugepages = scenario

    # === Step 1: Start source VM ===
    huge_pages = (
        HugePagesConfig.HUGETLBFS_2MB if use_hugepages else HugePagesConfig.NONE
    )

    vm = microvm_factory.build(guest_kernel_linux_6_1, rootfs)
    vm.memory_monitor = None
    vm.spawn()
    vm.basic_config(
        vcpu_count=2,
        mem_size_mib=MEM_SIZE_MIB,
        huge_pages=huge_pages,
        track_dirty_pages=True,
    )
    vm.add_net_iface()
    vm.start()

    # Let VM run briefly
    time.sleep(0.5)

    # === Step 2: Snapshot to disk ===
    snapshot = vm.snapshot_full()

    # === Step 3: Kill source VM ===
    vm.kill()

    # === Step 4: Restore from snapshot ===
    vm2 = microvm_factory.build(guest_kernel_linux_6_1, rootfs)
    vm2.memory_monitor = None
    vm2.spawn()

    # Get THP stats before restore
    thp_before = _get_thp_stats()

    if use_uffd:
        # UFFD: lazy page fault handling
        spawn_pf_handler(vm2, uffd_handler("on_demand"), snapshot)
        vm2.restore_from_snapshot(resume=True)
    else:
        # File mmap: shared or private
        vm2.restore_from_snapshot(
            snapshot=snapshot,
            resume=True,
            shared=use_shared,
            thp=use_thp,
        )

    # === Step 5: Run workload and time it ===
    duration = _run_workload(vm2)

    # Get THP stats after workload
    thp_after = _get_thp_stats()

    # === Step 6: Kill VM ===
    vm2.kill()

    # Calculate THP changes
    file_hp_delta = thp_after.get("FileHugePages", 0) - thp_before.get(
        "FileHugePages", 0
    )
    anon_hp_delta = thp_after.get("AnonHugePages", 0) - thp_before.get(
        "AnonHugePages", 0
    )

    # Report result
    print(f"\n{'='*60}")
    print(f"[{scenario_name}] WORKLOAD TIME: {duration:.3f}s")
    print(
        f"[{scenario_name}] THP delta: File={file_hp_delta}KB, Anon={anon_hp_delta}KB"
    )
    print(f"{'='*60}\n")


@pytest.mark.nonci
def test_anonymous_memory_baseline(microvm_factory, guest_kernel_linux_6_1, rootfs):
    """Baseline: Anonymous memory with THP (no snapshot restore).

    This shows the BEST possible performance - VM running with
    anonymous memory that has THP enabled. No snapshot/restore.
    """
    vm = microvm_factory.build(guest_kernel_linux_6_1, rootfs)
    vm.memory_monitor = None
    vm.spawn()
    vm.basic_config(
        vcpu_count=2,
        mem_size_mib=MEM_SIZE_MIB,
        track_dirty_pages=False,
    )
    vm.add_net_iface()
    vm.start()

    # Get THP stats before workload
    thp_before = _get_thp_stats()

    # Run workload
    duration = _run_workload(vm)

    # Get THP stats after workload
    thp_after = _get_thp_stats()

    vm.kill()

    anon_hp_delta = thp_after.get("AnonHugePages", 0) - thp_before.get(
        "AnonHugePages", 0
    )

    print(f"\n{'='*60}")
    print(f"[anonymous_baseline] WORKLOAD TIME: {duration:.3f}s")
    print(f"[anonymous_baseline] AnonHugePages delta: {anon_hp_delta}KB")
    print(f"{'='*60}\n")


@pytest.mark.nonci
@pytest.mark.parametrize(
    "scenario",
    [
        # (name, use_shared, use_thp)
        ("block_private_no_thp", False, False),
        ("block_private_with_thp", False, True),
        ("block_shared_no_thp", True, False),
        ("block_shared_with_thp", True, True),
    ],
    ids=[
        "block_private_no_thp",
        "block_private_with_thp",
        "block_shared_no_thp",
        "block_shared_with_thp",
    ],
)
def test_block_device_restore_workload(
    microvm_factory, guest_kernel_linux_6_1, rootfs, scenario
):
    """Benchmark memory workload after snapshot restore using block device.

    This test creates a loop device over the memory snapshot file and
    uses that block device for the restore. This bypasses the filesystem
    layer and tests if THP works differently for block devices.
    """
    scenario_name, use_shared, use_thp = scenario

    # === Step 1: Start source VM ===
    vm = microvm_factory.build(guest_kernel_linux_6_1, rootfs)
    vm.memory_monitor = None
    vm.spawn()
    vm.basic_config(
        vcpu_count=2,
        mem_size_mib=MEM_SIZE_MIB,
        track_dirty_pages=True,
    )
    vm.add_net_iface()
    vm.start()

    # Let VM run briefly
    time.sleep(0.5)

    # === Step 2: Snapshot to disk ===
    snapshot = vm.snapshot_full()
    mem_file = snapshot.mem

    # === Step 3: Kill source VM ===
    vm.kill()

    # === Step 4: Create loop device over memory file ===
    loop_dev = _create_loop_device(mem_file)
    print(f"Created loop device: {loop_dev}")

    try:
        # === Step 5: Restore from snapshot using block device ===
        vm2 = microvm_factory.build(guest_kernel_linux_6_1, rootfs)
        vm2.memory_monitor = None
        vm2.spawn()

        # Create block device node in chroot
        jailed_block_dev = _create_block_dev_in_chroot(loop_dev, Path(vm2.chroot()))

        # Get THP stats before restore
        thp_before = _get_thp_stats()

        # Use restore_from_snapshot which handles rootfs and vmstate copying
        # but override the memory file path with our block device
        vm2.restore_from_snapshot(
            snapshot=snapshot,
            resume=True,
            shared=use_shared,
            thp=use_thp,
            mem_file_path=Path(jailed_block_dev),
        )

        # === Step 6: Run workload and time it ===
        duration = _run_workload(vm2)

        # Get THP stats after workload
        thp_after = _get_thp_stats()

        # === Step 7: Kill VM ===
        vm2.kill()

    finally:
        # === Cleanup: detach loop device ===
        _detach_loop_device(loop_dev)
        print(f"Detached loop device: {loop_dev}")
        # Clean up the mknod'd device in chroot
        dev_name = Path(loop_dev).name
        jailed_dev_full_path = Path(vm2.chroot()) / dev_name
        subprocess.run(["rm", "-f", str(jailed_dev_full_path)], check=False)

    # Calculate THP changes
    file_hp_delta = thp_after.get("FileHugePages", 0) - thp_before.get(
        "FileHugePages", 0
    )
    anon_hp_delta = thp_after.get("AnonHugePages", 0) - thp_before.get(
        "AnonHugePages", 0
    )

    # Report result
    print(f"\n{'='*60}")
    print(f"[{scenario_name}] WORKLOAD TIME: {duration:.3f}s")
    print(
        f"[{scenario_name}] THP delta: File={file_hp_delta}KB, Anon={anon_hp_delta}KB"
    )
    print(f"{'='*60}\n")
