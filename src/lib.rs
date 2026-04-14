pub mod api;
pub mod config;
pub mod decoder;
pub mod encoder;
pub mod framer;
pub mod gui;
pub mod logging;
pub mod wav;

use std::path::{Path, PathBuf};

pub use decoder::{bits_to_bytes, decode, decode_progress};
pub use encoder::{encode, encode_progress};
pub use framer::{deframe, frame, Decoded};
pub use wav::{read, read_from_bytes, write, write_to_bytes};

pub fn encode_file(input: impl AsRef<Path>, output: impl AsRef<Path>) -> Result<(), String> {
    let input = input.as_ref();
    let output = output.as_ref();

    let data =
        std::fs::read(input).map_err(|e| format!("cannot read '{}': {e}", input.display()))?;
    let filename = input
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("cannot derive UTF-8 filename from '{}'", input.display()))?;
    let framed = frame(&data, filename)?;
    let samples = encode(&framed);
    write(output, &samples).map_err(|e| format!("cannot write '{}': {e}", output.display()))
}

pub fn decode_file(input: impl AsRef<Path>, output: Option<PathBuf>) -> Result<PathBuf, String> {
    let input = input.as_ref();

    let samples = read(input).map_err(|e| format!("cannot read '{}': {e}", input.display()))?;
    let decoded = decode(&samples).map_err(|e| format!("decode failed: {e}"))?;
    let out_path = output.unwrap_or_else(|| {
        input
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(&decoded.filename)
    });
    std::fs::write(&out_path, &decoded.data)
        .map_err(|e| format!("cannot write '{}': {e}", out_path.display()))?;
    Ok(out_path)
}
