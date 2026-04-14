//! API error type for `RustWave`.
//!
//! Every handler returns `Result<_, ApiError>`. axum automatically calls
//! `IntoResponse` on the error path.

use crate::api::models::{ErrorDetail, ErrorEnvelope};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    EncodeFailed(String),
    DecodeFailed(String),
    BroadcasterUnavailable(String),
    QueueFull(String),
    Internal(String),
}

impl ApiError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "BAD_REQUEST",
            Self::EncodeFailed(_) => "ENCODE_FAILED",
            Self::DecodeFailed(_) => "DECODE_FAILED",
            Self::BroadcasterUnavailable(_) => "BROADCASTER_UNAVAILABLE",
            Self::QueueFull(_) => "QUEUE_FULL",
            Self::Internal(_) => "INTERNAL_ERROR",
        }
    }

    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::EncodeFailed(_) | Self::DecodeFailed(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::BroadcasterUnavailable(_) => StatusCode::BAD_GATEWAY,
            Self::QueueFull(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let message = match &self {
            Self::BadRequest(m)
            | Self::EncodeFailed(m)
            | Self::DecodeFailed(m)
            | Self::BroadcasterUnavailable(m)
            | Self::QueueFull(m)
            | Self::Internal(m) => m.clone(),
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
