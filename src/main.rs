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
