# Async block I/O snapshot correctness analysis

Read-only analysis of the async-engine (`io_uring`) virtio-block code path and
how it interacts with snapshot save/restore. The goal is to enumerate every
correctness issue that could cause the "post-resume guest freeze" originally
reproduced by [`e2b-dev/firecracker#6`][pr6] (the `Freeze test reproduction`
branch). **No fixes are proposed here — only findings, with links to every
relevant source location.**

[pr6]: https://github.com/e2b-dev/firecracker/pull/6

## Scope

- Target branch: [`firecracker-v1.14-direct-mem`][br] — i.e. the head of the
  open [`e2b-dev/firecracker#8`][pr8] ("[v1.14] Expose memory mapping & dirty
  pages; Make memfile dump optional"). This is the branch the orchestrator
  is going to consume next.
- Baseline commit at time of writing: [`f0a35a156`][head] (
  `docs(balloon): document WAIT_ON_ACK feature`). All file links below are
  pinned to that SHA so they don't bitrot if the branch advances.
- Active branch used in production today: [`firecracker-v1.12-direct-mem`][br112]
  — same root causes apply, see [Per-branch backport status](#per-branch-backport-status).
- Downstream consumer: the orchestrator's snapshot path in
  [`e2b-dev/infra`][infra-pause]
  (`packages/orchestrator/pkg/sandbox/sandbox.go::Sandbox.Pause`) which calls
  `Pause` + `CreateSnapshot` on a paused VM with heavy guest I/O in-flight.

[br]: https://github.com/e2b-dev/firecracker/tree/firecracker-v1.14-direct-mem
[br112]: https://github.com/e2b-dev/firecracker/tree/firecracker-v1.12-direct-mem
[pr8]: https://github.com/e2b-dev/firecracker/pull/8
[head]: https://github.com/e2b-dev/firecracker/tree/f0a35a156
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
save path. That fix is on `upstream/main` and was backported to
`upstream/firecracker-v1.15` as [`48a5ae3b2`][upstream-vsock]; it was **never
backported to `upstream/firecracker-v1.14` or `upstream/firecracker-v1.12`**,
and the e2b fork inherits that gap.

[upstream-fix]: https://github.com/firecracker-microvm/firecracker/commit/67ba7a20692a5d1a2fd9218523a9b3ccde9e4a37
[upstream-vsock]: https://github.com/firecracker-microvm/firecracker/commit/48a5ae3b2

## Save-path call chain (where the bugs live)

The save side of `CreateSnapshot` walks roughly like this on PR #8 head:

| Step | Symbol | Source |
|---|---|---|
| 1 | `pub fn create_snapshot(vmm, vm_info, params)` | [`src/vmm/src/persist/mod.rs:159-182`][create_snapshot] |
| 2 | `Vmm::save_state(vm_info)` | [`src/vmm/src/lib.rs:444-471`][save_state] |
| 3 | `save_vcpu_states()` (KVM_GET_LAPIC, KVM_GET_VCPU_EVENTS, …) | [`src/vmm/src/lib.rs:447`][save_state] / [`src/vmm/src/arch/x86_64/vcpu.rs:555-614`][vcpu_save] |
| 4 | `self.kvm.save_state()` (KVM_GET_IRQCHIP) | [`src/vmm/src/lib.rs:448`][save_state] |
| 5 | `self.vm.save_state()` | [`src/vmm/src/lib.rs:449-460`][save_state] |
| 6 | `self.device_manager.save()` → per-transport `transport_state.save()` + per-device `block.prepare_save()` + `block.save()` | [`src/vmm/src/lib.rs:461`][save_state] |
| 7 | `VirtioBlock::prepare_save()` (only when activated): `drain_and_flush(false)` → `process_async_completion_queue()` | [`src/vmm/src/devices/virtio/block/virtio/device.rs:730-740`][prepare_save] |
| 8 | `AsyncFileEngine::drain_and_flush(false)` → `drain(false)` (submit + wait) + `file.sync_all()` | [`src/vmm/src/devices/virtio/block/virtio/io/async_io.rs:271-280`][drain_flush] |
| 9 | `process_async_completion_queue()` pops every CQE, `queue.add_used(...)`, `interrupt.trigger(Queue(0))` | [`src/vmm/src/devices/virtio/block/virtio/device.rs:575-624`][completion] |

[create_snapshot]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/persist/mod.rs#L159-L182
[save_state]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/lib.rs#L444-L471
[vcpu_save]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/arch/x86_64/vcpu.rs#L555-L614
[prepare_save]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/devices/virtio/block/virtio/device.rs#L730-L740
[drain_flush]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/devices/virtio/block/virtio/io/async_io.rs#L271-L280
[completion]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/devices/virtio/block/virtio/device.rs#L575-L624

The interrupt actually delivered to KVM goes through one of:

- MMIO `IrqTrigger::trigger_irq`
  — bumps `irq_status` then writes to the irqfd:
  [`src/vmm/src/devices/virtio/transport/mmio.rs:405-477`][mmio_trigger]
- PCI `VirtioInterruptMsix::trigger`
  — sets PBA bit if masked, otherwise writes the MSI-X eventfd:
  [`src/vmm/src/devices/virtio/transport/pci/device.rs:669-697`][pci_trigger]

[mmio_trigger]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/devices/virtio/transport/mmio.rs#L405-L477
[pci_trigger]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/devices/virtio/transport/pci/device.rs#L669-L697

The transport state captured into the snapshot lives in:

- MMIO: `MmioTransportState { …, interrupt_status }` — captured atomically from
  `self.interrupt.irq_status` at
  [`src/vmm/src/devices/virtio/persist.rs:195-233`][mmio_state].
- PCI: `VirtioPciDeviceState { …, msix_state, … }` — captured from `MsixConfig`
  (which holds the masked / PBA bits) at
  [`src/vmm/src/devices/virtio/transport/pci/device.rs:614-639`][pci_state].

[mmio_state]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/devices/virtio/persist.rs#L195-L233
[pci_state]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/devices/virtio/transport/pci/device.rs#L614-L639

---

## Findings

Severity legend: **C** = correctness bug that can plausibly cause a guest
freeze under the orchestrator's snapshot flow; **M** = correctness bug that
can corrupt or hang under stress / signals; **L** = low / cosmetic.

### Per-branch backport status

Two distinct upstream commits are involved:

- `67ba7a206` — main fix for Bugs 1, 2, 3 (ordering of `device_manager.save()` vs
  KVM state, and ordering of `prepare_save()` vs `transport_state.save()`).
- `48a5ae3b2` — companion fix that moves vsock's `send_transport_reset_event`
  into `prepare_save()` so it benefits from the (now correct) save ordering;
  without this, vsock still has its own copy of the bug. Addresses
  **Bug 9** below.

| Branch | Bug 1 (lib.rs) | Bug 2 (MMIO) | Bug 3 (PCI) | Bug 9 (vsock) | Notes |
|---|---|---|---|---|---|
| `upstream/main` | fixed | fixed | fixed | fixed | `67ba7a206` + `48a5ae3b2` |
| `upstream/firecracker-v1.15` | fixed | fixed | fixed | fixed | both backported |
| `upstream/firecracker-v1.14` | **MISSING** | **MISSING** | **MISSING** | **MISSING** | last release v1.14.4 (2026-04-02) |
| `upstream/firecracker-v1.12` | **MISSING** | **MISSING** | N/A (no PCI yet) | **MISSING** | |
| `firecracker-v1.14-direct-mem` (PR #8) | **MISSING** | **MISSING** | **MISSING** | **MISSING** | This branch |
| `firecracker-v1.12-direct-mem` (prod today) | **MISSING** | **MISSING** | N/A | **MISSING** | |

Bugs 4–8 and 10 below are not addressed by either upstream fix and apply to
all branches.

---

### Bug 1 — `Vmm::save_state` saves KVM state **before** device state [C]

[`src/vmm/src/lib.rs:444-471`][save_state]

```rust
pub fn save_state(&mut self, vm_info: &VmInfo) -> Result<MicrovmState, MicrovmStateError> {
    use self::MicrovmStateError::SaveVmState;
    let vcpu_states = self.save_vcpu_states()?;     // KVM_GET_LAPIC, KVM_GET_VCPU_EVENTS, …
    let kvm_state = self.kvm.save_state();           // KVM_GET_IRQCHIP
    let vm_state = { /* arch-specific VM state */ };
    let device_states = self.device_manager.save();  // <-- prepare_save() runs here, TOO LATE
    Ok(MicrovmState { vm_info: vm_info.clone(), kvm_state, vm_state, vcpu_states, device_states })
}
```

`device_manager.save()` is the only caller of `VirtioBlock::prepare_save()`,
which (a) drains `io_uring`, (b) writes completed entries into the used ring,
and (c) calls `interrupt.trigger(VirtioInterruptType::Queue(0))` — see step 9
above. The IRQ delivery path for both transports writes to an eventfd that
KVM uses to pend an interrupt in the IRQ chip / LAPIC. By that time, **steps 3
and 4 have already taken `KVM_GET_LAPIC` / `KVM_GET_VCPU_EVENTS` /
`KVM_GET_IRQCHIP`**, so the just-triggered IRQ is not in the snapshot.

On restore, the guest sees new used-ring entries (those *are* in guest memory
and therefore in the memfile/UFFD) but the IRQ that was supposed to tell it
"go look" is gone. Linux's virtio_blk waits on that IRQ in
`submit_bio`/`blk_mq_*`, so it sleeps forever. This matches the symptom in
[`PR #6`][pr6] and the [upstream fix commit message][upstream-fix].

**Upstream fix:** move `let device_states = self.device_manager.save();` to be
the first line in `save_state`. See upstream `lib.rs` on
[`firecracker-v1.15`][upstream-v115-save] for the corrected ordering and the
comment explaining it.

[upstream-v115-save]: https://github.com/firecracker-microvm/firecracker/blob/firecracker-v1.15/src/vmm/src/lib.rs

---

### Bug 2 — MMIO transport state captured **before** `prepare_save` runs [C]

[`src/vmm/src/device_manager/persist.rs:228-267`][mmio_dm]

```rust
let _: Result<(), ()> = self.for_each_virtio_device(|_, devid, device| {
    let mmio_transport_locked = device.inner.lock().expect("Poisoned lock");
    let transport_state = mmio_transport_locked.save();    // <-- L230, captures irq_status NOW
    let device_info = device.resources;
    let device_id = devid.clone();

    let mut locked_device = mmio_transport_locked.locked_device();
    match locked_device.device_type() {
        ...
        virtio_ids::VIRTIO_ID_BLOCK => {
            let block = locked_device.as_mut_any().downcast_mut::<Block>().unwrap();
            if block.is_vhost_user() { … } else {
                block.prepare_save();                       // <-- L258, mutates irq_status
                let device_state = block.save();
                states.block_devices.push(VirtioDeviceState {
                    device_id, device_state, transport_state /* L263, stale */, device_info,
                });
            }
        }
```

`MmioTransportState` includes `interrupt_status` (the `VIRTIO_MMIO_INT_VRING`
bit) — see [`src/vmm/src/devices/virtio/persist.rs:195-233`][mmio_state] —
which `prepare_save → process_async_completion_queue → trigger_irq(Vring)`
sets via `irq_status.fetch_or(VIRTIO_MMIO_INT_VRING, …)`
([`src/vmm/src/devices/virtio/transport/mmio.rs:464-477`][mmio_trigger]).

Saving `transport_state` *before* `prepare_save` means the snapshot records
`interrupt_status == 0` even though new used-ring entries are present. The
Linux virtio-mmio ISR reads `InterruptStatus` (offset `0x60`), sees `0`, and
returns `IRQ_NONE` without scanning the used ring.

[mmio_dm]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/device_manager/persist.rs#L228-L267

**Upstream fix:** invert the order — call `prepare_save()` (or its
`VirtioDevice` trait equivalent) before reading `mmio_transport_locked.save()`.

---

### Bug 3 — PCI transport state captured **before** `prepare_save` runs [C]

[`src/vmm/src/device_manager/pci_mngr.rs:284-328`][pci_dm]

```rust
for pci_dev in self.virtio_devices.values() {
    let locked_pci_dev = pci_dev.lock().expect("Poisoned lock");
    let transport_state = locked_pci_dev.state();          // <-- L286, captures msix_state NOW
    let virtio_dev = locked_pci_dev.virtio_device();
    let mut locked_virtio_dev = virtio_dev.lock().expect("Poisoned lock");
    ...
    virtio_ids::VIRTIO_ID_BLOCK => {
        let block_dev = locked_virtio_dev.as_mut_any().downcast_mut::<Block>().unwrap();
        if block_dev.is_vhost_user() { … } else {
            block_dev.prepare_save();                       // <-- L319, may set PBA bit
            let device_state = block_dev.save();
            state.block_devices.push(VirtioDeviceState {
                device_id: block_dev.id().to_string(), pci_device_bdf,
                device_state, transport_state /* L325, stale */,
            });
        }
    }
```

Same shape as Bug 2, on the PCI side. `prepare_save →
process_async_completion_queue → trigger(Queue(0))` enters
[`VirtioInterruptMsix::trigger`][pci_trigger] which, for the masked-vector
path, calls `config.set_pba_bit(vector, false)`. That PBA bit lives in
`MsixConfig` and is captured into `msix_state` by `VirtioPciDevice::state()`.
Saving `transport_state` before `prepare_save` loses any PBA bit set there.

The unmasked path writes the MSI-X eventfd, hitting the same race against
KVM as Bug 1 (see also Bug 7 below).

In production today PR #8 boots with `pci=off` so the MMIO path (Bug 2) is the
one that fires; this bug becomes load-bearing the moment PCI is turned on.

[pci_dm]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/device_manager/pci_mngr.rs#L284-L328

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

This is the same external symptom as Bug 1 but the proximate cause is
different — fixing the ordering bugs does not fix this. The
[PR #6][pr6] test branch began addressing it by introducing a
`PendingAsyncOperations(u32)` error variant; that change is not present on
PR #8.

[submit_syscall]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/io_uring/queue/submission.rs#L122-L153

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
[num_ops]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/io_uring/mod.rs#L240-L244

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

[eopnotsupp]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/devices/virtio/block/virtio/device.rs#L590-L668

[cqe]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/io_uring/operation/cqe.rs#L25-L40
[cqe_test]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/io_uring/operation/cqe.rs#L60-L79

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

[process_queue]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/devices/virtio/block/virtio/device.rs#L494-L570
[kick]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/devices/virtio/block/virtio/io/async_io.rs#L250-L255

---

### Bug 7 — irqfd injection race with KVM state save [C-leaning M, not fixed by upstream]

Even with Bugs 1–3 fixed (i.e. device save runs first), the IRQ that
`prepare_save → trigger` produces is delivered to KVM **asynchronously**:

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
external symptom as Bug 1.

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
[`device.rs:219-228`][block_kick]: "kick the block queue(s) to make up for
any pending or in-flight epoll events we may have not captured in
snapshot"). A future refactor that drops `resume_vm` (e.g. autoresume during
load) would re-introduce the risk. See also **Bug 10** below — block's kick
mitigates Bug 8 but does **not** mitigate Bug 1 / Bug 7.

[queue_evts]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/devices/virtio/block/virtio/device.rs#L312-L313
[queue_evts_restore]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/devices/virtio/block/virtio/persist.rs#L99
[resume_kick]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/lib.rs#L386-L388
[kick_dm]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/device_manager/mod.rs#L278-L301
[block_kick]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/devices/virtio/block/device.rs#L219-L228
[infra-resume]: https://github.com/e2b-dev/infra/blob/main/packages/orchestrator/pkg/sandbox/fc/process.go

---

### Bug 9 — vsock has the same ordering bug pattern; both upstream commits missing on this branch [C]

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

**Two upstream commits are needed to fix this on PR #8**:

1. [`67ba7a206`][upstream-fix] — establishes the contract that
   `prepare_save()` runs *before* `transport_state.save()` (also fixes Bugs
   1, 2, 3).
2. [`48a5ae3b2`][upstream-vsock] — refactors vsock so that
   `send_transport_reset_event` is moved into a new `Vsock::prepare_save()`,
   which (post-fix #1) runs before `transport_state.save()`. The commit
   adds a note in `vsock/device.rs` documenting the dependency on the kick
   path for redundancy.

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

[vsock_mmio_save]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/device_manager/persist.rs#L287-L318
[vsock_pci_save]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/device_manager/pci_mngr.rs#L351-L380
[vsock_reset]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/devices/virtio/vsock/device.rs#L259-L281
[vsock_kick]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/devices/virtio/vsock/device.rs#L382-L393

---

### Bug 10 — Block `kick()` doesn't re-fire an interrupt for already-completed used-ring entries [C, fixes blocked by Bug 1/7]

Compare the two `kick()` implementations:

`VirtioBlock::kick` ([`device.rs:219-228`][block_kick]):

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
triggered**. This is exactly the failure mode that Bug 1 (lost IRQ for
already-processed descriptors) and Bug 7 (irqfd workqueue race) produce —
new used-ring entries already in guest memory, IRQ never delivered, guest
parked on the old descriptors waiting for completion.

For vsock the equivalent failure is masked by the unconditional re-trigger.
For block (which is the device the original PR #6 freeze test exercises)
there is no such safety net, so even with Bugs 1, 2, 3 fixed via upstream
backports, a separate Bug 7 occurrence at snapshot time can still freeze
the guest. The robust answer would be for `VirtioBlock::kick` (and net's
kick at [`net/device.rs:1062-1071`][net_kick]) to call `prepare_kick` on each
queue and re-trigger the IRQ if there are unacked used-ring entries — i.e.
mirror what vsock does. Filing as **C** because it's the residual hole that
makes a coordinated Bug 1 / Bug 7 fix not fully sufficient on its own.

[net_kick]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/devices/virtio/net/device.rs#L1062-L1071

---

## Snapshot-related "kick" semantics — quick reference

A single table of where IRQ retrigger does and doesn't happen on resume:

| Device | `kick()` behavior | Recovers lost-IRQ class? |
|---|---|---|
| Block ([`device.rs:219-228`][block_kick]) | `process_virtio_queues()` — reads avail ring only | **No** — needs new descriptors to fire any IRQ |
| Net ([`net/device.rs:1062-1071`][net_kick]) | `process_virtio_queues()` — reads RX+TX | **No** — same as block |
| Vsock ([`vsock/device.rs:382-393`][vsock_kick]) | `signal_used_queue(EVQ)` — unconditional IRQ retrigger on EVQ | Yes (for EVQ only) |
| Balloon ([`balloon/device.rs:1000-1008`][balloon_kick]) | replays queued FPH work via `process_virtio_queues` | **No** for IRQ; OK for FPH replay |
| RNG/Entropy / PMem / virtio-mem | default `fn kick(&mut self) {}` (no-op) | **No** |

[balloon_kick]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/devices/virtio/balloon/device.rs#L1000-L1008

This means the Bug 1 / Bug 7 class is **only** mitigated by:

- (a) Correct ordering on the save side (upstream `67ba7a206` + `48a5ae3b2`),
  and
- (b) For irqfd's workqueue race (Bug 7), explicit kernel-state-drain logic
  that doesn't exist anywhere in this tree.

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
  used-ring state is re-armed, *provided* it was correctly snapshotted (the
  precondition Bugs 1–3 violate).
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

[nodrop]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/io_uring/mod.rs#L334-L346
[throttle]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/devices/virtio/block/virtio/device.rs#L520-L540
[throttle_clear]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/devices/virtio/block/virtio/device.rs#L674-L687
[throttle_persist]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/devices/virtio/block/virtio/persist.rs#L149
[rl_persist]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/rate_limiter/persist.rs#L11-L50
[rl_restore]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/rate_limiter/persist.rs#L73-L87
[dirty]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/devices/virtio/block/virtio/io/async_io.rs#L53-L67
[dirty_pop]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/devices/virtio/block/virtio/io/async_io.rs#L286-L299
[queue_used]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/devices/virtio/queue.rs#L557-L606
[seccomp]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/resources/seccomp/x86_64-unknown-linux-musl.json
[restore_activate]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/device_manager/persist.rs#L388-L427
[restore_order]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/builder.rs#L497-L510
[net_prep]: https://github.com/e2b-dev/firecracker/blob/f0a35a156/src/vmm/src/devices/virtio/net/device.rs#L943-L960

## Implications for net-vs-block VMM contention

The original motivation for going async is that the VMM thread is the
single-threaded emulator for all virtio devices, and the sync block engine
blocks on `pread`/`pwrite`/`fsync` — starving net. None of the bugs above
re-introduce that contention; the async path's per-`process_queue` cost is one
non-blocking `io_uring_enter` from `kick_submission_queue`
([`async_io.rs:250-255`][kick]), independent of in-flight depth. So the
performance argument for async stands; the open question is purely
correctness during snapshot, which is what this document is about.

## References

- Upstream fixes (both required to fully cover the bugs catalogued here):
  - [`firecracker-microvm/firecracker@67ba7a206`][upstream-fix]
    ("fix: saving/restoring async IO engine transport state", 2025-12-15) —
    addresses Bugs 1, 2, 3 by reordering `device_manager.save()` before KVM
    state and `prepare_save()` before `transport_state.save()`.
  - [`firecracker-microvm/firecracker@48a5ae3b2`][upstream-vsock]
    ("refactor(vsock): Send reset event before saving transport state",
    2026-02-16) — addresses Bug 9 by moving vsock's reset event into
    `Vsock::prepare_save()` so it benefits from the new ordering. Depends
    on the prior commit.
- Freeze reproducer (closed):
  [`e2b-dev/firecracker#6`][pr6] (`test_snapshot_with_heavy_async_io`).
- Current target branch:
  [`e2b-dev/firecracker#8`][pr8] — head
  [`f0a35a156`][head].
- Sibling pause/snapshot-related fork patches that demonstrate signal delivery
  *can* occur during snapshot work:
  [`e2b-dev/firecracker#11`][pr11] (closed),
  [`e2b-dev/firecracker#12`][pr12] (closed).
- Orchestrator pause path:
  [`infra/packages/orchestrator/pkg/sandbox/sandbox.go::Sandbox.Pause`][infra-pause].

[pr12]: https://github.com/e2b-dev/firecracker/pull/12
