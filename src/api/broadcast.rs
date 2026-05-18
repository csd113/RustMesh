//! Handlers for the /broadcast/* channel network endpoints.
//! Exposed in both `serve` mode and `gui` mode.

use axum::{
    extract::{Multipart, State},
    http::{header, StatusCode},
    response::{IntoResponse as _, Response},
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
        queue_limit: state.max_queue_depth,
    })
}

async fn check_broadcaster_reachable(url: &str) -> bool {
    reqwest::Client::new()
        .get(url)
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
}

// ── POST /broadcast/transmit ───────────────────────────────────────────────

/// Accept a file, encode it as a WAV payload, and forward it to the broadcaster.
///
/// # Errors
///
/// Returns an error if the multipart upload is invalid, encoding fails, the
/// blocking task fails, or the broadcaster rejects the generated WAV.
pub async fn broadcast_transmit(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<TransmitResponse>, ApiError> {
    let (filename, file_bytes) = extract_file_field(&mut multipart).await?;

    info!(filename = %filename, input_bytes = file_bytes.len(), "POST /broadcast/transmit received file");

    let wav_bytes: Vec<u8> = tokio::task::spawn_blocking({
        let filename = filename.clone();
        move || {
            let framed = framer::frame(&file_bytes, &filename)?;
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

    Ok(Json(TransmitResponse {
        status: "ok",
        tx_id,
        wav_bytes: wav_size,
    }))
}

/// Forward an encoded WAV payload to the configured broadcaster endpoint.
///
/// # Errors
///
/// Returns an error if the multipart request cannot be built, the broadcaster
/// cannot be reached, or it returns a non-success HTTP status.
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
        .map_err(|e| {
            ApiError::BroadcasterUnavailable(format!(
                "could not reach Broadcaster at {broadcaster_url}: {e}"
            ))
        })?;

    if !resp.status().is_success() {
        return Err(ApiError::BroadcasterUnavailable(format!(
            "Broadcaster returned HTTP {} for tx_id {tx_id}",
            resp.status()
        )));
    }

    debug!(%tx_id, "Broadcaster accepted WAV");
    Ok(())
}

// ── POST /broadcast/receive ────────────────────────────────────────────────

/// Decode an uploaded WAV and queue the restored payload for `ChanNet`.
///
/// # Errors
///
/// Returns an error if the multipart upload is invalid, WAV decoding fails, the
/// blocking task fails, or the incoming queue is full.
pub async fn broadcast_receive(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<ReceiveResponse>, ApiError> {
    let (_filename, wav_bytes) = extract_file_field(&mut multipart).await?;

    info!(
        wav_bytes = wav_bytes.len(),
        "POST /broadcast/receive decoding WAV"
    );

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

    state
        .try_enqueue(QueuedFile {
            queued_id,
            bytes: Bytes::from(decoded.data),
        })
        .await
        .map_err(|file| {
            ApiError::QueueFull(format!(
                "incoming queue full at {} entries; queued_id {} dropped",
                state.max_queue_depth, file.queued_id
            ))
        })?;

    Ok(Json(ReceiveResponse {
        status: "ok",
        queued_id,
        decoded_bytes: decoded_size,
    }))
}

// ── GET /broadcast/incoming ────────────────────────────────────────────────

pub async fn broadcast_incoming(State(state): State<AppState>) -> Response {
    if let Some(file) = state.dequeue().await {
        info!(queued_id = %file.queued_id, bytes = file.bytes.len(),
            "GET /broadcast/incoming dequeuing file");
        (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/octet-stream"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"snapshot.zip\"",
                ),
            ],
            file.bytes,
        )
            .into_response()
    } else {
        debug!("GET /broadcast/incoming queue is empty");
        (StatusCode::OK, Json(QueueEmptyResponse { status: "empty" })).into_response()
    }
}

// ── Shared helper ──────────────────────────────────────────────────────────

async fn extract_file_field(multipart: &mut Multipart) -> Result<(String, Vec<u8>), ApiError> {
    let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("multipart error: {e}")))?
    else {
        return Err(ApiError::BadRequest(
            "no file field found in multipart body".into(),
        ));
    };

    let filename = field.file_name().unwrap_or("upload").to_owned();
    let data = field
        .bytes()
        .await
        .map_err(|e| ApiError::BadRequest(format!("could not read field bytes: {e}")))?;

    if data.is_empty() {
        warn!(filename = %filename, "received empty file field");
        return Err(ApiError::BadRequest("file field is empty".into()));
    }

    Ok((filename, data.to_vec()))
}
