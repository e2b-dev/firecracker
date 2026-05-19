# Snapshot correctness — open findings

Baseline [`639196c95`][head] on PR #8 (Bugs 1–3 closed by that cherry-pick of upstream [`67ba7a206`][upstream-fix]).

Severity: **C** can freeze guest after resume · **M** can corrupt / hang under stress · **L** scenario-bound.

Minimal reproducers for Bugs 5, 9, 10, P2-5 live in [`snapshot-bug-repros.md`](snapshot-bug-repros.md).

[head]: https://github.com/e2b-dev/firecracker/tree/639196c95
[upstream-fix]: https://github.com/firecracker-microvm/firecracker/commit/67ba7a206
[upstream-vsock]: https://github.com/firecracker-microvm/firecracker/commit/48a5ae3b2
[pr6]: https://github.com/e2b-dev/firecracker/pull/6

---

### Bug 4 — `prepare_save` silently swallows drain/sync errors [M]

[`block/virtio/device.rs:724-740`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/device.rs#L724-L740) ·
[`io_uring/queue/submission.rs:122-153`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/io_uring/queue/submission.rs#L122-L153) ·
[`io_uring/mod.rs:240-244`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/io_uring/mod.rs#L240-L244)

`drain_and_flush` calls `submit_and_wait_all` (one `io_uring_enter` with no `EINTR` retry) + `file.sync_all`. Errors are logged and dropped, then `process_async_completion_queue` runs anyway. If the drain didn't actually finish, in-flight ops have no CQEs → `PendingRequest` stays in the slab → `desc_idx` was already advanced past in the avail ring → no used-ring entry produced. Snapshot is written claiming `next_avail = N`; on restore that descriptor is permanently lost → guest blk_mq tag never returned → in-kernel hang. Same external symptom as the (now-fixed) ordering bugs, different cause. Also missing: invariant check that `engine.num_ops() == 0` after a successful drain (PR [#6][pr6] added `pending_ops()` for exactly this).

#### Analysis

This seems correct, regarding the effect: Firecracker handles EINTR as any other error. in the `io_uring_enter()` syscall. The handling Firecracker does in this case
is that it logs the error and continues, instead of crashing. We can try to make the handling a bit more robust for the case of EINTR. Changing the way errors are handled
in general here would be a much bigger change.

One thing to note here, is that EINTR is only possible if we're passing the `IORING_ENTER_GETEVENTS` flag, which we only do if there are actual pending events in the queue.
This is of course possible but maybe not as common during snapshot time.

Another thing that worries me is the `vmm::io_uring::submission::SubmissionQueue::submit` implementation of the `io_uring_enter()` system call. This one
seems to use `into_result()` for translating the return value of the system call in a `Result` type. `into_result()` translates a -1 into an `Err` (using `errno`) otherwise `Ok`.

The `man io_uring_enter(2)` says:

```
RETURN VALUE
     io_uring_enter(2) returns the number of I/Os successfully consumed.  This can be zero if to_submit was zero or if the submission queue was empty. Note that if the ring was created with IORING_SETUP_SQPOLL specified, then the return value will generally be the same as to_submit as submission happens outside the context of the system call.

     The errors related to a submission queue entry will be returned through a completion queue entry (see section CQE ERRORS), rather than through the system call itself.

     Errors that occur not on behalf of a submission queue entry are returned via the system call directly. On such an error, a negative error code is returned. The caller should not rely on errno variable.
```

So, `io_uring_enter()` doesn't return -1 on error; it returns an actual (negative) error number. Which means that the error handling here is wrong.

---

### Bug 5 — `Cqe::result` passes negative kernel errno to `from_raw_os_error` [M]

[`io_uring/operation/cqe.rs:31-40`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/io_uring/operation/cqe.rs#L31-L40)

io_uring puts a *negative* errno into `cqe.res`; `from_raw_os_error` expects positive. With negative, `ErrorKind` collapses to `Other` regardless of the real errno. Load-bearing on PR #8: the discard / write-zeroes "host doesn't support this" detection at [`block/virtio/device.rs:590-668`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/device.rs#L590-L668) compares against `Some(-libc::EOPNOTSUPP)` and only works because `Cqe::result` is broken. Any sign-flip fix must also flip that comparison to positive `EOPNOTSUPP`, or discard / write_zeroes will start returning `VIRTIO_BLK_S_IOERR` to the guest on every unsupported-fs request.

#### Analysis

AFAICT, `Error::from_os_error` [expects a `RawOsError`](https://doc.rust-lang.org/std/io/struct.Error.html#method.from_raw_os_error). `RawOsError` [is an `i32`](https://doc.rust-lang.org/std/io/type.RawOsError.html) on all currently supported platforms.

---

### Bug 6 — `process_queue` early-`?` leaves pushed SQEs un-kicked [L]

[`block/virtio/device.rs:494-570`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/device.rs#L494-L570) ·
[`io/async_io.rs:250-255`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/io/async_io.rs#L250-L255)

If `pop_or_enable_notification` returns `Err(InvalidAvailIdx)`, the function exits via `?` *before* `kick_submission_queue` is called; any SQEs already pushed in this call sit in the SQ until the next `process_queue` invocation. Separately, `kick_submission_queue` itself issues `io_uring_enter` with no `EINTR` retry — same flavor as Bug 4 but in the steady-state path. Low severity because `InvalidAvailIdx` shouldn't happen with a non-malicious guest; defense-in-depth only.

#### Analysis

Re `pop_or_enable_notification`: `InvalidAvailIdx` is a hard error for Firecracker. If `InvalidAvailIdx` ever returns `pop_or_enable_notification` Firecracker will crash. This is expected as this error signifies a malicious/buggy guest.

Re `kick_submission_queue` calls `squeue::submit()` with `min_complete == 0` (it doesn't block at all). According to the man page this shouldn't enable `IORING_ENTER_GETEVENTS` so this should never return an `EINTR`  

---

### Bug 7 — irqfd injection race with KVM state save [C/M]

[`transport/mmio.rs:464-477`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/transport/mmio.rs#L464-L477) ·
[`transport/pci/device.rs:669-697`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/transport/pci/device.rs#L669-L697)

`prepare_save → trigger` writes to an irqfd, but KVM injects the IRQ into the IRQ chip / LAPIC *asynchronously* (kernel workqueue for legacy / level IRQs; faster but still async for MSI-X). There is no explicit "drain irqfd workqueue" before `save_vcpu_states` / `kvm.save_state`. Under heavy I/O + snapshot stress (the PR [#6][pr6] workload) the IRQ can land *after* KVM state was captured, producing the same lost-IRQ symptom as the now-fixed ordering bugs via a different mechanism. Bug 10 below explains why the resume-side `kick` can't recover this for block / net.

---

### Bug 8 — Pending `queue_evt` count is lost across snapshot [L, mitigated]

[`block/virtio/device.rs:312-313`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/device.rs#L312-L313) ·
[`block/virtio/persist.rs:99`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/persist.rs#L99) ·
[`lib.rs:386-388`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/lib.rs#L386-L388)

The block device's `queue_evts` is rebuilt fresh on restore; any pending QueueNotify count the host hadn't drained before pause is lost. The host's `next_avail` then lags the guest's `avail_ring->idx` on the restored device. **Mitigated** by `Vmm::resume_vm() → device_manager.kick_virtio_devices()`: block's `kick` ([`block/device.rs:212-222`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/device.rs#L212-L222)) reads the avail ring, so the orchestrator's `resumeVM` call recovers it. Latent if `resume_vm` is ever bypassed (e.g. autoresume during load).

---

### Bug 9 — Vsock has the same ordering bug; companion upstream commit missing [C]

[`device_manager/persist.rs:287-318`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/device_manager/persist.rs#L287-L318) ·
[`device_manager/pci_mngr.rs:351-380`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/device_manager/pci_mngr.rs#L351-L380) ·
[`vsock/device.rs:259-281`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/vsock/device.rs#L259-L281)

`Vsock::send_transport_reset_event` is still called from `device_manager` *after* `transport_state.save()` was captured. It writes to the event virtqueue and calls `signal_used_queue → trigger_irq(Vring) → irq_status.fetch_or(...)`, mutating exactly the `interrupt_status` (MMIO) / `msix_state` PBA (PCI) that the snapshot just froze. Same shape as the closed Bug 2 / Bug 3 for block. The `639196c95` cherry-pick doesn't fix vsock because `Vsock` doesn't implement `prepare_save` (verify: `grep "fn prepare_save" src/vmm/src/devices/virtio/vsock/device.rs` → empty). Fix: cherry-pick [`48a5ae3b2`][upstream-vsock] which adds `Vsock::prepare_save` and removes the manual call. *Partial mitigation today*: `Vsock::kick` ([`vsock/device.rs:382-393`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/vsock/device.rs#L382-L393)) unconditionally re-fires `signal_used_queue(EVQ)` on resume, so the guest usually gets `TRANSPORT_RESET` anyway — fragile, relies on `resumeVM` + correct notification-suppression behavior.

---

### Bug 10 — Block / Net `kick()` doesn't re-fire IRQ for already-completed used-ring entries [C]

[`block/device.rs:212-222`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/device.rs#L212-L222) ·
[`net/device.rs:1042-1052`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/net/device.rs#L1042-L1052) ·
[`vsock/device.rs:382-393`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/vsock/device.rs#L382-L393) (for contrast)

Block / Net `kick` only call `process_virtio_queues()`, which reads the *avail* ring. If the avail ring is empty on resume — i.e. the guest hasn't submitted anything new — `prepare_kick` returns false and no IRQ is triggered, *even if* used-ring entries from before the snapshot are sitting in guest memory waiting for an IRQ that Bug 7 caused to be lost. Vsock's `kick` retriggers `signal_used_queue` unconditionally, which is exactly the recovery shape needed. This is the residual hole that makes Bug 7 unrecoverable on the resume side for block / net.

---

### Finding P2-1 — `mem_file_path = None` skips KVM dirty-log reset [M, diff snapshots]

[`persist/mod.rs:159-181`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/persist/mod.rs#L159-L181) ·
[`vstate/vm.rs:385-389`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/vstate/vm.rs#L385-L389)

`snapshot_memory_to_file` is where `reset_dirty_bitmap()` + `guest_memory().reset_dirty()` run for `SnapshotType::Full`. When the orchestrator passes `mem_file_path = None` (its production mode — memory is extracted externally), that reset never runs. The next diff snapshot then includes pages dirty from *before* the current snapshot, over-reporting the diff. Wrong incremental footprint in chained template → diff → diff workflows; benign for one-shot full snapshots.

---

### Finding P2-2 — `/memory/mappings` is not gated on `VmState::Paused` [M]

[`rpc_interface.rs:963-1010`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/rpc_interface.rs#L963-L1010)

`GetMemory` (L981) and `GetMemoryDirty` (L1000) require paused; `GetMemoryMappings` doesn't. If anything reads from the returned host VAs after vCPUs are resumed, the reads race with guest writes → torn data. Not exploited today (orchestrator calls it only after pause), but a contract footgun for any future external tooling.

---

### Finding P2-3 — `/memory/mappings` layout disagrees with `dump()` under virtio-mem hotplug [C, latent]

[`lib.rs:701-724`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/lib.rs#L701-L724) ·
[`vstate/memory.rs:689-701`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/vstate/memory.rs#L689-L701)

`guest_memory_mappings` accumulates offsets over `plugged_slots()` only. `dump()` walks *all* slots and `seek`s the file forward over unplugged ones, leaving zeroed holes. With any unplugged slot the two offset layouts diverge: the memfile has holes; the mapping API reports collapsed-over offsets. The orchestrator's `pauseProcessMemory` does linear math against the memfile using these offsets — every region after the first hole reads the wrong bytes. Latent today (virtio-mem not used); becomes corruption the moment it's turned on.

---

### Finding P2-4 — Empty `{}` rate-limiter PATCH doesn't clear snapshot-restored limits [M]

[`rpc_interface.rs:914-940`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/rpc_interface.rs#L914-L940) ·
[`vmm_config/mod.rs:96-115`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/vmm_config/mod.rs#L96-L115) ·
orchestrator [`fc/client.go`](https://github.com/e2b-dev/infra/blob/main/packages/orchestrator/pkg/sandbox/fc/client.go) (`setTxRateLimit`)

Orchestrator comment says an empty `{}` PATCH is sent to "reset any limit persisted in a snapshot". But the Go model serializes `{}` with both fields omitted, and FC only calls `update_block_rate_limiter` when `rate_limiter.is_some()`; `get_bucket_update(None)` returns `BucketUpdate::None`. Net result: empty `{}` is a no-op on the persisted buckets, so a sandbox restored from a snapshot taken with limits silently keeps those limits even when the new deployment wants them removed.

---

### Finding P2-5 — DrainBalloon timeout + force-reset `cmd_id` mismatch [L]

[`balloon/persist.rs:180-210`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/balloon/persist.rs#L180-L210)

`BalloonState.hinting_state` is persisted but `BalloonConfig.free_page_hint_cmd_id` is force-reset to `FREE_PAGE_HINT_DONE` on restore. If the orchestrator's `DrainBalloon` times out mid-cycle, the restored device has `hinting_state` saying "mid-chain" and `cmd_id` saying "done". Whether the guest driver tolerates that depends on its implementation.

---

### Finding P2-6 — MMDS data not persisted; uninitialized-MMDS window after resume [L]

[`device_manager/persist.rs:119-123`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/device_manager/persist.rs#L119-L123)

Only `{version, imds_compat}` is saved; the IMDS JSON datastore is not. The orchestrator calls `setMmds` only *after* `resumeVM`, so for a brief window after resume the guest sees an uninitialized MMDS and reads return `NotInitialized`. No cross-sandbox leak — the previous sandbox's data genuinely isn't there. Annoying for guests that read MMDS at first boot.

---

### Finding P2-7 — aarch64 serial only partially persisted [L]

[`device_manager/persist.rs:211-226`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/device_manager/persist.rs#L211-L226) ·
[`device_manager/mod.rs:495-540`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/device_manager/mod.rs#L495-L540)

Only `DeviceType + MMIODeviceInfo` (address, length, gsi) are saved; UART FIFO, IER, LCR are not. Restore runs `emulate_serial_init` which explicitly sets `IER_RDA` as a workaround for RX interrupts. The serial output rate-limiter (upstream `9a49dcb03`) is not on this branch yet, so its persistence question doesn't apply here.

---

### Finding P2-8 — SIGTERM during `CreateSnapshot` can truncate the snapfile [L]

[`persist/mod.rs:184-203`](https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/persist/mod.rs#L184-L203) ·
orchestrator [`fc/process.go`](https://github.com/e2b-dev/infra/blob/main/packages/orchestrator/pkg/sandbox/fc/process.go) (`Stop`)

`snapshot_state_to_file` opens with `create + truncate + write`, then `flush` + `sync_all`. A SIGTERM landing between truncate and sync_all leaves a syntactically invalid file. Reachable from orchestrator cancellation chains during pause errors, not from the happy path. Restore-side validation rejects partial files, so failure is loud — cleanup is the operator's problem.
