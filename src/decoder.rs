use crate::{
    config::{BAUD_RATE, MARK_FREQ, SAMPLE_RATE, SPACE_FREQ},
    framer::Decoded,
};
use std::f64::consts::TAU;

/// Precomputed sync-word bit pattern (`0x7E 0x7E`), avoiding heap allocation.
const SYNC_BITS: [bool; 16] = {
    let bytes = [0x7E_u8, 0x7E];
    let mut bits = [false; 16];
    let mut byte_idx = 0;
    while byte_idx < 2 {
        let mut bit_idx = 0;
        while bit_idx < 8 {
            bits[byte_idx * 8 + bit_idx] = (bytes[byte_idx] >> (7 - bit_idx)) & 1 == 1;
            bit_idx += 1;
        }
        byte_idx += 1;
    }
    bits
};

/// Decode PCM samples back to the original filename and payload bytes.
///
/// # Errors
///
/// Returns an error when no valid frame with a matching sync word and CRC can
/// be decoded from the provided samples.
pub fn decode_progress(samples: &[f64], on_progress: impl Fn(f32)) -> Result<Decoded, String> {
    let spb = f64::from(SAMPLE_RATE) / f64::from(BAUD_RATE);
    let spb_int = spb.round() as usize;

    for offset in 0..spb_int {
        let bits = samples_to_bits(samples, offset, spb, &on_progress);
        if let Ok(decoded) = find_frame_in_bits(&bits) {
            on_progress(1.0);
            return Ok(decoded);
        }
    }

    Err("could not decode signal — sync word not found at any clock phase".into())
}

/// Convenience wrapper with no progress reporting.
///
/// # Errors
///
/// Returns an error when no valid frame with a matching sync word and CRC can
/// be decoded from the provided samples.
pub fn decode(samples: &[f64]) -> Result<Decoded, String> {
    decode_progress(samples, |_| {})
}

// ---------------------------------------------------------------------------
// Step 1 — sample → bit stream via Goertzel
// ---------------------------------------------------------------------------

fn samples_to_bits(
    samples: &[f64],
    offset: usize,
    spb: f64,
    on_progress: &impl Fn(f32),
) -> Vec<bool> {
    let total = samples.len().max(1);
    let estimated = (samples.len().saturating_sub(offset)) as f64 / spb;
    let mut bits = Vec::with_capacity(estimated as usize);

    for idx in 0usize.. {
        let start = offset + (idx as f64 * spb).round() as usize;
        let end = offset + ((idx + 1) as f64 * spb).round() as usize;
        if end > samples.len() {
            break;
        }

        let w = &samples[start..end];
        bits.push(goertzel(w, MARK_FREQ, SAMPLE_RATE) > goertzel(w, SPACE_FREQ, SAMPLE_RATE));

        if idx % 32 == 0 {
            on_progress(end as f32 / total as f32);
        }
    }
    bits
}

// ---------------------------------------------------------------------------
// Step 2 — search the bit stream for the frame
// ---------------------------------------------------------------------------

// All slice indexing in this function is guarded by explicit bounds checks
// immediately above each access, so indexing_slicing is a false positive here.
// try_into() on a Vec produced by bits_to_bytes with an exact bit-width input is
// guaranteed to succeed.
fn find_frame_in_bits(bits: &[bool]) -> Result<Decoded, String> {
    let sync_len = SYNC_BITS.len(); // 16

    let mut search = 0usize;
    while search + sync_len <= bits.len() {
        let Some(rel) = bits[search..]
            .windows(sync_len)
            .position(|w| w == SYNC_BITS.as_slice())
        else {
            break;
        };

        let sync_start = search + rel;
        let mut cursor = sync_start + sync_len;

        // ── name_len (u16 LE) ─────────────────────────────────────────
        if cursor + 16 > bits.len() {
            break;
        }
        let name_len = {
            let b = bits_to_bytes(&bits[cursor..cursor + 16]);
            u16::from_le_bytes(b.try_into().expect("bits_to_bytes(16 bits) = 2 bytes")) as usize
        };
        cursor += 16;

        // ── name bytes ────────────────────────────────────────────────
        let name_bits = name_len * 8;
        if cursor + name_bits > bits.len() {
            search = sync_start + 1;
            continue;
        }
        let name_bytes = bits_to_bytes(&bits[cursor..cursor + name_bits]);
        let Ok(filename) = String::from_utf8(name_bytes) else {
            search = sync_start + 1;
            continue;
        };
        cursor += name_bits;

        // ── payload_len (u32 LE) ──────────────────────────────────────
        if cursor + 32 > bits.len() {
            break;
        }
        let payload_len = {
            let b = bits_to_bytes(&bits[cursor..cursor + 32]);
            u32::from_le_bytes(b.try_into().expect("bits_to_bytes(32 bits) = 4 bytes")) as usize
        };
        cursor += 32;

        // ── payload ───────────────────────────────────────────────────
        let payload_start = cursor;
        let Some(payload_bits) = payload_len.checked_mul(8) else {
            search = sync_start + 1;
            continue;
        };
        let Some(payload_end) = payload_start.checked_add(payload_bits) else {
            search = sync_start + 1;
            continue;
        };
        let Some(crc_end) = payload_end.checked_add(16) else {
            search = sync_start + 1;
            continue;
        };
        if crc_end > bits.len() {
            search = sync_start + 1;
            continue;
        }

        // ── CRC check ─────────────────────────────────────────────────
        let frame_bytes = bits_to_bytes(&bits[sync_start..payload_end]);
        let computed = crate::framer::crc16(&frame_bytes);
        let stored = {
            let b = bits_to_bytes(&bits[payload_end..crc_end]);
            u16::from_le_bytes(b.try_into().expect("bits_to_bytes(16 bits) = 2 bytes"))
        };

        if stored == computed {
            return Ok(Decoded {
                filename,
                data: bits_to_bytes(&bits[payload_start..payload_end]),
            });
        }

        search = sync_start + 1;
    }

    Err("sync word not found".into())
}

// ---------------------------------------------------------------------------
// Non-integer Goertzel DFT
// ---------------------------------------------------------------------------

fn goertzel(samples: &[f64], freq: f64, sample_rate: u32) -> f64 {
    let w = TAU * freq / f64::from(sample_rate);
    let cos_w = w.cos();
    let sin_w = w.sin();
    let coeff = 2.0 * cos_w;
    let (mut s1, mut s2) = (0.0_f64, 0.0_f64);
    for &x in samples {
        let s0 = x + coeff.mul_add(s1, -s2);
        s2 = s1;
        s1 = s0;
    }
    let real = cos_w.mul_add(-s2, s1);
    let imag = sin_w * s2;
    real.mul_add(real, imag * imag)
}

// ---------------------------------------------------------------------------
// Bit / byte helpers
// ---------------------------------------------------------------------------

pub fn bits_to_bytes(bits: &[bool]) -> Vec<u8> {
    bits.chunks(8)
        .map(|chunk| {
            chunk
                .iter()
                .enumerate()
                .fold(0u8, |acc, (i, &b)| acc | if b { 1 << (7 - i) } else { 0 })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{encoder, framer};

    #[test]
    fn goertzel_discriminates_mark_vs_space() {
        let spb = (f64::from(SAMPLE_RATE) / f64::from(BAUD_RATE)).round() as usize;
        let mark_samples: Vec<f64> = (0..spb)
            .map(|i| (TAU * MARK_FREQ * i as f64 / f64::from(SAMPLE_RATE)).sin())
            .collect();
        let space_samples: Vec<f64> = (0..spb)
            .map(|i| (TAU * SPACE_FREQ * i as f64 / f64::from(SAMPLE_RATE)).sin())
            .collect();

        assert!(
            goertzel(&mark_samples, MARK_FREQ, SAMPLE_RATE)
                > goertzel(&mark_samples, SPACE_FREQ, SAMPLE_RATE)
        );
        assert!(
            goertzel(&space_samples, SPACE_FREQ, SAMPLE_RATE)
                > goertzel(&space_samples, MARK_FREQ, SAMPLE_RATE)
        );
    }

    #[test]
    fn full_round_trip_text() -> Result<(), String> {
        let payload = b"Hello, AFSK!";
        let framed = framer::frame(payload, "hello.txt")?;
        let samples = encoder::encode(&framed);
        let decoded = decode(&samples)?;
        assert_eq!(decoded.data, payload);
        assert_eq!(decoded.filename, "hello.txt");
        Ok(())
    }

    #[test]
    fn full_round_trip_all_bytes() -> Result<(), String> {
        let payload: Vec<u8> = (0u8..=255).collect();
        let framed = framer::frame(&payload, "all.bin")?;
        let samples = encoder::encode(&framed);
        let decoded = decode(&samples)?;
        assert_eq!(decoded.data, payload);
        assert_eq!(decoded.filename, "all.bin");
        Ok(())
    }

    #[test]
    fn full_round_trip_empty() -> Result<(), String> {
        let framed = framer::frame(&[], "empty.bin")?;
        let samples = encoder::encode(&framed);
        let decoded = decode(&samples)?;
        assert!(decoded.data.is_empty());
        Ok(())
    }

    #[test]
    fn filename_with_dots_preserved() -> Result<(), String> {
        let payload = b"compressed archive";
        let framed = framer::frame(payload, "archive.tar.gz")?;
        let samples = encoder::encode(&framed);
        let decoded = decode(&samples)?;
        assert_eq!(decoded.filename, "archive.tar.gz");
        Ok(())
    }
}
