use crate::config::{AMPLITUDE, BAUD_RATE, MARK_FREQ, SAMPLE_RATE, SPACE_FREQ};
use std::f64::consts::TAU;

/// Encode `framed` bytes into PCM audio samples.
/// Calls `on_progress` with a value in 0.0..=1.0 as bits are processed.
pub fn encode_progress(framed: &[u8], on_progress: impl Fn(f32)) -> Vec<f64> {
    let spb = f64::from(SAMPLE_RATE) / f64::from(BAUD_RATE);
    let mark_inc = TAU * MARK_FREQ / f64::from(SAMPLE_RATE);
    let space_inc = TAU * SPACE_FREQ / f64::from(SAMPLE_RATE);

    let total_bits = framed.len() * 8;
    let silence_len = (f64::from(SAMPLE_RATE) * 0.05).round() as usize;
    let signal_len = (total_bits as f64 * spb).round() as usize;
    let mut samples = Vec::with_capacity(silence_len * 2 + signal_len);

    samples.extend(std::iter::repeat_n(0.0_f64, silence_len));

    let mut phase = 0.0_f64;
    let mut bit_idx = 0usize;

    for &byte in framed {
        for i in (0..8u8).rev() {
            let bit = (byte >> i) & 1 == 1;
            let phase_inc = if bit { mark_inc } else { space_inc };

            let start = (bit_idx as f64 * spb).round() as usize;
            let end = ((bit_idx + 1) as f64 * spb).round() as usize;

            for _ in start..end {
                samples.push(AMPLITUDE * phase.sin());
                phase = (phase + phase_inc) % TAU;
            }

            if bit_idx.is_multiple_of(64) {
                on_progress(bit_idx as f32 / total_bits as f32);
            }
            bit_idx += 1;
        }
    }

    samples.extend(std::iter::repeat_n(0.0_f64, silence_len));
    on_progress(1.0);
    samples
}

/// Convenience wrapper with no progress reporting (used by CLI and tests).
pub fn encode(framed: &[u8]) -> Vec<f64> {
    encode_progress(framed, |_| {})
}
