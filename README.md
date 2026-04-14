# RustWave

Encode arbitrary bytes into a WAV file using Bell-202-style Audio Frequency-Shift Keying (AFSK), and decode them back — losslessly.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)
[![docs](https://img.shields.io/badge/docs-cargo%20doc-green.svg)](#api-documentation)

---

## Why RustWave?

Most data-over-audio tools are tightly coupled to specific protocols (APRS, KISS, etc.) or sacrifice correctness for simplicity. RustWave is a self-contained, dependency-light Rust library and CLI that gives you a clean primitive: **any byte buffer in → a valid WAV file out → the same byte buffer back**, with a robust framing layer, CRC-16 integrity checking, and a Goertzel-filter decoder that tolerates real-world signal noise and arbitrary byte offsets.

It is also the audio codec layer underpinning [HAMNET-RELAY](docs/hamnet-relay-build-roadmap.md), a planned HAM radio data relay that bridges AX.25 packet radio with a local HTTP/WebSocket API.

---

## Installation

Requires **Rust 1.75+** (2021 edition).

```bash
cargo install rustwave
```

Or build from source:

```bash
git clone https://github.com/csd113/rustwave
cd rustwave
cargo build --release
# binary at: target/release/rustwave-cli
```

---

## Basic Usage

### CLI

```bash
# Encode a file to WAV
rustwave-cli encode -i data.bin -o signal.wav

# Decode a WAV back to bytes
rustwave-cli decode -i signal.wav -o data.bin

# Launch the graphical interface
rustwave-cli --gui
```

### Library

```rust
use rustwave::{decode_file, encode_file};
use std::path::PathBuf;

fn main() {
    let input = PathBuf::from("data.bin");
    let output = PathBuf::from("signal.wav");
    encode_file(&input, &output).expect("encode failed");

    let recovered = decode_file(&output, None).expect("decode failed");
    assert_eq!(recovered.file_name().and_then(|s| s.to_str()), Some("data.bin"));
}
```

---

## How It Works

```
[ your bytes ]
      │
   FRAMER      preamble (24 × 0xAA) | sync (0x7E 0x7E) | name_len (u16 LE) | name | payload_len (u32 LE) | payload | CRC-16
      │
  ENCODER      each bit → sine wave at 1200 Hz (mark=1) or 2200 Hz (space=0)
               continuous-phase FSK at 1200 baud, 44100 Hz / 16-bit mono WAV
      │
  [ .wav ]
      │
  DECODER      non-integer Goertzel filter per bit-window; bit-level sync-word
               search (handles any byte offset); CRC verification
      │
   FRAMER      reconstruct original bytes
      │
[ your bytes ]
```

### Signal Parameters

| Parameter      | Value                                     |
|----------------|-------------------------------------------|
| Sample rate    | 44 100 Hz                                 |
| Bit rate       | 1 200 baud                                |
| Mark (1)       | 1 200 Hz                                  |
| Space (0)      | 2 200 Hz                                  |
| Modulation     | CPFSK (continuous-phase FSK)              |
| WAV format     | 16-bit signed PCM, mono                   |
| Frame overhead | 30 bytes + filename length (preamble + sync + name_len + payload_len + CRC) |

---

## Advanced Usage

### Framing & CRC

The framer wraps every payload in a Bell-202-compatible envelope. A 24-byte `0xAA` preamble allows the decoder to lock onto the signal, followed by a `0x7E 0x7E` sync word, a UTF-8 filename length field, the filename bytes, a 4-byte little-endian payload length field, the raw payload, and a CRC-16/CCITT checksum. The decoder performs a bit-level sync-word search that handles arbitrary byte offsets in the audio stream.

### GUI Mode

Launch the built-in `eframe`-powered GUI for drag-and-drop encoding and decoding without the CLI:

```bash
./rustwave-cli --gui
```

### HAMNET-RELAY Integration

RustWave is designed as the AFSK codec layer for [HAMNET-RELAY](docs/hamnet-relay-build-roadmap.md), a multi-phase HAM radio data relay project that adds AX.25 packet framing, CSMA channel access, a local HTTP/WebSocket API, and full RustChan imageboard integration on top of this codec. See [`docs/hamnet-relay-build-roadmap.md`](docs/hamnet-relay-build-roadmap.md) for the full architecture.

---

## Project Layout

```
src/
  main.rs       CLI (clap — encode / decode subcommands, GUI flag + subcommand)
  lib.rs        Public library helpers + re-exports
  config.rs     Shared constants (sample rate, baud rate, frequencies)
  framer.rs     Byte envelope: preamble, sync word, length, CRC-16/CCITT
  wav.rs        WAV file I/O via hound
  encoder.rs    Bytes → CPFSK audio samples
  decoder.rs    Audio samples → bytes (Goertzel filter, bit-level sync search)
```

---

## Running Tests

```bash
cargo test
```

12 unit and integration tests cover:

- Framer round-trips (empty payload, ASCII, all 256 byte values, corrupt CRC)
- CRC-16/CCITT known-value verification
- WAV write/read round-trip (silence and sine wave)
- Goertzel mark-vs-space discrimination
- Full encode → decode round-trips (empty, text, all byte values)

### Development Quality Gate

A strict dev-check script runs `fmt`, `clippy` (pedantic + nursery), `cargo test`, `cargo audit`, and `cargo deny` in sequence:

```bash
./dev-check-strict.sh
```

---

## API Documentation

Generate and open local docs:

```bash
cargo doc --open
```

---

## Contributing

1. Fork the repository and create a feature branch off `main`.
2. Run `./dev-check-strict.sh` — all checks must pass before submitting.
3. Open a pull request with a clear description of the change and any relevant test coverage.
4. Follow standard Rust style (`rustfmt` defaults). Clippy pedantic warnings are treated as errors.

---

## License

This project is licensed under the [MIT License](LICENSE).  
Copyright © 2026 csd113.

---

## Acknowledgements

- [hound](https://github.com/ruuda/hound) — WAV file I/O
- [clap](https://github.com/clap-rs/clap) — CLI argument parsing
- [eframe / egui](https://github.com/emilk/egui) — immediate-mode GUI
- Bell 202 modem standard — 1200/2200 Hz AFSK tone pair
- Goertzel algorithm — efficient single-frequency DFT for decoding
- [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) — license and advisory enforcement
