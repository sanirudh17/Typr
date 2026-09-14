//! Local Nemotron transcription on the CPU.
//!
//! FastConformer-RNNT (transducer) streaming ASR engine running in-process via sherpa-onnx.
//! Supports v3.5 (multilingual, 40 languages) and v3 (English-only).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Directory name for a model variant under app data dir.
pub fn model_dir_name(variant: &str) -> &'static str {
    match variant {
        "v3" => "nemotron-speech-streaming-en-0.6b-int8",
        _ => "nemotron-3.5-asr-streaming-0.6b-int8",
    }
}

/// Download URL for a variant.
pub fn model_download_url(variant: &str) -> String {
    match variant {
        "v3" => "https://huggingface.co/csukuangfj2/sherpa-onnx-nemotron-speech-streaming-en-0.6b-1120ms-int8-2026-04-25/resolve/main".to_string(),
        _ => "https://huggingface.co/csukuangfj2/sherpa-onnx-nemotron-3.5-asr-streaming-0.6b-1120ms-int8-2026-06-11/resolve/main".to_string(),
    }
}

/// The four files a sherpa-onnx offline transducer needs.
pub fn model_files_present(model_dir: &Path) -> bool {
    ["encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx", "tokens.txt"]
        .iter()
        .all(|f| model_dir.join(f).is_file())
}

static RECOGNIZER: Mutex<Option<(PathBuf, sherpa_onnx::OnlineRecognizer)>> = Mutex::new(None);

fn build_recognizer(model_dir: &Path) -> Result<sherpa_onnx::OnlineRecognizer, String> {
    let mut config = sherpa_onnx::OnlineRecognizerConfig::default();
    config.model_config.transducer.encoder =
        Some(model_dir.join("encoder.int8.onnx").to_string_lossy().into_owned());
    config.model_config.transducer.decoder =
        Some(model_dir.join("decoder.int8.onnx").to_string_lossy().into_owned());
    config.model_config.transducer.joiner =
        Some(model_dir.join("joiner.int8.onnx").to_string_lossy().into_owned());
    config.model_config.tokens =
        Some(model_dir.join("tokens.txt").to_string_lossy().into_owned());
    let num_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(2, 4);
    config.model_config.num_threads = num_threads as i32;
    config.decoding_method = Some("greedy_search".to_string());
    sherpa_onnx::OnlineRecognizer::create(&config)
        .ok_or_else(|| "Failed to load the Nemotron streaming model.".to_string())
}


pub fn release_model() {
    if let Ok(mut guard) = RECOGNIZER.lock() {
        *guard = None;
    }
}

pub fn prewarm(model_dir: &Path) -> Result<(), String> {
    if !model_files_present(model_dir) {
        return Err(format!(
            "Nemotron model not found in {}. Download it from the Engine tab.",
            model_dir.display()
        ));
    }
    let mut guard = RECOGNIZER
        .lock()
        .map_err(|_| "Nemotron model lock poisoned; restart Typr.".to_string())?;
    let needs_build = !matches!(&*guard, Some((dir, _)) if dir == model_dir);
    if needs_build {
        let started = std::time::Instant::now();
        println!(
            "[Typr] Prewarming Nemotron model {}...",
            model_dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        );
        *guard = Some((model_dir.to_path_buf(), build_recognizer(model_dir)?));
        println!("[Typr] Nemotron model ready in {:?}", started.elapsed());
    }
    Ok(())
}

pub struct NemotronLiveSession {
    pub model_dir: PathBuf,
    stop_signal: std::sync::Arc<std::sync::atomic::AtomicBool>,
    worker_handle: Option<std::thread::JoinHandle<(sherpa_onnx::OnlineStream, usize)>>,
    audio_recorder: std::sync::Arc<std::sync::Mutex<crate::audio::AudioRecorder>>,
    input_gain_db: f32,
}

impl NemotronLiveSession {
    pub fn finish(mut self) -> Result<String, String> {
        let started = std::time::Instant::now();
        self.stop_signal.store(true, std::sync::atomic::Ordering::Relaxed);
        let (stream, last_read) = if let Some(h) = self.worker_handle.take() {
            h.join().map_err(|_| "Nemotron stream worker panicked".to_string())?
        } else {
            return Err("Nemotron stream worker already finished".to_string());
        };

        // Drain any remaining unread samples from the live stream
        let (new_samples, _, src_rate, src_channels) = {
            let rec = self.audio_recorder.lock().unwrap();
            rec.get_raw_samples_from(last_read)
        };
        if !new_samples.is_empty() {
            let mut mono: Vec<f32> = if src_channels > 1 {
                new_samples
                    .chunks(src_channels as usize)
                    .map(|f| f.iter().sum::<f32>() / f.len() as f32)
                    .collect()
            } else {
                new_samples
            };
            if self.input_gain_db.abs() >= 0.01 {
                let factor = 10.0f32.powf(self.input_gain_db / 20.0);
                for s in mono.iter_mut() {
                    *s *= factor;
                }
            }
            let resampled = crate::audio::resample(&mono, src_rate, 16000);
            stream.accept_waveform(16000, &resampled);
        }

        let guard = RECOGNIZER
            .lock()
            .map_err(|_| "Nemotron model lock poisoned; restart Typr.".to_string())?;
        let recognizer = &guard
            .as_ref()
            .ok_or_else(|| "Nemotron recognizer missing".to_string())?
            .1;

        // Comfort tail silence padding (1.0s = 16000 samples @ 16kHz) to fully flush the 1120ms chunk
        let tail_silence = vec![0.0f32; 16000];
        stream.accept_waveform(16000, &tail_silence);
        while recognizer.is_ready(&stream) {
            recognizer.decode(&stream);
        }

        stream.input_finished();
        while recognizer.is_ready(&stream) {
            recognizer.decode(&stream);
        }

        let result = recognizer.get_result(&stream);
        let final_text = if let Some(ref r) = result {
            r.text.trim().to_string()
        } else {
            String::new()
        };

        println!(
            "[Typr] Nemotron live streaming completed tail decode in {:?}",
            started.elapsed()
        );
        Ok(final_text)
    }
}

impl Drop for NemotronLiveSession {
    fn drop(&mut self) {
        self.stop_signal.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(h) = self.worker_handle.take() {
            let _ = h.join();
        }
    }
}

pub fn start_live_session(
    model_dir: &Path,
    audio_recorder: std::sync::Arc<std::sync::Mutex<crate::audio::AudioRecorder>>,
    input_gain_db: f32,
) -> Result<NemotronLiveSession, String> {
    if !model_files_present(model_dir) {
        return Err(format!(
            "Nemotron model not found in {}. Download it from the Engine tab.",
            model_dir.display()
        ));
    }

    let mut guard = RECOGNIZER
        .lock()
        .map_err(|_| "Nemotron model lock poisoned; restart Typr.".to_string())?;
    let needs_build = !matches!(&*guard, Some((dir, _)) if dir == model_dir);
    if needs_build {
        *guard = Some((model_dir.to_path_buf(), build_recognizer(model_dir)?));
    }
    let recognizer = &guard.as_ref().expect("just built").1;

    let stream = recognizer.create_stream();
    let is_v3_5 = model_dir.to_string_lossy().contains("3.5")
        || !model_dir.to_string_lossy().contains("speech-streaming-en");
    if is_v3_5 {
        stream.set_option("language", "en");
    }

    // Drop guard before spawning worker so the worker can acquire RECOGNIZER lock when decoding
    drop(guard);

    let stop_signal = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_clone = stop_signal.clone();
    let audio_clone = audio_recorder.clone();

    let worker_handle = std::thread::Builder::new()
        .name("nemotron-live-stream".to_string())
        .spawn(move || {
            let mut last_read = 0usize;
            while !stop_clone.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(150));
                let (new_samples, new_end, src_rate, src_channels) = {
                    let rec = audio_clone.lock().unwrap();
                    rec.get_raw_samples_from(last_read)
                };
                if new_samples.is_empty() {
                    continue;
                }
                last_read = new_end;
                let mut mono: Vec<f32> = if src_channels > 1 {
                    new_samples
                        .chunks(src_channels as usize)
                        .map(|f| f.iter().sum::<f32>() / f.len() as f32)
                        .collect()
                } else {
                    new_samples
                };
                if input_gain_db.abs() >= 0.01 {
                    let factor = 10.0f32.powf(input_gain_db / 20.0);
                    for s in mono.iter_mut() {
                        *s *= factor;
                    }
                }
                let resampled = crate::audio::resample(&mono, src_rate, 16000);

                stream.accept_waveform(16000, &resampled);
                if let Ok(guard) = RECOGNIZER.lock() {
                    if let Some((_, ref rec)) = *guard {
                        while rec.is_ready(&stream) {
                            rec.decode(&stream);
                        }
                    }
                }
            }
            (stream, last_read)
        })
        .map_err(|e| format!("Failed to spawn Nemotron stream worker: {}", e))?;

    Ok(NemotronLiveSession {
        model_dir: model_dir.to_path_buf(),
        stop_signal,
        worker_handle: Some(worker_handle),
        audio_recorder,
        input_gain_db,
    })
}

#[cfg(test)]
fn compute_mel_stats(samples: &[f32], sample_rate: u32) -> (f32, f32, f32) {
    if samples.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let n_fft = 512;
    let hop_length = 160;
    let win_length = 400;
    let n_mels = 128;

    let window: Vec<f32> = (0..win_length)
        .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (win_length - 1) as f32).cos()))
        .collect();

    let hz_to_mel = |hz: f32| 2595.0 * (1.0 + hz / 700.0).log10();
    let mel_to_hz = |mel: f32| 700.0 * (10.0f32.powf(mel / 2595.0) - 1.0);
    let mel_min = hz_to_mel(0.0);
    let mel_max = hz_to_mel(8000.0);
    let mel_points: Vec<f32> = (0..=n_mels + 1)
        .map(|i| mel_to_hz(mel_min + (mel_max - mel_min) * (i as f32) / ((n_mels + 1) as f32)))
        .collect();
    let bin_points: Vec<usize> = mel_points
        .iter()
        .map(|&hz| (((n_fft + 1) as f32 * hz / sample_rate as f32).floor() as usize).min(n_fft / 2))
        .collect();

    let mut planner = rustfft::FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n_fft);

    let mut min_val = f32::MAX;
    let mut max_val = f32::MIN;
    let mut sum_val = 0.0f64;
    let mut count = 0usize;

    for frame_start in (0..samples.len()).step_by(hop_length) {
        if frame_start + win_length > samples.len() {
            break;
        }
        let mut buffer: Vec<rustfft::num_complex::Complex<f32>> = Vec::with_capacity(n_fft);
        for i in 0..n_fft {
            let val = if i < win_length {
                samples[frame_start + i] * window[i]
            } else {
                0.0
            };
            buffer.push(rustfft::num_complex::Complex::new(val, 0.0));
        }
        fft.process(&mut buffer);

        let num_bins = n_fft / 2 + 1;
        let power_spec: Vec<f32> = buffer[..num_bins]
            .iter()
            .map(|c| c.norm_sqr())
            .collect();

        for m in 0..n_mels {
            let start = bin_points[m];
            let center = bin_points[m + 1];
            let end = bin_points[m + 2];
            let mut mel_energy = 0.0f32;

            for k in start..center {
                if center > start {
                    let weight = (k - start) as f32 / (center - start) as f32;
                    mel_energy += weight * power_spec.get(k).copied().unwrap_or(0.0);
                }
            }
            for k in center..end {
                if end > center {
                    let weight = (end - k) as f32 / (end - center) as f32;
                    mel_energy += weight * power_spec.get(k).copied().unwrap_or(0.0);
                }
            }
            let log_mel = (mel_energy + 1e-10).ln();
            min_val = min_val.min(log_mel);
            max_val = max_val.max(log_mel);
            sum_val += log_mel as f64;
            count += 1;
        }
    }

    if count == 0 {
        (0.0, 0.0, 0.0)
    } else {
        (min_val, max_val, (sum_val / count as f64) as f32)
    }
}

pub async fn transcribe_nemotron(
    model_dir: &PathBuf,
    audio_path: &PathBuf,
) -> Result<String, String> {
    if !model_files_present(model_dir) {
        return Err(format!(
            "Nemotron model not found in {}. Download it from the Engine tab.",
            model_dir.display()
        ));
    }

    let model_dir = model_dir.clone();
    let audio_path = audio_path.clone();

    tokio::task::spawn_blocking(move || {
        let mut reader = hound::WavReader::open(&audio_path)
            .map_err(|e| format!("Failed to read audio file: {}", e))?;
        let sample_rate = reader.spec().sample_rate;
        let num_samples = reader.len() as usize;
        let mut samples = Vec::with_capacity(num_samples);
        for s in reader.samples::<i16>() {
            let v = s.map_err(|e| format!("Failed to decode audio sample: {}", e))?;
            samples.push(v as f32 / 32768.0);
        }

        let mut guard = RECOGNIZER
            .lock()
            .map_err(|_| "Nemotron model lock poisoned; restart Typr.".to_string())?;
        let needs_build = !matches!(&*guard, Some((dir, _)) if dir == &model_dir);
        if needs_build {
            *guard = Some((model_dir.clone(), build_recognizer(&model_dir)?));
        }
        let recognizer = &guard.as_ref().expect("just built").1;

        let stream = recognizer.create_stream();

        // Condition 3.5 multilingual on English prompt (prompt index 0 = en-US).
        // The English-only v3 model needs no prompt.
        let is_v3_5 = model_dir.to_string_lossy().contains("3.5")
            || !model_dir.to_string_lossy().contains("speech-streaming-en");
        if is_v3_5 {
            stream.set_option("language", "en");
        }

        println!(
            "[Typr] Nemotron transcribing {:.1}s in streaming mode, model {}",
            samples.len() as f32 / sample_rate as f32,
            model_dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        );
        let started = std::time::Instant::now();

        stream.accept_waveform(sample_rate as i32, &samples);
        while recognizer.is_ready(&stream) {
            recognizer.decode(&stream);
        }

        // Comfort tail silence padding (1.0s = 16000 samples @ 16kHz) to fully flush FastConformer chunks
        let tail_silence = vec![0.0f32; 16000];
        stream.accept_waveform(16000, &tail_silence);
        while recognizer.is_ready(&stream) {
            recognizer.decode(&stream);
        }

        // Flush tail tokens with final frames
        stream.input_finished();
        while recognizer.is_ready(&stream) {
            recognizer.decode(&stream);
        }

        let result = recognizer.get_result(&stream);
        let final_text = if let Some(ref r) = result {
            r.text.trim().to_string()
        } else {
            String::new()
        };

        println!("[Typr] Nemotron completed in {:?}", started.elapsed());
        Ok(final_text)
    })
    .await
    .map_err(|e| format!("Nemotron task panicked: {}", e))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_chunker;

    static TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[test]
    fn test_nemotron_model_dir_name_maps_variants() {
        assert_eq!(model_dir_name("v3_5"), "nemotron-3.5-asr-streaming-0.6b-int8");
        assert_eq!(model_dir_name("v3.5"), "nemotron-3.5-asr-streaming-0.6b-int8");
        assert_eq!(model_dir_name("v3"), "nemotron-speech-streaming-en-0.6b-int8");
        assert_eq!(model_dir_name(""), "nemotron-3.5-asr-streaming-0.6b-int8");
        assert_eq!(model_dir_name("bogus"), "nemotron-3.5-asr-streaming-0.6b-int8");
    }

    #[test]
    fn test_nemotron_model_download_url_per_variant() {
        assert!(model_download_url("v3_5").contains("nemotron-3.5-asr"));
        assert!(model_download_url("v3").contains("nemotron-speech-streaming-en"));
        assert!(model_download_url("bogus").contains("nemotron-3.5-asr"));
    }

    #[test]
    fn test_nemotron_model_files_present_false_for_missing_dir() {
        assert!(!model_files_present(Path::new("does-not-exist-anywhere")));
    }

    #[tokio::test]
    async fn test_nemotron_missing_model_errors_clearly() {
        let r = transcribe_nemotron(
            &PathBuf::from("does-not-exist-anywhere"),
            &PathBuf::from("nope.wav"),
        )
        .await;
        assert!(r.is_err());
        let e = r.unwrap_err();
        assert!(e.contains("Nemotron model not found"));
    }

    #[test]
    fn test_nemotron_seam_merge_retains_all_words() {
        let parts = vec![
            ("Hello world item one and item two".to_string(), false),
            ("item two and item three".to_string(), true),
        ];
        let merged = audio_chunker::merge_chunk_texts(&parts);
        assert_eq!(merged, "Hello world item one and item two and item three");
    }

    #[test]
    fn test_nemotron_mel_stats_diff() {
        let clip_path = Path::new("../scripts/test_clips/clip1_5s.wav");
        if !clip_path.exists() {
            return;
        }
        let mut reader = hound::WavReader::open(clip_path).unwrap();
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect();
        let (min, max, mean) = compute_mel_stats(&samples, 16000);
        println!("Rust computed mel stats: min={}, max={}, mean={}", min, max, mean);
        // Compare with Python reference: min=-23.0259, max=6.4420, mean=-9.0704
        assert!((min - (-23.0259)).abs() < 1e-3, "min diff too large: {}", min);
        assert!((max - 6.4420).abs() < 1e-3, "max diff too large: {}", max);
        assert!((mean - (-9.0704)).abs() < 1e-3, "mean diff too large: {}", mean);
    }

    fn word_accuracy(hyp: &str, ref_text: &str) -> f32 {
        let clean = |s: &str| -> Vec<String> {
            s.to_lowercase()
                .replace(|c: char| !c.is_alphanumeric() && !c.is_whitespace(), "")
                .split_whitespace()
                .map(|w| w.to_string())
                .collect()
        };
        let hyp_words = clean(hyp);
        let ref_words = clean(ref_text);
        if ref_words.is_empty() {
            return if hyp_words.is_empty() { 1.0 } else { 0.0 };
        }
        let m = ref_words.len();
        let n = hyp_words.len();
        let mut dp = vec![vec![0usize; n + 1]; m + 1];
        for i in 0..=m { dp[i][0] = i; }
        for j in 0..=n { dp[0][j] = j; }
        for i in 1..=m {
            for j in 1..=n {
                let cost = if ref_words[i - 1] == hyp_words[j - 1] { 0 } else { 1 };
                dp[i][j] = (dp[i - 1][j] + 1)
                    .min(dp[i][j - 1] + 1)
                    .min(dp[i - 1][j - 1] + cost);
            }
        }
        let dist = dp[m][n];
        let wer = dist as f32 / m as f32;
        (1.0 - wer).max(0.0)
    }

    #[tokio::test]
    async fn test_nemotron_golden_accuracy_3_clips() {
        let _lock = TEST_MUTEX.lock().await;
        let model_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("com.typr.app")
            .join("nemotron-3.5-asr-streaming-0.6b-int8");
        if !model_files_present(&model_dir) {
            eprintln!("Nemotron model not downloaded in test env; skipping golden accuracy test");
            return;
        }

        let clips = [
            (
                "../scripts/test_clips/clip1_5s.wav",
                "Let me know whether there are some changes that you would like to make."
            ),
            (
                "../scripts/test_clips/clip2_30s.wav",
                "We are conducting a comprehensive evaluation of the speech recognition engine to determine transcription accuracy across different speech models and acoustic conditions. The quick brown fox jumps over the lazy dog. Please confirm that all parameters are functioning properly and that no words are being dropped at chunk seams. Local inference requires consistent acoustic processing, zero padded frames, and robust decoding algorithms."
            ),
            (
                "../scripts/test_clips/clip3_75s.wav",
                "First item check the audio pipeline and ensure sixteen kHz sample rate with mono float values. Second item verify that the encoder cache states are properly preserved and carried between consecutive chunks. Third item make sure language ID prompt tokens are properly provided for all multilingual speech models. Fourth item check the input dynamic range and avoid overly aggressive soft knee limiting or audio clipping. Fifth item ensure windowing and fast Fourier transform parameters match the model preprocessor configuration exactly. Sixth item verify that the token vocabulary correctly handles subwords, word pieces, and special language tags. Seventh item validate that the hallucination guard rejects diverged outputs and restores deterministic transcripts. Eighth item test long audio recordings to ensure that tail truncation never silently drops the final clauses. Ninth item review the accuracy and word error rate across all local and cloud speech engines before deployment. Tenth item confirm that streaming inference maintains low latency and stable memory usage throughout the dictation. Let me know whether there are some changes that you would like to make."
            ),
        ];

        for (path_str, expected_ref) in clips {
            let wav_path = PathBuf::from(path_str);
            if !wav_path.exists() {
                continue;
            }
            let res = transcribe_nemotron(&model_dir, &wav_path).await;
            assert!(res.is_ok(), "Transcription failed: {:?}", res);
            let hyp = res.unwrap();
            let acc = word_accuracy(&hyp, expected_ref);
            println!("Clip: {} -> Word Accuracy: {:.1}% | Transcript: {}", path_str, acc * 100.0, hyp);
            assert!(acc >= 0.95, "Accuracy {:.2} below 95% threshold for {}", acc, path_str);
        }
    }

    #[tokio::test]
    async fn test_nemotron_streaming_chunks() {
        let model_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("com.typr.app")
            .join("nemotron-3.5-asr-streaming-0.6b-int8");
        if !model_files_present(&model_dir) {
            return;
        }

        let wav_path = PathBuf::from("../scripts/test_clips/clip2_30s.wav");
        if !wav_path.exists() {
            return;
        }

        let mut reader = hound::WavReader::open(&wav_path).unwrap();
        let sample_rate = reader.spec().sample_rate;
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect();

        let recognizer = build_recognizer(&model_dir).unwrap();
        let stream = recognizer.create_stream();
        stream.set_option("language", "en");

        // Feed in 0.5s chunks (8000 samples @ 16kHz) as if streaming in real-time
        let started = std::time::Instant::now();
        for chunk in samples.chunks(8000) {
            stream.accept_waveform(sample_rate as i32, chunk);
            while recognizer.is_ready(&stream) {
                recognizer.decode(&stream);
            }
        }
        stream.input_finished();
        while recognizer.is_ready(&stream) {
            recognizer.decode(&stream);
        }
        let elapsed = started.elapsed();

        let result = recognizer.get_result(&stream).map(|r| r.text.trim().to_string()).unwrap_or_default();
        let expected = "We are conducting a comprehensive evaluation of the speech recognition engine to determine transcription accuracy across different speech models and acoustic conditions. The quick brown fox jumps over the lazy dog. Please confirm that all parameters are functioning properly and that no words are being dropped at chunk seams. Local inference requires consistent acoustic processing, zero padded frames, and robust decoding algorithms.";
        let acc = word_accuracy(&result, expected);
        println!("Streaming chunk decode: acc={:.1}% in {:?} -> {}", acc * 100.0, elapsed, result);
        assert!(acc >= 0.95);
    }

    #[test]
    fn test_stream_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<sherpa_onnx::OnlineStream>();
    }

    #[tokio::test]
    async fn test_nemotron_live_session_lifecycle() {
        let _lock = TEST_MUTEX.lock().await;
        let model_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("com.typr.app")
            .join("nemotron-3.5-asr-streaming-0.6b-int8");
        if !model_files_present(&model_dir) {
            return;
        }

        let wav_path = PathBuf::from("../scripts/test_clips/clip1_5s.wav");
        if !wav_path.exists() {
            return;
        }

        let mut reader = hound::WavReader::open(&wav_path).unwrap();
        let sample_rate = reader.spec().sample_rate;
        let channels = reader.spec().channels;
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect();

        let mut rec = crate::audio::AudioRecorder::new();
        rec.set_source_format_for_test(sample_rate, channels);
        let recorder = std::sync::Arc::new(std::sync::Mutex::new(rec));

        let session = start_live_session(&model_dir, recorder.clone(), 0.0)
            .expect("start_live_session should succeed");

        // Feed audio in chunks of 2400 samples (150ms @ 16kHz)
        for chunk in samples.chunks(2400) {
            recorder.lock().unwrap().push_raw_samples_for_test(chunk);
            std::thread::sleep(std::time::Duration::from_millis(30));
        }

        // Give the background worker thread a moment to ingest
        std::thread::sleep(std::time::Duration::from_millis(200));

        let finish_start = std::time::Instant::now();
        let text = session.finish().expect("finish should succeed");
        let finish_elapsed = finish_start.elapsed();

        let expected = "Let me know whether there are some changes that you would like to make.";
        let acc = word_accuracy(&text, expected);
        println!(
            "Live session lifecycle: acc={:.1}%, finish_latency={:?} -> '{}'",
            acc * 100.0,
            finish_elapsed,
            text
        );
        assert!(acc >= 0.90, "Live session accuracy too low: {:.2}", acc);
        assert!(
            finish_elapsed < std::time::Duration::from_millis(1500),
            "Finish tail latency took too long: {:?}",
            finish_elapsed
        );
    }
}
