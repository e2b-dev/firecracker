# Snapshot bug reproducers

Two suites covering the findings in [`async-io-snapshot-analysis.md`](async-io-snapshot-analysis.md):

- **bug-present** (default): pass today (bug present); will fail when the bug is fixed.
- **post-fix** (`#[ignore]`d): fail today (bug present); will pass when the fix lands. Run with `--ignored`.

## Run

Bug-present (8 tests, all should pass):

```sh
cargo test -p vmm --lib -- \
    test_bug5_cqe_result_sign_flip \
    test_bug8_block_queue_evt_count_not_persisted \
    test_bug9_vsock_post_snapshot_irq_mutation \
    test_bug10_block_kick_misses_pre_snapshot_used_entries \
    test_p2_2_get_memory_mappings_not_gated_on_paused \
    test_p2_4_empty_rate_limiter_patch_is_noop \
    test_p2_5_balloon_cmd_id_hinting_state_mismatch \
    test_p2_6_mmds_data_store_not_persisted
```

Post-fix (5 tests, all should fail on this branch — and pass once each fix is applied):

```sh
cargo test -p vmm --lib -- --ignored \
    test_bug5_fix_cqe_result_preserves_errno \
    test_bug8_fix_block_queue_evt_count_round_trips \
    test_bug10_fix_block_kick_fires_irq_for_pre_snapshot_used_entries \
    test_p2_2_fix_get_memory_mappings_requires_paused \
    test_p2_5_fix_balloon_cmd_id_honors_persisted_value
```

| Bug   | bug-present test                                                                                        | post-fix test                                                                                           |
| ----- | ------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| 5     | `io_uring::operation::cqe::tests::test_bug5_cqe_result_sign_flip`                                       | `io_uring::operation::cqe::tests::test_bug5_fix_cqe_result_preserves_errno`                             |
| 8     | `devices::virtio::block::virtio::persist::tests::test_bug8_block_queue_evt_count_not_persisted`         | `devices::virtio::block::virtio::persist::tests::test_bug8_fix_block_queue_evt_count_round_trips`       |
| 9     | `devices::virtio::vsock::device::tests::test_bug9_vsock_post_snapshot_irq_mutation`                     | — (needs device-manager flow)                                                                           |
| 10    | `devices::virtio::block::virtio::device::tests::test_bug10_block_kick_misses_pre_snapshot_used_entries` | `devices::virtio::block::virtio::device::tests::test_bug10_fix_block_kick_fires_irq_for_pre_snapshot_used_entries` |
| P2-2  | `rpc_interface::tests::test_p2_2_get_memory_mappings_not_gated_on_paused`                               | `rpc_interface::tests::test_p2_2_fix_get_memory_mappings_requires_paused`                               |
| P2-4  | `vmm_config::tests::test_p2_4_empty_rate_limiter_patch_is_noop`                                         | — (orchestrator-side or schema change)                                                                  |
| P2-5  | `devices::virtio::balloon::persist::tests::test_p2_5_balloon_cmd_id_hinting_state_mismatch`             | `devices::virtio::balloon::persist::tests::test_p2_5_fix_balloon_cmd_id_honors_persisted_value` (option 1) |
| P2-6  | `mmds::persist::tests::test_p2_6_mmds_data_store_not_persisted`                                         | — (`MmdsState` schema change must precede the test)                                                     |

## Bug 5 — `Cqe::result` sign flip

**Test**: `Cqe::new(-EOPNOTSUPP).result()` returns an error with `raw_os_error() == Some(-EOPNOTSUPP)` and a `kind()` that differs from `Error::from_raw_os_error(EOPNOTSUPP).kind()`.

**Fix**: in `io_uring/operation/cqe.rs`, change `Error::from_raw_os_error(self.res)` to `Error::from_raw_os_error(-self.res)` in the `res < 0` branch. Then flip the `Some(-libc::EOPNOTSUPP)` comparisons in `block/virtio/device.rs:594, 620` to `Some(libc::EOPNOTSUPP)`. Without the companion flip, discard / write-zeroes start returning `VIRTIO_BLK_S_IOERR` on every unsupported-fs request.

## Bug 8 — block `queue_evt` count not persisted

**Test**: write `3` to original `queue_events()[0]`, save+restore, read restored `queue_events()[0]`, get `EAGAIN`.

**Fix**: add `queue_evt_counts: Vec<u64>` to `BlockState`. On save, `read()` each counter and `write()` it back (device is paused). On restore, after `EventFd::new()` at `block/virtio/persist.rs:99`, `write()` the saved count. Bump persist schema. Same fix for net.

## Bug 9 — vsock `send_transport_reset_event` after `transport_state.save`

**Test**: mirrors `device_manager/persist.rs:230,297` order — load `interrupt.status()`, call `send_transport_reset_event`, load again. The post-call value has `VIRTIO_MMIO_INT_VRING` set while the captured one is `0`.

**Caveat**: tests the precondition (the call mutates `interrupt_status`), not the device-manager-level ordering. The unit test will keep passing post-fix; a true regression test belongs in `device_manager::persist::tests` and would round-trip a full `MMIODeviceManager`.

**Fix**: cherry-pick upstream [`48a5ae3b2`](https://github.com/firecracker-microvm/firecracker/commit/48a5ae3b2) — adds `Vsock::prepare_save` and removes the post-save call from `device_manager/persist.rs` + `device_manager/pci_mngr.rs`.

## Bug 10 — block/net `kick()` doesn't re-fire IRQ for pre-snapshot used entries

**Test**: activate block, `queue.add_used(0, 0) + advance_used_ring_idx()`, call `process_virtio_queues()` (what `Block::kick` runs), assert no IRQ pending.

**Fix**: in `block/device.rs:219-228` (and `net/device.rs:1042-1052`), trigger the used-queue IRQ unconditionally after `process_virtio_queues`:

```rust
fn kick(&mut self) {
    if self.is_activated() {
        let _ = self.process_virtio_queues();
        if let Self::Virtio(b) = self
            && let Some(active) = b.device_state.active_state()
        {
            let _ = active.interrupt.trigger(VirtioInterruptType::Queue(0));
        }
    }
}
```

Post-fix assertion: `has_pending_interrupt(Queue(0)) == true`.

## P2-2 — `/memory/mappings` not gated on Paused

**Test**: build a `RuntimeApiController` with `default_vmm()` (state = `NotStarted`), call `handle_request(GetMemoryMappings)`, get `Ok(MemoryMappings(_))` — but `GetMemory` (L981) and `GetMemoryDirty` (L1000) reject the same state with `OperationNotSupportedWhileRunning`.

**Fix**: in `rpc_interface.rs:963-973`, add the same gate as `get_guest_memory_info`:

```rust
if vmm.instance_info.state != VmState::Paused {
    return Err(VmmActionError::OperationNotSupportedWhileRunning);
}
```

Post-fix assertion: `Err(OperationNotSupportedWhileRunning)`.

## P2-4 — empty `{}` rate-limiter PATCH is a no-op

**Test**: `serde_json::from_str::<RateLimiterConfig>("{}")` → both fields `None` → `RateLimiterUpdate` with both fields `BucketUpdate::None`.

**Caveat**: documents the contract, doesn't flip on fix.

**Fix**: orchestrator sends an explicit disable payload (any `size: 0` or `refill_time: 0` already maps to `BucketUpdate::Disabled` via `TokenBucket::new` failing). Alternative: add `disable_bandwidth: bool` / `disable_ops: bool` to `RateLimiterConfig` (API schema bump).

## P2-5 — balloon `free_page_hint_cmd_id` reset to DONE while `hinting_state` keeps mid-chain

**Test**: set `config_space.free_page_hint_cmd_id = 42` + matching mid-chain `hinting_state`, save+restore. Restored `hinting_state.host_cmd == 42` but `config_space.free_page_hint_cmd_id == FREE_PAGE_HINT_DONE`.

**Fix** (pick one):
1. Add `free_page_hint_cmd_id: u32` to `BalloonConfigSpaceState`; restore from state instead of force-reset at `balloon/persist.rs:189`. Schema bump.
2. Also reset `hinting_state` to default on restore. Drops in-flight bookkeeping.

## P2-6 — MMDS data store not persisted

**Test**: populate `Mmds`, build `MmdsState` the way `device_manager/persist.rs:271` does (only `version` + `imds_compat`), apply to fresh `Mmds::default()`. Restored `data_store_value()` is `null`.

**Fix**: extend `MmdsState` with `data_store: Value`, `is_initialized: bool`, `data_store_limit: usize`. Populate from `mmds.data_store_value()` etc. on save. On restore, call `put_data` + `set_data_store_limit`. Schema bump. Restore call-sites: `builder.rs:930`, `net/persist.rs:222`.

## Not covered by a unit test

These need infrastructure a small unit test can't provide — explanation + fix for each:

- **Bug 4** (silent drain/sync errors) — would need fault injection into a private io_uring engine, or a logger sink to observe the swallowed `error!`. Closing the sync engine's fd via `update_file` proves only "no panic", which is trivially true. **Fix**: change `Block::prepare_save → Result<(), _>` and propagate through `device_manager::persist::save`. Add the `engine.num_ops() == 0` invariant check after a successful drain.
- **Bug 6** (early-`?` skips kick) — only manifests when prior SQEs were left unkicked (e.g. via a previous `EINTR`-truncated kick) AND the current call's avail ring is corrupt. The two conditions can't be set up cleanly without manipulating engine internals. **Fix**: move `kick_submission_queue` into a `Drop` guard or `finally`-style block; add `EINTR` retry to `kick_submission_queue` and `submit_and_wait_all`.
- **Bug 7** (irqfd / KVM save race) — kernel-level race; the irqfd workqueue and `save_vcpu_states` would have to be scheduled on opposite cores under load to reproduce. **Fix**: add a barrier before `save_vcpu_states` — `KVM_GET_IRQCHIP` (x86) or `KVM_GET_DEVICE_ATTR` on the GIC (aarch64) serializes against in-flight injection.
- **P2-1** (`mem_file_path = None` skips dirty reset) — `create_snapshot` calls `save_state` which blocks waiting on vcpu thread responses; `default_vmm()` doesn't start vcpus, so the call hangs. Belongs in `src/vmm/tests/integration_tests.rs` once that target compiles again. **Fix**: hoist `reset_dirty_bitmap()` + `guest_memory().reset_dirty()` out of `snapshot_memory_to_file` and always run them after a `Full` snapshot in `persist/mod.rs:159-181`.
- **P2-3** (virtio-mem mappings vs `dump()`) — divergence only appears with at least one unplugged slot. Requires a fully-wired `VirtioMem` device + plug/unplug cycle. **Fix**: in `lib.rs:701-724`, walk all slots (matching `dump()`'s seek-forward layout) so external readers reconstruct identical offsets.
- **P2-7** (aarch64 serial partial state) — `vm-superio`'s `Serial` doesn't expose FIFO/IER/LCR for save/restore; no accessors to write a test against. **Fix**: upstream `serial.save()`/`serial.restore()` helpers in vm-superio, then extend `ConnectedLegacyState` to carry the full register state. Cherry-pick upstream `9a49dcb03` for the serial rate-limiter at the same time.
- **P2-8** (SIGTERM truncates snapfile) — kernel-level signal-timing race between `truncate` and `sync_all`. **Fix**: write to `*.tmp`, `sync_all`, then atomic `rename()`. Alternative: `sigprocmask` SIGTERM for the duration of `snapshot_state_to_file`.

## Note

Running the broader `devices::virtio::block::virtio::device::tests` module in parallel hits a pre-existing race on shared per-`drive_id` block metrics (reproducible on `git stash` of these repros). The targeted invocations above run only the listed tests and are unaffected.
