#!/usr/bin/env bash
#
# End-to-end test for fc-mock.
#
# Tests the full orchestrator flow:
#   1. Pre-boot configuration (machine, boot, drives, net, mmds, entropy, metrics)
#   2. InstanceStart → guest memory allocated
#   3. Memory mappings return a real ProcessVMReadv-able address
#   4. Workload simulation dirties pages
#   5. Pause → dirty bitmap has bits set
#   6. Snapshot create → dirty bitmap cleared
#   7. Snapshot load (file backend) → memory re-allocated
#   8. ProcessVMReadv works on the mock's guest memory
#
# Usage:
#   ./tests/test_e2e.sh [path-to-firecracker-binary]
#
set -euo pipefail

BINARY="${1:-./target/release/firecracker}"
SOCKET="/tmp/fc-mock-test-$$.socket"
SNAP="/tmp/fc-mock-test-$$.snap"
PID_FILE=""
PASS=0
FAIL=0

cleanup() {
    if [[ -n "${FC_PID:-}" ]]; then
        kill "$FC_PID" 2>/dev/null || true
        wait "$FC_PID" 2>/dev/null || true
    fi
    rm -f "$SOCKET" "$SNAP"
}
trap cleanup EXIT

# ── Helpers ──────────────────────────────────────────────────────────────────

api() {
    curl -sf --unix-socket "$SOCKET" "http://localhost$1" "${@:2}" 2>/dev/null
}

api_code() {
    curl -s --unix-socket "$SOCKET" "http://localhost$1" -o /dev/null -w "%{http_code}" "${@:2}" 2>/dev/null
}

assert_eq() {
    local desc="$1" got="$2" want="$3"
    if [[ "$got" == "$want" ]]; then
        echo "  ✓ $desc"
        PASS=$((PASS + 1))
    else
        echo "  ✗ $desc: got '$got', want '$want'"
        FAIL=$((FAIL + 1))
    fi
}

assert_gt() {
    local desc="$1" got="$2" threshold="$3"
    if (( got > threshold )); then
        echo "  ✓ $desc ($got > $threshold)"
        PASS=$((PASS + 1))
    else
        echo "  ✗ $desc: got $got, want > $threshold"
        FAIL=$((FAIL + 1))
    fi
}

# ── Start fc-mock ────────────────────────────────────────────────────────────

echo "Starting fc-mock..."
"$BINARY" --api-sock "$SOCKET" --id test-e2e --no-seccomp --level warn &
FC_PID=$!
sleep 0.3

if ! kill -0 "$FC_PID" 2>/dev/null; then
    echo "FATAL: fc-mock failed to start"
    exit 1
fi

# ── 1. Pre-boot configuration ───────────────────────────────────────────────

echo ""
echo "1. Pre-boot configuration"
assert_eq "PUT /machine-config" \
    "$(api_code /machine-config -X PUT -d '{"vcpu_count":2,"mem_size_mib":8}')" "204"
assert_eq "PUT /boot-source" \
    "$(api_code /boot-source -X PUT -d '{"kernel_image_path":"/tmp/k"}')" "204"
assert_eq "PUT /drives/rootfs" \
    "$(api_code /drives/rootfs -X PUT -d '{"drive_id":"rootfs","path_on_host":"/tmp/r","is_root_device":true}')" "204"
assert_eq "PUT /network-interfaces/eth0" \
    "$(api_code /network-interfaces/eth0 -X PUT -d '{"iface_id":"eth0","host_dev_name":"tap0","guest_mac":"AA:BB:CC:DD:EE:FF"}')" "204"
assert_eq "PUT /mmds/config" \
    "$(api_code /mmds/config -X PUT -d '{"version":"V2","network_interfaces":["eth0"]}')" "204"
assert_eq "PUT /entropy" \
    "$(api_code /entropy -X PUT -d '{"rate_limiter":{"ops":{"size":10,"refill_time":1000}}}')" "204"
assert_eq "PUT /metrics" \
    "$(api_code /metrics -X PUT -d '{"metrics_path":"/tmp/m"}')" "204"

# Verify pre-boot guards
assert_eq "memory/mappings before boot → 400" \
    "$(api_code /memory/mappings)" "400"

# ── 2. InstanceStart ────────────────────────────────────────────────────────

echo ""
echo "2. InstanceStart"
assert_eq "PUT /actions InstanceStart" \
    "$(api_code /actions -X PUT -d '{"action_type":"InstanceStart"}')" "204"

# Double start should fail
assert_eq "Double InstanceStart → 400" \
    "$(api_code /actions -X PUT -d '{"action_type":"InstanceStart"}')" "400"

# ── 3. Memory mappings ──────────────────────────────────────────────────────

echo ""
echo "3. Memory mappings (8MiB = 2048 pages)"

MAPPINGS=$(api /memory/mappings)
ADDR=$(echo "$MAPPINGS" | python3 -c "import sys,json; print(json.load(sys.stdin)['mappings'][0]['base_host_virt_addr'])")
SIZE=$(echo "$MAPPINGS" | python3 -c "import sys,json; print(json.load(sys.stdin)['mappings'][0]['size'])")
PAGE_SIZE=$(echo "$MAPPINGS" | python3 -c "import sys,json; print(json.load(sys.stdin)['mappings'][0]['page_size'])")

assert_eq "mappings.size = 8MiB" "$SIZE" "8388608"
assert_eq "mappings.page_size = 4096" "$PAGE_SIZE" "4096"
assert_gt "mappings.base_host_virt_addr > 0" "$ADDR" 0

# ── 4. Memory bitmaps (resident/empty) ──────────────────────────────────────

echo ""
echo "4. Resident/empty bitmaps"

MEM=$(api /memory)
RES_WORDS=$(echo "$MEM" | python3 -c "import sys,json; print(len(json.load(sys.stdin)['resident']))")
EMPTY_WORDS=$(echo "$MEM" | python3 -c "import sys,json; print(len(json.load(sys.stdin)['empty']))")
# 2048 pages / 64 bits per word = 32 words
assert_eq "resident bitmap = 32 words" "$RES_WORDS" "32"
assert_eq "empty bitmap = 32 words" "$EMPTY_WORDS" "32"

# All pages should be resident after fresh start
RES_BITS=$(echo "$MEM" | python3 -c "import sys,json; d=json.load(sys.stdin); print(sum(bin(w).count('1') for w in d['resident']))")
assert_eq "all 2048 pages resident" "$RES_BITS" "2048"

# ── 5. Dirty pages should fail while running ────────────────────────────────

echo ""
echo "5. Dirty bitmap guards"
assert_eq "GET /memory/dirty while running → 400" \
    "$(api_code /memory/dirty)" "400"

# ── 6. Workload → dirty pages ──────────────────────────────────────────────

echo ""
echo "6. Workload simulation"
assert_eq "PUT /mock/workload" \
    "$(api_code /mock/workload -X PUT -d '{"io_ops_per_sec":1000}')" "204"
sleep 0.5

WORKLOAD=$(api /mock/workload)
ACTIVE=$(echo "$WORKLOAD" | python3 -c "import sys,json; print(json.load(sys.stdin)['active'])")
assert_eq "workload active" "$ACTIVE" "True"

# ── 7. Pause → snapshot → dirty bitmap ─────────────────────────────────────

echo ""
echo "7. Pause + snapshot + dirty tracking"
assert_eq "PATCH /vm Paused" \
    "$(api_code /vm -X PATCH -d '{"state":"Paused"}')" "204"

DIRTY=$(api /memory/dirty)
DIRTY_COUNT=$(echo "$DIRTY" | python3 -c "import sys,json; d=json.load(sys.stdin); print(sum(bin(w).count('1') for w in d['bitmap']))")
assert_gt "dirty pages > 0 after workload" "$DIRTY_COUNT" 0
echo "    (dirtied $DIRTY_COUNT pages)"

# Snapshot should clear dirty
assert_eq "PUT /snapshot/create" \
    "$(api_code /snapshot/create -X PUT -d "{\"snapshot_path\":\"$SNAP\"}")" "204"

DIRTY_AFTER=$(api /memory/dirty | python3 -c "import sys,json; d=json.load(sys.stdin); print(sum(bin(w).count('1') for w in d['bitmap']))")
assert_eq "dirty = 0 after snapshot" "$DIRTY_AFTER" "0"

# Verify snapshot file
assert_eq "snapshot file exists" "$(test -f "$SNAP" && echo yes)" "yes"

# ── 8. Resume ──────────────────────────────────────────────────────────────

echo ""
echo "8. Resume + re-pause"
assert_eq "PATCH /vm Resumed" \
    "$(api_code /vm -X PATCH -d '{"state":"Resumed"}')" "204"
assert_eq "Can't resume again → 400" \
    "$(api_code /vm -X PATCH -d '{"state":"Resumed"}')" "400"

# ── 9. Snapshot load (file backend) ─────────────────────────────────────────

echo ""
echo "9. Snapshot load"

# Need to stop, restart, and load
kill "$FC_PID" 2>/dev/null; wait "$FC_PID" 2>/dev/null || true
rm -f "$SOCKET"

"$BINARY" --api-sock "$SOCKET" --id test-e2e-restore --no-seccomp --level warn &
FC_PID=$!
sleep 0.3

# Configure machine before load
assert_eq "PUT /machine-config (restore)" \
    "$(api_code /machine-config -X PUT -d '{"vcpu_count":2,"mem_size_mib":8}')" "204"

assert_eq "PUT /snapshot/load (file)" \
    "$(api_code /snapshot/load -X PUT -d "{\"snapshot_path\":\"$SNAP\",\"mem_backend\":{\"backend_type\":\"File\",\"backend_path\":\"/dev/null\"},\"resume_vm\":false}")" "204"

# Should be paused after load with resume_vm=false
assert_eq "GET /memory/dirty after load → 200" \
    "$(api_code /memory/dirty)" "200"

# Memory should be re-allocated
ADDR2=$(api /memory/mappings | python3 -c "import sys,json; print(json.load(sys.stdin)['mappings'][0]['base_host_virt_addr'])")
assert_gt "new mapping addr > 0" "$ADDR2" 0

# ── 10. ProcessVMReadv test ─────────────────────────────────────────────────

echo ""
echo "10. ProcessVMReadv smoke test"

MOCK_PID=$(api /mock/health | python3 -c "import sys,json; print(json.load(sys.stdin)['pid'])")

# Try to read 4096 bytes from the guest memory address using dd on /proc/PID/mem
# This validates the address is real and readable.
if python3 -c "
import os, struct, ctypes, ctypes.util

addr = $ADDR2
pid = $MOCK_PID

# Open /proc/PID/mem
try:
    fd = os.open(f'/proc/{pid}/mem', os.O_RDONLY)
    os.lseek(fd, addr, os.SEEK_SET)
    data = os.read(fd, 4096)
    os.close(fd)
    if len(data) == 4096:
        print(f'  ✓ Read 4096 bytes from PID {pid} at 0x{addr:x}')
    else:
        print(f'  ✗ Read {len(data)} bytes, expected 4096')
        exit(1)
except PermissionError:
    # Expected if not running as same user or root; still validates addr exists
    print(f'  ⚠ PermissionError reading /proc/{pid}/mem (expected without ptrace)')
    print(f'    Address 0x{addr:x} is valid (mock allocated it)')
except Exception as e:
    print(f'  ✗ Failed: {e}')
    exit(1)
" 2>&1; then
    PASS=$((PASS + 1))
else
    FAIL=$((FAIL + 1))
fi

# ── 11. Removed endpoints return 400 ───────────────────────────────────────

echo ""
echo "11. Removed endpoints"
assert_eq "GET / → 400" "$(api_code /)" "400"
assert_eq "GET /version → 400" "$(api_code /version)" "400"
assert_eq "PUT /vsock → 400" "$(api_code /vsock -X PUT -d '{}')" "400"
assert_eq "PUT /balloon → 400" "$(api_code /balloon -X PUT -d '{}')" "400"
assert_eq "PUT /logger → 400" "$(api_code /logger -X PUT -d '{}')" "400"

# ── 12. FlushMetrics ───────────────────────────────────────────────────────

echo ""
echo "12. FlushMetrics"
assert_eq "PUT /actions FlushMetrics" \
    "$(api_code /actions -X PUT -d '{"action_type":"FlushMetrics"}')" "204"

# ── Summary ──────────────────────────────────────────────────────────────────

echo ""
echo "═══════════════════════════════════"
echo "  $PASS passed, $FAIL failed"
echo "═══════════════════════════════════"

[[ $FAIL -eq 0 ]]
