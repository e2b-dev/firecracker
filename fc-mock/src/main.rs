//! fc-mock: drop-in Firecracker replacement for testing without KVM.
//!
//! Produces a binary named `firecracker` that accepts the same CLI arguments
//! and exposes the HTTP/Unix-socket API endpoints used by e2b-dev/infra,
//! but runs no actual VM machinery.

mod api_server;
mod api_types;
mod guest_mem;
mod uffd;
mod vm_state;
mod workload;

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use http_body_util::BodyExt;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::UnixListener;
use tokio::sync::Mutex;
use tracing::{error, info};

use vm_state::VmState;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Firecracker-compatible CLI.
///
/// Accepts the same flags as the real Firecracker binary.
/// KVM-only flags (seccomp, boot-timer, etc.) are accepted but ignored.
#[derive(Parser)]
#[command(name = "firecracker", version = VERSION)]
struct Args {
    #[arg(long = "api-sock", default_value = "/run/firecracker.socket")]
    api_sock: PathBuf,

    #[arg(long, default_value = "anonymous-instance")]
    id: String,

    #[arg(long = "config-file")]
    config_file: Option<PathBuf>,

    #[arg(long = "metadata")]
    metadata: Option<PathBuf>,

    #[arg(long = "no-api", requires = "config_file")]
    no_api: bool,

    // Accepted for CLI compatibility — no-op
    #[arg(long = "no-seccomp")] no_seccomp: bool,
    #[arg(long = "seccomp-filter")] seccomp_filter: Option<PathBuf>,
    #[arg(long = "start-time-us")] start_time_us: Option<u64>,
    #[arg(long = "start-time-cpu-us")] start_time_cpu_us: Option<u64>,
    #[arg(long = "parent-cpu-time-us")] parent_cpu_time_us: Option<u64>,
    #[arg(long = "log-path")] log_path: Option<PathBuf>,
    #[arg(long)] level: Option<String>,
    #[arg(long)] module: Option<String>,
    #[arg(long = "show-level")] show_level: bool,
    #[arg(long = "show-log-origin")] show_log_origin: bool,
    #[arg(long = "metrics-path")] metrics_path: Option<PathBuf>,
    #[arg(long = "boot-timer")] boot_timer: bool,
    #[arg(long = "enable-pci")] enable_pci: bool,
    #[arg(long = "http-api-max-payload-size", default_value = "51200")]
    http_api_max_payload_size: usize,
    #[arg(long = "mmds-size-limit")] mmds_size_limit: Option<usize>,
    #[arg(long = "snapshot-version")] snapshot_version: bool,
    #[arg(long = "describe-snapshot")] describe_snapshot: Option<PathBuf>,

    /// Initial workload simulation config (mock-specific)
    #[arg(long = "workload-config")]
    workload_config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(
                    args.level.as_deref().unwrap_or("info"),
                )),
        )
        .init();

    if args.snapshot_version {
        println!("v8.0.0");
        return Ok(());
    }

    if let Some(ref p) = args.describe_snapshot {
        match std::fs::read(p) {
            Ok(data) => {
                let v = serde_json::from_slice::<serde_json::Value>(&data).ok()
                    .and_then(|j| j.get("version").and_then(|v| v.as_str()).map(String::from))
                    .unwrap_or_else(|| "8.0.0".into());
                println!("v{v}");
            }
            Err(e) => { eprintln!("Error: {e}"); std::process::exit(1); }
        }
        return Ok(());
    }

    info!("Running Firecracker v{VERSION} (fc-mock)");

    let state = Arc::new(Mutex::new(VmState::new(args.id.clone())));

    if let Some(ref p) = args.workload_config {
        let cfg: api_types::WorkloadConfig = serde_json::from_str(&std::fs::read_to_string(p)?)?;
        state.lock().await.workload = Some(cfg);
    }

    if let Some(ref p) = args.metadata {
        state.lock().await.mmds_data = serde_json::from_str(&std::fs::read_to_string(p)?)?;
    }

    if let Some(ref p) = args.config_file {
        let cfg: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p)?)?;
        apply_config(&state, &cfg).await;
        info!("Config loaded from {}", p.display());

        if args.no_api {
            let mut s = state.lock().await;
            s.lifecycle = vm_state::Lifecycle::Running;
            s.started_at = Some(std::time::Instant::now());
            if let Some(w) = s.workload.clone() {
                let sc = state.clone();
                drop(s);
                workload::start(sc, w);
            } else {
                drop(s);
            }
            state.lock().await.shutdown.clone().notified().await;
            return Ok(());
        }
    }

    let _ = std::fs::remove_file(&args.api_sock);
    let listener = UnixListener::bind(&args.api_sock)?;
    info!("API listening on {}", args.api_sock.display());

    let shutdown = state.lock().await.shutdown.clone();
    let max_payload = args.http_api_max_payload_size;

    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, _) = result?;
                let state = state.clone();
                tokio::spawn(async move {
                    let svc = service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                        let state = state.clone();
                        async move {
                            let method = req.method().clone();
                            let path = req.uri().path().to_string();
                            let body = match req.into_body().collect().await {
                                Ok(c) => c.to_bytes(),
                                Err(e) => return Ok::<_, hyper::Error>(
                                    hyper::Response::builder().status(400)
                                        .body(http_body_util::Full::new(bytes::Bytes::from(
                                            format!("{{\"fault_message\":\"{e}\"}}")
                                        ))).unwrap()
                                ),
                            };
                            if body.len() > max_payload {
                                return Ok(hyper::Response::builder().status(400)
                                    .body(http_body_util::Full::new(bytes::Bytes::from(
                                        "{\"fault_message\":\"Payload too large\"}"
                                    ))).unwrap());
                            }
                            Ok::<_, hyper::Error>(api_server::handle(method, &path, &body, state).await)
                        }
                    });
                    if let Err(e) = http1::Builder::new().serve_connection(TokioIo::new(stream), svc).await {
                        if !e.is_incomplete_message() { error!("HTTP error: {e}"); }
                    }
                });
            }
            _ = shutdown.notified() => { info!("Shutdown"); break; }
        }
    }

    let _ = std::fs::remove_file(&args.api_sock);
    Ok(())
}

/// Apply JSON config file — only the fields the orchestrator uses.
async fn apply_config(state: &Arc<Mutex<VmState>>, cfg: &serde_json::Value) {
    let mut s = state.lock().await;
    if let Some(v) = cfg.get("boot-source").or(cfg.get("boot_source")) {
        s.boot_source = serde_json::from_value(v.clone()).ok();
    }
    if let Some(v) = cfg.get("machine-config").or(cfg.get("machine_config")) {
        if let Ok(mc) = serde_json::from_value(v.clone()) { s.machine_config = mc; }
    }
    if let Some(arr) = cfg.get("drives").and_then(|v| v.as_array()) {
        for d in arr {
            if let Ok(drive) = serde_json::from_value::<api_types::BlockDeviceConfig>(d.clone()) {
                s.drives.insert(drive.drive_id.clone(), drive);
            }
        }
    }
    if let Some(arr) = cfg.get("network-interfaces").or(cfg.get("network_interfaces")).and_then(|v| v.as_array()) {
        for n in arr {
            if let Ok(iface) = serde_json::from_value::<api_types::NetworkInterfaceConfig>(n.clone()) {
                s.network_interfaces.insert(iface.iface_id.clone(), iface);
            }
        }
    }
    if let Some(v) = cfg.get("metrics") { s.metrics = serde_json::from_value(v.clone()).ok(); }
}
