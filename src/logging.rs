//! Logging initialisation for `RustWave`.
//!
//! Call `logging::init()` once at the start of `main()`.
//!
//! Log output:
//!   - stderr:              INFO and above, human-readable
//!   - rustwave.log (file): DEBUG and above, JSON format, rolling daily
//!
//! The log file is written next to the binary.
//! Set `RUSTWAVE_LOG`=debug to see debug output on stderr too.

use std::path::PathBuf;
use tracing_appender::rolling;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

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
    let stderr_filter =
        EnvFilter::try_from_env("RUSTWAVE_LOG").unwrap_or_else(|_| EnvFilter::new("info"));

    let stderr_layer = fmt::layer().with_target(false).with_filter(stderr_filter);

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
