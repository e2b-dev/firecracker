//! Background workload simulation — CPU, memory pressure, and failures.
//!
//! The key addition over a simple stress tool: workload memory activity
//! touches pages in the guest memory allocation and marks them dirty,
//! so GET /memory and GET /memory/dirty return meaningful bitmaps that
//! the orchestrator's ExportMemory / DiffMetadata flows act on.

use std::time::Duration;

use tracing::{info, warn};

use crate::api_types::{FailureMode, WorkloadConfig};
use crate::vm_state::{Lifecycle, Shared};

pub fn start(state: Shared, config: WorkloadConfig) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        info!(
            cpu = config.cpu_load_percent,
            mem = config.memory_usage_mib,
            io = config.io_ops_per_sec,
            "Workload simulation started"
        );

        if let Some(ref fm) = config.failure_mode {
            let s = state.clone();
            let fm = fm.clone();
            tokio::spawn(async move { run_failure(s, fm).await });
        }

        if config.cpu_load_percent > 0.0 {
            let pct = config.cpu_load_percent;
            tokio::spawn(async move { burn_cpu(pct).await });
        }

        // Continuous memory activity: touch pages at io_ops_per_sec rate.
        // This dirties pages in the guest memory allocation so bitmaps are realistic.
        if config.io_ops_per_sec > 0 || config.memory_usage_mib > 0 {
            let ops = config.io_ops_per_sec.max(1);
            let s = state.clone();
            tokio::spawn(async move { dirty_pages_loop(s, ops).await });
        }

        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if state.lock().await.lifecycle != Lifecycle::Running {
                break;
            }
        }
    })
}

/// Periodically touch random pages in guest memory, marking them dirty.
async fn dirty_pages_loop(state: Shared, ops_per_sec: u64) {
    let interval = Duration::from_micros(1_000_000 / ops_per_sec.max(1));

    loop {
        tokio::time::sleep(interval).await;

        let mut s = state.lock().await;
        if s.lifecycle != Lifecycle::Running { break; }
        if let Some(ref mut mem) = s.guest_mem {
            mem.simulate_activity(1);
        }
    }
}

async fn burn_cpu(percent: f64) {
    let frac = (percent / 100.0).clamp(0.0, 1.0);
    let cycle = 100u64;
    let busy = (cycle as f64 * frac) as u64;
    let idle = cycle - busy;

    loop {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(busy);
        while tokio::time::Instant::now() < deadline {
            std::hint::spin_loop();
            if rand::random::<u8>() < 4 { tokio::task::yield_now().await; }
        }
        if idle > 0 { tokio::time::sleep(Duration::from_millis(idle)).await; }
    }
}

async fn run_failure(state: Shared, mode: FailureMode) {
    match mode {
        FailureMode::Crash { after_ms } => {
            tokio::time::sleep(Duration::from_millis(after_ms)).await;
            warn!("Simulated crash");
            state.lock().await.shutdown.notify_waiters();
            std::process::exit(134);
        }
        FailureMode::Hang { after_ms } => {
            tokio::time::sleep(Duration::from_millis(after_ms)).await;
            warn!("Simulated hang — API will stop responding");
            std::future::pending::<()>().await;
        }
        FailureMode::Oom { after_ms } => {
            tokio::time::sleep(Duration::from_millis(after_ms)).await;
            warn!("Simulated OOM");
            state.lock().await.shutdown.notify_waiters();
            std::process::exit(137);
        }
        FailureMode::IoError { .. } => {}
        FailureMode::RandomExit { min_ms, max_ms, exit_code } => {
            let delay = if max_ms > min_ms {
                min_ms + (rand::random::<u64>() % (max_ms - min_ms))
            } else { min_ms };
            tokio::time::sleep(Duration::from_millis(delay)).await;
            warn!(delay, exit_code, "Simulated random exit");
            state.lock().await.shutdown.notify_waiters();
            std::process::exit(exit_code);
        }
        FailureMode::KernelPanic { after_ms } => {
            tokio::time::sleep(Duration::from_millis(after_ms)).await;
            warn!("Simulated kernel panic");
            state.lock().await.shutdown.notify_waiters();
            std::process::exit(148);
        }
    }
}
