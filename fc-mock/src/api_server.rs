//! HTTP API — endpoints used by the e2b-dev/infra orchestrator.
//!
//! Memory endpoint response formats match the Go generated models:
//!   GET /memory/mappings → { mappings: [{base_host_virt_addr, offset, page_size, size}] }
//!   GET /memory          → { resident: [u64 bitmap], empty: [u64 bitmap] }
//!   GET /memory/dirty    → { bitmap: [u64 bitmap] }  (400 if not paused)

use std::time::{Duration, Instant};

use bytes::Bytes;
use http::{Method, Response, StatusCode};
use http_body_util::Full;
use tracing::{info, warn};

use crate::api_types::*;
use crate::guest_mem::GuestMemory;
use crate::vm_state::{Lifecycle, Shared};
use crate::{uffd, workload};

type Resp = Response<Full<Bytes>>;

fn json(status: StatusCode, body: &impl serde::Serialize) -> Resp {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(serde_json::to_vec(body).unwrap())))
        .unwrap()
}

fn no_content() -> Resp {
    Response::builder().status(StatusCode::NO_CONTENT).body(Full::new(Bytes::new())).unwrap()
}

fn bad(msg: &str) -> Resp {
    json(StatusCode::BAD_REQUEST, &Error { fault_message: msg.into() })
}

fn err500(msg: &str) -> Resp {
    json(StatusCode::INTERNAL_SERVER_ERROR, &Error { fault_message: msg.into() })
}

macro_rules! parse {
    ($body:expr, $T:ty) => {
        match serde_json::from_slice::<$T>($body) {
            Ok(v) => v,
            Err(e) => return bad(&e.to_string()),
        }
    };
}

pub async fn handle(method: Method, path: &str, body: &[u8], state: Shared) -> Resp {
    // Workload: optional response delay + IO error injection
    {
        let s = state.lock().await;
        let delay = s.workload.as_ref().map(|w| w.response_delay_ms).unwrap_or(0);
        let io_err_p = s.workload.as_ref()
            .and_then(|w| w.failure_mode.as_ref())
            .and_then(|f| match f { FailureMode::IoError { probability } => Some(*probability), _ => None })
            .unwrap_or(0.0);
        drop(s);
        if delay > 0 { tokio::time::sleep(Duration::from_millis(delay)).await; }
        if io_err_p > 0.0 && rand::random::<f64>() < io_err_p {
            return err500("Simulated IO error");
        }
    }

    let seg: Vec<&str> = path.trim_matches('/').split('/').collect();

    match (method, seg.as_slice()) {
        // ── Pre-boot configuration ──────────────────────────────────
        (Method::PUT, ["boot-source"]) => {
            let cfg = parse!(body, BootSourceConfig);
            let mut s = state.lock().await;
            if s.lifecycle != Lifecycle::NotStarted { return bad("Cannot update after boot"); }
            s.boot_source = Some(cfg);
            no_content()
        }

        (Method::PUT, ["machine-config"]) => {
            let cfg = parse!(body, MachineConfig);
            let mut s = state.lock().await;
            if s.lifecycle != Lifecycle::NotStarted { return bad("Cannot update after boot"); }
            s.machine_config = cfg;
            no_content()
        }

        (Method::PUT, ["drives", id]) => {
            let mut cfg = parse!(body, BlockDeviceConfig);
            cfg.drive_id = id.to_string();
            state.lock().await.drives.insert(id.to_string(), cfg);
            no_content()
        }

        (Method::PATCH, ["drives", id]) => {
            let upd = parse!(body, BlockDeviceUpdateConfig);
            let mut s = state.lock().await;
            match s.drives.get_mut(*id) {
                Some(d) => {
                    if let Some(p) = upd.path_on_host { d.path_on_host = Some(p); }
                    if upd.rate_limiter.is_some() { d.rate_limiter = upd.rate_limiter; }
                    no_content()
                }
                None => bad(&format!("Drive {id} not found")),
            }
        }

        (Method::PUT, ["network-interfaces", id]) => {
            let mut cfg = parse!(body, NetworkInterfaceConfig);
            cfg.iface_id = id.to_string();
            state.lock().await.network_interfaces.insert(id.to_string(), cfg);
            no_content()
        }

        (Method::PATCH, ["network-interfaces", id]) => {
            let upd = parse!(body, NetworkInterfaceUpdateConfig);
            let mut s = state.lock().await;
            match s.network_interfaces.get_mut(*id) {
                Some(iface) => {
                    if upd.rx_rate_limiter.is_some() { iface.rx_rate_limiter = upd.rx_rate_limiter; }
                    if upd.tx_rate_limiter.is_some() { iface.tx_rate_limiter = upd.tx_rate_limiter; }
                    no_content()
                }
                None => bad(&format!("Interface {id} not found")),
            }
        }

        (Method::PUT, ["mmds", "config"]) => {
            state.lock().await.mmds_config = Some(parse!(body, MmdsConfig));
            no_content()
        }

        (Method::PUT, ["mmds"]) => {
            state.lock().await.mmds_data = parse!(body, serde_json::Value);
            no_content()
        }

        (Method::PUT, ["metrics"]) => {
            state.lock().await.metrics = Some(parse!(body, MetricsConfig));
            no_content()
        }

        (Method::PUT, ["entropy"]) => {
            state.lock().await.entropy = Some(parse!(body, EntropyDeviceConfig));
            no_content()
        }

        // ── Actions (InstanceStart, FlushMetrics) ───────────────────
        (Method::PUT, ["actions"]) => {
            let a = parse!(body, InstanceActionInfo);
            match a.action_type.as_str() {
                "InstanceStart" => {
                    let mut s = state.lock().await;
                    if s.lifecycle != Lifecycle::NotStarted { return bad("VM already started"); }

                    if let Err(e) = s.allocate_guest_memory() {
                        return err500(&format!("Failed to allocate guest memory: {e}"));
                    }

                    s.lifecycle = Lifecycle::Running;
                    s.started_at = Some(Instant::now());
                    info!("VM started (mem={}MiB)", s.machine_config.mem_size_mib);
                    if let Some(cfg) = s.workload.clone() {
                        let sc = state.clone();
                        drop(s);
                        workload::start(sc, cfg);
                    }
                    no_content()
                }
                "FlushMetrics" => no_content(),
                other => bad(&format!("Unknown action_type: {other}")),
            }
        }

        // ── Snapshots ───────────────────────────────────────────────
        (Method::PUT, ["snapshot", "create"]) => {
            let params = parse!(body, CreateSnapshotParams);
            let mut s = state.lock().await;
            if s.lifecycle != Lifecycle::Paused { return bad("VM must be paused to create snapshot"); }

            let snap = serde_json::json!({
                "mock": true, "version": "8.0.0",
                "machine_config": s.machine_config,
            });
            if let Err(e) = std::fs::write(&params.snapshot_path, serde_json::to_vec_pretty(&snap).unwrap()) {
                return err500(&format!("Write snapshot: {e}"));
            }
            if let Some(ref mp) = params.mem_file_path {
                if let Err(e) = std::fs::write(mp, vec![0u8; 4096]) {
                    return err500(&format!("Write mem file: {e}"));
                }
            }

            // Clear dirty bitmap — snapshot captures current state
            if let Some(ref mut mem) = s.guest_mem {
                mem.clear_dirty();
            }

            info!(path = %params.snapshot_path.display(), "Snapshot created");
            no_content()
        }

        #[allow(deprecated)]
        (Method::PUT, ["snapshot", "load"]) => {
            let params = parse!(body, LoadSnapshotConfig);
            if !params.snapshot_path.exists() {
                return bad(&format!("Snapshot not found: {}", params.snapshot_path.display()));
            }

            // UFFD handshake when backend_type == Uffd
            if let Some(ref mb) = params.mem_backend {
                if mb.backend_type == MemBackendType::Uffd {
                    let mem_mib = state.lock().await.machine_config.mem_size_mib;
                    let handshake_result: Result<(i32, usize, usize), String> =
                        uffd::handshake(&mb.backend_path, mem_mib, 4096)
                            .map(|us| {
                                let info = (us.uffd_fd, us.ptr as usize, us.size);
                                std::mem::forget(us);
                                info
                            })
                            .map_err(|e| e.to_string());
                    match handshake_result {
                        Ok((fd, addr, size)) => {
                            info!(uffd_fd = fd, "UFFD handshake done");
                            let mut s = state.lock().await;
                            s.guest_mem = Some(GuestMemory::from_existing(addr as *mut u8, size));
                        }
                        Err(e) => warn!("UFFD handshake failed: {e}"),
                    }
                }
            }

            let mut s = state.lock().await;
            if s.guest_mem.is_none() {
                // File-based restore — allocate fresh memory
                if let Err(e) = s.allocate_guest_memory() {
                    warn!("Failed to allocate guest memory: {e}");
                }
            }
            s.lifecycle = if params.resume_vm { Lifecycle::Running } else { Lifecycle::Paused };
            s.started_at = Some(Instant::now());

            if params.resume_vm {
                if let Some(cfg) = s.workload.clone() {
                    let sc = state.clone();
                    drop(s);
                    workload::start(sc, cfg);
                }
            }
            info!(path = %params.snapshot_path.display(), resume = params.resume_vm, "Snapshot loaded");
            no_content()
        }

        // ── VM state (pause / resume) ───────────────────────────────
        (Method::PATCH, ["vm"]) => {
            let vm = parse!(body, Vm);
            let mut s = state.lock().await;
            match vm.state {
                SnapshotVmState::Paused => {
                    if s.lifecycle != Lifecycle::Running { return bad("Can only pause a running VM"); }
                    s.lifecycle = Lifecycle::Paused;
                    info!("VM paused");
                    no_content()
                }
                SnapshotVmState::Resumed => {
                    if s.lifecycle != Lifecycle::Paused { return bad("Can only resume a paused VM"); }
                    s.lifecycle = Lifecycle::Running;
                    info!("VM resumed");
                    if let Some(cfg) = s.workload.clone() {
                        let sc = state.clone();
                        drop(s);
                        workload::start(sc, cfg);
                    }
                    no_content()
                }
            }
        }

        // ── Memory introspection ────────────────────────────────────
        // Response formats match Go models in packages/shared/pkg/fc/models/

        (Method::GET, ["memory", "mappings"]) => {
            let s = state.lock().await;
            match &s.guest_mem {
                Some(mem) => json(StatusCode::OK, &mem.mappings_response()),
                None => bad("Guest memory not allocated"),
            }
        }

        (Method::GET, ["memory"]) => {
            let s = state.lock().await;
            match &s.guest_mem {
                Some(mem) => json(StatusCode::OK, &mem.memory_response()),
                None => bad("Guest memory not allocated"),
            }
        }

        (Method::GET, ["memory", "dirty"]) => {
            let s = state.lock().await;
            if s.lifecycle != Lifecycle::Paused {
                return bad("VM must be paused to read dirty pages");
            }
            match &s.guest_mem {
                Some(mem) => json(StatusCode::OK, &mem.dirty_response()),
                None => bad("Guest memory not allocated"),
            }
        }

        // ── Mock simulation control ─────────────────────────────────
        (Method::PUT, ["mock", "workload"]) => {
            let cfg = parse!(body, WorkloadConfig);
            let mut s = state.lock().await;
            s.workload = Some(cfg.clone());
            if s.lifecycle == Lifecycle::Running {
                let sc = state.clone();
                drop(s);
                workload::start(sc, cfg);
            }
            no_content()
        }

        (Method::GET, ["mock", "workload"]) => {
            json(StatusCode::OK, &state.lock().await.workload_status())
        }

        (Method::DELETE, ["mock", "workload"]) => {
            state.lock().await.workload = None;
            no_content()
        }

        (Method::GET, ["mock", "health"]) => {
            json(StatusCode::OK, &serde_json::json!({
                "status": "ok", "mock": true,
                "version": env!("CARGO_PKG_VERSION"),
                "pid": std::process::id(),
            }))
        }

        // ── Fallback ────────────────────────────────────────────────
        _ => {
            warn!(path, "Unknown API endpoint");
            bad(&format!("Unknown endpoint: {path}"))
        }
    }
}
