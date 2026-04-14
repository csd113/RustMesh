/// Frame layout (v2 — stores original filename)
///
/// ┌──────────────┬────────┬──────────────┬──────┬──────────────┬─────────┬────────┐
/// │ preamble N×AA│ 7E 7E  │ `name_len` u16 │ name │ `payload_len`  │ payload │ CRC-16 │
/// └──────────────┴────────┴──────────────┴──────┴──────────────┴─────────┴────────┘
///                 ◄──────────────────── CRC covers this span ──────────────────────►
use crate::config::{PREAMBLE_LEN, SYNC};
use std::path::Path;

/// Decoded frame: original filename and payload bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub filename: String,
    pub data: Vec<u8>,
}

/// Wrap `data` in a transmittable frame, embedding the basename of `filename`
/// so the decoder can reconstruct the file with the correct name and extension.
///
/// # Panics
///
/// Panics if `data.len()` exceeds `u32::MAX` (≈ 4 GiB).
#[allow(
    clippy::arithmetic_side_effects,  // capacity arithmetic is safe; values are small by construction
    clippy::cast_possible_truncation, // name_len ≤ u16::MAX, data.len ≤ u32::MAX — guarded above each cast
    clippy::indexing_slicing,         // out[PREAMBLE_LEN..] is valid: preamble bytes are always pushed first
)]
pub fn frame(data: &[u8], filename: &str) -> Result<Vec<u8>, String> {
    let filename = Path::new(filename)
        .file_name()
        .ok_or_else(|| "filename must contain a final path component".to_string())?;
    let filename = filename
        .to_str()
        .ok_or_else(|| "filename must be valid UTF-8".to_string())?;

    let name_bytes = filename.as_bytes();
    let name_len = u16::try_from(name_bytes.len())
        .map_err(|_| "filename exceeds maximum frame size (u16::MAX bytes)".to_string())?;

    assert!(
        u32::try_from(data.len()).is_ok(),
        "payload exceeds maximum frame size (u32::MAX bytes)"
    );

    let capacity = PREAMBLE_LEN + 2 + 2 + name_bytes.len() + 4 + data.len() + 2;
    let mut out = Vec::with_capacity(capacity);

    // Preamble
    out.extend(std::iter::repeat_n(0xAA_u8, PREAMBLE_LEN));

    // Sync word
    out.extend_from_slice(&SYNC);

    // Filename length (u16 LE) + filename bytes — name_len is validated above
    out.extend_from_slice(&name_len.to_le_bytes());
    out.extend_from_slice(name_bytes);

    // Payload length (u32 LE) + payload — assert above guarantees cast is lossless
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);

    // CRC-16/CCITT over everything from sync word onwards (not the preamble)
    let crc = crc16(&out[PREAMBLE_LEN..]);
    out.extend_from_slice(&crc.to_le_bytes());

    Ok(out)
}

/// Find and validate a frame inside `raw`, returning the embedded filename and payload.
///
/// Used only by the byte-level path (tests / CLI verification).
/// The audio decoder uses `find_frame_in_bits` in decoder.rs directly.
#[allow(
    dead_code,
    clippy::arithmetic_side_effects,  // cursor arithmetic is bounds-checked before each use
    clippy::cast_possible_truncation, // u32 as usize: payload_len is bounds-checked before use
    clippy::indexing_slicing,         // all slices are bounds-checked immediately above each access
)]
pub fn deframe(raw: &[u8]) -> Result<Decoded, String> {
    let sync_pos = raw
        .windows(SYNC.len())
        .position(|w| w == SYNC)
        .ok_or_else(|| "sync word not found".to_string())?;

    let mut cursor = sync_pos + SYNC.len();

    // name_len (u16)
    if raw.len() < cursor + 2 {
        return Err("frame too short: missing name_len".into());
    }
    let name_len = u16::from_le_bytes(
        raw[cursor..cursor + 2]
            .try_into()
            .map_err(|_| "internal: name_len slice error".to_string())?,
    ) as usize;
    cursor += 2;

    // name bytes
    if raw.len() < cursor + name_len {
        return Err("frame too short: missing filename".into());
    }
    let filename = String::from_utf8(raw[cursor..cursor + name_len].to_vec())
        .map_err(|_| "filename is not valid UTF-8".to_string())?;
    cursor += name_len;

    // payload_len (u32)
    if raw.len() < cursor + 4 {
        return Err("frame too short: missing payload_len".into());
    }
    let payload_len = u32::from_le_bytes(
        raw[cursor..cursor + 4]
            .try_into()
            .map_err(|_| "internal: payload_len slice error".to_string())?,
    ) as usize;
    cursor += 4;

    let payload_end = cursor + payload_len;
    let crc_end = payload_end + 2;

    if raw.len() < crc_end {
        return Err(format!(
            "frame truncated: need {} bytes after sync, have {}",
            crc_end - sync_pos,
            raw.len() - sync_pos,
        ));
    }

    let stored_crc = u16::from_le_bytes(
        raw[payload_end..crc_end]
            .try_into()
            .map_err(|_| "internal: CRC slice error".to_string())?,
    );
    let computed_crc = crc16(&raw[sync_pos..payload_end]);

    if stored_crc != computed_crc {
        return Err(format!(
            "CRC mismatch: stored {stored_crc:#06x}, computed {computed_crc:#06x}"
        ));
    }

    Ok(Decoded {
        filename,
        data: raw[cursor..payload_end].to_vec(),
    })
}

// ---------------------------------------------------------------------------
// CRC-16/CCITT  (polynomial 0x1021, init 0xFFFF, no bit-reflection)
// ---------------------------------------------------------------------------

/// Pre-computed CRC-16/CCITT lookup table (polynomial 0x1021).
#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::indexing_slicing
)]
const CRC16_TABLE: [u16; 256] = {
    let mut table = [0u16; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut crc = (i as u16) << 8;
        let mut j = 0u8;
        while j < 8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

/// CRC-16/CCITT via table lookup (polynomial 0x1021, init 0xFFFF, no reflection).
#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::indexing_slicing
)]
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        // idx is always 0..=255: (u16 >> 8) ^ u16::from(u8) ≤ 0xFF, then cast to u8.
        let idx = ((crc >> 8) ^ u16::from(byte)) as u8 as usize;
        crc = (crc << 8) ^ CRC16_TABLE[idx];
    }
    crc
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(data: &[u8], name: &str) -> Decoded {
        #[allow(clippy::unwrap_used)]
        deframe(&frame(data, name).unwrap()).unwrap()
    }

    #[test]
    fn round_trip_empty_payload() {
        let d = rt(&[], "empty.bin");
        assert!(d.data.is_empty());
        assert_eq!(d.filename, "empty.bin");
    }

    #[test]
    fn round_trip_ascii() {
        let d = rt(b"Hello, AFSK!", "hello.txt");
        assert_eq!(d.data, b"Hello, AFSK!");
        assert_eq!(d.filename, "hello.txt");
    }

    #[test]
    fn round_trip_binary() {
        let data: Vec<u8> = (0u8..=255).collect();
        let d = rt(&data, "all_bytes.bin");
        assert_eq!(d.data, data);
    }

    #[test]
    fn filename_preserved() {
        let d = rt(b"data", "archive.tar.gz");
        assert_eq!(d.filename, "archive.tar.gz");
    }

    #[test]
    fn long_filename_is_preserved() {
        let filename = format!("{}.txt", "a".repeat(260));
        let d = rt(b"data", &filename);
        assert_eq!(d.filename, filename);
    }

    #[test]
    fn deframe_ignores_leading_garbage() {
        let mut framed = frame(b"test", "test.txt").unwrap();
        framed.insert(0, 0xFF);
        framed.insert(0, 0x42);
        #[allow(clippy::unwrap_used)]
        let d = deframe(&framed).unwrap();
        assert_eq!(d.data, b"test");
        assert_eq!(d.filename, "test.txt");
    }

    #[test]
    #[allow(
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )]
    fn crc_detects_corruption() {
        let mut framed = frame(b"integrity check", "check.txt").unwrap();
        // Corrupt a payload byte (past preamble+sync+namelen+name+payloadlen)
        let corrupt_pos = PREAMBLE_LEN + 2 + 2 + "check.txt".len() + 4 + 2;
        framed[corrupt_pos] ^= 0xFF;
        assert!(deframe(&framed).is_err());
    }

    #[test]
    fn crc16_known_value() {
        assert_eq!(crc16(b"123456789"), 0x29B1);
    }
}
