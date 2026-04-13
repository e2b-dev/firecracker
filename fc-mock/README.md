# fc-mock

Drop-in Firecracker replacement for testing without KVM. Produces a binary
named `firecracker` that accepts the same CLI flags and exposes the
HTTP/Unix-socket API endpoints used by the
[e2b-dev/infra](https://github.com/e2b-dev/infra) orchestrator, but runs
no VM machinery.

Designed for environments where nested virtualization / KVM is unavailable,
to test the orchestration, proxy, messaging, and storage layers end-to-end.

## Building

```bash
cd fc-mock
CARGO_TARGET_DIR=target cargo build --release
# → target/release/firecracker  (3.1 MB)
```

## Implemented endpoints

Only the API surface that the e2b orchestrator
(`packages/orchestrator/pkg/sandbox/fc/client.go`) actually calls:

| Method | Path | Description |
|--------|------|-------------|
| PUT | `/boot-source` | Kernel path + boot args |
| PUT | `/machine-config` | vCPU count, memory size |
| PUT | `/drives/{id}` | Root drive config |
| PATCH | `/drives/{id}` | Drive rate limiter update |
| PUT | `/network-interfaces/{id}` | Network device |
| PATCH | `/network-interfaces/{id}` | Tx/Rx rate limiter update |
| PUT | `/mmds/config` | MMDS version + interfaces |
| PUT | `/mmds` | MMDS metadata |
| PUT | `/metrics` | Metrics path |
| PUT | `/entropy` | Entropy device |
| PUT | `/actions` | `InstanceStart`, `FlushMetrics` |
| PATCH | `/vm` | Pause / Resume |
| PUT | `/snapshot/create` | Snapshot state to file |
| PUT | `/snapshot/load` | Restore from snapshot (UFFD) |
| GET | `/memory/mappings` | Guest memory region mappings |
| GET | `/memory` | Resident/empty page bitmaps |
| GET | `/memory/dirty` | Dirty page bitmap (paused only) |

## Memory introspection

The memory endpoints return the exact JSON shapes the orchestrator expects
(matching the Go generated models in `packages/shared/pkg/fc/models/`):

- **`GET /memory/mappings`** → `{"mappings": [{"base_host_virt_addr", "offset", "page_size", "size"}]}`
  where `base_host_virt_addr` is a **real address** in the mock process —
  the orchestrator uses `ProcessVMReadv` to read from it.

- **`GET /memory`** → `{"resident": [u64...], "empty": [u64...]}`
  — page bitmaps packed as `[]uint64`, one bit per page.

- **`GET /memory/dirty`** → `{"bitmap": [u64...]}`
  — dirty page bitmap since last snapshot. Returns 400 if VM is not paused.

Guest memory is allocated as a real anonymous mapping. The workload
simulation touches pages in this allocation, so dirty bitmaps reflect
actual activity visible to `ProcessVMReadv` and `ExportMemory`.

## UFFD support

On `PUT /snapshot/load` with `mem_backend.backend_type: "Uffd"`, the mock
performs the real userfaultfd handshake from `src/vmm/src/persist.rs`:
anonymous mmap → userfaultfd → register → connect to handler → send
`GuestRegionUffdMapping` JSON + UFFD fd via SCM_RIGHTS.

## Workload simulation

Extra `/mock/*` endpoints control simulated workloads:

```
PUT    /mock/workload   — configure simulation
GET    /mock/workload   — query status
DELETE /mock/workload   — stop simulation
GET    /mock/health     — health check (includes PID for ProcessVMReadv)
```

### Configuration

```json
{
  "cpu_load_percent": 30.0,
  "memory_usage_mib": 64,
  "io_ops_per_sec": 500,
  "response_delay_ms": 5,
  "failure_mode": {"type": "crash", "after_ms": 30000}
}
```

- **`cpu_load_percent`** — busy-spin for this fraction of each 100ms cycle
- **`memory_usage_mib`** — (reserved for future use)
- **`io_ops_per_sec`** — page touch rate in guest memory; dirtied pages
  appear in `GET /memory/dirty` bitmap after pause
- **`response_delay_ms`** — artificial latency on every API response
- **`failure_mode`** — one of: `crash`, `hang`, `oom`, `io_error`,
  `random_exit`, `kernel_panic`

## File structure

```
fc-mock/
├── Cargo.toml
├── Dockerfile
└── src/
    ├── main.rs         — CLI, HTTP server over Unix socket
    ├── api_types.rs    — Serde DTOs (mirrors vmm_config)
    ├── api_server.rs   — Request routing (orchestrator endpoints only)
    ├── vm_state.rs     — In-memory VM state
    ├── guest_mem.rs    — Anonymous memory allocation + page bitmaps
    ├── uffd.rs         — UFFD handshake (mirrors persist.rs)
    └── workload.rs     — CPU/memory/failure simulation
```
