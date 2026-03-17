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
