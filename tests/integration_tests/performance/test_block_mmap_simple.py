#!/usr/bin/env python3
"""Compare mmap performance for different memory backing strategies.

This simulates the different ways Firecracker can serve guest memory:

1. ANONYMOUS (baseline) - Pure anonymous memory, best THP support
2. FILE_PRIVATE (normal FC snapshot restore) - MAP_PRIVATE on file
   - On write, pages become anonymous via CoW, so THP works!
3. FILE_SHARED (msync/live migration) - MAP_SHARED on file
   - Pages stay file-backed, THP needs CONFIG_READ_ONLY_THP_FOR_FS
4. BLOCK_PRIVATE - MAP_PRIVATE on block device (loop)
5. BLOCK_SHARED - MAP_SHARED on block device (loop)

Run directly (NOT through devtool):
    sudo python3 tests/integration_tests/performance/test_block_mmap_simple.py
"""

import ctypes
import mmap
import os
import subprocess
import tempfile
import time

# Size of test memory region (1 GB to match FC test workload)
SIZE = 1 * 1024 * 1024 * 1024

# libc for madvise
libc = ctypes.CDLL("libc.so.6", use_errno=True)
MADV_HUGEPAGE = 14


def get_thp_stats():
    """Get THP stats from /proc/meminfo."""
    with open("/proc/meminfo") as f:
        meminfo = f.read()
    stats = {}
    for line in meminfo.splitlines():
        if "Huge" in line or "Thp" in line.lower():
            parts = line.split()
            if len(parts) >= 2:
                stats[parts[0].rstrip(":")] = int(parts[1])
    return stats


def create_loop_device(file_path: str) -> str:
    """Create a loop device over a file."""
    result = subprocess.run(
        ["losetup", "-f", "--show", file_path],
        capture_output=True,
        text=True,
        check=True,
    )
    return result.stdout.strip()


def detach_loop_device(loop_dev: str):
    """Detach a loop device."""
    subprocess.run(["losetup", "-d", loop_dev], check=False)


def touch_all_pages(mm: mmap.mmap, size: int) -> float:
    """Touch all pages by writing to them. Returns time in seconds."""
    start = time.perf_counter()
    # Write 1 byte per 4KB page (simulates guest memory access pattern)
    for offset in range(0, size, 4096):
        mm[offset] = 0x42
    elapsed = time.perf_counter() - start
    return elapsed


def call_madvise_hugepage(mm: mmap.mmap, size: int) -> tuple:
    """Call madvise(MADV_HUGEPAGE) on the mmap region."""
    mv = memoryview(mm)
    c_buf = (ctypes.c_char * len(mv)).from_buffer(mv)
    addr = ctypes.addressof(c_buf)

    ret = libc.madvise(ctypes.c_void_p(addr), ctypes.c_size_t(size), MADV_HUGEPAGE)

    del c_buf
    mv.release()

    if ret != 0:
        errno = ctypes.get_errno()
        return False, errno
    return True, 0


def test_anonymous(use_madvise: bool = True) -> dict:
    """Test anonymous mmap - pure baseline, best THP support."""
    flags = mmap.MAP_PRIVATE | mmap.MAP_ANONYMOUS

    thp_before = get_thp_stats()
    mm = mmap.mmap(-1, SIZE, flags=flags, prot=mmap.PROT_READ | mmap.PROT_WRITE)

    madvise_ok = None
    if use_madvise:
        ok, errno = call_madvise_hugepage(mm, SIZE)
        madvise_ok = "OK" if ok else f"err={errno}"

    elapsed = touch_all_pages(mm, SIZE)
    thp_after = get_thp_stats()
    mm.close()

    return {
        "elapsed": elapsed,
        "anon_hp_mb": (
            thp_after.get("AnonHugePages", 0) - thp_before.get("AnonHugePages", 0)
        )
        / 1024,
        "file_hp_mb": (
            thp_after.get("FileHugePages", 0) - thp_before.get("FileHugePages", 0)
        )
        / 1024,
        "madvise": madvise_ok,
    }


def test_file_backed(path: str, use_shared: bool, use_madvise: bool = True) -> dict:
    """Test file-backed mmap."""
    flags = mmap.MAP_SHARED if use_shared else mmap.MAP_PRIVATE
    fd = os.open(path, os.O_RDWR)

    try:
        thp_before = get_thp_stats()
        mm = mmap.mmap(fd, SIZE, flags=flags, prot=mmap.PROT_READ | mmap.PROT_WRITE)

        madvise_ok = None
        if use_madvise:
            ok, errno = call_madvise_hugepage(mm, SIZE)
            madvise_ok = "OK" if ok else f"err={errno}"

        elapsed = touch_all_pages(mm, SIZE)

        if use_shared:
            mm.flush()

        thp_after = get_thp_stats()
        mm.close()

        return {
            "elapsed": elapsed,
            "anon_hp_mb": (
                thp_after.get("AnonHugePages", 0) - thp_before.get("AnonHugePages", 0)
            )
            / 1024,
            "file_hp_mb": (
                thp_after.get("FileHugePages", 0) - thp_before.get("FileHugePages", 0)
            )
            / 1024,
            "madvise": madvise_ok,
        }
    finally:
        os.close(fd)


def print_result(name: str, result: dict, fc_equivalent: str = ""):
    """Print a single test result."""
    print(f"\n{name}:")
    if fc_equivalent:
        print(f"  FC equivalent: {fc_equivalent}")
    print(f"  Time: {result['elapsed']:.3f}s")
    print(f"  AnonHugePages: {result['anon_hp_mb']:.0f} MB")
    print(f"  FileHugePages: {result['file_hp_mb']:.0f} MB")
    if result.get("madvise"):
        print(f"  MADV_HUGEPAGE: {result['madvise']}")


def main():
    print("=" * 70)
    print("Firecracker Memory Backend Performance Comparison")
    print("=" * 70)
    print(f"Test size: {SIZE / (1024**3):.1f} GB (simulating 1GB memory workload)")
    print()

    # Check THP settings
    try:
        with open("/sys/kernel/mm/transparent_hugepage/enabled") as f:
            thp_enabled = f.read().strip()
        print(f"THP enabled: {thp_enabled}")
        with open("/sys/kernel/mm/transparent_hugepage/defrag") as f:
            thp_defrag = f.read().strip()
        print(f"THP defrag: {thp_defrag}")
    except Exception as e:
        print(f"THP: could not read status: {e}")

    print()
    results = {}

    # =========================================================================
    # Test 1: Anonymous memory (pure baseline)
    # =========================================================================
    print("-" * 70)
    print("1. ANONYMOUS MEMORY (baseline - represents fresh VM start)")
    print("-" * 70)

    result = test_anonymous(use_madvise=True)
    results["anonymous"] = result
    print_result("anonymous", result, "Fresh VM start with anonymous memory")

    # =========================================================================
    # Create test file
    # =========================================================================
    with tempfile.NamedTemporaryFile(delete=False, suffix=".mem") as f:
        temp_file = f.name
        print(f"\nCreating {SIZE / (1024**3):.1f} GB test file: {temp_file}")
        f.truncate(SIZE)
        f.flush()
        os.fsync(f.fileno())

    try:
        # =====================================================================
        # Test 2: FILE + MAP_PRIVATE (normal FC snapshot restore)
        # =====================================================================
        print()
        print("-" * 70)
        print("2. FILE + MAP_PRIVATE (normal FC snapshot restore)")
        print("   Pages become ANONYMOUS on write (CoW) -> THP should work!")
        print("-" * 70)

        result = test_file_backed(temp_file, use_shared=False, use_madvise=True)
        results["file_private"] = result
        print_result("file_private", result, "restore_from_snapshot(shared=False)")

        # =====================================================================
        # Test 3: FILE + MAP_SHARED (msync/live migration)
        # =====================================================================
        print()
        print("-" * 70)
        print("3. FILE + MAP_SHARED (msync/live migration mode)")
        print("   Pages stay FILE-BACKED -> THP needs CONFIG_READ_ONLY_THP_FOR_FS")
        print("-" * 70)

        result = test_file_backed(temp_file, use_shared=True, use_madvise=True)
        results["file_shared"] = result
        print_result("file_shared", result, "restore_from_snapshot(shared=True)")

        # =====================================================================
        # Test 4: BLOCK DEVICE tests
        # =====================================================================
        print()
        print("-" * 70)
        print("4. BLOCK DEVICE (loop) tests - for ublk use case")
        print("-" * 70)

        loop_dev = create_loop_device(temp_file)
        print(f"Created loop device: {loop_dev}")

        try:
            result = test_file_backed(loop_dev, use_shared=False, use_madvise=True)
            results["block_private"] = result
            print_result("block_private", result, "ublk + shared=False")

            result = test_file_backed(loop_dev, use_shared=True, use_madvise=True)
            results["block_shared"] = result
            print_result("block_shared", result, "ublk + shared=True")
        finally:
            detach_loop_device(loop_dev)
            print(f"\nDetached loop device: {loop_dev}")

        # =====================================================================
        # Summary
        # =====================================================================
        print()
        print("=" * 70)
        print("SUMMARY")
        print("=" * 70)
        print()
        print("Legend:")
        print("  - AnonHP: Anonymous Huge Pages (from THP on anonymous/CoW pages)")
        print("  - FileHP: File Huge Pages (from THP on file-backed pages)")
        print("  - Expected: 1024 MB if THP is working for 1GB region")
        print()
        print(
            f"{'Scenario':<20} {'Time(s)':<10} {'AnonHP(MB)':<12} {'FileHP(MB)':<12} {'madvise':<10}"
        )
        print("-" * 70)
        for name, result in results.items():
            madvise_str = result.get("madvise", "-") or "-"
            print(
                f"{name:<20} {result['elapsed']:<10.3f} {result['anon_hp_mb']:<12.0f} {result['file_hp_mb']:<12.0f} {madvise_str:<10}"
            )

        print()
        print("=" * 70)
        print("ANALYSIS")
        print("=" * 70)
        anon_time = results["anonymous"]["elapsed"]
        print(f"\nBaseline (anonymous): {anon_time:.3f}s")
        print()
        for name, result in results.items():
            if name != "anonymous":
                slowdown = (result["elapsed"] - anon_time) / anon_time * 100
                thp_worked = result["anon_hp_mb"] > 500 or result["file_hp_mb"] > 500
                print(
                    f"{name}: {result['elapsed']:.3f}s ({slowdown:+.1f}% vs anon), THP: {'YES' if thp_worked else 'NO'}"
                )

    finally:
        os.unlink(temp_file)
        print(f"\nCleaned up: {temp_file}")


if __name__ == "__main__":
    if os.geteuid() != 0:
        print("This script needs to be run as root (for loop device creation)")
        print("Run: sudo python3", __file__)
        exit(1)
    main()
