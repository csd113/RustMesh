//! `RustWave` HTTP API server.
//!
//! `full_router()` — /wave/* + /broadcast/* + /chan/* (serve subcommand)
//! `gui_router()`  — /broadcast/* + /chan/*            (gui subcommand)

pub mod broadcast;
pub mod chan;
pub mod errors;
pub mod models;
pub mod state;
pub mod wave;

use axum::{routing::get, routing::post, Router};
use std::net::SocketAddr;
use tower_http::cors::CorsLayer;
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
        .layer(CorsLayer::permissive())
        .layer(RequestBodyLimitLayer::new(BODY_LIMIT))
        .with_state(state)
}

pub fn gui_router(state: AppState) -> Router {
    Router::new()
        .merge(broadcast_routes())
        .merge(chan_routes())
        .layer(CorsLayer::permissive())
        .layer(RequestBodyLimitLayer::new(BODY_LIMIT))
        .with_state(state)
}

fn broadcast_routes() -> Router<AppState> {
    Router::new()
        .route("/broadcast/status", get(broadcast::broadcast_status))
        .route("/broadcast/transmit", post(broadcast::broadcast_transmit))
        .route("/broadcast/receive", post(broadcast::broadcast_receive))
        .route("/broadcast/incoming", get(broadcast::broadcast_incoming))
}

fn chan_routes() -> Router<AppState> {
    Router::new().route("/chan/request", post(chan::chan_request))
}

pub async fn run_server(router: Router, bind_addr: SocketAddr) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    info!(addr = %bind_addr, "RustWave API server listening");
    axum::serve(listener, router).await?;
    Ok(())
}

#[cfg(test)]
pub mod tests;
