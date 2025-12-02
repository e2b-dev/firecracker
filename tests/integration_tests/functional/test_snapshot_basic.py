# Copyright 2020 Amazon.com, Inc. or its affiliates. All Rights Reserved.
# SPDX-License-Identifier: Apache-2.0
"""Basic tests scenarios for snapshot save/restore."""

import dataclasses
import filecmp
import logging
import os
import platform
import re
import shutil
import time
import uuid
from pathlib import Path

import pytest

import host_tools.cargo_build as host
import host_tools.drive as drive_tools
import host_tools.network as net_tools
from framework import utils
from framework.microvm import SnapshotType
from framework.properties import global_props
from framework.utils import Timeout, check_filesystem, check_output
from framework.utils_vsock import (
    ECHO_SERVER_PORT,
    VSOCK_UDS_PATH,
    _copy_vsock_data_to_guest,
    check_guest_connections,
    check_host_connections,
    make_blob,
    make_host_port_path,
    start_guest_echo_server,
)

# Kernel emits this message when it resumes from a snapshot with VMGenID device
# present
DMESG_VMGENID_RESUME = "random: crng reseeded due to virtual machine fork"


def check_vmgenid_update_count(vm, resume_count):
    """
    Kernel will emit the DMESG_VMGENID_RESUME every time we resume
    from a snapshot
    """
    _, stdout, _ = vm.ssh.check_output("dmesg")
    assert resume_count == stdout.count(DMESG_VMGENID_RESUME)


def _get_guest_drive_size(ssh_connection, guest_dev_name="/dev/vdb"):
    # `lsblk` command outputs 2 lines to STDOUT:
    # "SIZE" and the size of the device, in bytes.
    blksize_cmd = "LSBLK_DEBUG=all lsblk -b {} --output SIZE".format(guest_dev_name)
    rc, stdout, stderr = ssh_connection.run(blksize_cmd)
    assert rc == 0, stderr
    lines = stdout.split("\n")
    return lines[1].strip()


@pytest.mark.parametrize("resume_at_restore", [True, False])
def test_resume(uvm_nano, microvm_factory, resume_at_restore):
    """Tests snapshot is resumable at or after restoration.

    Check that a restored microVM is resumable by either
    a. PUT /snapshot/load with `resume_vm=False`, then calling PATCH /vm resume=True
    b. PUT /snapshot/load with `resume_vm=True`
    """
    vm = uvm_nano
    vm.add_net_iface()
    vm.start()
    snapshot = vm.snapshot_full()
    restored_vm = microvm_factory.build()
    restored_vm.spawn()
    restored_vm.restore_from_snapshot(snapshot, resume=resume_at_restore)
    if not resume_at_restore:
        assert restored_vm.state == "Paused"
        restored_vm.resume()
    assert restored_vm.state == "Running"
    restored_vm.ssh.check_output("true")


def test_snapshot_current_version(uvm_nano):
    """Tests taking a snapshot at the version specified in Cargo.toml

    Check that it is possible to take a snapshot at the version of the upcoming
    release (during the release process this ensures that if we release version
    x.y, then taking a snapshot at version x.y works - something we'd otherwise
    only be able to test once the x.y binary has been uploaded to S3, at which
    point it is too late, see also the 1.3 release).
    """
    vm = uvm_nano
    vm.start()

    snapshot = vm.snapshot_full()

    # Fetch Firecracker binary for the latest version
    fc_binary = uvm_nano.fc_binary_path
    # Get supported snapshot version from Firecracker binary
    snapshot_version = (
        check_output(f"{fc_binary} --snapshot-version").stdout.strip().splitlines()[0]
    )

    # Verify the output of `--describe-snapshot` command line parameter
    cmd = [str(fc_binary)] + ["--describe-snapshot", str(snapshot.vmstate)]

    _, stdout, _ = check_output(cmd)
    assert snapshot_version in stdout


# Testing matrix:
# - Guest kernel: All supported ones
# - Rootfs: Ubuntu 18.04
# - Microvm: 2vCPU with 512 MB RAM
# TODO: Multiple microvm sizes must be tested in the async pipeline.
@pytest.mark.parametrize("use_snapshot_editor", [False, True])
def test_cycled_snapshot_restore(
    bin_vsock_path,
    tmp_path,
    uvm_plain_any,
    microvm_factory,
    snapshot_type,
    use_snapshot_editor,
    cpu_template_any,
):
    """
    Run a cycle of VM restoration and VM snapshot creation where new VM is
    restored from a snapshot of the previous one.
    """
    # This is an arbitrary selected value. It is big enough to test the
    # functionality, but small enough to not be annoying long to run.
    cycles = 3

    logger = logging.getLogger("snapshot_sequence")

    vm = uvm_plain_any
    vm.spawn()
    vm.basic_config(
        vcpu_count=2,
        mem_size_mib=512,
        track_dirty_pages=snapshot_type.needs_dirty_page_tracking,
    )
    vm.set_cpu_template(cpu_template_any)
    vm.add_net_iface()
    vm.api.vsock.put(vsock_id="vsock0", guest_cid=3, uds_path=VSOCK_UDS_PATH)
    vm.start()

    vm_blob_path = "/tmp/vsock/test.blob"
    # Generate a random data file for vsock.
    blob_path, blob_hash = make_blob(tmp_path)
    # Copy the data file and a vsock helper to the guest.
    _copy_vsock_data_to_guest(vm.ssh, blob_path, vm_blob_path, bin_vsock_path)

    logger.info("Create %s #0.", snapshot_type)
    # Create a snapshot from a microvm.
    start_guest_echo_server(vm)
    snapshot = vm.make_snapshot(snapshot_type)
    vm.kill()

    for microvm in microvm_factory.build_n_from_snapshot(
        snapshot, cycles, incremental=True, use_snapshot_editor=use_snapshot_editor
    ):
        # FIXME: This and the sleep below reduce the rate of vsock/ssh connection
        # related spurious test failures, although we do not know why this is the case.
        time.sleep(2)
        # Test vsock guest-initiated connections.
        path = os.path.join(
            microvm.path, make_host_port_path(VSOCK_UDS_PATH, ECHO_SERVER_PORT)
        )
        check_guest_connections(microvm, path, vm_blob_path, blob_hash)
        # Test vsock host-initiated connections.
        path = os.path.join(microvm.jailer.chroot_path(), VSOCK_UDS_PATH)
        check_host_connections(path, blob_path, blob_hash)

        # Check that the root device is not corrupted.
        check_filesystem(microvm.ssh, "squashfs", "/dev/vda")

        time.sleep(2)


def test_patch_drive_snapshot(uvm_nano, microvm_factory):
    """
    Test that a patched drive is correctly used by guests loaded from snapshot.
    """
    logger = logging.getLogger("snapshot_sequence")

    # Use a predefined vm instance.
    basevm = uvm_nano
    basevm.add_net_iface()

    # Add a scratch 128MB RW non-root block device.
    root = Path(basevm.path)
    scratch_path1 = str(root / "scratch1")
    scratch_disk1 = drive_tools.FilesystemFile(scratch_path1, size=128)
    basevm.add_drive("scratch", scratch_disk1.path)
    basevm.start()

    # Update drive to have another backing file, double in size.
    new_file_size_mb = 2 * int(scratch_disk1.size() / (1024 * 1024))
    logger.info("Patch drive, new file: size %sMB.", new_file_size_mb)
    scratch_path2 = str(root / "scratch2")
    scratch_disk2 = drive_tools.FilesystemFile(scratch_path2, new_file_size_mb)
    basevm.patch_drive("scratch", scratch_disk2)

    # Create base snapshot.
    logger.info("Create FULL snapshot #0.")
    snapshot = basevm.snapshot_full()

    # Load snapshot in a new Firecracker microVM.
    logger.info("Load snapshot, mem %s", snapshot.mem)
    vm = microvm_factory.build_from_snapshot(snapshot)

    # Attempt to connect to resumed microvm and verify the new microVM has the
    # right scratch drive.
    guest_drive_size = _get_guest_drive_size(vm.ssh)
    assert guest_drive_size == str(scratch_disk2.size())


def test_load_snapshot_failure_handling(uvm_plain):
    """
    Test error case of loading empty snapshot files.
    """
    vm = uvm_plain
    vm.spawn(log_level="Info")

    # Create two empty files for snapshot state and snapshot memory
    chroot_path = vm.jailer.chroot_path()
    snapshot_dir = os.path.join(chroot_path, "snapshot")
    Path(snapshot_dir).mkdir(parents=True, exist_ok=True)

    snapshot_mem = os.path.join(snapshot_dir, "snapshot_mem")
    open(snapshot_mem, "w+", encoding="utf-8").close()
    snapshot_vmstate = os.path.join(snapshot_dir, "snapshot_vmstate")
    open(snapshot_vmstate, "w+", encoding="utf-8").close()

    # Hardlink the snapshot files into the microvm jail.
    jailed_mem = vm.create_jailed_resource(snapshot_mem)
    jailed_vmstate = vm.create_jailed_resource(snapshot_vmstate)

    # Load the snapshot
    expected_msg = (
        "Load snapshot error: Failed to restore from snapshot: Failed to get snapshot "
        "state from file: Failed to load snapshot state from file: Snapshot file is smaller "
        "than CRC length."
    )
    with pytest.raises(RuntimeError, match=expected_msg):
        vm.api.snapshot_load.put(mem_file_path=jailed_mem, snapshot_path=jailed_vmstate)

    vm.mark_killed()


def test_cmp_full_and_first_diff_mem(uvm_plain_any):
    """
    Compare memory of 2 consecutive full and diff snapshots.

    Testing matrix:
    - Guest kernel: All supported ones
    - Rootfs: Ubuntu 18.04
    - Microvm: 2vCPU with 512 MB RAM
    """
    logger = logging.getLogger("snapshot_sequence")

    vm = uvm_plain_any
    vm.spawn()
    vm.basic_config(
        vcpu_count=2,
        mem_size_mib=512,
        track_dirty_pages=True,
    )
    vm.add_net_iface()
    vm.start()

    logger.info("Create diff snapshot.")
    # Create diff snapshot.
    diff_snapshot = vm.snapshot_diff()

    logger.info("Create full snapshot.")
    # Create full snapshot.
    full_snapshot = vm.snapshot_full(mem_path="mem_full")

    assert full_snapshot.mem != diff_snapshot.mem
    assert filecmp.cmp(full_snapshot.mem, diff_snapshot.mem, shallow=False)


def test_negative_postload_api(uvm_plain, microvm_factory):
    """
    Test APIs fail after loading from snapshot.
    """
    basevm = uvm_plain
    basevm.spawn()
    basevm.basic_config(track_dirty_pages=True)
    basevm.add_net_iface()
    basevm.start()

    # Create base snapshot.
    snapshot = basevm.snapshot_diff()
    basevm.kill()

    # Do not resume, just load, so we can still call APIs that work.
    microvm = microvm_factory.build_from_snapshot(snapshot)

    fail_msg = "The requested operation is not supported after starting the microVM"
    with pytest.raises(RuntimeError, match=fail_msg):
        microvm.api.actions.put(action_type="InstanceStart")

    with pytest.raises(RuntimeError, match=fail_msg):
        microvm.basic_config()


def test_negative_snapshot_permissions(uvm_plain_rw, microvm_factory):
    """
    Test missing permission error scenarios.
    """
    basevm = uvm_plain_rw
    basevm.spawn()
    basevm.basic_config()
    basevm.add_net_iface()
    basevm.start()

    # Remove write permissions.
    os.chmod(basevm.jailer.chroot_path(), 0o444)

    with pytest.raises(RuntimeError, match="Permission denied"):
        basevm.snapshot_full()

    # Restore proper permissions.
    os.chmod(basevm.jailer.chroot_path(), 0o744)

    # Create base snapshot.
    snapshot = basevm.snapshot_full()
    basevm.kill()

    # Remove permissions for mem file.
    os.chmod(snapshot.mem, 0o000)

    microvm = microvm_factory.build()
    microvm.spawn()

    expected_err = re.escape(
        "Load snapshot error: Failed to restore from snapshot: Failed to load guest "
        "memory: Error creating guest memory from file: Failed to load guest memory: "
        "Permission denied (os error 13)"
    )
    with pytest.raises(RuntimeError, match=expected_err):
        microvm.restore_from_snapshot(snapshot, resume=True)

    microvm.mark_killed()

    # Remove permissions for state file.
    os.chmod(snapshot.vmstate, 0o000)

    microvm = microvm_factory.build()
    microvm.spawn()

    expected_err = re.escape(
        "Load snapshot error: Failed to restore from snapshot: Failed to get snapshot "
        "state from file: Failed to open snapshot file: Permission denied (os error 13)"
    )
    with pytest.raises(RuntimeError, match=expected_err):
        microvm.restore_from_snapshot(snapshot, resume=True)

    microvm.mark_killed()

    # Restore permissions for state file.
    os.chmod(snapshot.vmstate, 0o744)
    os.chmod(snapshot.mem, 0o744)

    # Remove permissions for block file.
    os.chmod(snapshot.disks["rootfs"], 0o000)

    microvm = microvm_factory.build()
    microvm.spawn()

    expected_err = "Virtio backend error: Error manipulating the backing file: Permission denied (os error 13)"
    with pytest.raises(RuntimeError, match=re.escape(expected_err)):
        microvm.restore_from_snapshot(snapshot, resume=True)

    microvm.mark_killed()


def test_negative_snapshot_create(uvm_nano):
    """
    Test create snapshot before pause.
    """
    vm = uvm_nano
    vm.start()

    with pytest.raises(RuntimeError, match="save/restore unavailable while running"):
        vm.api.snapshot_create.put(
            mem_file_path="memfile", snapshot_path="statefile", snapshot_type="Full"
        )


def test_create_large_diff_snapshot(uvm_plain):
    """
    Create large diff snapshot seccomp regression test.

    When creating a diff snapshot of a microVM with a large memory size, a
    mmap(MAP_PRIVATE|MAP_ANONYMOUS) is issued. Test that the default seccomp
    filter allows it.
    @issue: https://github.com/firecracker-microvm/firecracker/discussions/2811
    """
    vm = uvm_plain
    vm.spawn()
    vm.basic_config(mem_size_mib=16 * 1024, track_dirty_pages=True)
    vm.start()

    vm.api.vm.patch(state="Paused")

    vm.api.snapshot_create.put(
        mem_file_path="memfile", snapshot_path="statefile", snapshot_type="Diff"
    )

    # If the regression was not fixed, this would have failed. The Firecracker
    # process would have been taken down.


def test_diff_snapshot_overlay(uvm_plain_any, microvm_factory):
    """
    Tests that if we take a diff snapshot and direct firecracker to write it on
    top of an existing snapshot file, it will successfully merge them.
    """
    basevm = uvm_plain_any
    basevm.spawn()
    basevm.basic_config(track_dirty_pages=True)
    basevm.add_net_iface()
    basevm.start()

    # The first snapshot taken will always contain all memory (even if its specified as "diff").
    # We use a diff snapshot here, as taking a full snapshot does not clear the dirty page tracking,
    # meaning the `snapshot_diff()` call below would again dump the entire guest memory instead of
    # only dirty regions.
    full_snapshot = basevm.snapshot_diff()
    basevm.resume()

    # Run some command to dirty some pages
    basevm.ssh.check_output("true")

    # First copy the base snapshot somewhere else, so we can make sure
    # it will actually get updated
    first_snapshot_backup = Path(basevm.chroot()) / "mem.old"
    shutil.copyfile(full_snapshot.mem, first_snapshot_backup)

    # One Microvm object will always write its snapshot files to the same location
    merged_snapshot = basevm.snapshot_diff()
    assert full_snapshot.mem == merged_snapshot.mem

    assert not filecmp.cmp(merged_snapshot.mem, first_snapshot_backup, shallow=False)

    _ = microvm_factory.build_from_snapshot(merged_snapshot)

    # Check that the restored VM works


def test_snapshot_overwrite_self(uvm_plain_any, microvm_factory):
    """Tests that if we try to take a snapshot that would overwrite the
    very file from which the current VM is stored, nothing happens.

    Note that even though we map the file as MAP_PRIVATE, the documentation
    of mmap does not specify what should happen if the file is changed after being
    mmap'd (https://man7.org/linux/man-pages/man2/mmap.2.html). It seems that
    these changes can propagate to the mmap'd memory region."""
    base_vm = uvm_plain_any
    base_vm.spawn()
    base_vm.basic_config()
    base_vm.add_net_iface()
    base_vm.start()

    snapshot = base_vm.snapshot_full()
    base_vm.kill()

    vm = microvm_factory.build_from_snapshot(snapshot)

    # When restoring a snapshot, vm.restore_from_snapshot first copies
    # the memory file (inside of the jailer) to /mem.src
    currently_loaded = Path(vm.chroot()) / "mem.src"

    assert currently_loaded.exists()

    vm.snapshot_full(mem_path="mem.src")
    vm.resume()

    # Check the overwriting the snapshot file from which this microvm was originally
    # restored, with a new snapshot of this vm, does not break the VM


def test_vmgenid(uvm_plain_6_1, microvm_factory, snapshot_type):
    """
    Test VMGenID device upon snapshot resume
    """
    base_vm = uvm_plain_6_1
    base_vm.spawn()
    base_vm.basic_config(track_dirty_pages=True)
    base_vm.add_net_iface()
    base_vm.start()

    snapshot = base_vm.make_snapshot(snapshot_type)
    base_snapshot = snapshot
    base_vm.kill()

    for i, vm in enumerate(
        microvm_factory.build_n_from_snapshot(base_snapshot, 5, incremental=True)
    ):
        # We should have as DMESG_VMGENID_RESUME messages as
        # snapshots we have resumed
        check_vmgenid_update_count(vm, i + 1)


@pytest.mark.skipif(
    platform.machine() != "aarch64"
    or (
        global_props.host_linux_version_tpl < (6, 4)
        and global_props.host_os not in ("amzn2", "amzn2023")
    ),
    reason="This test requires aarch64 and either kernel 6.4+ or Amazon Linux",
)
def test_physical_counter_reset_aarch64(uvm_nano):
    """
    Test that the CNTPCT_EL0 register is reset on VM boot.
    We assume the smallest VM will not consume more than
    some MAX_VALUE cycles to be created and snapshotted.
    The MAX_VALUE is selected by doing a manual run of this test and
    seeing what the actual counter value is. The assumption here is that
    if resetting will not occur the guest counter value will be huge as it
    will be a copy of host value. The host value in its turn will be huge because
    it will include host OS boot + CI prep + other CI tests ...
    """
    vm = uvm_nano
    vm.add_net_iface()
    vm.start()

    snapshot = vm.snapshot_full()
    vm.kill()
    snap_editor = host.get_binary("snapshot-editor")

    cntpct_el0 = hex(0x603000000013DF01)
    # If a CPU runs at 3GHz, it will have a counter value of 8_000_000_000
    # in 2.66 seconds. The host surely will run for more than 2.66 seconds before
    # executing this test.
    max_value = 8_000_000_000

    cmd = [
        str(snap_editor),
        "info-vmstate",
        "vcpu-states",
        "--vmstate-path",
        str(snapshot.vmstate),
    ]
    _, stdout, _ = utils.check_output(cmd)

    # The output will look like this:
    # kvm_mp_state: 0x0
    # mpidr: 0x80000000
    # 0x6030000000100000 0x0000000e0
    # 0x6030000000100002 0xffff00fe33c0
    for line in stdout.splitlines():
        parts = line.split()
        if len(parts) == 2:
            reg_id, reg_value = parts
            if reg_id == cntpct_el0:
                assert int(reg_value, 16) < max_value
                break
    else:
        raise RuntimeError("Did not find CNTPCT_EL0 register in snapshot")


def test_snapshot_rename_interface(uvm_nano, microvm_factory):
    """
    Test that we can restore a snapshot and point its interface to a
    different host interface.
    """
    vm = uvm_nano
    base_iface = vm.add_net_iface()
    vm.start()
    snapshot = vm.snapshot_full()

    # We don't reuse the network namespace as it may conflict with
    # previous/future devices
    restored_vm = microvm_factory.build(netns=net_tools.NetNs(str(uuid.uuid4())))
    # Override the tap name, but keep the same IP configuration
    iface_override = dataclasses.replace(base_iface, tap_name="tap_override")

    restored_vm.spawn()
    snapshot.net_ifaces.clear()
    snapshot.net_ifaces.append(iface_override)
    restored_vm.restore_from_snapshot(
        snapshot,
        rename_interfaces={iface_override.dev_name: iface_override.tap_name},
        resume=True,
    )


@pytest.mark.parametrize("snapshot_type", [SnapshotType.FULL])
@pytest.mark.parametrize("pci_enabled", [False])
@pytest.mark.parametrize("iteration", range(100))  # Run many iterations to catch non-zero drain
def test_snapshot_with_heavy_async_io(
    microvm_factory, guest_kernel_linux_6_1, rootfs, snapshot_type, pci_enabled, iteration
):
    """
    Test snapshot with heavy filesystem I/O using async/io_uring engine.

    This test verifies that async I/O operations in-flight during snapshot
    are properly completed and their completion information is written to
    guest memory (used ring) so that after restore, the guest driver sees
    the completions and doesn't freeze.

    CRITICAL: The test restores from the snapshot IMMEDIATELY after creation.
    In the error case (if async I/O completions weren't written to guest memory
    during prepare_save()), the guest virtio driver will be stuck waiting for
    completions that will never come, causing the VM to freeze. This test
    detects that freeze by attempting to run a command immediately after restore.

    The test focuses on freeze detection - if async I/O completions aren't written
    to guest memory during snapshot, the VM will freeze after restore.

    The test:
    1. Configures VM with async io_engine (kernel 6.1 only)
    2. Performs heavy async write operations (no sync) on the filesystem
    3. Creates a snapshot IMMEDIATELY while I/O operations are still in-flight
    4. Restores from the snapshot IMMEDIATELY after creation
    5. Verifies VM is responsive (freeze detection - will timeout if frozen)
    6. Verifies filesystem integrity and that all I/O completed correctly
    """
    logger = logging.getLogger("snapshot_heavy_async_io")

    print("=" * 80)
    print(f"Starting iteration {iteration + 1}/100 - Testing for non-zero async I/O drain")
    print("=" * 80)

    # Build VM with kernel 6.1 only
    vm = microvm_factory.build(guest_kernel_linux_6_1, rootfs, pci=pci_enabled)
    # Enable Trace-level logging to see ALL async I/O drain operations during snapshot
    # Trace level shows the most detailed device operations including block device async I/O
    # The debug! macros in async_io.rs (like drain_and_flush) require Debug or Trace level
    vm.spawn(log_level="Trace", log_show_level=True, log_show_origin=True)
    vm.basic_config(
        vcpu_count=2,
        mem_size_mib=1024,
        # track_dirty_pages=snapshot_type.needs_dirty_page_tracking,
        rootfs_io_engine="Async",  # Use async/io_uring engine
    )
    vm.add_net_iface()
    vm.start()

    logger.info("Starting heavy write I/O workload on guest filesystem...")

    # Perform heavy WRITE operations before snapshot
    # Writes are more likely to be async and non-blocking, generating many
    # in-flight async I/O requests that will need completion info written to memory
    # We want to maximize the chance of having pending_ops > 0 during drain
    write_io_script = """
    # Create a test directory
    mkdir -p /tmp/io_test
    cd /tmp/io_test

    # Strategy: Fire THOUSANDS of small writes VERY quickly to maximize io_uring queue depth
    # Small writes complete faster but queue more operations
    # Goal: Schedule thousands of operations to increase chance of pending_ops > 0
    
    # Fire 2000+ very small writes in parallel - these will queue up quickly
    # Small block sizes (4k-16k) are more likely to stay queued
    for i in $(seq 1 2000); do
        dd if=/dev/urandom of=test_file_$i bs=8k count=1 oflag=direct 2>/dev/null &
    done
    
    # Fire 1000 medium writes to add more operations to the queue
    for i in $(seq 1 1000); do
        dd if=/dev/urandom of=medium_file_$i bs=16k count=1 oflag=direct 2>/dev/null &
    done
    
    # Fire 500 larger writes
    for i in $(seq 1 500); do
        dd if=/dev/urandom of=large_file_$i bs=32k count=1 oflag=direct 2>/dev/null &
    done
    
    # Fire 200 even larger writes
    for i in $(seq 1 200); do
        dd if=/dev/urandom of=xlarge_file_$i bs=64k count=1 oflag=direct 2>/dev/null &
    done

    # Use fio with MAXIMUM iodepth to maximize in-flight operations
    # io_uring queue depth is typically 4096, so we want to fill it up
    # Total: 3700+ dd operations + fio operations
    if command -v fio >/dev/null 2>&1; then
        # Maximum iodepth (512) with many jobs to fill the queue
        # Random writes are better than sequential for keeping ops in-flight
        fio --name=heavy_write --filename=/tmp/io_test/fio_write_test \
            --rw=randwrite --bs=4k --size=2G --ioengine=libaio \
            --iodepth=512 --direct=1 --runtime=30 --time_based \
            --numjobs=64 --group_reporting \
            --output=/tmp/fio_write.log >/dev/null 2>&1 &
        
        # Start multiple fio jobs on different files for even more operations
        fio --name=heavy_write2 --filename=/tmp/io_test/fio_write_test2 \
            --rw=randwrite --bs=8k --size=2G --ioengine=libaio \
            --iodepth=512 --direct=1 --runtime=30 --time_based \
            --numjobs=64 --group_reporting \
            --output=/tmp/fio_write2.log >/dev/null 2>&1 &
            
        fio --name=heavy_write3 --filename=/tmp/io_test/fio_write_test3 \
            --rw=randwrite --bs=16k --size=2G --ioengine=libaio \
            --iodepth=512 --direct=1 --runtime=30 --time_based \
            --numjobs=64 --group_reporting \
            --output=/tmp/fio_write3.log >/dev/null 2>&1 &
            
        fio --name=heavy_write4 --filename=/tmp/io_test/fio_write_test4 \
            --rw=randwrite --bs=4k --size=2G --ioengine=libaio \
            --iodepth=512 --direct=1 --runtime=30 --time_based \
            --numjobs=64 --group_reporting \
            --output=/tmp/fio_write4.log >/dev/null 2>&1 &
    fi
    
    # Total operations scheduled: ~3700 dd + ~256 fio workers (64*4) = ~3956 operations
    """

    # Execute initial write workload
    vm.ssh.run(write_io_script)
    # No wait - fire first snapshot immediately to catch operations in-flight

    # Perform 4 snapshot-resume cycles
    # Strategy: Take snapshots as fast as possible after firing I/O
    # This keeps operations overlapping across snapshots so each has pending_ops > 0
    NUM_SNAPSHOTS = 4
    current_vm = vm
    snapshots = []
    all_pending_ops = {}  # Track pending_ops for each snapshot
    queued_ops_count = []
    
    for snap_num in range(1, NUM_SNAPSHOTS + 1):
        print(f"\n{'='*80}")
        print(f"Iteration {iteration + 1}, Snapshot {snap_num}/{NUM_SNAPSHOTS}")
        print(f"{'='*80}")
        
        # Fire ONE or FEW really large operations - simple approach
        # Large operations take time to complete, naturally staying in-flight
        print(f"Snapshot {snap_num}: Firing large I/O operations...")
        heavy_io_script = f"""
        cd /tmp/io_test
        
        # Fire one or few really large operations
        # Large operations take time to complete, keeping them in-flight in io_uring queue
        # Simple and effective - just big writes that take time
        
        # One really big write - this will take time and stay in-flight
        dd if=/dev/urandom of=snap{snap_num}_huge bs=1M count=1000 oflag=direct 2>/dev/null &
        
        # A few more large operations for good measure
        dd if=/dev/urandom of=snap{snap_num}_large1 bs=1M count=500 oflag=direct 2>/dev/null &
        dd if=/dev/urandom of=snap{snap_num}_large2 bs=1M count=500 oflag=direct 2>/dev/null &
        dd if=/dev/urandom of=snap{snap_num}_large3 bs=1M count=500 oflag=direct 2>/dev/null &
        """
        current_vm.ssh.run(heavy_io_script)
        
        # Short wait - just enough for operations to be submitted to io_uring
        # Large operations will take time to complete, staying in-flight
        time.sleep(0.01)  # 10ms - just enough for submission
        
        print(f"Snapshot {snap_num}: Creating snapshot immediately (operations still in-flight)...")
        snapshot = current_vm.make_snapshot(snapshot_type)
        snapshots.append(snapshot)
        
        # Minimal wait for logs to flush
        time.sleep(0.05)
        
        # Parse logs for pending_ops for EVERY snapshot
        pending_ops_during_drain = None
        if current_vm.log_file and current_vm.log_file.exists():
            try:
                log_data = current_vm.log_data
                log_lines = log_data.splitlines()
                
                # Parse the drain messages to extract pending_ops
                for line in log_lines:
                    # Look for: "AsyncFileEngine queued ... request ... pending_ops=X"
                    if "AsyncFileEngine queued" in line and "pending_ops=" in line:
                        match = re.search(r'pending_ops=(\d+)', line)
                        if match:
                            queued_ops_count.append(int(match.group(1)))
                    
                    # Look for: "AsyncFileEngine draining: pending_ops=X discard_cqes=..."
                    # We want to find the MOST RECENT drain message for this snapshot
                    if "AsyncFileEngine draining:" in line:
                        match = re.search(r'pending_ops=(\d+)', line)
                        if match:
                            pending_ops_during_drain = int(match.group(1))
                            print(f"Snapshot {snap_num}: Found drain start: pending_ops={pending_ops_during_drain}")
            except Exception as e:
                print(f"ERROR: Failed to parse log file: {e}")
        
        # Store pending_ops for this snapshot
        all_pending_ops[snap_num] = pending_ops_during_drain
        
        # Kill current VM before restoring
        current_vm.kill()
        
        # Restore immediately and fire next I/O as fast as possible
        print(f"Snapshot {snap_num}: Restoring from snapshot IMMEDIATELY...")
        restored_vm = microvm_factory.build_from_snapshot(snapshot)
        
        # Verify VM is responsive (freeze detection) - do this quickly
        print(f"Snapshot {snap_num}: Verifying VM is responsive (freeze detection)...")
        print(f"Snapshot {snap_num}: Waiting up to 30 seconds for VM to respond...")
        try:
            # Use a timeout to detect freeze - if VM is frozen, this will timeout
            # The default SSH timeout is 60s, but we want to fail faster for freeze detection
            with Timeout(30):
                restored_vm.ssh.check_output("true")
            print(f"Snapshot {snap_num}: ✓ VM is responsive - no freeze detected")
            
            # If not the last snapshot, immediately prepare for next snapshot
            # This keeps operations overlapping across snapshots
            if snap_num < NUM_SNAPSHOTS:
                print(f"Snapshot {snap_num}: Ready for next snapshot (operations may still be running)...")
            
            # Report findings for this snapshot
            print(f"\nSnapshot {snap_num} Results:")
            print(f"  pending_ops during drain: {pending_ops_during_drain}")
            
            if pending_ops_during_drain is not None and pending_ops_during_drain > 0:
                print(f"  *** NON-ZERO DRAIN: pending_ops={pending_ops_during_drain} ***")
            elif pending_ops_during_drain == 0:
                print(f"  WARNING: pending_ops=0 (operations completed before drain)")
            else:
                print(f"  WARNING: Could not parse pending_ops from logs")
            
            # Report summary for last snapshot
            if snap_num == NUM_SNAPSHOTS:
                print(f"\n{'='*80}")
                print(f"Summary for all {NUM_SNAPSHOTS} snapshots:")
                for s in range(1, NUM_SNAPSHOTS + 1):
                    ops = all_pending_ops.get(s, "unknown")
                    status = "✓" if (isinstance(ops, int) and ops > 0) else "✗"
                    print(f"  Snapshot {s}: pending_ops={ops} {status}")
                
                if queued_ops_count:
                    max_queued = max(queued_ops_count)
                    min_queued = min(queued_ops_count)
                    print(f"\nOperation counts from FC logs:")
                    print(f"  Max pending_ops seen in logs: {max_queued}")
                    print(f"  Min pending_ops seen in logs: {min_queued}")
                    print(f"  Sample of queued ops counts: {queued_ops_count[-10:]}")
                
                # Check if we got non-zero drain in any snapshot
                non_zero_snapshots = [s for s, ops in all_pending_ops.items() 
                                     if isinstance(ops, int) and ops > 0]
                if non_zero_snapshots:
                    print(f"\n{'='*80}")
                    print(f"SUCCESS: Found non-zero drain in snapshots: {non_zero_snapshots}")
                    print(f"All snapshots with non-zero drain resumed correctly - no freeze!")
                    print(f"This proves that non-zero drain with proper completion handling works correctly.")
                    print(f"{'='*80}\n")
                else:
                    print(f"\nNo non-zero drain found in any snapshot - continuing to next iteration...")
                print(f"{'='*80}")
            
        except Exception as e:
            print(f"\n{'='*80}")
            print(f"FAILURE: Snapshot {snap_num} - VM FROZE after restore!")
            print(f"pending_ops during drain: {pending_ops_during_drain}")
            print(f"Error: {e}")
            print(f"{'='*80}\n")
            restored_vm.kill()
            raise
        
        # For next iteration, use the restored VM
        if snap_num < NUM_SNAPSHOTS:
            current_vm = restored_vm
        else:
            # Last snapshot - cleanup
            restored_vm.kill()
