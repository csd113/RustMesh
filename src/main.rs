use clap::{CommandFactory as _, Parser, Subcommand};
use std::{
    fmt::Arguments,
    io::{self, Write as _},
    net::SocketAddr,
    path::PathBuf,
};

#[derive(Parser)]
#[command(name = "rustwave-cli", version, about = "RustWave audio codec", long_about = None)]
struct Cli {
    /// Launch the drag-and-drop GUI.
    #[arg(long)]
    gui: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Launch the drag-and-drop GUI.
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
    let _log_guard = rustwave::logging::init();
    if let Err(e) = run() {
        if let Err(_write_err) = write_stderr_line(format_args!("error: {e}")) {}
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();

    if cli.gui || matches!(cli.command.as_ref(), Some(Command::Gui)) {
        rustwave::gui::run().map_err(|e| format!("GUI error: {e}"))?;
    } else {
        match cli.command {
            Some(Command::Gui) => unreachable!(),
            Some(Command::Serve { bind }) => {
                let addr = bind
                    .or_else(|| {
                        let bind = std::env::var("RUSTWAVE_BIND").ok()?;
                        bind.parse().ok()
                    })
                    .unwrap_or_else(|| std::net::SocketAddr::from(([127, 0, 0, 1], 7071)));

                let rt = tokio::runtime::Runtime::new()
                    .map_err(|e| format!("failed to build Tokio runtime: {e}"))?;

                rt.block_on(async move {
                    let state = rustwave::api::state::AppState::new(true);
                    let router = rustwave::api::full_router(state);
                    rustwave::api::run_server(router, addr)
                        .await
                        .map_err(|e| format!("server error: {e}"))
                })?;
            }

            Some(Command::Encode { input, output }) => {
                rustwave::encode_file(&input, &output)?;
                let data_len = std::fs::metadata(&input)
                    .map_err(|e| format!("cannot read '{}': {e}", input.display()))?
                    .len() as usize;
                let duration = data_len as f64 / f64::from(rustwave::config::SAMPLE_RATE);
                let filename = input
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("<unknown>");
                write_stderr_line(format_args!(
                    "encoded '{}' ({} byte{}) -> {} ({duration:.2} s)",
                    filename,
                    data_len,
                    plural(data_len),
                    output.display()
                ))?;
            }

            Some(Command::Decode { input, output }) => {
                let samples = rustwave::wav::read(&input)
                    .map_err(|e| format!("cannot read '{}': {e}", input.display()))?;
                let decoded =
                    rustwave::decode(&samples).map_err(|e| format!("decode failed: {e}"))?;
                let out_path = output.unwrap_or_else(|| {
                    input
                        .parent()
                        .unwrap_or_else(|| std::path::Path::new("."))
                        .join(&decoded.filename)
                });
                std::fs::write(&out_path, &decoded.data)
                    .map_err(|e| format!("cannot write '{}': {e}", out_path.display()))?;
                write_stderr_line(format_args!(
                    "decoded {} byte{} -> '{}' (original filename: '{}')",
                    decoded.data.len(),
                    plural(decoded.data.len()),
                    out_path.display(),
                    decoded.filename
                ))?;
            }

            None => {
                Cli::command().print_help().map_err(|e| e.to_string())?;
                write_stderr_line(format_args!(""))?;
            }
        }
    }

    Ok(())
}

const fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn write_stderr_line(args: Arguments<'_>) -> Result<(), String> {
    let mut stderr = io::stderr().lock();
    writeln!(stderr, "{args}").map_err(|e| e.to_string())
}
