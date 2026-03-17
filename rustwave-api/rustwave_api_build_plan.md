# RustWave API — Build Plan
*Derived from `rustwave_api_implementation_plan.md` · March 2026*

---

## Files to Edit or Create — Summary

| File | Action | Why |
|---|---|---|
| `Cargo.toml` | **EDIT** | Add tokio, axum, tower, tower-http, reqwest, serde, uuid, tracing, bytes |
| `src/main.rs` | **EDIT** | Add `serve` subcommand, logging init, `mod api`, `mod logging` |
| `src/gui.rs` | **EDIT** | Spawn broadcast API thread before launching eframe |
| `src/wav.rs` | **EDIT** | Add `write_to_bytes` and `read_from_bytes` + new unit test |
| `src/logging.rs` | **CREATE** | Logging initialisation (stderr + rolling JSON file) |
| `src/api/mod.rs` | **CREATE** | Router builder: `full_router`, `gui_router`, `run_server` |
| `src/api/models.rs` | **CREATE** | All JSON request/response structs |
| `src/api/errors.rs` | **CREATE** | `ApiError` enum with `IntoResponse` impl |
| `src/api/state.rs` | **CREATE** | `AppState`, `IncomingQueue`, `QueuedFile` |
| `src/api/wave.rs` | **CREATE** | Handlers: `GET /wave/status`, `POST /wave/encode`, `POST /wave/decode` |
| `src/api/broadcast.rs` | **CREATE** | Handlers: `GET /broadcast/status`, `POST /broadcast/transmit`, `POST /broadcast/receive`, `GET /broadcast/incoming`; `pub forward_to_broadcaster()` shared with `chan.rs` |
| `src/api/chan.rs` | **CREATE** | ChanNet HTTP client (`check_channet_reachable`, `send_chan_command`) and `POST /chan/request` proxy handler — fetches ZIP from ChanNet, AFSK-encodes it, forwards WAV to Broadcaster for over-the-air transmission |
| `src/api/tests.rs` | **CREATE** | Unit test: queue enqueue/dequeue round-trip |

**Unchanged:** `src/config.rs`, `src/encoder.rs`, `src/decoder.rs`, `src/framer.rs`, `deny.toml`, `Cargo.lock` (auto-updated).

---

## Step-by-Step Build Plan

Each step ends with `cargo check`. Do **not** advance until it is green. Commit after each step.

---

### Step 1 — `Cargo.toml`

Replace the `[dependencies]` block:

```toml
[package]
name = "rustwave"
version = "0.1.0"
edition = "2021"
license = "MIT"
description = "RustWave audio codec — encode bytes to WAV, decode WAV to bytes"

[[bin]]
name = "rustwave-cli"
path = "src/main.rs"

[dependencies]
# — existing —
clap    = { version = "4", features = ["derive"] }
hound   = "3"
eframe  = "0.31"

# — NEW: async runtime (required for axum) —
tokio   = { version = "1", features = ["full"] }

# — NEW: HTTP server —
axum    = { version = "0.7", features = ["multipart"] }
tower   = "0.4"
tower-http = { version = "0.5", features = ["cors", "limit"] }

# — NEW: HTTP client (forward WAV to Broadcaster) —
reqwest = { version = "0.12", features = ["multipart"] }

# — NEW: serialisation —
serde       = { version = "1", features = ["derive"] }
serde_json  = "1"

# — NEW: UUID generation for tx_id / queued_id —
uuid    = { version = "1", features = ["v4"] }

# — NEW: logging / tracing —
tracing             = "0.1"
tracing-subscriber  = { version = "0.3", features = ["env-filter", "json"] }
tracing-appender    = "0.2"

# — NEW: in-memory byte buffers (WAV bytes without temp files) —
bytes   = "1"
```

```bash
cargo check   # must be green before continuing
```

**Commit:** `chore: add API dependencies to Cargo.toml`

---

### Step 2 — `src/logging.rs` *(CREATE)*

```rust
//! Logging initialisation for RustWave.
//!
//! Call `logging::init()` once at the start of `main()`.
//!
//! Log output:
//!   - stderr:              INFO and above, human-readable
//!   - rustwave.log (file): DEBUG and above, JSON format, rolling daily
//!
//! The log file is written next to the binary.
//! Set RUSTWAVE_LOG=debug to see debug output on stderr too.

use std::path::PathBuf;
use tracing_appender::rolling;
use tracing_subscriber::{
    fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter,
};

/// Initialise logging. Must be called once before any tracing macros are used.
///
/// Returns the `_guard` from `tracing_appender::non_blocking`. The caller MUST
/// hold this value for the lifetime of the process; dropping it flushes and
/// closes the log file.
pub fn init() -> tracing_appender::non_blocking::WorkerGuard {
    let log_dir: PathBuf = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));

    // Rolling daily log file: rustwave.YYYY-MM-DD
    let file_appender = rolling::daily(&log_dir, "rustwave.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    // stderr layer — human readable, INFO+ by default, respects RUSTWAVE_LOG
    let stderr_filter = EnvFilter::try_from_env("RUSTWAVE_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let stderr_layer = fmt::layer()
        .with_target(false)
        .with_filter(stderr_filter);

    // file layer — JSON, DEBUG+
    let file_layer = fmt::layer()
        .json()
        .with_writer(non_blocking)
        .with_filter(EnvFilter::new("debug"));

    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(file_layer)
        .init();

    guard
}
```

```bash
cargo check
```

**Commit:** `feat: add logging module`

---

### Step 3 — `src/api/models.rs` *(CREATE)*

```bash
mkdir src/api
```

```rust
//! JSON request and response types for the RustWave API.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── /wave/* responses ──────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct WaveStatusResponse {
    pub service: &'static str,
    pub codec: &'static str,
    pub version: &'static str,
}

// ── /broadcast/* responses ─────────────────────────────────────────────────

#[derive(Serialize)]
pub struct BroadcastStatusResponse {
    pub service: &'static str,
    pub broadcaster_connected: bool,
    pub channet_connected: bool,
    pub broadcaster_url: String,
    pub queue_depth: usize,
}

#[derive(Serialize)]
pub struct TransmitResponse {
    pub status: &'static str,
    pub tx_id: Uuid,
    pub wav_bytes: usize,
}

#[derive(Serialize)]
pub struct ReceiveResponse {
    pub status: &'static str,
    pub queued_id: Uuid,
    pub decoded_bytes: usize,
}

/// Returned by GET /broadcast/incoming when the queue is empty.
#[derive(Serialize, Deserialize)]
pub struct QueueEmptyResponse {
    pub status: &'static str,
}

// ── ChanNet /chan/command request types ────────────────────────────────────
//
// Mirrors the six commands defined in the ChanNet API reference exactly.
// The `type` field is serialised as the serde tag so the JSON sent to
// /chan/command matches the format ChanNet expects.

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChanCommand {
    /// All boards + all active (non-archived) posts. Optional delta via `since`.
    FullExport { since: Option<u64> },
    /// All active posts on a single board. Optional delta via `since`.
    BoardExport { board: String, since: Option<u64> },
    /// All posts in a single thread. Optional delta via `since`.
    ThreadExport { thread_id: u64, since: Option<u64> },
    /// All archived threads + posts for a single board. Always a full export.
    ArchiveExport { board: String },
    /// Entire database — all boards, threads, archives, posts. Use for initial
    /// sync or recovery only; RustChan logs a warning when this is received.
    ForceRefresh,
    /// Post a new reply to an existing thread (the only write command).
    ReplyPush {
        board:     String,
        thread_id: u64,
        author:    String,
        content:   String,
        timestamp: u64,
    },
}

/// Returned by POST /chan/request on success.
#[derive(Serialize)]
pub struct ChanRequestResponse {
    pub status:    &'static str, // "transmitted"
    pub tx_id:     uuid::Uuid,
    pub zip_bytes: usize,
}

// ── Error envelope (Section 2.4) ───────────────────────────────────────────

#[derive(Serialize)]
pub struct ErrorDetail {
    pub code: String,
    pub message: String,
    pub status: u16,
}

#[derive(Serialize)]
pub struct ErrorEnvelope {
    pub error: ErrorDetail,
}
```

```bash
cargo check
```

**Commit:** `feat: add api/models.rs`

---

### Step 4 — `src/api/errors.rs` *(CREATE)*

```rust
//! API error type for RustWave.
//!
//! Every handler returns `Result<_, ApiError>`. axum automatically calls
//! `IntoResponse` on the error path.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use crate::api::models::{ErrorDetail, ErrorEnvelope};

#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    PayloadTooLarge,
    EncodeFailed(String),
    DecodeFailed(String),
    BroadcasterUnavailable(String),
    Internal(String),
}

impl ApiError {
    fn code(&self) -> &'static str {
        match self {
            Self::BadRequest(_)             => "BAD_REQUEST",
            Self::PayloadTooLarge           => "PAYLOAD_TOO_LARGE",
            Self::EncodeFailed(_)           => "ENCODE_FAILED",
            Self::DecodeFailed(_)           => "DECODE_FAILED",
            Self::BroadcasterUnavailable(_) => "BROADCASTER_UNAVAILABLE",
            Self::Internal(_)               => "INTERNAL_ERROR",
        }
    }

    fn status_code(&self) -> StatusCode {
        match self {
            Self::BadRequest(_)             => StatusCode::BAD_REQUEST,
            Self::PayloadTooLarge           => StatusCode::PAYLOAD_TOO_LARGE,
            Self::EncodeFailed(_)           => StatusCode::UNPROCESSABLE_ENTITY,
            Self::DecodeFailed(_)           => StatusCode::UNPROCESSABLE_ENTITY,
            Self::BroadcasterUnavailable(_) => StatusCode::BAD_GATEWAY,
            Self::Internal(_)               => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let message = match &self {
            Self::BadRequest(m)             => m.clone(),
            Self::PayloadTooLarge           => "Request body exceeds the 10 MB limit.".into(),
            Self::EncodeFailed(m)           => m.clone(),
            Self::DecodeFailed(m)           => m.clone(),
            Self::BroadcasterUnavailable(m) => m.clone(),
            Self::Internal(m)               => m.clone(),
        };

        tracing::error!(
            code = self.code(),
            http_status = status.as_u16(),
            %message,
            "api error"
        );

        let body = ErrorEnvelope {
            error: ErrorDetail {
                code: self.code().into(),
                message,
                status: status.as_u16(),
            },
        };

        (status, Json(body)).into_response()
    }
}
```

```bash
cargo check
```

**Commit:** `feat: add api/errors.rs`

---

### Step 5 — `src/api/state.rs` *(CREATE)*

```rust
//! Shared state for the RustWave API server.

use std::{collections::VecDeque, sync::Arc};
use bytes::Bytes;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug)]
pub struct QueuedFile {
    pub queued_id: Uuid,
    pub bytes: Bytes,
}

pub type IncomingQueue = Arc<Mutex<VecDeque<QueuedFile>>>;

#[derive(Clone)]
pub struct AppState {
    pub broadcaster_url: String,
    pub channet_url: String,
    pub wave_routes_enabled: bool,
    pub incoming_queue: IncomingQueue,
}

impl AppState {
    pub fn new(wave_routes_enabled: bool) -> Self {
        let broadcaster_url = std::env::var("RUSTWAVE_BROADCASTER_URL")
            .unwrap_or_else(|_| "http://localhost:9090".to_string());

        let channet_url = std::env::var("RUSTWAVE_CHANNET_URL")
            .unwrap_or_else(|_| "http://localhost:7070".to_string());

        Self {
            broadcaster_url,
            channet_url,
            wave_routes_enabled,
            incoming_queue: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub async fn queue_depth(&self) -> usize {
        self.incoming_queue.lock().await.len()
    }

    pub async fn enqueue(&self, file: QueuedFile) {
        self.incoming_queue.lock().await.push_back(file);
    }

    pub async fn dequeue(&self) -> Option<QueuedFile> {
        self.incoming_queue.lock().await.pop_front()
    }
}
```

```bash
cargo check
```

**Commit:** `feat: add api/state.rs`

---

### Step 6 — `src/api/wave.rs` *(CREATE)*

```rust
//! Handlers for the /wave/* general-purpose codec endpoints.
//! Only registered in `serve` mode — NOT in GUI mode.

use axum::{
    extract::Multipart,
    http::header,
    response::{IntoResponse, Response},
    Json,
};
use tracing::{info, warn};

use crate::{
    api::{errors::ApiError, models::WaveStatusResponse},
    decoder, encoder, framer, wav,
};

// ── GET /wave/status ───────────────────────────────────────────────────────

pub async fn wave_status() -> Json<WaveStatusResponse> {
    info!("GET /wave/status");
    Json(WaveStatusResponse {
        service: "rustwave",
        codec: "afsk-1200",
        version: env!("CARGO_PKG_VERSION"),
    })
}

// ── POST /wave/encode ──────────────────────────────────────────────────────

pub async fn wave_encode(mut multipart: Multipart) -> Result<Response, ApiError> {
    let (filename, file_bytes) = extract_file_field(&mut multipart).await?;

    info!(filename = %filename, input_bytes = file_bytes.len(), "POST /wave/encode starting");

    let result = tokio::task::spawn_blocking(move || {
        let framed = framer::frame(&file_bytes, &filename);
        let samples = encoder::encode(&framed);
        let wav_bytes = wav::write_to_bytes(&samples)?;
        Ok::<(String, Vec<u8>), String>((filename, wav_bytes))
    })
    .await
    .map_err(|e| ApiError::Internal(format!("task panic: {e}")))?
    .map_err(ApiError::EncodeFailed)?;

    let (original_filename, wav_bytes) = result;
    let stem = std::path::Path::new(&original_filename)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let out_name = format!("{stem}_encoded.wav");

    info!(output_filename = %out_name, wav_bytes = wav_bytes.len(), "POST /wave/encode complete");

    Ok((
        [
            (header::CONTENT_TYPE, "audio/wav"),
            (header::CONTENT_DISPOSITION, &format!("attachment; filename=\"{out_name}\"")),
        ],
        wav_bytes,
    )
        .into_response())
}

// ── POST /wave/decode ──────────────────────────────────────────────────────

pub async fn wave_decode(mut multipart: Multipart) -> Result<Response, ApiError> {
    let (_field_name, wav_bytes) = extract_file_field(&mut multipart).await?;

    info!(wav_bytes = wav_bytes.len(), "POST /wave/decode starting");

    let result = tokio::task::spawn_blocking(move || {
        let samples = wav::read_from_bytes(&wav_bytes)?;
        let decoded = decoder::decode(&samples)?;
        Ok::<crate::framer::Decoded, String>(decoded)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("task panic: {e}")))?
    .map_err(ApiError::DecodeFailed)?;

    info!(
        original_filename = %result.filename,
        decoded_bytes = result.data.len(),
        "POST /wave/decode complete"
    );

    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (header::CONTENT_DISPOSITION, &format!("attachment; filename=\"{}\"", result.filename)),
        ],
        result.data,
    )
        .into_response())
}

// ── Shared helper ──────────────────────────────────────────────────────────

async fn extract_file_field(
    multipart: &mut Multipart,
) -> Result<(String, Vec<u8>), ApiError> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("multipart error: {e}")))?
    {
        let filename = field.file_name().unwrap_or("upload").to_string();
        let data = field
            .bytes()
            .await
            .map_err(|e| ApiError::BadRequest(format!("could not read field bytes: {e}")))?;

        if data.is_empty() {
            warn!(filename = %filename, "received empty file field");
            return Err(ApiError::BadRequest("file field is empty".into()));
        }

        return Ok((filename, data.to_vec()));
    }

    Err(ApiError::BadRequest("no file field found in multipart body".into()))
}
```

```bash
cargo check
```

**Commit:** `feat: add api/wave.rs handlers`

---

### Step 7 — `src/api/broadcast.rs` *(CREATE)*

```rust
//! Handlers for the /broadcast/* channel network endpoints.
//! Exposed in both `serve` mode and `gui` mode.

use axum::{
    extract::{Multipart, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use bytes::Bytes;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::{
    api::{
        errors::ApiError,
        models::{BroadcastStatusResponse, QueueEmptyResponse, ReceiveResponse, TransmitResponse},
        state::{AppState, QueuedFile},
    },
    decoder, encoder, framer, wav,
};

// ── GET /broadcast/status ──────────────────────────────────────────────────

pub async fn broadcast_status(State(state): State<AppState>) -> Json<BroadcastStatusResponse> {
    let queue_depth = state.queue_depth().await;
    let broadcaster_connected = check_broadcaster_reachable(&state.broadcaster_url).await;
    let channet_connected = crate::api::chan::check_channet_reachable(&state.channet_url).await;

    info!(
        queue_depth,
        broadcaster_connected,
        channet_connected,
        broadcaster_url = %state.broadcaster_url,
        channet_url = %state.channet_url,
        "GET /broadcast/status"
    );

    Json(BroadcastStatusResponse {
        service: "rustwave",
        broadcaster_connected,
        channet_connected,
        broadcaster_url: state.broadcaster_url.clone(),
        queue_depth,
    })
}

async fn check_broadcaster_reachable(url: &str) -> bool {
    match reqwest::Client::new()
        .get(url)
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
    {
        Ok(r) => r.status().is_success(),
        Err(_) => false,
    }
}

// ── POST /broadcast/transmit ───────────────────────────────────────────────

pub async fn broadcast_transmit(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<TransmitResponse>, ApiError> {
    let (filename, file_bytes) = extract_file_field(&mut multipart).await?;

    info!(filename = %filename, input_bytes = file_bytes.len(), "POST /broadcast/transmit received file");

    let wav_bytes: Vec<u8> = tokio::task::spawn_blocking({
        let filename = filename.clone();
        move || {
            let framed = framer::frame(&file_bytes, &filename);
            let samples = encoder::encode(&framed);
            wav::write_to_bytes(&samples)
        }
    })
    .await
    .map_err(|e| ApiError::Internal(format!("task panic: {e}")))?
    .map_err(ApiError::EncodeFailed)?;

    let wav_size = wav_bytes.len();
    let tx_id = Uuid::new_v4();

    info!(%tx_id, wav_bytes = wav_size, broadcaster_url = %state.broadcaster_url,
        "POST /broadcast/transmit forwarding to Broadcaster");

    forward_to_broadcaster(&state.broadcaster_url, &filename, wav_bytes, tx_id).await?;

    Ok(Json(TransmitResponse { status: "ok", tx_id, wav_bytes: wav_size }))
}

pub async fn forward_to_broadcaster(
    broadcaster_url: &str,
    original_filename: &str,
    wav_bytes: Vec<u8>,
    tx_id: Uuid,
) -> Result<(), ApiError> {
    let stem = std::path::Path::new(original_filename)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let wav_filename = format!("{stem}_encoded.wav");

    let part = reqwest::multipart::Part::bytes(wav_bytes)
        .file_name(wav_filename)
        .mime_str("audio/wav")
        .map_err(|e| ApiError::Internal(format!("mime error: {e}")))?;
    let form = reqwest::multipart::Form::new().part("file", part);

    let resp = reqwest::Client::new()
        .post(broadcaster_url)
        .multipart(form)
        .send()
        .await
        .map_err(|e| ApiError::BroadcasterUnavailable(
            format!("could not reach Broadcaster at {broadcaster_url}: {e}")
        ))?;

    if !resp.status().is_success() {
        return Err(ApiError::BroadcasterUnavailable(format!(
            "Broadcaster returned HTTP {} for tx_id {tx_id}", resp.status()
        )));
    }

    debug!(%tx_id, "Broadcaster accepted WAV");
    Ok(())
}

// ── POST /broadcast/receive ────────────────────────────────────────────────

pub async fn broadcast_receive(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<ReceiveResponse>, ApiError> {
    let (_filename, wav_bytes) = extract_file_field(&mut multipart).await?;

    info!(wav_bytes = wav_bytes.len(), "POST /broadcast/receive decoding WAV");

    let decoded = tokio::task::spawn_blocking(move || {
        let samples = wav::read_from_bytes(&wav_bytes)?;
        decoder::decode(&samples)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("task panic: {e}")))?
    .map_err(ApiError::DecodeFailed)?;

    let decoded_size = decoded.data.len();
    let queued_id = Uuid::new_v4();

    info!(
        %queued_id,
        original_filename = %decoded.filename,
        decoded_bytes = decoded_size,
        "POST /broadcast/receive queuing decoded file"
    );

    state.enqueue(QueuedFile { queued_id, bytes: Bytes::from(decoded.data) }).await;

    Ok(Json(ReceiveResponse { status: "ok", queued_id, decoded_bytes: decoded_size }))
}

// ── GET /broadcast/incoming ────────────────────────────────────────────────

pub async fn broadcast_incoming(State(state): State<AppState>) -> Response {
    match state.dequeue().await {
        Some(file) => {
            info!(queued_id = %file.queued_id, bytes = file.bytes.len(),
                "GET /broadcast/incoming dequeuing file");
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "application/octet-stream"),
                    (header::CONTENT_DISPOSITION, "attachment; filename=\"snapshot.zip\""),
                ],
                file.bytes,
            )
                .into_response()
        }
        None => {
            debug!("GET /broadcast/incoming queue is empty");
            (StatusCode::OK, Json(QueueEmptyResponse { status: "empty" })).into_response()
        }
    }
}

// ── Shared helper ──────────────────────────────────────────────────────────

async fn extract_file_field(
    multipart: &mut Multipart,
) -> Result<(String, Vec<u8>), ApiError> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("multipart error: {e}")))?
    {
        let filename = field.file_name().unwrap_or("upload").to_string();
        let data = field
            .bytes()
            .await
            .map_err(|e| ApiError::BadRequest(format!("could not read field bytes: {e}")))?;

        if data.is_empty() {
            warn!(filename = %filename, "received empty file field");
            return Err(ApiError::BadRequest("file field is empty".into()));
        }

        return Ok((filename, data.to_vec()));
    }

    Err(ApiError::BadRequest("no file field found in multipart body".into()))
}
```

```bash
cargo check
```

**Commit:** `feat: add api/broadcast.rs handlers`

---

### Step 7.5 — `src/api/chan.rs` *(CREATE)*

```rust
//! ChanNet HTTP client and /chan/request proxy handler.
//!
//! ChanNet already calls RustWave on:
//!   POST /broadcast/transmit  — pushes a ZIP snapshot for AFSK encoding & over-air transmission
//!   GET  /broadcast/incoming  — pulls decoded ZIP snapshots that arrived over radio
//!
//! This module adds the outbound direction:
//!   POST /chan/request  — operator sends a typed ChanCommand; RustWave forwards it to
//!                        ChanNet's /chan/command, receives the ZIP response, AFSK-encodes
//!                        it into WAV, and calls forward_to_broadcaster() for transmission.

use axum::{extract::State, Json};
use tracing::info;
use uuid::Uuid;

use crate::api::{
    errors::ApiError,
    models::{ChanCommand, ChanRequestResponse},
    state::AppState,
};
use crate::{encoder, framer, wav};

// ── Reachability probe ─────────────────────────────────────────────────────

/// Hits ChanNet's GET /chan/status. Used by broadcast_status to report
/// whether the paired ChanNet node is reachable.
pub async fn check_channet_reachable(channet_url: &str) -> bool {
    match reqwest::Client::new()
        .get(format!("{channet_url}/chan/status"))
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
    {
        Ok(r) => r.status().is_success(),
        Err(_) => false,
    }
}

// ── ChanNet command client ─────────────────────────────────────────────────

/// POST a typed ChanCommand to ChanNet's /chan/command endpoint.
/// Returns the raw ZIP bytes from the response body.
pub async fn send_chan_command(
    channet_url: &str,
    command: &ChanCommand,
) -> Result<bytes::Bytes, ApiError> {
    let resp = reqwest::Client::new()
        .post(format!("{channet_url}/chan/command"))
        .json(command)
        .send()
        .await
        .map_err(|e| ApiError::BroadcasterUnavailable(
            format!("ChanNet unreachable at {channet_url}: {e}")
        ))?;

    if !resp.status().is_success() {
        return Err(ApiError::BroadcasterUnavailable(
            format!("ChanNet /chan/command returned HTTP {}", resp.status()),
        ));
    }

    resp.bytes()
        .await
        .map_err(|e| ApiError::Internal(format!("reading ChanNet response body: {e}")))
}

// ── POST /chan/request ─────────────────────────────────────────────────────

pub async fn chan_request(
    State(state): State<AppState>,
    Json(command): Json<ChanCommand>,
) -> Result<Json<ChanRequestResponse>, ApiError> {
    info!(?command, channet_url = %state.channet_url, "POST /chan/request");

    // 1. Fetch ZIP from ChanNet.
    let zip_bytes = send_chan_command(&state.channet_url, &command).await?;
    let zip_len = zip_bytes.len();

    // 2. AFSK-encode the ZIP into WAV bytes (CPU-bound; run on blocking thread).
    let wav_bytes: Vec<u8> = tokio::task::spawn_blocking(move || {
        let framed = framer::frame(&zip_bytes, "channet_payload.zip");
        let samples = encoder::encode(&framed);
        wav::write_to_bytes(&samples)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("task panic: {e}")))?
    .map_err(ApiError::EncodeFailed)?;

    // 3. Forward the WAV to the external Broadcaster for over-air transmission.
    //    Reuses the same helper as /broadcast/transmit.
    let tx_id = Uuid::new_v4();
    crate::api::broadcast::forward_to_broadcaster(
        &state.broadcaster_url,
        "channet_payload.zip",
        wav_bytes,
        tx_id,
    )
    .await?;

    info!(%tx_id, zip_bytes = zip_len, "POST /chan/request transmitted successfully");

    Ok(Json(ChanRequestResponse {
        status: "transmitted",
        tx_id,
        zip_bytes: zip_len,
    }))
}
```

```bash
cargo check
```

**Commit:** `feat: add api/chan.rs — ChanNet client and /chan/request handler`

---

```rust
//! RustWave HTTP API server.
//!
//! full_router() — /wave/* + /broadcast/* + /chan/* (serve subcommand)
//! gui_router()  — /broadcast/* + /chan/*            (gui subcommand)

pub mod broadcast;
pub mod chan;
pub mod errors;
pub mod models;
pub mod state;
pub mod wave;

use axum::{routing::get, routing::post, Router};
use std::net::SocketAddr;
use tower_http::limit::RequestBodyLimitLayer;
use tracing::info;

use state::AppState;

const BODY_LIMIT: usize = 10 * 1024 * 1024; // 10 MB

pub fn full_router(state: AppState) -> Router {
    let wave_routes = Router::new()
        .route("/wave/status", get(wave::wave_status))
        .route("/wave/encode", post(wave::wave_encode))
        .route("/wave/decode", post(wave::wave_decode));

    Router::new()
        .merge(wave_routes)
        .merge(broadcast_routes())
        .merge(chan_routes())
        .layer(RequestBodyLimitLayer::new(BODY_LIMIT))
        .with_state(state)
}

pub fn gui_router(state: AppState) -> Router {
    Router::new()
        .merge(broadcast_routes())
        .merge(chan_routes())
        .layer(RequestBodyLimitLayer::new(BODY_LIMIT))
        .with_state(state)
}

fn broadcast_routes() -> Router<AppState> {
    Router::new()
        .route("/broadcast/status",   get(broadcast::broadcast_status))
        .route("/broadcast/transmit", post(broadcast::broadcast_transmit))
        .route("/broadcast/receive",  post(broadcast::broadcast_receive))
        .route("/broadcast/incoming", get(broadcast::broadcast_incoming))
}

fn chan_routes() -> Router<AppState> {
    Router::new()
        .route("/chan/request", post(chan::chan_request))
}

pub async fn run_server(router: Router, bind_addr: SocketAddr) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    info!(addr = %bind_addr, "RustWave API server listening");
    axum::serve(listener, router).await?;
    Ok(())
}
```

> **Note:** `run_server` uses `anyhow::Result` — add `anyhow = "1"` to `Cargo.toml` if not already present.

```bash
cargo check
```

**Commit:** `feat: add api/mod.rs — router builder and run_server`

---

### Step 9 — `src/main.rs` *(EDIT — full replacement)*

```rust
mod api;
mod config;
mod decoder;
mod encoder;
mod framer;
mod gui;
mod logging;
mod wav;

use clap::{Parser, Subcommand};
use std::{net::SocketAddr, path::PathBuf};

#[derive(Parser)]
#[command(name = "rustwave-cli", version, about = "RustWave audio codec", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Launch the drag-and-drop GUI (also starts /broadcast/* API on 127.0.0.1:7071)
    Gui,
    /// Start the HTTP API server (both /wave/* and /broadcast/* on 127.0.0.1:7071)
    Serve {
        #[arg(short, long, value_name = "ADDR")]
        bind: Option<SocketAddr>,
    },
    /// Encode a file into an AFSK WAV
    Encode {
        #[arg(short, long, value_name = "FILE")]
        input: PathBuf,
        #[arg(short, long, value_name = "FILE")]
        output: PathBuf,
    },
    /// Decode an AFSK WAV — restores the original filename automatically
    Decode {
        #[arg(short, long, value_name = "FILE")]
        input: PathBuf,
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
    },
}

fn main() {
    let _log_guard = logging::init();
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();

    match cli.command {
        Command::Gui => {
            gui::run().map_err(|e| format!("GUI error: {e}"))?;
        }

        Command::Serve { bind } => {
            let addr = bind
                .or_else(|| std::env::var("RUSTWAVE_BIND").ok().and_then(|s| s.parse().ok()))
                .unwrap_or_else(|| "127.0.0.1:7071".parse().unwrap());

            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| format!("failed to build Tokio runtime: {e}"))?;

            rt.block_on(async move {
                let state = api::state::AppState::new(true);
                let router = api::full_router(state);
                api::run_server(router, addr)
                    .await
                    .map_err(|e| format!("server error: {e}"))
            })?;
        }

        Command::Encode { input, output } => {
            let data = std::fs::read(&input)
                .map_err(|e| format!("cannot read '{}': {e}", input.display()))?;
            let filename = input.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let framed = framer::frame(&data, &filename);
            let samples = encoder::encode(&framed);
            wav::write(&output, &samples)
                .map_err(|e| format!("cannot write '{}': {e}", output.display()))?;
            #[allow(clippy::cast_precision_loss)]
            let duration = samples.len() as f64 / f64::from(config::SAMPLE_RATE);
            eprintln!("encoded '{}' ({} byte{}) -> {} ({duration:.2} s)",
                filename, data.len(), plural(data.len()), output.display());
        }

        Command::Decode { input, output } => {
            let samples = wav::read(&input)
                .map_err(|e| format!("cannot read '{}': {e}", input.display()))?;
            let decoded = decoder::decode(&samples).map_err(|e| format!("decode failed: {e}"))?;
            let out_path = output.unwrap_or_else(|| {
                input.parent().unwrap_or_else(|| std::path::Path::new(".")).join(&decoded.filename)
            });
            std::fs::write(&out_path, &decoded.data)
                .map_err(|e| format!("cannot write '{}': {e}", out_path.display()))?;
            eprintln!("decoded {} byte{} -> '{}' (original filename: '{}')",
                decoded.data.len(), plural(decoded.data.len()), out_path.display(), decoded.filename);
        }
    }

    Ok(())
}

const fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}
```

```bash
cargo check
cargo test
./target/debug/rustwave-cli --help
```

**Commit:** `feat: add serve subcommand and wire logging in main.rs`

---

### Step 10 — `src/gui.rs` *(EDIT — replace `pub fn run()` only)*

Find the existing `pub fn run() -> eframe::Result<()>` function at the bottom of `gui.rs` and replace it with:

```rust
pub fn run() -> eframe::Result<()> {
    // Spawn the /broadcast/* API server on a background OS thread.
    std::thread::spawn(|| {
        let addr: std::net::SocketAddr = std::env::var("RUSTWAVE_BIND")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| "127.0.0.1:7071".parse().unwrap());

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build Tokio runtime for GUI API server");

        rt.block_on(async move {
            let state = crate::api::state::AppState::new(false);
            let router = crate::api::gui_router(state);
            if let Err(e) = crate::api::run_server(router, addr).await {
                tracing::error!("GUI API server error: {e}");
            }
        });
    });

    tracing::info!("GUI mode: /broadcast/* API started on 127.0.0.1:7071");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([480.0, 340.0])
            .with_min_inner_size([360.0, 260.0])
            .with_title("RustWave")
            .with_drag_and_drop(true),
        ..Default::default()
    };

    eframe::run_native(
        "RustWave",
        options,
        Box::new(|cc| Ok(Box::new(AfskGui::new(cc)) as Box<dyn eframe::App>)),
    )
}
```

**No other changes to `gui.rs`.** All existing drag-and-drop, progress bar, and encode/decode logic is untouched.

```bash
cargo check
cargo clippy
```

**Commit:** `feat: spawn broadcast API server in GUI mode`

---

### Step 11 — `src/wav.rs` *(EDIT — append before `#[cfg(test)]`)*

Add to the bottom of `src/wav.rs`, before the existing `#[cfg(test)]` block:

```rust
// ── In-memory variants used by the HTTP API ──────────────────────────────

pub fn write_to_bytes(samples: &[f64]) -> Result<Vec<u8>, String> {
    use std::io::Cursor;
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut buf: Vec<u8> = Vec::new();
    let cursor = Cursor::new(&mut buf);
    let mut writer = hound::WavWriter::new(cursor, spec).map_err(|e| e.to_string())?;

    for &s in samples {
        #[allow(clippy::cast_possible_truncation)]
        let v = (s.clamp(-1.0, 1.0) * 32_767.0) as i16;
        writer.write_sample(v).map_err(|e| e.to_string())?;
    }

    writer.finalize().map_err(|e| e.to_string())?;
    Ok(buf)
}

pub fn read_from_bytes(data: &[u8]) -> Result<Vec<f64>, String> {
    use std::io::Cursor;
    let cursor = Cursor::new(data);
    let mut reader = hound::WavReader::new(cursor).map_err(|e| e.to_string())?;
    let spec = reader.spec();

    match (spec.bits_per_sample, spec.sample_format) {
        (16, hound::SampleFormat::Int) => {
            let channels = usize::from(spec.channels);
            if channels == 0 {
                return Err("invalid WAV: 0 channels".into());
            }
            reader
                .samples::<i16>()
                .step_by(channels)
                .map(|s| s.map(|v| f64::from(v) / 32_768.0).map_err(|e| e.to_string()))
                .collect()
        }
        (bits, fmt) => Err(format!(
            "unsupported WAV format: {bits}-bit {fmt:?} (rustwave-cli expects 16-bit integer PCM)"
        )),
    }
}
```

Also add inside the existing `#[cfg(test)]` block:

```rust
#[test]
fn memory_round_trip() -> Result<(), String> {
    use std::f64::consts::TAU;
    #[allow(clippy::cast_precision_loss)]
    let original: Vec<f64> = (0..4_410_i32)
        .map(|i| 0.5 * (TAU * 440.0 * f64::from(i) / 44_100.0).sin())
        .collect();
    let bytes = write_to_bytes(&original)?;
    let recovered = read_from_bytes(&bytes)?;
    assert_eq!(original.len(), recovered.len());
    for (a, b) in original.iter().zip(recovered.iter()) {
        assert!((a - b).abs() < 5e-5, "quantisation error: {a} vs {b}");
    }
    Ok(())
}
```

```bash
cargo check
cargo test
```

**Commit:** `feat: add wav::write_to_bytes and read_from_bytes for API use`

---

### Step 12 — `src/api/tests.rs` *(CREATE)*

```rust
#[cfg(test)]
mod tests {
    use crate::api::state::AppState;

    #[tokio::test]
    async fn queue_enqueue_dequeue() {
        use crate::api::state::QueuedFile;
        use bytes::Bytes;
        use uuid::Uuid;

        let state = AppState::new(false);
        assert_eq!(state.queue_depth().await, 0);

        state.enqueue(QueuedFile {
            queued_id: Uuid::new_v4(),
            bytes: Bytes::from_static(b"hello"),
        }).await;

        assert_eq!(state.queue_depth().await, 1);
        let file = state.dequeue().await.unwrap();
        assert_eq!(file.bytes.as_ref(), b"hello");
        assert!(state.dequeue().await.is_none());
    }
}
```

```bash
cargo test
```

**Commit:** `test: add integration test script and api/tests.rs`

---

## Final Verification

```bash
cargo deny check
cargo build --release
git tag v0.2.0-api
```

---

## Quick Reference — Route Table

| Mode | Route | Method | Registered? |
|---|---|---|---|
| `serve` | `/wave/status` | GET | ✓ |
| `serve` | `/wave/encode` | POST | ✓ |
| `serve` | `/wave/decode` | POST | ✓ |
| `serve` + `gui` | `/broadcast/status` | GET | ✓ |
| `serve` + `gui` | `/broadcast/transmit` | POST | ✓ |
| `serve` + `gui` | `/broadcast/receive` | POST | ✓ |
| `serve` + `gui` | `/broadcast/incoming` | GET | ✓ |
| `serve` + `gui` | `/chan/request` | POST | ✓ |
| `gui` | `/wave/*` | any | ✗ (404) |

**ChanNet ↔ RustWave call directions:**
- *ChanNet → RustWave:* ChanNet's `/chan/refresh` posts ZIP snapshots to `POST /broadcast/transmit`; ChanNet's `/chan/poll` reads decoded inbound payloads from `GET /broadcast/incoming`. No new routes are needed for this direction.
- *RustWave → ChanNet:* `POST /chan/request` accepts a `ChanCommand` JSON body, forwards it to ChanNet's `/chan/command`, AFSK-encodes the returned ZIP, and forwards the WAV to the external Broadcaster for over-air transmission.

## Environment Variables

| Variable | Default | Description |
|---|---|---|
| `RUSTWAVE_BIND` | `127.0.0.1:7071` | API server bind address |
| `RUSTWAVE_BROADCASTER_URL` | `http://localhost:9090` | URL to forward encoded WAV to |
| `RUSTWAVE_CHANNET_URL` | `http://localhost:7070` | Base URL of the paired ChanNet node |
| `RUSTWAVE_LOG` | `info` | stderr log filter (tracing-subscriber syntax) |
