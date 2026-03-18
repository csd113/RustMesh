//! JSON request and response types for the `RustWave` API.

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
    /// sync or recovery only; `RustChan` logs a warning when this is received.
    ForceRefresh,
    /// Post a new reply to an existing thread (the only write command).
    ReplyPush {
        board: String,
        thread_id: u64,
        author: String,
        content: String,
        timestamp: u64,
    },
}

/// Returned by POST /chan/request on success.
#[derive(Serialize)]
pub struct ChanRequestResponse {
    pub status: &'static str, // "transmitted"
    pub tx_id: uuid::Uuid,
    pub zip_bytes: usize,
}

// ── Error envelope ─────────────────────────────────────────────────────────

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
