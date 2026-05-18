# Snapshot correctness analysis

Read-only analysis of snapshot save/restore correctness on
[`e2b-dev/firecracker#8`][pr8]'s branch. Originally scoped to the async-engine
(`io_uring`) virtio-block code path and the "post-resume guest freeze"
reproduced by [`e2b-dev/firecracker#6`][pr6]; extended to cover the broader
snapshot flow and the e2b-specific extensions on this branch (optional
memfile, `/memory/mappings` API, UFFD WP, FPH balloon, MMDS, rate-limiter
PATCH semantics, etc.). **No fixes are proposed here — only findings, with
links to every relevant source location.**

The document is in two parts:

- **Findings 4–10**: the async-block snapshot freeze class and adjacent
  device-state issues (vsock, kick semantics, irqfd race). Findings 1, 2, 3
  (the upstream ordering bugs) were addressed by [`639196c95`][fix-639]
  ("fix: saving/restoring async IO engine transport state", cherry-pick of
  upstream `67ba7a206`) and have been removed from this document.
- **Findings P2-1–P2-8**: broader snapshot-flow correctness in the
  orchestrator's usage pattern, beyond async-block.

[pr6]: https://github.com/e2b-dev/firecracker/pull/6
[fix-639]: https://github.com/e2b-dev/firecracker/commit/639196c9548302bb62b52629e82c6ea6f54c4710

## Scope

- Target branch: [`firecracker-v1.14-direct-mem`][br] — i.e. the head of the
  open [`e2b-dev/firecracker#8`][pr8] ("[v1.14] Expose memory mapping & dirty
  pages; Make memfile dump optional"). This is the branch the orchestrator
  is going to consume next.
- Baseline commit: [`639196c95`][head] (the upstream ordering-fix
  cherry-pick). All file links below are pinned to that SHA so they don't
  bitrot if the branch advances.
- Active branch used in production today: [`firecracker-v1.12-direct-mem`][br112]
  — same root causes apply, **including the now-fixed ordering bugs**, since
  the v1.12 branch has not received the cherry-pick yet. See
  [Per-branch backport status](#per-branch-backport-status).
- Downstream consumer: the orchestrator's snapshot path in
  [`e2b-dev/infra`][infra-pause]
  (`packages/orchestrator/pkg/sandbox/sandbox.go::Sandbox.Pause`) which calls
  `Pause` + `CreateSnapshot` on a paused VM with heavy guest I/O in-flight.

[br]: https://github.com/e2b-dev/firecracker/tree/firecracker-v1.14-direct-mem
[br112]: https://github.com/e2b-dev/firecracker/tree/firecracker-v1.12-direct-mem
[pr8]: https://github.com/e2b-dev/firecracker/pull/8
[head]: https://github.com/e2b-dev/firecracker/tree/639196c95
[infra-pause]: https://github.com/e2b-dev/infra/blob/main/packages/orchestrator/pkg/sandbox/sandbox.go

## Background

The Firecracker async block engine submits guest virtio-block requests via
`io_uring` instead of blocking the VMM thread on `pread`/`pwrite`/`fsync`. The
benefit relevant to us is that the VMM thread stays free for net device work
while disk I/O is in flight; the price is that the VMM-side bookkeeping
(descriptor tracking, used-ring updates, IRQ injection) becomes asynchronous
relative to the kernel.

[`e2b-dev/firecracker#6`][pr6] introduced a reproducer
(`test_snapshot_with_heavy_async_io`) that demonstrably freezes the guest after
snapshot-resume when async block I/O is heavy at pause time. The
[upstream fix][upstream-fix] (`67ba7a206`, "fix: saving/restoring async IO
engine transport state", 2025-12-15) addressed three ordering bugs in the
save path. That fix was cherry-picked onto PR #8 as [`639196c95`][fix-639]
during the lifetime of this audit, so Findings 1, 2, 3 are no longer present
on this branch. The cherry-pick does **not** include the upstream companion
commit [`48a5ae3b2`][upstream-vsock] ("refactor(vsock): Send reset event
before saving transport state"), which is still required to address
**Bug 9** below.

[upstream-fix]: https://github.com/firecracker-microvm/firecracker/commit/67ba7a20692a5d1a2fd9218523a9b3ccde9e4a37
[upstream-vsock]: https://github.com/firecracker-microvm/firecracker/commit/48a5ae3b2

## Save-path call chain

The save side of `CreateSnapshot` walks roughly like this on PR #8 head
**after** the [`639196c95`][fix-639] cherry-pick:

| Step | Symbol | Source |
|---|---|---|
| 1 | `pub fn create_snapshot(vmm, vm_info, params)` | [`src/vmm/src/persist/mod.rs:159-182`][create_snapshot] |
| 2 | `Vmm::save_state(vm_info)` | [`src/vmm/src/lib.rs:444-475`][save_state] |
| 3 | `self.device_manager.save()` (now FIRST) → per-device `prepare_save()` → per-transport `transport_state.save()` → device `save()` | [`src/vmm/src/lib.rs:451`][save_state] |
| 4 | `save_vcpu_states()` (KVM_GET_LAPIC, KVM_GET_VCPU_EVENTS, …) | [`src/vmm/src/lib.rs:452`][save_state] / [`src/vmm/src/arch/x86_64/vcpu.rs:555-614`][vcpu_save] |
| 5 | `self.kvm.save_state()` (KVM_GET_IRQCHIP) | [`src/vmm/src/lib.rs:453`][save_state] |
| 6 | `self.vm.save_state()` | [`src/vmm/src/lib.rs:454-465`][save_state] |
| 7 | `VirtioBlock::prepare_save()` (only when activated): `drain_and_flush(false)` → `process_async_completion_queue()` | [`src/vmm/src/devices/virtio/block/virtio/device.rs:730-740`][prepare_save] |
| 8 | `AsyncFileEngine::drain_and_flush(false)` → `drain(false)` (submit + wait) + `file.sync_all()` | [`src/vmm/src/devices/virtio/block/virtio/io/async_io.rs:271-280`][drain_flush] |
| 9 | `process_async_completion_queue()` pops every CQE, `queue.add_used(...)`, `interrupt.trigger(Queue(0))` | [`src/vmm/src/devices/virtio/block/virtio/device.rs:575-624`][completion] |

[create_snapshot]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/persist/mod.rs#L159-L182
[save_state]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/lib.rs#L444-L471
[vcpu_save]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/arch/x86_64/vcpu.rs#L555-L614
[prepare_save]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/device.rs#L730-L740
[drain_flush]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/io/async_io.rs#L271-L280
[completion]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/device.rs#L575-L624

The interrupt actually delivered to KVM goes through one of:

- MMIO `IrqTrigger::trigger_irq`
  — bumps `irq_status` then writes to the irqfd:
  [`src/vmm/src/devices/virtio/transport/mmio.rs:405-477`][mmio_trigger]
- PCI `VirtioInterruptMsix::trigger`
  — sets PBA bit if masked, otherwise writes the MSI-X eventfd:
  [`src/vmm/src/devices/virtio/transport/pci/device.rs:669-697`][pci_trigger]

[mmio_trigger]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/transport/mmio.rs#L405-L477
[pci_trigger]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/transport/pci/device.rs#L669-L697

The transport state captured into the snapshot lives in:

- MMIO: `MmioTransportState { …, interrupt_status }` — captured atomically from
  `self.interrupt.irq_status` at
  [`src/vmm/src/devices/virtio/persist.rs:195-233`][mmio_state].
- PCI: `VirtioPciDeviceState { …, msix_state, … }` — captured from `MsixConfig`
  (which holds the masked / PBA bits) at
  [`src/vmm/src/devices/virtio/transport/pci/device.rs:614-639`][pci_state].

[mmio_state]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/persist.rs#L195-L233
[pci_state]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/transport/pci/device.rs#L614-L639

---

## Findings

Severity legend: **C** = correctness bug that can plausibly cause a guest
freeze under the orchestrator's snapshot flow; **M** = correctness bug that
can corrupt or hang under stress / signals; **L** = low / cosmetic.

### Per-branch backport status

Two distinct upstream commits are involved:

- `67ba7a206` — main ordering fix (`device_manager.save()` before KVM state,
  `prepare_save()` before `transport_state.save()`). **Cherry-picked onto
  PR #8 as [`639196c95`][fix-639] — Findings 1, 2, 3 are now closed on this
  branch.**
- `48a5ae3b2` — companion fix that moves vsock's `send_transport_reset_event`
  into `prepare_save()` so it benefits from the (now correct) save ordering;
  without this, vsock still has its own copy of the bug. Addresses
  **Bug 9** below. **Not yet applied to PR #8.**

| Branch | Ordering fix (`67ba7a206`) | Vsock fix (`48a5ae3b2`) — i.e. **Bug 9** |
|---|---|---|
| `upstream/main` | fixed | fixed |
| `upstream/firecracker-v1.15` | fixed | fixed |
| `upstream/firecracker-v1.14` | **MISSING** | **MISSING** |
| `upstream/firecracker-v1.12` | **MISSING** | **MISSING** |
| `firecracker-v1.14-direct-mem` (PR #8) | **fixed** by `639196c95` | **MISSING** ← still needs cherry-pick |
| `firecracker-v1.12-direct-mem` (prod today) | **MISSING** ← needs backport | **MISSING** |

Findings 4–8 and 10 below are not addressed by either upstream fix and apply
to all branches (including PR #8 post-cherry-pick).

---

### Bug 4 — `prepare_save` silently swallows drain/sync errors [M, not fixed by upstream]

[`src/vmm/src/devices/virtio/block/virtio/device.rs:724-740`][prepare_save]

```rust
fn drain_and_flush(&mut self, discard: bool) {
    if let Err(err) = self.disk.file_engine.drain_and_flush(discard) {
        error!("Failed to drain ops and flush block data: {:?}", err);   // <-- L726, only logged
    }
}

/// Prepare device for being snapshotted.
pub fn prepare_save(&mut self) {
    if !self.is_activated() {
        return;
    }
    self.drain_and_flush(false);                                          // <-- L736, return value dropped
    if let FileEngine::Async(ref _engine) = self.disk.file_engine {
        self.process_async_completion_queue();                            // <-- L738, runs even on partial drain
    }
}
```

`AsyncFileEngine::drain_and_flush` calls
`submit_and_wait_all()` + `file.sync_all()` at
[`src/vmm/src/devices/virtio/block/virtio/io/async_io.rs:271-280`][drain_flush],
which in turn invokes one `io_uring_enter` syscall at
[`src/vmm/src/io_uring/queue/submission.rs:122-153`][submit_syscall] **with no
`EINTR` retry**. If that syscall returns an error (or `sync_all` does):

- The `Err` is logged and dropped.
- `process_async_completion_queue` runs anyway and pops whatever CQEs *did*
  land, but in-flight ops have no CQEs yet → their `PendingRequest` stays in
  the slab; the `desc_idx` has already been advanced past in the avail ring
  (we never `undo_pop` here); no used-ring entry is ever produced.
- The snapshot is written claiming "queue state at next_avail = N" and the
  device is destroyed when the VMM exits.
- On restore, that `desc_idx` is permanently lost — the guest's
  blk_mq tag is never returned → permanent in-kernel hang on that request →
  often a hang on *every* subsequent request once tag tracking diverges.

This produces the same external symptom as the (now-fixed) ordering bugs,
but the proximate cause is different — fixing the ordering does not fix
this. The [PR #6][pr6] test branch began addressing it by introducing a
`PendingAsyncOperations(u32)` error variant; that change is not present on
PR #8.

[submit_syscall]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/io_uring/queue/submission.rs#L122-L153

Sub-issues bundled here:

- No `EINTR` retry around `io_uring_enter` in
  [`SubmissionQueue::submit`][submit_syscall]. The snapshot save thread is the
  VMM thread, which can receive at least the timerfd events of rate limiters
  and any signal the orchestrator decides to send; an `io_uring_enter` that
  returns `EINTR` mid-drain is exactly the silent-loss-of-completions case.
  Related context: [`e2b-dev/firecracker#11`][pr11] documents that snapshot
  paths can interact with kernel async-I/O machinery in non-obvious ways
  (via UFFD+hugepages on aarch64) — so "the syscalls in this code path
  always succeed cleanly in production" is not a safe assumption.
- No invariant check on `engine.num_ops()` after `drain_and_flush` returns
  `Ok` — `num_ops` is exposed inside the crate and tracks `SQ + in-flight +
  unpopped CQ` ([`src/vmm/src/io_uring/mod.rs:240-244`][num_ops]). The PR #6
  branch added `pending_ops()` and used it for exactly this purpose; on the
  current branch the only consumer of that counter is the (now-deleted)
  debug log.

[pr11]: https://github.com/e2b-dev/firecracker/pull/11
[num_ops]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/io_uring/mod.rs#L240-L244

---

### Bug 5 — `Cqe::result` passes negative kernel errno to `from_raw_os_error` [M, latent regression risk]

[`src/vmm/src/io_uring/operation/cqe.rs:25-40`][cqe]

```rust
/// Return the number of bytes successfully transferred by this operation.
pub fn count(&self) -> u32 {
    u32::try_from(self.res).unwrap_or(0)
}

/// Return the result associated to the IO operation.
pub fn result(&self) -> Result<u32, std::io::Error> {
    let res = self.res;
    if res < 0 {
        Err(std::io::Error::from_raw_os_error(res))         // <-- L35: should be -res
    } else {
        Ok(u32::try_from(self.res).unwrap())
    }
}
```

io_uring puts a **negative** errno into `cqe.res` (kernel convention). Rust's
`std::io::Error::from_raw_os_error` expects a **positive** value; passing it
negative makes `ErrorKind` collapse to `Other` regardless of which errno
actually fired. The corresponding test at [`cqe.rs:60-79`][cqe_test]
reproduces the same mistake on both sides of the `assert_eq!`, so it doesn't
catch it.

**This is not cosmetic on PR #8 — there are now consumers that depend on the
broken behaviour.** The discard / write-zeroes feature added by PR #8 detects
"host filesystem doesn't support this" via a `raw_os_error()` comparison
against the *negative* errno:

[`src/vmm/src/devices/virtio/block/virtio/device.rs:590-619`][eopnotsupp]

```rust
let cqe_result = cqe.result();
let pending = cqe.user_data();

// io_uring CQE errors use negated errno, so EOPNOTSUPP is -95.
let is_eopnotsupp = matches!(
    &cqe_result,
    Err(e) if e.raw_os_error() == Some(-libc::EOPNOTSUPP)     // <-- L596: compares against NEGATIVE
);
if is_eopnotsupp && pending.request_type() == RequestType::Discard {
    if !self.disk.discard_unsupported { … self.disk.discard_unsupported = true; }
    …
    continue;
}
```

`-libc::EOPNOTSUPP` is `-95`. Because `Cqe::result` already stored the value
as `-95` (the bug), `e.raw_os_error()` returns `Some(-95)` and the match
succeeds — so the discard/write-zeroes "host doesn't support, disable
locally" path *works on PR #8 precisely because Bug 5 is unfixed*. The
corresponding test cache at lines 2195 and 2453 of the same file pins this
behaviour.

A future "fix" to `Cqe::result` that converts to positive errno (which is
what it should always have done) will silently break this match: the block
device will then return `VIRTIO_BLK_S_IOERR` for every discard / write_zeroes
on a filesystem that doesn't support them, instead of disabling the feature.
So the correct fix is *coordinated* — flip the sign in `Cqe::result` **and**
update this consumer to compare against `Some(libc::EOPNOTSUPP)` (positive).
Worth flagging in any future bug-4 hardening PR that also wants
`ErrorKind::WouldBlock`/`NoSpace`-style branching.

(There are also similar `cqe_result` matches in the e2b-discard path at
[`device.rs:620-668`][eopnotsupp] for `WriteZeroes`.)

[eopnotsupp]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/device.rs#L590-L668

[cqe]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/io_uring/operation/cqe.rs#L25-L40
[cqe_test]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/io_uring/operation/cqe.rs#L60-L79

---

### Bug 6 — `process_queue` can return early leaving SQEs un-kicked [L, defensive]

[`src/vmm/src/devices/virtio/block/virtio/device.rs:494-570`][process_queue]

```rust
pub fn process_queue(&mut self, queue_index: usize) -> Result<(), InvalidAvailIdx> {
    let active_state = self.device_state.active_state().unwrap();
    let queue = &mut self.queues[queue_index];
    let mut used_any = false;

    while let Some(head) = queue.pop_or_enable_notification()? {  // <-- early `?` return
        ...
        // request.process() may call AsyncFileEngine::push_{read,write,flush,write_zeroes}
    }
    queue.advance_used_ring_idx();
    ...
    if let FileEngine::Async(ref mut engine) = self.disk.file_engine
        && let Err(err) = engine.kick_submission_queue()                   // <-- only on normal exit
    {
        error!("BlockError submitting pending block requests: {:?}", err); // <-- swallowed
    }
}
```

Two related sub-issues:

1. If `pop_or_enable_notification` returns `Err(InvalidAvailIdx)`,
   `kick_submission_queue` is never reached and any SQEs already pushed in this
   call sit in the io_uring SQ until the next process_queue invocation. With a
   non-malicious guest this should not happen in practice, but it is
   defense-in-depth that the sync engine doesn't need.
2. `kick_submission_queue` itself (`AsyncFileEngine::kick_submission_queue` at
   [`async_io.rs:250-255`][kick]) issues `io_uring_enter` with no `EINTR`
   retry. Same flavor as Bug 4, in the steady-state path.

[process_queue]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/device.rs#L494-L570
[kick]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/io/async_io.rs#L250-L255

---

### Bug 7 — irqfd injection race with KVM state save [C-leaning M, not fixed by upstream]

Even with the ordering bugs fixed by `639196c95` (device save runs first),
the IRQ that `prepare_save → trigger` produces is delivered to KVM
**asynchronously**:

- MMIO: `IrqTrigger::trigger_irq` writes to `irq_evt` which is registered with
  KVM as an irqfd. For legacy/level IRQs, KVM's irqfd path uses a kernel
  workqueue to inject into the IRQ chip — there is a small window where
  `irq_evt.write(1)` has returned but the IRQ has not yet been observed by
  `KVM_GET_IRQCHIP`/`KVM_GET_LAPIC`.
  See [`mmio.rs:464-477`][mmio_trigger].
- PCI MSI-X (unmasked): `VirtioInterruptMsix::trigger` calls
  `self.vectors.trigger(vector)` which writes the per-vector irqfd. MSI uses
  the lockless fast path in modern kernels, but the syscall round-trip is
  still asynchronous w.r.t. the userspace `write`.

Under heavy I/O + snapshot stress (which is exactly the workload PR #6
exercises), this window is observably non-zero and produces the same
external symptom (lost IRQ → guest stuck in `submit_bio`) that the
ordering fix targeted, but via a different mechanism.

There is no explicit "drain irqfd workqueue" before `save_vcpu_states` / 
`kvm.save_state`. Three known shapes for a fix exist (write+read companion
eventfd; replace irqfd path during snapshot with a synchronous `KVM_SIGNAL_MSI`
/ `KVM_IRQ_LINE`; or briefly resume + re-pause vCPUs to let KVM drain
injected IRQs). Upstream doesn't do any of these today.

---

### Bug 8 — Pending `queue_evt` count is lost across snapshot [L, mitigated by resume_vm.kick]

The ioeventfd backing virtio-mmio QueueNotify and virtio-pci notify writes is
a per-VMM-process eventfd, owned by `VirtioBlock` and created fresh in
`VirtioBlock::new` (at [`device.rs:311-314`][queue_evts]) and rebuilt fresh by
`VirtioBlock::restore` (at [`persist.rs:99`][queue_evts_restore]). If the
guest writes to QueueNotify and the VMM event loop has *not* yet drained that
`queue_evt` count before pause + snapshot, the host-side `next_avail` lags
the guest-side `avail_ring->idx` and the fresh `queue_evt` is empty on the
restored device.

**Mitigation found:** `Vmm::resume_vm()` calls
`device_manager.kick_virtio_devices()` ([`lib.rs:386-388`][resume_kick]) which
calls each VirtIO device's `kick()` trait method
([`device_manager/mod.rs:278-301`][kick_dm]). For block,
[`VirtioBlock`'s `kick`][block_kick] calls `process_virtio_queues()` which
reads the avail ring from guest memory and processes anything it finds —
including any descriptors the guest had written but never had a chance to
notify about. So this scenario *does not freeze* the guest as long as
`resume_vm` runs after the snapshot is loaded — which the orchestrator
[does][infra-resume].

Initially filed as severity-M; downgrading to **L** because the kick path is
defensive coverage for exactly this case (comment at
[`device.rs:212-222`][block_kick]: "kick the block queue(s) to make up for
any pending or in-flight epoll events we may have not captured in
snapshot"). A future refactor that drops `resume_vm` (e.g. autoresume during
load) would re-introduce the risk. See also **Bug 10** below — block's kick
mitigates Bug 8 but does **not** mitigate Bug 7.

[queue_evts]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/device.rs#L312-L313
[queue_evts_restore]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/persist.rs#L99
[resume_kick]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/lib.rs#L386-L388
[kick_dm]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/device_manager/mod.rs#L278-L301
[block_kick]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/device.rs#L219-L228
[infra-resume]: https://github.com/e2b-dev/infra/blob/main/packages/orchestrator/pkg/sandbox/fc/process.go

---

### Bug 9 — vsock has the same ordering bug pattern; companion upstream commit still missing [C]

[`src/vmm/src/device_manager/persist.rs:287-318`][vsock_mmio_save]

```rust
virtio_ids::VIRTIO_ID_VSOCK => {
    let vsock = locked_device.as_mut_any()
        .downcast_mut::<Vsock<VsockUnixBackend>>().unwrap();

    // Send Transport event to reset connections if device
    // is activated.
    if vsock.is_activated() {
        vsock.send_transport_reset_event().unwrap_or_else(|err| {     // <-- AFTER transport_state.save()
            error!("Failed to send reset transport event: {:?}", err);
        });
    }

    // Save state after potential notification to the guest. This
    // way we save changes to the queue the notification can cause.
    let device_state = VsockState { ... };
    states.vsock_device = Some(VirtioDeviceState {
        device_id, device_state,
        transport_state /* <-- captured at L230, BEFORE the reset event */, device_info,
    });
}
```

`send_transport_reset_event` puts a `VIRTIO_VSOCK_EVENT_TRANSPORT_RESET` into
the event virtqueue, advances the used ring, and calls `signal_used_queue(EVQ)
→ trigger_irq(Vring) → irq_status.fetch_or(VIRTIO_MMIO_INT_VRING, …)` —
see [`vsock/device.rs:259-281`][vsock_reset]. The transport `interrupt_status`
captured at line 230 doesn't have that bit set.

PCI has the same pattern at [`pci_mngr.rs:351-380`][vsock_pci_save] (also
affects `msix_state` / PBA bits).

The in-source comment ("Save state after potential notification … This way we
save changes to the queue the notification can cause") is misleading — it
covers the `device_state` save below, but **not** the `transport_state` which
was already taken above. This bug predates the block bug; the upstream commit
message of [`67ba7a206`][upstream-fix] explicitly says vsock was the only
other device using `prepare_save`-style hooks but "doesn't modify VirtIO
state, neither sends interrupts" — at the time it was written, that was true
of vsock's `prepare_save` (which didn't exist for vsock yet), not of the
device_manager-level reset call.

**Two upstream commits are needed in series**:

1. [`67ba7a206`][upstream-fix] — establishes the contract that
   `prepare_save()` runs *before* `transport_state.save()`. **Already
   applied to PR #8 as [`639196c95`][fix-639].**
2. [`48a5ae3b2`][upstream-vsock] — refactors vsock so that
   `send_transport_reset_event` is moved into a new `Vsock::prepare_save()`,
   which (post-fix #1) runs before `transport_state.save()`. The commit
   adds a note in `vsock/device.rs` documenting the dependency on the kick
   path for redundancy. **Still missing on PR #8.** As verified by
   `grep "fn prepare_save" src/vmm/src/devices/virtio/vsock/device.rs` →
   empty.

**Mitigation today**: `Vsock::kick()`
([`vsock/device.rs:382-393`][vsock_kick]) **unconditionally** re-fires
`signal_used_queue(EVQ_INDEX)` on resume — unlike block's `kick`, this is a
direct IRQ retrigger that does not depend on the avail ring containing
anything. So in practice the guest receives the `TRANSPORT_RESET` IRQ on
resume even with the bug present. This is fragile — it relies on (a)
`resume_vm` being called, and (b) the guest re-arming its used-event such
that the second trigger is delivered (notification-suppression-aware guests
will see it).

Connections to the host UDS are intentionally discarded across snapshot
(comment at [`vsock/device.rs:382-388`][vsock_kick]: "Vsock has complicated
protocol that isn't resilient to any packet loss"), so there's no half-open
state to preserve. The fragility is purely about the reset event delivery.

[vsock_mmio_save]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/device_manager/persist.rs#L287-L318
[vsock_pci_save]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/device_manager/pci_mngr.rs#L351-L380
[vsock_reset]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/vsock/device.rs#L259-L281
[vsock_kick]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/vsock/device.rs#L382-L393

---

### Bug 10 — Block `kick()` doesn't re-fire an interrupt for already-completed used-ring entries [C, fixes blocked by Bug 7]

Compare the two `kick()` implementations:

`VirtioBlock::kick` ([`device.rs:212-222`][block_kick]):

```rust
fn kick(&mut self) {
    // If device is activated, kick the block queue(s) to make up for any
    // pending or in-flight epoll events we may have not captured in
    // snapshot. No need to kick Ratelimiters
    // because they are restored 'unblocked' so
    // any inflight `timer_fd` events can be safely discarded.
    if self.is_activated() {
        info!("kick block {}.", self.id());
        self.process_virtio_queues();
    }
}
```

`Vsock::kick` ([`vsock/device.rs:382-393`][vsock_kick]):

```rust
fn kick(&mut self) {
    if self.is_activated() {
        info!("kick vsock {}.", self.id());
        self.signal_used_queue(EVQ_INDEX).unwrap();    // <-- unconditional IRQ retrigger
    }
}
```

Block's kick only drains the **avail** ring; if the avail ring is empty (no
new descriptors since the snapshot), `process_virtio_queues` produces zero
used-ring entries, `prepare_kick` returns false, and **no interrupt is
triggered**. This is exactly the failure mode that Bug 7 (irqfd workqueue
race) produces — new used-ring entries already in guest memory, IRQ never
delivered, guest parked on the old descriptors waiting for completion.

For vsock the equivalent failure is masked by the unconditional re-trigger.
For block (which is the device the original PR #6 freeze test exercises)
there is no such safety net, so even with the ordering bugs fixed by
`639196c95`, a Bug 7 occurrence at snapshot time can still freeze the guest.
The robust answer would be for `VirtioBlock::kick` (and net's kick at
[`net/device.rs:1042-1052`][net_kick]) to call `prepare_kick` on each queue
and re-trigger the IRQ if there are unacked used-ring entries — i.e. mirror
what vsock does. Filing as **C** because it's the residual hole that makes
a Bug 7 occurrence not recoverable.

[net_kick]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/net/device.rs#L1042-L1052

---

## Snapshot-related "kick" semantics — quick reference

A single table of where IRQ retrigger does and doesn't happen on resume:

| Device | `kick()` behavior | Recovers lost-IRQ class? |
|---|---|---|
| Block ([`device.rs:212-222`][block_kick]) | `process_virtio_queues()` — reads avail ring only | **No** — needs new descriptors to fire any IRQ |
| Net ([`net/device.rs:1042-1052`][net_kick]) | `process_virtio_queues()` — reads RX+TX | **No** — same as block |
| Vsock ([`vsock/device.rs:382-393`][vsock_kick]) | `signal_used_queue(EVQ)` — unconditional IRQ retrigger on EVQ | Yes (for EVQ only) |
| Balloon ([`balloon/device.rs:1000-1008`][balloon_kick]) | replays queued FPH work via `process_virtio_queues` | **No** for IRQ; OK for FPH replay |
| RNG/Entropy / PMem / virtio-mem | default `fn kick(&mut self) {}` (no-op) | **No** |

[balloon_kick]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/balloon/device.rs#L1000-L1008

This means the Bug 7 lost-IRQ class is **only** mitigated by explicit
kernel-state-drain logic at snapshot time, which doesn't exist anywhere in
this tree. The (now-applied) save-side ordering fix `639196c95` closes the
synchronous lost-IRQ window but does not handle the kernel-side asynchronous
irqfd injection window.

There is no resume-side safety net for block/net/RNG/PMem/virtio-mem.

---

## What I verified is correct

To narrow the search space, the following were checked and are not bugs:

- **`IORING_FEAT_NODROP` is required at setup.** The kernel will not silently
  drop completed entries — see
  [`io_uring/mod.rs:334-346`][nodrop].
- **`FullSQueue` / `FullCQueue` throttling** correctly returns the descriptor
  to the avail ring via `queue.undo_pop()` and sets `is_io_engine_throttled`
  ([`device.rs:520-540`][throttle]). The completion path resets it
  ([`device.rs:674-687`][throttle_clear]). Sync vs Async paths agree.
- **`is_io_engine_throttled` not persisted across snapshot.** Restored to
  `false` ([`block/virtio/persist.rs:149`][throttle_persist]). This is
  correct because the restored io_uring is empty (no throttling can apply)
  *and* any descriptors that were undo_pop'd back to the avail ring will be
  re-picked up by `resume_vm → kick → process_virtio_queues`. So no freeze.
- **Rate-limiter blocked at snapshot is recovered on resume.** `RateLimiter`
  state is persisted via `TokenBucketState { size, one_time_burst, refill_time,
  budget, elapsed_ns }` ([`rate_limiter/persist.rs:11-50`][rl_persist]). On
  restore the timerfd is rebuilt disarmed and `timer_active = false`
  ([`rate_limiter/persist.rs:73-87`][rl_restore]). The `last_update` reflects
  the snapshot-time elapsed-since-last-activity, so `auto_replenish` does the
  right thing in restore-time coordinates (no "free bucket refill" exploit;
  no over-replenishment). If the bucket was exhausted at snapshot, the first
  `consume()` after restore fails and re-arms the timer; the
  `process_rate_limiter_event` then re-fires `process_queue`. Combined with
  `kick → process_virtio_queues` on resume, descriptors that were
  rate-limit-throttled at snapshot time are eventually processed.
  (Caveat: `OverConsumption` penalty timer state is not preserved, so a
  one-time burst window may be slightly larger after restore than before.
  Behavioural delta, not correctness.)
- **Dirty memory tracking on reads** uses
  `WrappedRequest::new_with_dirty_tracking(addr, req)` at push time and
  `mark_dirty_mem_and_unwrap(mem, count)` at pop time
  ([`async_io.rs:53-67`][dirty] and [`async_io.rs:286-299`][dirty_pop]).
  On error `count == 0`, so `mark_dirty` is a no-op. Correct.
- **Used-ring publication ordering.** `add_used` writes the used element
  then `advance_used_ring_idx` issues a `release` fence before publishing
  `used_ring->idx`
  ([`queue.rs:557-606`][queue_used]).
- **Pointers handed to io_uring are stable.** `mem.get_slice(addr,
  count).ptr_guard_mut().as_ptr()` returns a host VMA pointer; guest memory
  regions are pinned for VM lifetime. Snapshot save reads guest memory but
  doesn't munmap.
- **Seccomp covers the io_uring syscalls.** `io_uring_setup`,
  `io_uring_enter`, `io_uring_register` plus the mmap path are in
  [`resources/seccomp/x86_64-unknown-linux-musl.json`][seccomp].
- **Snapshot restore activates the device.** `MMIODeviceManager::restore`
  → `restore_helper` calls `device.lock().activate(mem, interrupt)` when the
  saved `is_activated()` is true
  ([`device_manager/persist.rs:388-427`][restore_activate]) — so the IRQ /
  used-ring state is re-armed, *provided* it was correctly snapshotted
  (which the now-fixed ordering bugs used to violate).
- **VMGenID interrupt on restore** is correctly handled by saving devices
  *after* KVM state on the restore side (see
  [`src/vmm/src/builder.rs:497-510`][restore_order] and the comment there);
  this is the opposite ordering of save and is intentional.
- **Net `prepare_save`** does not trigger interrupts or touch transport state
  ([`net/device.rs:943-960`][net_prep]); the upstream commit message is
  accurate about this. There is a delayed-publish edge case
  (advances `next_used` without calling `advance_used_ring_idx`), but it
  resolves on the next RX frame after restore — not a freeze, just a
  one-event-delay for a single deferred frame.

[nodrop]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/io_uring/mod.rs#L334-L346
[throttle]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/device.rs#L520-L540
[throttle_clear]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/device.rs#L674-L687
[throttle_persist]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/persist.rs#L149
[rl_persist]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/rate_limiter/persist.rs#L11-L50
[rl_restore]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/rate_limiter/persist.rs#L73-L87
[dirty]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/io/async_io.rs#L53-L67
[dirty_pop]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/block/virtio/io/async_io.rs#L286-L299
[queue_used]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/queue.rs#L557-L606
[seccomp]: https://github.com/e2b-dev/firecracker/blob/639196c95/resources/seccomp/x86_64-unknown-linux-musl.json
[restore_activate]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/device_manager/persist.rs#L388-L427
[restore_order]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/builder.rs#L497-L510
[net_prep]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/net/device.rs#L943-L960

## Implications for net-vs-block VMM contention

The original motivation for going async is that the VMM thread is the
single-threaded emulator for all virtio devices, and the sync block engine
blocks on `pread`/`pwrite`/`fsync` — starving net. None of the bugs above
re-introduce that contention; the async path's per-`process_queue` cost is one
non-blocking `io_uring_enter` from `kick_submission_queue`
([`async_io.rs:250-255`][kick]), independent of in-flight depth. So the
performance argument for async stands; the open question is purely
correctness during snapshot, which is what this document is about.

## Broader snapshot audit — beyond the async-block path

The findings below come from a second-pass audit covering:

- the e2b-specific snapshot extensions on this branch (`/memory/mappings`,
  optional `mem_file_path`, UFFD WP),
- the orchestrator's `Pause` / `Resume` flow
  ([`infra::Sandbox.Pause`][infra-pause], [`fc::Process.Resume`][infra-resume]),
- the non-async devices (vsock was covered in Bug 9; this pass adds balloon
  FPH, MMDS, serial, ACPI, KVM-clock/TSC),
- the rate-limiter PATCH semantics used by the orchestrator on resume.

Same severity rubric as above.

---

### Finding P2-1 — Optional `mem_file_path = None` skips KVM dirty-log + region-bitmap reset [M, scenario-bound, diff-snapshot only]

[`src/vmm/src/persist/mod.rs:159-181`][create_snapshot]:

```rust
snapshot_state_to_file(&microvm_state, &params.snapshot_path)?;

if let Some(mem_file_path) = params.mem_file_path.as_ref() {
    vmm.vm
        .snapshot_memory_to_file(mem_file_path, params.snapshot_type, vmm.page_size)?;
}

// We need to mark queues as dirty again for all activated devices. …
vmm.device_manager
    .mark_virtio_queue_memory_dirty(vmm.vm.guest_memory());
```

The `Vm::snapshot_memory_to_file` body
[`vstate/vm.rs:335-395`][snapshot_mem] is what calls
`reset_dirty_bitmap()` + `guest_memory().reset_dirty()` at
[`vstate/vm.rs:385-389`][reset_dirty] for the `SnapshotType::Full` branch:

```rust
SnapshotType::Full => {
    self.reset_dirty_bitmap();
    self.guest_memory().reset_dirty();
}
```

When the orchestrator passes `mem_file_path = None` (its production mode,
since it extracts memory externally), `snapshot_memory_to_file` is skipped
entirely, so **the dirty-state reset never runs**. The next diff snapshot
in a chained workflow will then over-report dirty pages — pages that became
dirty *before* the current snapshot will still be flagged dirty going into
the *next* one.

For Full snapshots in isolation this is benign. For chained
template → diff → diff workflows (which `pauseProcessRootfs` /
`pauseProcessMemory` in the orchestrator support) it inflates the diff
footprint and can mis-represent which pages actually changed.

[create_snapshot]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/persist/mod.rs#L159-L181
[snapshot_mem]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/vstate/vm.rs#L335-L395
[reset_dirty]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/vstate/vm.rs#L385-L389

---

### Finding P2-2 — `/memory/mappings` is not gated on `VmState::Paused` [M, contract gap]

[`src/vmm/src/rpc_interface.rs:963-1010`][meminfo_rpc]:

```rust
fn get_guest_memory_mappings(&self) -> Result<VmmData, VmmActionError> {
    // … no VmState::Paused check
}

fn get_guest_memory_info(&self) -> Result<VmmData, VmmActionError> {
    let vmm = …;
    if vmm.instance_info.state != VmState::Paused {                 // L981
        return Err(…);
    }
    …
}

fn get_dirty_memory_info(&self) -> Result<VmmData, VmmActionError> {
    let vmm = …;
    if vmm.instance_info.state != VmState::Paused {                 // L1000
        return Err(…);
    }
    …
}
```

`GetMemory` and `GetMemoryDirty` correctly require paused; `GetMemoryMappings`
doesn't. If the host-virtual addresses returned by `/memory/mappings` are
later used to read memory **after** the vCPUs were resumed (or were never
paused in the first place), the read will race with guest writes and produce
torn data.

The orchestrator currently calls these endpoints only after pause, so this
is a contract bug, not an exploited one — but `/memory/mappings` is exactly
the kind of API external tooling integrates against, and the missing gate
makes it a footgun.

[meminfo_rpc]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/rpc_interface.rs#L963-L1010

---

### Finding P2-3 — `/memory/mappings` layout disagrees with `dump()` when virtio-mem hotplug has unplugged slots [C, latent]

Two layouts for "linear memfile offset" exist in this tree:

A. [`src/vmm/src/lib.rs:701-724`][guest_mappings] — `guest_memory_mappings`
   iterates `flat_map(|r| r.plugged_slots())` and accumulates `offset` by
   plugged-slot size only.

B. [`src/vmm/src/vstate/memory.rs:689-701`][dump_memory] —
   `GuestMemoryExtension::dump` walks **all** slots; for unplugged slots it
   only `seek`s the file forward, leaving zeroed holes.

If virtio-mem is enabled and any slot in a region is unplugged, these two
layouts diverge: the memfile produced by `dump()` has a hole at the unplugged
slot position, but `/memory/mappings` reports a contiguous offset that
collapses over the hole. External tooling that correlates the API output
against the memfile (which is exactly what the orchestrator does in
`pauseProcessMemory`) reads the wrong bytes for any region following the
first hole.

virtio-mem is not used by the orchestrator today, so this is latent. It
becomes load-bearing the moment virtio-mem is turned on (which is on the
roadmap, given that PR #13 already added virtio-block discard/write-zeroes).

[guest_mappings]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/lib.rs#L701-L724
[dump_memory]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/vstate/memory.rs#L689-L701

---

### Finding P2-4 — Empty `{}` rate-limiter PATCH does not clear snapshot-restored limits [M, orchestrator expectation mismatch]

The orchestrator's [`fc/client.go:setTxRateLimit`][orch_setrl] documents:

> Both buckets are disabled when their `BucketSize < 0`; if all are disabled
> **an empty RateLimiter is sent to reset any limit persisted in a snapshot**.

The Go model [`shared/pkg/fc/models/rate_limiter.go`][orch_rl_model] uses
`json:"bandwidth,omitempty"` + `json:"ops,omitempty"`, so the empty case
serializes as `{}` with **both fields omitted**.

On the FC side, [`src/vmm/src/rpc_interface.rs:914-940`][fc_patch] only
applies an update when `rate_limiter` is `Some`:

```rust
if new_cfg.rate_limiter.is_some() {
    vmm.update_block_rate_limiter(
        &new_cfg.drive_id,
        RateLimiterUpdate::from(new_cfg.rate_limiter).bandwidth,
        RateLimiterUpdate::from(new_cfg.rate_limiter).ops,
    )
```

And [`src/vmm/src/vmm_config/mod.rs:96-115`][bucket_update] returns
`BucketUpdate::None` for `tb_cfg: None`. So an empty `{}` JSON object on the
wire produces `BandwidthUpdate::None` + `OpsUpdate::None` and **the
snapshot-restored buckets are left untouched**. The orchestrator's "reset"
path silently does nothing.

Concrete consequence: if a snapshot was taken with a rate limiter and the
new deployment configuration says "no limits", the restored sandbox still
runs with the snapshot-era limits. The guest sees throttling that the
operator believes they removed.

[orch_setrl]: https://github.com/e2b-dev/infra/blob/main/packages/orchestrator/pkg/sandbox/fc/client.go
[orch_rl_model]: https://github.com/e2b-dev/infra/blob/main/packages/shared/pkg/fc/models/rate_limiter.go
[fc_patch]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/rpc_interface.rs#L914-L940
[bucket_update]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/vmm_config/mod.rs#L96-L115

---

### Finding P2-5 — DrainBalloon timeout leaves `hinting_state` mid-protocol but restore force-resets `host_cmd` to DONE [L, scenario-bound]

Orchestrator code wraps [`process.DrainBalloon`][orch_drain] in a per-use-case
timeout; on timeout it logs and continues.

`BalloonState.hinting_state` is persisted at
[`balloon/persist.rs:100-135`][balloon_save] but `BalloonConfig.
free_page_hint_cmd_id` is force-reset to `FREE_PAGE_HINT_DONE` on restore at
[`balloon/persist.rs:180-210`][balloon_restore]:

```rust
let config = BalloonConfig {
    …,
    free_page_hint_cmd_id: FREE_PAGE_HINT_DONE,         // <-- always DONE on restore
};
…
let mut balloon = Balloon::new(config, … )?;
balloon.hinting_state = state.hinting_state;            // <-- but in-progress state preserved
```

So a snapshot taken while a hinting cycle was mid-chain restores with
`hinting_state` saying "mid-chain" and `cmd_id` saying "done". Whether the
guest driver gracefully handles that discrepancy depends on its
implementation; the spec-compliant behavior is to ignore stale hint state
when the cmd_id resets, but it's a delta worth noting.

[orch_drain]: https://github.com/e2b-dev/infra/blob/main/packages/orchestrator/pkg/sandbox/fc/process.go
[balloon_save]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/balloon/persist.rs#L100-L135
[balloon_restore]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/balloon/persist.rs#L180-L210

---

### Finding P2-6 — MMDS data contents are not persisted across snapshot [L, race window]

[`device_manager/persist.rs:119-123`][mmds_state]:

```rust
pub struct MmdsState {
    pub version: MmdsVersion,
    pub imds_compat: bool,
}
```

Only `version` and `imds_compat` are saved. The actual IMDS JSON datastore
(`Mmds::data_store`) is not in the snapshot. On restore,
`set_mmds_basic_config` rebuilds an empty MMDS; the orchestrator calls
`setMmds` only **after** `resumeVM` (see [`fc::Process.Resume`][infra-resume]
~line 596+), so guests that read MMDS during the brief window between
`resumeVM` and `setMmds` will see `NotInitialized` / empty content.

No cross-sandbox leak (the previous MMDS body genuinely isn't there) —
the issue is the "uninitialized for a few ms after resume" window for the
new guest.

[mmds_state]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/device_manager/persist.rs#L119-L123
[infra-resume]: https://github.com/e2b-dev/infra/blob/main/packages/orchestrator/pkg/sandbox/fc/process.go

---

### Finding P2-7 — Serial (aarch64) state is only partially persisted; the `IER_RDA` workaround is the only mitigation [L, documented]

[`device_manager/persist.rs:211-226`][serial_save] saves only `DeviceType` +
`MMIODeviceInfo` (address, length, gsi). UART FIFO contents, IER, LCR,
output rate-limiter (if any) are not persisted.

[`device_manager/mod.rs:495-540`][serial_init] documents an explicit
workaround (`emulate_serial_init`) that sets `IER_RDA` after restore to
re-enable RX interrupts. The serial output rate-limiter commit
[`9a49dcb03`][serial_rl] is **not** an ancestor of this branch, so the
rate-limiter persistence question doesn't apply here yet — but if it lands,
it inherits the same "rate-limiter timerfd disarmed on restore" pattern
discussed under Bug 5.

[serial_save]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/device_manager/persist.rs#L211-L226
[serial_init]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/device_manager/mod.rs#L495-L540
[serial_rl]: https://github.com/firecracker-microvm/firecracker/commit/9a49dcb03

---

### Finding P2-8 — `SIGTERM` during `CreateSnapshot` can leave a truncated snapshot file [L, scenario-bound]

Orchestrator [`process.Stop`][orch_stop] sends `SIGTERM`, then `SIGKILL`
after 10s. `snapshot_state_to_file` at
[`src/vmm/src/persist/mod.rs:184-203`][snapshot_state_write] opens the
snapfile with `OpenOptions::create + truncate + write`, then `flush` +
`sync_all`. If a SIGTERM lands between `truncate` and `sync_all`, the file
on disk is a syntactically invalid blob.

Concretely, this is only reachable if something in the snapshot pipeline
calls `Stop` while `CreateSnapshot` is in flight — which is not the
happy path, but is reachable from context cancellation chains during
pause-related errors. The reload-side `snapshot_state_from_file` does
strict version + format validation so a partial file will be rejected on
restore (failure rather than silent corruption), but cleanup of the
truncated file is the operator's problem.

[orch_stop]: https://github.com/e2b-dev/infra/blob/main/packages/orchestrator/pkg/sandbox/fc/process.go
[snapshot_state_write]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/persist/mod.rs#L184-L203

---

## What I verified is correct — broader audit

These are checks I did during the second pass that are not bugs and are
unlikely to need attention; documenting them so they don't get rediscovered.

- **`FlushMetrics` between `Pause` and `CreateSnapshot` is safe** — it only
  writes metrics, does not re-enter the virtio loop
  ([`rpc_interface.rs:851-858`][flush_metrics]). The orchestrator's
  `Sandbox.Pause` calls it between pause and snapshot deliberately.
- **Snapshot version validation is strict** — only major.minor `(8,0)`,
  `(6,0)`, `(4,0)` accepted, hard fail otherwise
  ([`persist/mod.rs:460-478`][snap_version]).
- **VMGenID generates a fresh random 128-bit ID on every restore** — not
  replayed from snapshot ([`vmgenid.rs:46-62`][vmgenid_make]). Each
  restored sandbox gets a unique generation ID; the crypto/RNG-reseeding
  contract holds.
- **TSC freq scaling + restore ordering** —
  [`builder.rs:462-482`][builder_tsc] calls `set_tsc_khz` on each vCPU
  before its `restore_state`, on the VMM thread, before any vCPU thread
  runs. No race.
- **`cc4bef8b2` (kvm-clock no-monotonic-jump fix) is correctly wired on
  this branch** — clock flags only carry `KVM_CLOCK_REALTIME` when the
  load request opts in ([`arch/x86_64/vm.rs:128-150`][vm_clock]).
- **UFFD WP registration happens before device restore.**
  `guest_memory_from_uffd` runs in `restore_from_snapshot` before
  `build_microvm_from_snapshot`; device-restore-induced writes can't race
  with WP setup.
- **`mark_virtio_queue_memory_dirty` after snapshot is safe.** It does not
  rewrite guest bytes — only marks Firecracker's bitmap for the next diff
  ([`device_manager/mod.rs:303-329`][mark_dirty],
  [`queue.rs:335-371`][queue_init]).
- **Rootfs symlink dance is correctly sequenced.** The `/dev/null`
  symlink at `SandboxCacheRootfsLinkPath` is overwritten with the real
  rootfs before `loadSnapshot` is called — `fc::Process.Resume` gates
  `loadSnapshot` behind an `errgroup.Wait()` that includes the symlink
  step (see [`fc::Process.Resume`][infra-resume]).
- **IoEngine type matches save → restore.** The orchestrator's `Resume`
  path does not call `setRootfsDrive`, so the engine type comes only from
  `VirtioBlockState.file_engine_type` restored from the snapshot.
- **VMGenID restore IRQ ordering vs KVM state.**
  `DeviceManager::restore` runs after `vm.restore_state` on x86_64
  ([`builder.rs:494-517`][builder_restore]); so the gsi-registered irqfd
  writes hit a fully-restored KVM IRQ chip. The comment at
  `builder.rs:497-510` is accurate. **No other device's `Persist::restore`
  injects IRQs**, so this is the only constraint.
- **`shared_mem` is not present in this tree.** No `MAP_SHARED` for the
  main guest memory mapping is implemented behind a `shared_mem` flag on
  PR #8. The only `MAP_SHARED` path is the existing `memfd_backed` route.

[flush_metrics]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/rpc_interface.rs#L851-L858
[snap_version]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/persist/mod.rs#L460-L478
[vmgenid_make]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/acpi/vmgenid.rs#L46-L62
[builder_tsc]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/builder.rs#L462-L482
[vm_clock]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/arch/x86_64/vm.rs#L128-L150
[mark_dirty]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/device_manager/mod.rs#L303-L329
[queue_init]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/devices/virtio/queue.rs#L335-L371
[builder_restore]: https://github.com/e2b-dev/firecracker/blob/639196c95/src/vmm/src/builder.rs#L494-L517

---

## Cosmetic / API drift (not bugs)

- **`meminfo.rs` doc comment** claims mappings are "guest physical to host
  virtual" but the payload is host VA + size + linear-backend offset +
  page size (guest PA only implied by correlating against
  `MicrovmState.vm_state.memory.regions`). Misleading docstring.
- **Swagger `/memory/mappings` description** mentions a "skippable pages
  bitmap" in the summary but the response schema only contains `mappings`.
- **`GuestRegionUffdMapping`** includes a deprecated `page_size_kib` field
  in the Rust struct that the swagger schema omits — strict JSON clients
  could choke on the extra field.

These are independent of the snapshot-correctness work; just leaving the
flag here so we don't keep rediscovering them.

---

## References

- Upstream fixes:
  - [`firecracker-microvm/firecracker@67ba7a206`][upstream-fix]
    ("fix: saving/restoring async IO engine transport state", 2025-12-15) —
    addresses the lib.rs / device-manager / pci-manager ordering bugs by
    reordering `device_manager.save()` before KVM state and `prepare_save()`
    before `transport_state.save()`. **Cherry-picked onto PR #8 as
    [`639196c95`][fix-639].**
  - [`firecracker-microvm/firecracker@48a5ae3b2`][upstream-vsock]
    ("refactor(vsock): Send reset event before saving transport state",
    2026-02-16) — addresses Bug 9 by moving vsock's reset event into
    `Vsock::prepare_save()` so it benefits from the new ordering. Depends
    on the prior commit. **Not yet on PR #8.**
- Freeze reproducer (closed):
  [`e2b-dev/firecracker#6`][pr6] (`test_snapshot_with_heavy_async_io`).
- Current target branch:
  [`e2b-dev/firecracker#8`][pr8] — head
  [`639196c95`][head].
- Sibling pause/snapshot-related fork patches that demonstrate signal delivery
  *can* occur during snapshot work:
  [`e2b-dev/firecracker#11`][pr11] (closed),
  [`e2b-dev/firecracker#12`][pr12] (closed).
- Orchestrator pause path:
  [`infra/packages/orchestrator/pkg/sandbox/sandbox.go::Sandbox.Pause`][infra-pause].

[pr12]: https://github.com/e2b-dev/firecracker/pull/12
