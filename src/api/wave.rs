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
