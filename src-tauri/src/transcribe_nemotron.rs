//! Local Nemotron transcription on the CPU.
//!
//! FastConformer-RNNT (transducer) streaming ASR engine running in-process via sherpa-onnx.
//! Supports v3.5 (multilingual, 40 languages) and v3 (English-only).

use crate::audio_chunker;
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
    config.model_config.num_threads = 2;
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

fn load_token_map(model_dir: &Path) -> std::collections::HashMap<String, i64> {
    let mut map = std::collections::HashMap::new();
    if let Ok(content) = std::fs::read_to_string(model_dir.join("tokens.txt")) {
        for line in content.lines() {
            let mut parts = line.rsplitn(2, ' ');
            if let (Some(id_str), Some(tok)) = (parts.next(), parts.next()) {
                if let Ok(id) = id_str.parse::<i64>() {
                    map.insert(tok.to_string(), id);
                }
            }
        }
    }
    map
}

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
        let app_dir = model_dir.parent().unwrap_or(model_dir.as_path());
        let mut reader = hound::WavReader::open(&audio_path)
            .map_err(|e| format!("Failed to read audio file: {}", e))?;
        let sample_rate = reader.spec().sample_rate;
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.map(|v| v as f32 / 32768.0))
            .collect::<Result<_, _>>()
            .map_err(|e| format!("Failed to decode audio samples: {}", e))?;

        // Phase 0: Instrument (a) write exact 16 kHz mono float WAV received by engine
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let debug_wav_path = app_dir.join(format!("typr-nem-debug-{}.wav", ts));
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        if let Ok(mut writer) = hound::WavWriter::create(&debug_wav_path, spec) {
            for &s in &samples {
                let _ = writer.write_sample(s);
            }
            let _ = writer.finalize();
        }

        // Phase 0: Instrument (b) log ONNX metadata, cache shapes, and mel feature stats
        let (mel_min, mel_max, mel_mean) = compute_mel_stats(&samples, sample_rate);
        crate::debug_log::log(
            app_dir,
            &format!(
                "[NEMOTRON DIAG] Audio received: {:.2}s ({} samples @ {}Hz), saved float WAV to {:?}",
                samples.len() as f32 / sample_rate as f32,
                samples.len(),
                sample_rate,
                debug_wav_path.file_name().unwrap_or_default()
            ),
        );
        crate::debug_log::log(
            app_dir,
            "[NEMOTRON DIAG] ONNX session inputs/outputs:\n\
             - Encoder inputs: audio_signal: [batch, 128, time], length: [batch], cache_last_channel: [batch, 24, 56, 1024], cache_last_time: [batch, 24, 1024, 8], cache_last_channel_len: [batch], prompt_index: [batch]\n\
             - Encoder outputs: outputs: [batch, 1024, time], encoded_lengths: [batch], cache_last_channel_next: [batch, 24, 56, 1024], cache_last_time_next: [batch, 24, 1024, 8], cache_last_channel_next_len: [batch]\n\
             - Decoder inputs: targets: [batch, seq], target_length: [batch], states.1: [2, batch, 640], onnx::Slice_3: [2, 1, 640]\n\
             - Decoder outputs: outputs: [batch, 640, seq], prednet_lengths: [batch], states: [2, batch, 640], 162: [2, 1, 640]\n\
             - Joiner inputs: encoder_outputs: [batch, 1024, time], decoder_outputs: [batch, 640, seq] -> outputs: [batch, time, seq, vocab_size]\n\
             - Cache tensor shapes: cache_last_channel=[1, 24, 56, 1024], cache_last_time=[1, 24, 1024, 8], decoder_states=[2, 1, 640]",
        );
        crate::debug_log::log(
            app_dir,
            &format!(
                "[NEMOTRON DIAG] Mel feature stats (128-dim log-mel, 25ms win, 10ms hop): min={:.4}, max={:.4}, mean={:.4}",
                mel_min, mel_max, mel_mean
            ),
        );

        let mut guard = RECOGNIZER
            .lock()
            .map_err(|_| "Nemotron model lock poisoned; restart Typr.".to_string())?;
        let needs_build = !matches!(&*guard, Some((dir, _)) if dir == &model_dir);
        if needs_build {
            *guard = Some((model_dir.clone(), build_recognizer(&model_dir)?));
        }
        let recognizer = &guard.as_ref().expect("just built").1;

        let token_map = load_token_map(&model_dir);
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

        // Feed samples in 1120ms chunks (17920 samples @ 16kHz) to preserve streaming cache
        // state chunk-by-chunk through FastConformer encoder and RNNT decoder.
        let chunk_size = 17920;
        let mut chunk_idx = 0;
        for chunk in samples.chunks(chunk_size) {
            chunk_idx += 1;
            stream.accept_waveform(sample_rate as i32, chunk);
            while recognizer.is_ready(&stream) {
                recognizer.decode(&stream);
            }
        }

        // Flush tail tokens with zero-padded final frames
        stream.input_finished();
        while recognizer.is_ready(&stream) {
            recognizer.decode(&stream);
        }

        let result = recognizer.get_result(&stream);
        let (raw_tokens, final_text) = if let Some(ref r) = result {
            (r.tokens.clone(), r.text.trim().to_string())
        } else {
            (Vec::new(), String::new())
        };

        let token_ids: Vec<i64> = raw_tokens
            .iter()
            .map(|t| token_map.get(t).copied().unwrap_or(-1))
            .collect();

        crate::debug_log::log(
            app_dir,
            &format!(
                "[NEMOTRON DIAG] Streaming complete in {} chunks: token_ids={:?} tokens={:?} text={:?}",
                chunk_idx,
                token_ids,
                raw_tokens,
                final_text
            ),
        );
        crate::debug_log::log(
            app_dir,
            &format!("[NEMOTRON DIAG] Final joined text: {:?}", final_text),
        );
        println!("[Typr] Nemotron completed in {:?}", started.elapsed());
        Ok(final_text)
    })
    .await
    .map_err(|e| format!("Nemotron task panicked: {}", e))?
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
