use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use std::path::Path;

use crate::config::SAMPLE_RATE;

/// Write normalised f64 samples (range −1.0 … 1.0) to a 16-bit mono PCM WAV.
pub fn write(path: &Path, samples: &[f64]) -> Result<(), String> {
    let spec = WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };

    let mut writer = WavWriter::create(path, spec).map_err(|e| e.to_string())?;

    for &s in samples {
        // clamp guarantees the value is in [-32767, 32767] before truncation
        #[allow(clippy::cast_possible_truncation)]
        let v = (s.clamp(-1.0, 1.0) * 32_767.0) as i16;
        writer.write_sample(v).map_err(|e| e.to_string())?;
    }

    writer.finalize().map_err(|e| e.to_string())
}

/// Read a 16-bit mono PCM WAV and return normalised f64 samples (range −1 … 1).
///
/// Stereo files are accepted; only the first (left) channel is used.
pub fn read(path: &Path) -> Result<Vec<f64>, String> {
    let mut reader = WavReader::open(path).map_err(|e| e.to_string())?;
    let spec = reader.spec();

    match (spec.bits_per_sample, spec.sample_format) {
        (16, SampleFormat::Int) => {
            let channels = usize::from(spec.channels);
            if channels == 0 {
                return Err("invalid WAV: 0 channels".into());
            }
            reader
                .samples::<i16>()
                .step_by(channels)
                .map(|s| {
                    s.map(|v| f64::from(v) / 32_768.0)
                        .map_err(|e| e.to_string())
                })
                .collect()
        }
        (bits, fmt) => Err(format!(
            "unsupported WAV format: {bits}-bit {fmt:?}  \
             (rustwave-cli expects 16-bit integer PCM)"
        )),
    }
}

// ── In-memory variants used by the HTTP API ──────────────────────────────

pub fn write_to_bytes(samples: &[f64]) -> Result<Vec<u8>, String> {
    use std::io::Cursor;
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut buf: Vec<u8> = Vec::new();
    let cursor = Cursor::new(&mut buf);
    let mut writer = hound::WavWriter::new(cursor, spec).map_err(|e| e.to_string())?;

    for &s in samples {
        #[allow(clippy::cast_possible_truncation)]
        let v = (s.clamp(-1.0, 1.0) * 32_767.0) as i16;
        writer.write_sample(v).map_err(|e| e.to_string())?;
    }

    writer.finalize().map_err(|e| e.to_string())?;
    Ok(buf)
}

pub fn read_from_bytes(data: &[u8]) -> Result<Vec<f64>, String> {
    use std::io::Cursor;
    let cursor = Cursor::new(data);
    let mut reader = hound::WavReader::new(cursor).map_err(|e| e.to_string())?;
    let spec = reader.spec();

    match (spec.bits_per_sample, spec.sample_format) {
        (16, hound::SampleFormat::Int) => {
            let channels = usize::from(spec.channels);
            if channels == 0 {
                return Err("invalid WAV: 0 channels".into());
            }
            reader
                .samples::<i16>()
                .step_by(channels)
                .map(|s| s.map(|v| f64::from(v) / 32_768.0).map_err(|e| e.to_string()))
                .collect()
        }
        (bits, fmt) => Err(format!(
            "unsupported WAV format: {bits}-bit {fmt:?} (rustwave-cli expects 16-bit integer PCM)"
        )),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU;

    /// Guard that removes a temp file on drop, even if the test panics.
    struct TempFile(std::path::PathBuf);

    impl TempFile {
        fn new(name: &str) -> Self {
            Self(std::env::temp_dir().join(name))
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn silence_round_trip() -> Result<(), String> {
        let tmp = TempFile::new("rustwave_wav_silence.wav");
        let original = vec![0.0_f64; 4_410];
        write(tmp.path(), &original)?;
        let recovered = read(tmp.path())?;
        assert_eq!(original.len(), recovered.len());
        for v in recovered {
            assert!(v.abs() < 2.0 / 32_768.0, "expected silence, got {v}");
        }
        Ok(())
    }

    #[test]
    fn sine_round_trip() -> Result<(), String> {
        let tmp = TempFile::new("rustwave_wav_sine.wav");
        #[allow(clippy::cast_precision_loss)]
        let original: Vec<f64> = (0..44_100_i32)
            .map(|i| 0.5 * (TAU * 440.0 * f64::from(i) / 44_100.0).sin())
            .collect();
        write(tmp.path(), &original)?;
        let recovered = read(tmp.path())?;
        assert_eq!(original.len(), recovered.len());
        for (a, b) in original.iter().zip(recovered.iter()) {
            assert!(
                (a - b).abs() < 5e-5,
                "quantisation error too large: {a} vs {b}"
            );
        }
        Ok(())
    }

    #[test]
    fn memory_round_trip() -> Result<(), String> {
        use std::f64::consts::TAU;
        #[allow(clippy::cast_precision_loss)]
        let original: Vec<f64> = (0..4_410_i32)
            .map(|i| 0.5 * (TAU * 440.0 * f64::from(i) / 44_100.0).sin())
            .collect();
        let bytes = write_to_bytes(&original)?;
        let recovered = read_from_bytes(&bytes)?;
        assert_eq!(original.len(), recovered.len());
        for (a, b) in original.iter().zip(recovered.iter()) {
            assert!((a - b).abs() < 5e-5, "quantisation error: {a} vs {b}");
        }
        Ok(())
    }
}
