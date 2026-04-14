//! Mock envd HTTP server — mimics the envd daemon that runs inside a sandbox.
//!
//! The orchestrator expects envd to be reachable at `http://<slot_ip>:49983`.
//! This module implements the subset of endpoints the orchestrator actually calls:
//!
//!   REST (from envd.yaml):
//!     GET  /health   → 204
//!     POST /init     → accept env vars, token, mounts, etc. → 204
//!     GET  /metrics  → mock resource usage
//!     GET  /envs     → stored environment variables
//!
//!   Connect RPC (from process.proto):
//!     POST /process.Process/Start  → server-streaming: start + end events
//!     POST /process.Process/List   → unary: empty process list

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use bytes::{BufMut, Bytes, BytesMut};
use http::{Method, Response, StatusCode};
use http_body_util::Full;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use prost::Message;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::vm_state::Shared;

// ─── Envd-specific state ─────────────────────────────────────────────────────

pub struct EnvdState {
    pub initialized: bool,
    pub env_vars: HashMap<String, String>,
    pub access_token: Option<String>,
    pub hyperloop_ip: Option<String>,
    pub default_user: String,
    pub default_workdir: String,
    pub volume_mounts: Vec<VolumeMount>,
    pub started_at: Instant,
}

impl EnvdState {
    pub fn new() -> Self {
        Self {
            initialized: false,
            env_vars: HashMap::new(),
            access_token: None,
            hyperloop_ip: None,
            default_user: String::new(),
            default_workdir: String::new(),
            volume_mounts: Vec::new(),
            started_at: Instant::now(),
        }
    }
}

pub type EnvdShared = Arc<Mutex<EnvdState>>;

// ─── REST API types (matching envd.yaml) ─────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvdInitBody {
    #[serde(rename = "volumeMounts", default)]
    pub volume_mounts: Option<Vec<VolumeMount>>,
    #[serde(rename = "hyperloopIP", default)]
    pub hyperloop_ip: Option<String>,
    #[serde(rename = "envVars", default)]
    pub env_vars: Option<HashMap<String, String>>,
    #[serde(rename = "accessToken", default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(rename = "defaultUser", default)]
    pub default_user: Option<String>,
    #[serde(rename = "defaultWorkdir", default)]
    pub default_workdir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeMount {
    pub nfs_target: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvdMetrics {
    pub ts: i64,
    pub cpu_count: i64,
    pub cpu_used_pct: f64,
    pub mem_total: i64,
    pub mem_used: i64,
    pub mem_cache: i64,
    pub mem_total_mib: i64,
    pub mem_used_mib: i64,
    pub disk_used: i64,
    pub disk_total: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvdError {
    pub message: String,
    pub code: u16,
}

// ─── Connect protocol protobuf types (manual prost definitions) ──────────────
// These match the process.proto and filesystem.proto definitions.

#[derive(Clone, Message)]
pub struct ProcessConfig {
    #[prost(string, tag = "1")]
    pub cmd: String,
    #[prost(string, repeated, tag = "2")]
    pub args: Vec<String>,
    #[prost(map = "string, string", tag = "3")]
    pub envs: HashMap<String, String>,
    #[prost(string, optional, tag = "4")]
    pub cwd: Option<String>,
}

#[derive(Clone, Message)]
pub struct StartRequest {
    #[prost(message, optional, tag = "1")]
    pub process: Option<ProcessConfig>,
    #[prost(message, optional, tag = "2")]
    pub pty: Option<Pty>,
    #[prost(string, optional, tag = "3")]
    pub tag: Option<String>,
    #[prost(bool, optional, tag = "4")]
    pub stdin: Option<bool>,
}

#[derive(Clone, Message)]
pub struct Pty {
    #[prost(message, optional, tag = "1")]
    pub size: Option<PtySize>,
}

#[derive(Clone, Message)]
pub struct PtySize {
    #[prost(uint32, tag = "1")]
    pub cols: u32,
    #[prost(uint32, tag = "2")]
    pub rows: u32,
}

#[derive(Clone, Message)]
pub struct StartResponse {
    #[prost(message, optional, tag = "1")]
    pub event: Option<ProcessEvent>,
}

#[derive(Clone, Message)]
pub struct ProcessEvent {
    #[prost(oneof = "ProcessEventKind", tags = "1, 2, 3, 4")]
    pub event: Option<ProcessEventKind>,
}

#[derive(Clone, prost::Oneof)]
pub enum ProcessEventKind {
    #[prost(message, tag = "1")]
    Start(StartEvent),
    #[prost(message, tag = "2")]
    Data(DataEvent),
    #[prost(message, tag = "3")]
    End(EndEvent),
    #[prost(message, tag = "4")]
    Keepalive(KeepAlive),
}

#[derive(Clone, Message)]
pub struct StartEvent {
    #[prost(uint32, tag = "1")]
    pub pid: u32,
}

#[derive(Clone, Message)]
pub struct DataEvent {
    #[prost(oneof = "DataEventOutput", tags = "1, 2, 3")]
    pub output: Option<DataEventOutput>,
}

#[derive(Clone, prost::Oneof)]
pub enum DataEventOutput {
    #[prost(bytes, tag = "1")]
    Stdout(Vec<u8>),
    #[prost(bytes, tag = "2")]
    Stderr(Vec<u8>),
    #[prost(bytes, tag = "3")]
    Pty(Vec<u8>),
}

#[derive(Clone, Message)]
pub struct EndEvent {
    #[prost(sint32, tag = "1")]
    pub exit_code: i32,
    #[prost(bool, tag = "2")]
    pub exited: bool,
    #[prost(string, tag = "3")]
    pub status: String,
    #[prost(string, optional, tag = "4")]
    pub error: Option<String>,
}

#[derive(Clone, Message)]
pub struct KeepAlive {}

#[derive(Clone, Message)]
#[allow(dead_code)]
pub struct ListRequest {}

#[derive(Clone, Message)]
pub struct ListResponse {
    #[prost(message, repeated, tag = "1")]
    pub processes: Vec<ProcessInfo>,
}

#[derive(Clone, Message)]
pub struct ProcessInfo {
    #[prost(message, optional, tag = "1")]
    pub config: Option<ProcessConfig>,
    #[prost(uint32, tag = "2")]
    pub pid: u32,
    #[prost(string, optional, tag = "3")]
    pub tag: Option<String>,
}

// ─── Connect envelope framing ────────────────────────────────────────────────

const CONNECT_FLAG_DATA: u8 = 0x00;
const CONNECT_FLAG_TRAILER: u8 = 0x02;

fn connect_envelope(flags: u8, data: &[u8]) -> Bytes {
    let mut buf = BytesMut::with_capacity(5 + data.len());
    buf.put_u8(flags);
    buf.put_u32(data.len() as u32);
    buf.put_slice(data);
    buf.freeze()
}

fn connect_stream_response(content_type: &str, frames: Vec<Bytes>) -> Resp {
    let total_len: usize = frames.iter().map(|f| f.len()).sum();
    let mut body = BytesMut::with_capacity(total_len);
    for frame in frames {
        body.put_slice(&frame);
    }
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", content_type)
        .body(Full::new(body.freeze()))
        .unwrap()
}

// ─── HTTP server ─────────────────────────────────────────────────────────────

type Resp = Response<Full<Bytes>>;

fn json_resp(status: StatusCode, body: &impl Serialize) -> Resp {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(serde_json::to_vec(body).unwrap())))
        .unwrap()
}

fn no_content() -> Resp {
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header("Cache-Control", "no-store")
        .body(Full::new(Bytes::new()))
        .unwrap()
}

fn envd_error(status: StatusCode, msg: &str) -> Resp {
    json_resp(
        status,
        &EnvdError {
            message: msg.into(),
            code: status.as_u16(),
        },
    )
}

pub async fn serve(addr: SocketAddr, envd_state: EnvdShared, vm_state: Shared) {
    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            warn!("Failed to bind envd mock server on {addr}: {e}");
            return;
        }
    };
    info!("envd mock listening on {addr}");

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                warn!("envd accept error: {e}");
                continue;
            }
        };
        let envd_state = envd_state.clone();
        let vm_state = vm_state.clone();
        tokio::spawn(async move {
            let svc = service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                let envd_state = envd_state.clone();
                let vm_state = vm_state.clone();
                async move {
                    use http_body_util::BodyExt;

                    let method = req.method().clone();
                    let path = req.uri().path().to_string();
                    let content_type = req
                        .headers()
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();

                    let body = match req.into_body().collect().await {
                        Ok(c) => c.to_bytes(),
                        Err(e) => {
                            return Ok::<_, hyper::Error>(envd_error(
                                StatusCode::BAD_REQUEST,
                                &e.to_string(),
                            ))
                        }
                    };

                    Ok::<_, hyper::Error>(
                        handle(method, &path, &body, &content_type, envd_state, vm_state).await,
                    )
                }
            });
            if let Err(e) = http1::Builder::new()
                .serve_connection(TokioIo::new(stream), svc)
                .await
            {
                if !e.is_incomplete_message() {
                    warn!("envd HTTP error: {e}");
                }
            }
        });
    }
}

async fn handle(
    method: Method,
    path: &str,
    body: &[u8],
    content_type: &str,
    envd_state: EnvdShared,
    vm_state: Shared,
) -> Resp {
    let seg: Vec<&str> = path.trim_matches('/').split('/').collect();

    match (method, seg.as_slice()) {
        // ── REST: health check ───────────────────────────────────────
        (Method::GET, ["health"]) => no_content(),

        // ── REST: init ───────────────────────────────────────────────
        (Method::POST, ["init"]) => handle_init(body, envd_state).await,

        // ── REST: metrics ────────────────────────────────────────────
        (Method::GET, ["metrics"]) => handle_metrics(envd_state, vm_state).await,

        // ── REST: env vars ───────────────────────────────────────────
        (Method::GET, ["envs"]) => {
            let s = envd_state.lock().await;
            json_resp(StatusCode::OK, &s.env_vars)
        }

        // ── Connect: process.Process/Start (server-streaming) ───────
        (Method::POST, ["process.Process", "Start"]) => {
            handle_process_start(body, content_type).await
        }

        // ── Connect: process.Process/List (unary) ───────────────────
        (Method::POST, ["process.Process", "List"]) => {
            handle_process_list(body, content_type).await
        }

        // ── Connect: process.Process/* (stubs) ──────────────────────
        (Method::POST, ["process.Process", method_name]) => {
            handle_process_stub(method_name, content_type).await
        }

        // ── Connect: filesystem.Filesystem/* (stubs) ────────────────
        (Method::POST, ["filesystem.Filesystem", method_name]) => {
            handle_filesystem_stub(method_name, content_type).await
        }

        // ── Fallback ─────────────────────────────────────────────────
        _ => {
            warn!(path, "Unknown envd endpoint");
            envd_error(
                StatusCode::NOT_FOUND,
                &format!("Unknown endpoint: {path}"),
            )
        }
    }
}

// ─── Handler implementations ─────────────────────────────────────────────────

async fn handle_init(body: &[u8], envd_state: EnvdShared) -> Resp {
    let init_body: EnvdInitBody = if body.is_empty() {
        EnvdInitBody {
            volume_mounts: None,
            hyperloop_ip: None,
            env_vars: None,
            access_token: None,
            timestamp: None,
            default_user: None,
            default_workdir: None,
        }
    } else {
        match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(e) => return envd_error(StatusCode::BAD_REQUEST, &e.to_string()),
        }
    };

    let mut s = envd_state.lock().await;
    s.initialized = true;

    if let Some(ref vars) = init_body.env_vars {
        for (k, v) in vars {
            s.env_vars.insert(k.clone(), v.clone());
        }
    }
    if let Some(ref token) = init_body.access_token {
        if token.is_empty() {
            s.access_token = None;
        } else {
            s.access_token = Some(token.clone());
        }
    }
    if let Some(ref ip) = init_body.hyperloop_ip {
        s.hyperloop_ip = Some(ip.clone());
    }
    if let Some(ref user) = init_body.default_user {
        if !user.is_empty() {
            s.default_user = user.clone();
        }
    }
    if let Some(ref workdir) = init_body.default_workdir {
        if !workdir.is_empty() {
            s.default_workdir = workdir.clone();
        }
    }
    if let Some(ref mounts) = init_body.volume_mounts {
        s.volume_mounts = mounts.clone();
    }

    info!(
        env_count = s.env_vars.len(),
        mounts = s.volume_mounts.len(),
        "envd initialized"
    );
    no_content()
}

async fn handle_metrics(envd_state: EnvdShared, vm_state: Shared) -> Resp {
    let vs = vm_state.lock().await;
    let es = envd_state.lock().await;

    let mem_total = (vs.machine_config.mem_size_mib as i64) * 1024 * 1024;
    let mem_used = vs
        .workload
        .as_ref()
        .map(|w| (w.memory_usage_mib as i64) * 1024 * 1024)
        .unwrap_or(mem_total / 10);
    let cpu_pct = vs
        .workload
        .as_ref()
        .map(|w| w.cpu_load_percent)
        .unwrap_or(1.0);

    let uptime_secs = es.started_at.elapsed().as_secs() as i64;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let metrics = EnvdMetrics {
        ts,
        cpu_count: vs.machine_config.vcpu_count as i64,
        cpu_used_pct: cpu_pct,
        mem_total,
        mem_used,
        mem_cache: mem_used / 4,
        mem_total_mib: vs.machine_config.mem_size_mib as i64,
        mem_used_mib: vs
            .workload
            .as_ref()
            .map(|w| w.memory_usage_mib as i64)
            .unwrap_or(vs.machine_config.mem_size_mib as i64 / 10),
        disk_used: 500 * 1024 * 1024 + (uptime_secs * 1024),
        disk_total: 10 * 1024 * 1024 * 1024,
    };

    json_resp(StatusCode::OK, &metrics)
}

/// Handle process.Process/Start — returns a mock streaming response with
/// StartEvent(pid) followed by EndEvent(exit_code=0).
async fn handle_process_start(body: &[u8], content_type: &str) -> Resp {
    let is_json = content_type.contains("json");

    if is_json {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) {
            let cmd = v
                .get("process")
                .and_then(|p| p.get("cmd"))
                .and_then(|c| c.as_str())
                .unwrap_or("<unknown>");
            info!(cmd, "process.Process/Start (JSON)");
        }
    } else if let Ok(req) = StartRequest::decode(body) {
        let cmd = req
            .process
            .as_ref()
            .map(|p| p.cmd.as_str())
            .unwrap_or("<unknown>");
        info!(cmd, "process.Process/Start (proto)");
    }

    let mock_pid: u32 = (std::process::id() * 100 + 1) % 65535;

    let start_resp = StartResponse {
        event: Some(ProcessEvent {
            event: Some(ProcessEventKind::Start(StartEvent { pid: mock_pid })),
        }),
    };

    let end_resp = StartResponse {
        event: Some(ProcessEvent {
            event: Some(ProcessEventKind::End(EndEvent {
                exit_code: 0,
                exited: true,
                status: "exited".into(),
                error: None,
            })),
        }),
    };

    if is_json {
        let start_json =
            serde_json::to_vec(&process_event_to_json(&start_resp)).unwrap_or_default();
        let end_json = serde_json::to_vec(&process_event_to_json(&end_resp)).unwrap_or_default();
        let trailers = b"{}";

        let frames = vec![
            connect_envelope(CONNECT_FLAG_DATA, &start_json),
            connect_envelope(CONNECT_FLAG_DATA, &end_json),
            connect_envelope(CONNECT_FLAG_TRAILER, trailers),
        ];
        connect_stream_response("application/connect+json", frames)
    } else {
        let start_bytes = start_resp.encode_to_vec();
        let end_bytes = end_resp.encode_to_vec();
        let trailers = b"{}";

        let frames = vec![
            connect_envelope(CONNECT_FLAG_DATA, &start_bytes),
            connect_envelope(CONNECT_FLAG_DATA, &end_bytes),
            connect_envelope(CONNECT_FLAG_TRAILER, trailers),
        ];
        connect_stream_response("application/connect+proto", frames)
    }
}

/// Handle process.Process/List — returns an empty process list.
async fn handle_process_list(_body: &[u8], content_type: &str) -> Resp {
    let is_json = content_type.contains("json");
    let resp = ListResponse {
        processes: vec![],
    };

    if is_json {
        let json_body = serde_json::json!({ "processes": [] });
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(
                serde_json::to_vec(&json_body).unwrap(),
            )))
            .unwrap()
    } else {
        let body = resp.encode_to_vec();
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/proto")
            .body(Full::new(Bytes::from(body)))
            .unwrap()
    }
}

/// Stub handler for other process.Process methods (Update, SendInput, etc.)
async fn handle_process_stub(method_name: &str, content_type: &str) -> Resp {
    let is_json = content_type.contains("json");
    info!(method_name, "process.Process stub called");

    if is_json {
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from("{}")))
            .unwrap()
    } else {
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/proto")
            .body(Full::new(Bytes::new()))
            .unwrap()
    }
}

/// Stub handler for filesystem.Filesystem methods.
async fn handle_filesystem_stub(method_name: &str, content_type: &str) -> Resp {
    let is_json = content_type.contains("json");
    info!(method_name, "filesystem.Filesystem stub called");

    if is_json {
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from("{}")))
            .unwrap()
    } else {
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/proto")
            .body(Full::new(Bytes::new()))
            .unwrap()
    }
}

// ─── JSON helpers for Connect+JSON mode ──────────────────────────────────────

fn process_event_to_json(resp: &StartResponse) -> serde_json::Value {
    let event = match resp.event.as_ref().and_then(|e| e.event.as_ref()) {
        Some(ProcessEventKind::Start(s)) => {
            serde_json::json!({ "event": { "start": { "pid": s.pid } } })
        }
        Some(ProcessEventKind::Data(d)) => {
            let output = match d.output.as_ref() {
                Some(DataEventOutput::Stdout(b)) => {
                    serde_json::json!({ "stdout": base64_encode(b) })
                }
                Some(DataEventOutput::Stderr(b)) => {
                    serde_json::json!({ "stderr": base64_encode(b) })
                }
                Some(DataEventOutput::Pty(b)) => serde_json::json!({ "pty": base64_encode(b) }),
                None => serde_json::json!({}),
            };
            serde_json::json!({ "event": { "data": output } })
        }
        Some(ProcessEventKind::End(e)) => {
            let mut end = serde_json::json!({
                "exitCode": e.exit_code,
                "exited": e.exited,
                "status": e.status,
            });
            if let Some(ref err) = e.error {
                end.as_object_mut()
                    .unwrap()
                    .insert("error".into(), serde_json::Value::String(err.clone()));
            }
            serde_json::json!({ "event": { "end": end } })
        }
        Some(ProcessEventKind::Keepalive(_)) => {
            serde_json::json!({ "event": { "keepalive": {} } })
        }
        None => serde_json::json!({ "event": null }),
    };
    event
}

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(data.len() * 4 / 3 + 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((n >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((n >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((n >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(n & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}
