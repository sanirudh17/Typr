//! Local speech/no-speech classification. Never trims the recognition audio.
//! A neural detector distinguishes speech from fans, clicks and room noise;
//! normalization lets a quiet voice reach it without raising the capture gate.
use sha2::{Digest, Sha256};
use std::sync::Mutex;

const MODEL: &[u8] = include_bytes!("../assets/silero_vad.onnx");
const HASH: &str = "1a153a22f4509e292a94e67d6f9b85e8deb25b4988682b7e174c65279d8788e3";
static DETECTOR: Mutex<Option<sherpa_onnx::VoiceActivityDetector>> = Mutex::new(None);

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
                model: Some(path.to_string_lossy().into_owned()), threshold: 0.35,
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
    fn keeps_reference_speech_at_normal_and_quiet_levels_and_resets_between_captures() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scripts/test_clips/clip1_5s.wav");
        let (samples, rate) = crate::audio_chunker::read_wav_samples(&path).unwrap();
        for scale in [1.0, 0.1, 0.025] {
            let quiet: Vec<f32> = samples.iter().map(|s| s * scale).collect();
            assert!(contains_speech(&quiet, rate).unwrap(), "speech scale {scale}");
            assert!(!contains_speech(&vec![0.0; 96_000], 16_000).unwrap());
        }
    }
}
