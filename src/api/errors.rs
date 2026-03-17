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
