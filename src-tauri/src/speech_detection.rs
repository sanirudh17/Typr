//! Local speech/no-speech classification. Never trims the recognition audio.
//! A neural detector distinguishes speech from fans, clicks and room noise;
//! normalization lets a quiet voice reach it without raising the capture gate.
use sha2::{Digest, Sha256};
use std::sync::Mutex;

const MODEL: &[u8] = include_bytes!("../assets/silero_vad.onnx");
const HASH: &str = "1a153a22f4509e292a94e67d6f9b85e8deb25b4988682b7e174c65279d8788e3";
static DETECTOR: Mutex<Option<sherpa_onnx::VoiceActivityDetector>> = Mutex::new(None);

/// Skip a decode chunk only when energy and neural checks agree it is silent.
/// Neural detection of a quiet word or a detector error preserves the chunk.
pub fn confidently_silent_chunk(samples: &[f32], sample_rate: u32) -> bool {
    sample_rate != 0 && crate::audio::is_silent_capture(samples, sample_rate, 1.0)
        && matches!(contains_speech(samples, sample_rate), Ok(false))
}

pub fn contains_speech(samples: &[f32], sample_rate: u32) -> Result<bool, String> {
    if samples.is_empty() || sample_rate == 0 { return Ok(false); }
    let mut audio = crate::audio::resample(samples, sample_rate, 16_000);
    if audio.iter().any(|x| !x.is_finite()) { return Err("Non-finite capture samples".into()); }
    crate::audio::remove_dc_offset(&mut audio);
    crate::audio::highpass_50hz(&mut audio, 16_000);
    if audio.iter().all(|x| x.abs() < 1e-7) { return Ok(false); }
    // Only this analysis copy is normalized. Recognition receives the whole
    // capture, including soft onsets/endings and all pauses, unchanged here.
    crate::audio::normalize_target_rms(&mut audio, 16_000, 0.08, 200.0);
    crate::audio::apply_tanh_limiter(&mut audio, 0.75, 0.95);
    let mut guard = DETECTOR.lock().map_err(|_| "Speech detector lock failed")?;
    if guard.is_none() {
        let directory = dirs::cache_dir().ok_or("No local cache directory")?.join("Typr/speech-detection");
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let path = directory.join(format!("silero-{HASH}.onnx"));
        let valid = std::fs::read(&path).is_ok_and(|bytes| format!("{:x}", Sha256::digest(bytes)) == HASH);
        if !valid { std::fs::write(&path, MODEL).map_err(|e| e.to_string())?; }
        let config = sherpa_onnx::VadModelConfig {
            silero_vad: sherpa_onnx::SileroVadModelConfig {
                // Silero's balanced probability threshold. Quiet audio is
                // normalized above; lowering this admits more room noise.
                model: Some(path.to_string_lossy().into_owned()), threshold: 0.5,
                min_silence_duration: 0.1, min_speech_duration: 0.096,
                window_size: 512, max_speech_duration: 30.0,
            },
            sample_rate: 16_000, num_threads: 1, provider: Some("cpu".into()),
            ..Default::default()
        };
        *guard = Some(sherpa_onnx::VoiceActivityDetector::create(&config, 1.0)
            .ok_or("Could not initialize local speech detector")?);
    }
    let detector = guard.as_ref().unwrap();
    detector.reset();
    for chunk in audio.chunks(512) {
        detector.accept_waveform(chunk);
        if detector.detected() || !detector.is_empty() { detector.reset(); return Ok(true); }
    }
    detector.accept_waveform(&[0.0; 5120]);
    detector.flush();
    let found = !detector.is_empty();
    detector.reset();
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_detector_is_pinned() { assert_eq!(format!("{:x}", Sha256::digest(MODEL)), HASH); }
    #[test]
    fn rejects_silence_dc_fan_and_single_click() {
        for kind in 0..4 {
            let mut samples: Vec<f32> = (0..96_000).map(|i| match kind {
                0 => 0.0, 1 => 0.015,
                2 => 0.01 * (std::f32::consts::TAU * 120.0 * i as f32 / 16_000.0).sin(),
                _ => 0.0,
            }).collect();
            if kind == 3 { samples[32_000] = 0.7; }
            assert!(!contains_speech(&samples, 16_000).unwrap(), "noise kind {kind}");
        }
    }
    #[test]
    fn chunk_silence_requires_low_energy_and_neural_agreement() {
        assert!(confidently_silent_chunk(&vec![0.0;16000],16000));
        assert!(!confidently_silent_chunk(&[f32::NAN;512],16000));
        let path=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scripts/test_clips/clip1_5s.wav");
        let (samples,rate)=crate::audio_chunker::read_wav_samples(&path).unwrap();
        let quiet: Vec<f32>=samples.iter().map(|s|s*0.005).collect();
        assert!(!confidently_silent_chunk(&quiet,rate),"quiet speech must not be filtered as a silent chunk");
    }
    #[test]
    fn rejects_seeded_hiss_with_short_endpoint_clicks_and_keeps_quiet_speech_in_hiss() {
        let mut seed = 42u32;
        let mut noise: Vec<f32> = (0..96000).map(|_| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed as f64 / u32::MAX as f64 * 2.0 - 1.0) as f32 * 0.0003
        }).collect();
        for offset in [0, 95000] {
            for i in 0..640 {
                noise[offset+i] += 0.12 * (-(i as f32) / 100.0).exp()
                    * (std::f32::consts::TAU * 1400.0 * i as f32 / 16000.0).sin();
            }
        }
        assert!(!contains_speech(&noise, 16000).unwrap());
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scripts/test_clips/clip1_5s.wav");
        let (speech, rate) = crate::audio_chunker::read_wav_samples(&path).unwrap();
        let mut mixed = crate::audio::resample(&speech, rate, 16000);
        for (i, s) in mixed.iter_mut().enumerate() { *s = *s * 0.025 + noise[i % noise.len()] / 6.0; }
        assert!(contains_speech(&mixed, 16000).unwrap());
    }
    #[test]
    fn keeps_reference_speech_at_normal_and_quiet_levels_and_resets_between_captures() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scripts/test_clips/clip1_5s.wav");
        let (samples, rate) = crate::audio_chunker::read_wav_samples(&path).unwrap();
        for scale in [1.0, 0.1, 0.025, 0.005] {
            let quiet: Vec<f32> = samples.iter().map(|s| s * scale).collect();
            assert!(contains_speech(&quiet, rate).unwrap(), "speech scale {scale}");
            assert!(!contains_speech(&vec![0.0; 96_000], 16_000).unwrap());
        }
    }
}
