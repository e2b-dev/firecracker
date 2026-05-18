# Snapshot correctness analysis

Open snapshot correctness findings for
[`e2b-dev/firecracker#8`][pr8] (baseline [`639196c95`][head]).

Bugs 1, 2, 3 were closed by the cherry-pick of upstream [`67ba7a206`][upstream-fix]
as [`639196c95`][fix-639] and have been removed from this document.

All source links are pinned to `639196c95`.

[pr6]: https://github.com/e2b-dev/firecracker/pull/6
[pr8]: https://github.com/e2b-dev/firecracker/pull/8
[head]: https://github.com/e2b-dev/firecracker/tree/639196c95
[fix-639]: https://github.com/e2b-dev/firecracker/commit/639196c95
[upstream-fix]: https://github.com/firecracker-microvm/firecracker/commit/67ba7a206
[upstream-vsock]: https://github.com/firecracker-microvm/firecracker/commit/48a5ae3b2
[infra-pause]: https://github.com/e2b-dev/infra/blob/main/packages/orchestrator/pkg/sandbox/sandbox.go
[infra-resume]: https://github.com/e2b-dev/infra/blob/main/packages/orchestrator/pkg/sandbox/fc/process.go

## Open findings at a glance

Severity: **C** = can plausibly freeze the guest after resume; **M** = can
corrupt / hang under stress or signals; **L** = low / scenario-bound.

| # | Severity | Summary | Where |
|---|---|---|---|
| 4 | M | `prepare_save` silently swallows drain/sync errors; `io_uring_enter` has no `EINTR` retry — in-flight descriptors can be permanently lost | [`block/virtio/device.rs:724-740`][prepare_save] |
| 5 | M | `Cqe::result` passes negative errno to `from_raw_os_error`; PR #8's EOPNOTSUPP detection now depends on this being broken | [`io_uring/operation/cqe.rs:25-40`][cqe] |
| 6 | L | `process_queue` early-`?` leaves pushed SQEs un-kicked; `kick_submission_queue` has no `EINTR` retry | [`block/virtio/device.rs:494-570`][process_queue] |
| 7 | C/M | irqfd injection is async in the kernel; KVM IRQ chip can be saved before the IRQ lands | [`mmio.rs:405-477`][mmio_trigger] |
| 8 | L | Pending `queue_evt` count lost across snapshot — mitigated by `resume_vm.kick` | [`device.rs:212-222`][block_kick] |
| 9 | C | Vsock has the same ordering bug; companion fix [`48a5ae3b2`][upstream-vsock] not yet applied to PR #8 | [`device_manager/persist.rs:287-318`][vsock_mmio_save] |
| 10 | C | Block / Net `kick()` only drain avail ring; cannot recover a lost IRQ for already-completed used-ring entries | [`device.rs:212-222`][block_kick] |
| P2-1 | M | `mem_file_path = None` skips KVM dirty-log reset → diff snapshots over-report dirty pages | [`persist/mod.rs:159-181`][create_snapshot] |
| P2-2 | M | `/memory/mappings` is not gated on `VmState::Paused` (unlike `/memory` and `/memory/dirty`) | [`rpc_interface.rs:963-1010`][meminfo_rpc] |
| P2-3 | C, latent | `/memory/mappings` offsets disagree with `dump()` layout when virtio-mem has any unplugged slot | [`lib.rs:701-724`][guest_mappings] |
| P2-4 | M | Empty `{}` rate-limiter PATCH does **not** clear snapshot-restored limits (orchestrator expects it does) | [`rpc_interface.rs:914-940`][fc_patch] |
| P2-5 | L | DrainBalloon timeout: `hinting_state` mid-protocol but `host_cmd` force-reset to DONE on restore | [`balloon/persist.rs:180-210`][balloon_restore] |
| P2-6 | L | MMDS data contents not persisted; brief uninitialized-MMDS window after `resumeVM` and before `setMmds` | [`device_manager/persist.rs:119-123`][mmds_state] |
| P2-7 | L | aarch64 serial only partially persisted; `IER_RDA` workaround on restore | [`device_manager/persist.rs:211-226`][serial_save] |
| P2-8 | L | SIGTERM during `CreateSnapshot` can leave a truncated snapfile | [`persist/mod.rs:184-203`][snapshot_state_write] |

[prepare_save]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/device.rs#L724-L740
[cqe]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/io_uring/operation/cqe.rs#L25-L40
[process_queue]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/device.rs#L494-L570
[mmio_trigger]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/transport/mmio.rs#L405-L477
[block_kick]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/device.rs#L212-L222
[vsock_mmio_save]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/device_manager/persist.rs#L287-L318
[create_snapshot]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/persist/mod.rs#L159-L181
[meminfo_rpc]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/rpc_interface.rs#L963-L1010
[guest_mappings]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/lib.rs#L701-L724
[fc_patch]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/rpc_interface.rs#L914-L940
[balloon_restore]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/balloon/persist.rs#L180-L210
[mmds_state]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/device_manager/persist.rs#L119-L123
[serial_save]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/device_manager/persist.rs#L211-L226
[snapshot_state_write]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/persist/mod.rs#L184-L203

### Backport status

| Branch | Ordering fix `67ba7a206` | Vsock fix `48a5ae3b2` (Bug 9) |
|---|---|---|
| PR #8 / `firecracker-v1.14-direct-mem` | fixed by `639196c95` | **MISSING** |
| `firecracker-v1.12-direct-mem` (prod today) | **MISSING** | **MISSING** |
| `upstream/firecracker-v1.14` | **MISSING** | **MISSING** |
| `upstream/firecracker-v1.12` | **MISSING** | **MISSING** |

---

## Bug 4 — `prepare_save` silently swallows drain/sync errors  [M]

[`src/vmm/src/devices/virtio/block/virtio/device.rs:724-740`][prepare_save]

```rust
fn drain_and_flush(&mut self, discard: bool) {
    if let Err(err) = self.disk.file_engine.drain_and_flush(discard) {
        error!("Failed to drain ops and flush block data: {:?}", err);  // <-- only logged
    }
}

pub fn prepare_save(&mut self) {
    if !self.is_activated() { return; }
    self.drain_and_flush(false);                          // <-- return value dropped
    if let FileEngine::Async(ref _engine) = self.disk.file_engine {
        self.process_async_completion_queue();            // <-- runs even on partial drain
    }
}
```

`AsyncFileEngine::drain_and_flush` calls `submit_and_wait_all` → one
[`io_uring_enter`][submit_syscall] with **no `EINTR` retry** + `file.sync_all`.
If either fails:

- Err is logged and dropped.
- `process_async_completion_queue` pops whatever CQEs landed, but in-flight
  ops have no CQEs yet → `PendingRequest` stays in the slab; `desc_idx` was
  already advanced past in the avail ring; no used-ring entry is ever
  produced.
- Snapshot is written claiming `next_avail = N`. On restore that `desc_idx`
  is permanently lost → guest blk_mq tag never returned → permanent
  in-kernel hang.

Same external symptom as the now-fixed ordering bugs, different proximate
cause — fixing ordering does not fix this. PR #6's test branch began
addressing it via a `PendingAsyncOperations(u32)` error variant; not on PR #8.

Also missing: invariant check that `engine.num_ops() == 0`
([`io_uring/mod.rs:240-244`][num_ops]) after a successful drain. PR #6 added
`pending_ops()` for exactly this.

[submit_syscall]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/io_uring/queue/submission.rs#L122-L153
[num_ops]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/io_uring/mod.rs#L240-L244

---

## Bug 5 — `Cqe::result` passes negative errno to `from_raw_os_error`  [M]

[`src/vmm/src/io_uring/operation/cqe.rs:25-40`][cqe]:

```rust
pub fn result(&self) -> Result<u32, std::io::Error> {
    let res = self.res;
    if res < 0 {
        Err(std::io::Error::from_raw_os_error(res))        // <-- should be -res
    } else {
        Ok(u32::try_from(self.res).unwrap())
    }
}
```

`from_raw_os_error` expects positive errno. With negative, `ErrorKind`
collapses to `Other` and the test at [`cqe.rs:60-79`][cqe_test] reproduces
the same mistake on both sides of `assert_eq!`.

**Now load-bearing on PR #8**: the discard / write-zeroes "host filesystem
doesn't support this" detection at
[`device.rs:590-668`][eopnotsupp] compares against `Some(-libc::EOPNOTSUPP)`
(negative) and works *only because* `Cqe::result` is unfixed. Any future
sign-flip fix must also update this comparison to `Some(libc::EOPNOTSUPP)`,
otherwise discard / write_zeroes on filesystems that don't support them
will return `VIRTIO_BLK_S_IOERR` instead of being disabled locally.

[cqe_test]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/io_uring/operation/cqe.rs#L60-L79
[eopnotsupp]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/device.rs#L590-L668

---

## Bug 6 — `process_queue` early-`?` leaves SQEs un-kicked  [L, defensive]

[`src/vmm/src/devices/virtio/block/virtio/device.rs:494-570`][process_queue]:

```rust
while let Some(head) = queue.pop_or_enable_notification()? {  // <-- early `?` return
    ...
}
queue.advance_used_ring_idx();
...
if let FileEngine::Async(ref mut engine) = self.disk.file_engine
    && let Err(err) = engine.kick_submission_queue()           // <-- only on normal exit
{
    error!("BlockError submitting pending block requests: {:?}", err);  // <-- swallowed
}
```

If `pop_or_enable_notification` returns `Err(InvalidAvailIdx)`,
`kick_submission_queue` is never reached and pushed SQEs sit in the SQ until
the next process_queue invocation. Also `kick_submission_queue` itself
([`async_io.rs:250-255`][kick]) issues `io_uring_enter` with no `EINTR`
retry — same flavor as Bug 4.

Defense-in-depth; not normally reachable from a non-malicious guest.

[kick]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/io/async_io.rs#L250-L255

---

## Bug 7 — irqfd injection race with KVM state save  [C-leaning M]

Even after the (now-applied) save-ordering fix, the IRQ that
`prepare_save → trigger` produces is delivered to KVM **asynchronously**:

- MMIO `IrqTrigger::trigger_irq` writes to an irqfd; for legacy / level IRQs
  KVM injects via a kernel workqueue.
  ([`mmio.rs:464-477`][mmio_trigger])
- PCI MSI-X (unmasked) writes the per-vector irqfd; fast path on modern
  kernels, but still async w.r.t. the userspace write.

There is no explicit "drain irqfd workqueue" before
`save_vcpu_states` / `kvm.save_state`. Under heavy I/O + snapshot stress
(the PR #6 workload) this race produces the same lost-IRQ symptom as the
old Bug 1 / Bug 2, via a different mechanism. **No mitigation in tree.**
See Bug 10 for why the resume-side `kick` cannot recover this for block /
net devices.

---

## Bug 8 — Pending `queue_evt` count is lost across snapshot  [L, mitigated]

`VirtioBlock::queue_evts` is rebuilt fresh on restore
([`device.rs:312-313`][queue_evts] /
[`persist.rs:99`][queue_evts_restore]). If the guest wrote to QueueNotify
and the VMM event loop hadn't drained `queue_evt` before pause + snapshot,
the host-side `next_avail` lags the guest-side `avail_ring->idx` on the
restored device.

**Mitigated** by `Vmm::resume_vm() → kick_virtio_devices()` at
[`lib.rs:386-388`][resume_kick]: block's `kick`
([`device.rs:212-222`][block_kick]) calls `process_virtio_queues()` which
reads the avail ring from guest memory. The orchestrator
[calls `resumeVM`][infra-resume], so no freeze in practice today.

[queue_evts]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/device.rs#L312-L313
[queue_evts_restore]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/persist.rs#L99
[resume_kick]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/lib.rs#L386-L388

---

## Bug 9 — Vsock has the same ordering bug; companion upstream commit still missing  [C]

[`src/vmm/src/device_manager/persist.rs:287-318`][vsock_mmio_save] (MMIO) and
[`pci_mngr.rs:351-380`][vsock_pci_save] (PCI):

```rust
virtio_ids::VIRTIO_ID_VSOCK => {
    if vsock.is_activated() {
        vsock.send_transport_reset_event()...;   // <-- AFTER transport_state.save() at L230
    }
    ...
}
```

`send_transport_reset_event` writes to the event virtqueue and calls
`signal_used_queue(EVQ) → trigger_irq(Vring) → irq_status.fetch_or(...)`
([`vsock/device.rs:259-281`][vsock_reset]). The transport `interrupt_status`
captured earlier is stale.

The (now-fixed) ordering bug fix `639196c95` doesn't fix vsock because
`Vsock` doesn't implement `prepare_save`. Verify:
`grep "fn prepare_save" src/vmm/src/devices/virtio/vsock/device.rs` → empty.

**Fix**: cherry-pick [`48a5ae3b2`][upstream-vsock] which adds
`Vsock::prepare_save` and removes the manual call from `device_manager`.

**Partial mitigation in practice**: `Vsock::kick`
([`vsock/device.rs:382-393`][vsock_kick]) unconditionally re-fires
`signal_used_queue(EVQ)` on resume. So the guest typically receives the
`TRANSPORT_RESET` IRQ on resume even with the bug present. This relies on
`resumeVM` being called and on the guest re-arming its used-event correctly
— fragile.

UDS connection state is intentionally discarded across snapshot, so there's
no half-open state to worry about.

[vsock_pci_save]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/device_manager/pci_mngr.rs#L351-L380
[vsock_reset]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/vsock/device.rs#L259-L281
[vsock_kick]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/vsock/device.rs#L382-L393

---

## Bug 10 — Block / Net `kick()` doesn't re-fire IRQ for already-completed used-ring entries  [C]

`VirtioBlock::kick` ([`device.rs:212-222`][block_kick]):

```rust
fn kick(&mut self) {
    if self.is_activated() {
        self.process_virtio_queues();   // <-- only drains avail ring
    }
}
```

vs `Vsock::kick` ([`vsock/device.rs:382-393`][vsock_kick]) which
unconditionally retriggers `signal_used_queue(EVQ)`.

If the avail ring is empty on resume (no new descriptors since snapshot),
block's `process_virtio_queues` produces zero used-ring entries,
`prepare_kick` returns false, **no IRQ is triggered** — even though
used-ring entries from before the snapshot may be sitting in guest memory
waiting for an IRQ that was lost via Bug 7.

Net's `kick` ([`net/device.rs:1042-1052`][net_kick]) has the same shape.

This is the residual hole that makes Bug 7 unrecoverable on the resume
side for block/net. The robust shape would mirror vsock's `kick`: call
`prepare_kick` on each queue, re-trigger if there are unacked used entries.

[net_kick]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/net/device.rs#L1042-L1052

---

## Finding P2-1 — `mem_file_path = None` skips KVM dirty-log reset  [M, diff snapshots]

[`src/vmm/src/persist/mod.rs:159-181`][create_snapshot]:

```rust
snapshot_state_to_file(&microvm_state, &params.snapshot_path)?;

if let Some(mem_file_path) = params.mem_file_path.as_ref() {
    vmm.vm.snapshot_memory_to_file(mem_file_path, params.snapshot_type, vmm.page_size)?;
}

vmm.device_manager.mark_virtio_queue_memory_dirty(vmm.vm.guest_memory());
```

`snapshot_memory_to_file` is where `reset_dirty_bitmap()` +
`guest_memory().reset_dirty()` run for `SnapshotType::Full`
([`vstate/vm.rs:385-389`][reset_dirty]). When the orchestrator passes
`mem_file_path = None` (its production mode, since memory is extracted
externally), that reset never runs and the next diff snapshot over-reports
dirty pages.

Benign for full snapshots in isolation; inflates and mis-represents diff
footprint for chained template → diff → diff workflows.

[reset_dirty]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/vstate/vm.rs#L385-L389

---

## Finding P2-2 — `/memory/mappings` is not gated on `VmState::Paused`  [M]

[`src/vmm/src/rpc_interface.rs:963-1010`][meminfo_rpc]:

```rust
fn get_guest_memory_mappings(&self) -> Result<VmmData, VmmActionError> {
    // … no VmState::Paused check
}
fn get_guest_memory_info(&self) -> Result<VmmData, VmmActionError> {
    if vmm.instance_info.state != VmState::Paused { return Err(…); }   // L981
}
fn get_dirty_memory_info(&self) -> Result<VmmData, VmmActionError> {
    if vmm.instance_info.state != VmState::Paused { return Err(…); }   // L1000
}
```

`GetMemory` and `GetMemoryDirty` require paused; `GetMemoryMappings`
doesn't. If a consumer reads from the returned host VAs after vCPUs are
resumed, they get torn reads. The orchestrator calls it only after pause
today, so this is an API contract gap, not exploited.

---

## Finding P2-3 — `/memory/mappings` layout disagrees with `dump()` under virtio-mem hotplug  [C, latent]

- `guest_memory_mappings` iterates `flat_map(|r| r.plugged_slots())`,
  `offset += plugged_slot_size`
  ([`lib.rs:701-724`][guest_mappings]).
- `GuestMemoryExtension::dump` walks **all** slots; unplugged slots
  `seek` the file forward leaving zeroed holes
  ([`memory.rs:689-701`][dump_memory]).

With any unplugged slot, the two offset layouts diverge: the memfile has
holes; `/memory/mappings` reports collapsed-over offsets. Anyone correlating
the two (the orchestrator's `pauseProcessMemory` does linear math against
the memfile) reads the wrong bytes for any region after the first hole.

Latent today (virtio-mem not used); becomes corruption the moment
virtio-mem is enabled.

[dump_memory]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/vstate/memory.rs#L689-L701

---

## Finding P2-4 — Empty `{}` rate-limiter PATCH doesn't clear snapshot-restored limits  [M]

Orchestrator [`fc/client.go`][orch_setrl] documents:

> if all are disabled an empty RateLimiter is sent to **reset any limit
> persisted in a snapshot**.

But the Go model serializes `{}` with both fields omitted, and on the FC
side [`rpc_interface.rs:914-940`][fc_patch] only calls
`update_block_rate_limiter` when `rate_limiter.is_some()`.
[`vmm_config/mod.rs:96-115`][bucket_update] returns `BucketUpdate::None`
for `tb_cfg: None`, so the snapshot-era buckets are left in place.

If a snapshot was taken with limits and the new deployment wants them
removed, the restored sandbox silently keeps the old limits.

[orch_setrl]: https://github.com/e2b-dev/infra/blob/main/packages/orchestrator/pkg/sandbox/fc/client.go
[bucket_update]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/vmm_config/mod.rs#L96-L115

---

## Finding P2-5 — DrainBalloon timeout + force-reset cmd_id mismatch  [L]

Orchestrator wraps `DrainBalloon` in a per-use-case timeout; on timeout it
continues. `BalloonState.hinting_state` is persisted, but
`free_page_hint_cmd_id` is force-reset to `FREE_PAGE_HINT_DONE` on restore
([`balloon/persist.rs:180-210`][balloon_restore]):

```rust
let config = BalloonConfig { …, free_page_hint_cmd_id: FREE_PAGE_HINT_DONE };
…
balloon.hinting_state = state.hinting_state;     // <-- mid-chain state preserved
```

A snapshot taken mid-cycle restores with `hinting_state` saying "mid-chain"
and `cmd_id` saying "done". Whether the guest driver handles that
gracefully is kernel-implementation dependent.

---

## Finding P2-6 — MMDS data not persisted; uninitialized-MMDS window after resume  [L]

[`device_manager/persist.rs:119-123`][mmds_state] saves only
`{version, imds_compat}`. The IMDS JSON datastore is not in the snapshot.

The orchestrator calls `setMmds` only **after** `resumeVM`, so for a brief
window after resume the guest sees an uninitialized MMDS. Guest reads in
that window get `NotInitialized`. No cross-sandbox leak.

---

## Finding P2-7 — aarch64 serial partially persisted; `IER_RDA` workaround  [L]

[`device_manager/persist.rs:211-226`][serial_save] saves only
`DeviceType + MMIODeviceInfo`. No UART FIFO contents, IER, LCR, or output
rate-limiter state. Restore runs `emulate_serial_init`
([`device_manager/mod.rs:495-540`][serial_init]) which explicitly sets
`IER_RDA` as a workaround.

The serial output rate-limiter (upstream commit `9a49dcb03`) is not on this
branch yet.

[serial_init]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/device_manager/mod.rs#L495-L540

---

## Finding P2-8 — SIGTERM during `CreateSnapshot` can truncate the snapfile  [L]

Orchestrator `Stop` sends SIGTERM then SIGKILL after 10s.
`snapshot_state_to_file` ([`persist/mod.rs:184-203`][snapshot_state_write])
opens with `create + truncate + write`, then `flush` + `sync_all`. A
SIGTERM landing between `truncate` and `sync_all` leaves a syntactically
invalid file on disk.

Restore-side validation will reject the partial file (so failure is loud,
not silent), but cleanup is the operator's problem. Reachable from
cancellation chains during pause errors, not from the happy path.

---

## References

- Upstream fixes:
  - [`67ba7a206`][upstream-fix] — ordering fix. **Applied as
    [`639196c95`][fix-639] on PR #8.** Closed Bugs 1, 2, 3.
  - [`48a5ae3b2`][upstream-vsock] — vsock companion. **Still missing on
    PR #8** (Bug 9).
- Freeze reproducer: [`e2b-dev/firecracker#6`][pr6]
  (`test_snapshot_with_heavy_async_io`).
- Orchestrator entry points:
  [`Sandbox.Pause`][infra-pause],
  [`Process.Resume`][infra-resume].
