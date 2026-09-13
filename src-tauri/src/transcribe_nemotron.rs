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

static RECOGNIZER: Mutex<Option<(PathBuf, sherpa_onnx::OfflineRecognizer)>> = Mutex::new(None);

fn build_recognizer(model_dir: &Path) -> Result<sherpa_onnx::OfflineRecognizer, String> {
    let mut config = sherpa_onnx::OfflineRecognizerConfig::default();
    config.model_config.transducer.encoder =
        Some(model_dir.join("encoder.int8.onnx").to_string_lossy().into_owned());
    config.model_config.transducer.decoder =
        Some(model_dir.join("decoder.int8.onnx").to_string_lossy().into_owned());
    config.model_config.transducer.joiner =
        Some(model_dir.join("joiner.int8.onnx").to_string_lossy().into_owned());
    config.model_config.tokens =
        Some(model_dir.join("tokens.txt").to_string_lossy().into_owned());
    config.model_config.num_threads = 2;
    config.decoding_method = Some("modified_beam_search".to_string());
    config.max_active_paths = 8;
    sherpa_onnx::OfflineRecognizer::create(&config)
        .ok_or_else(|| "Failed to load the Nemotron model.".to_string())
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
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.map(|v| v as f32 / 32768.0))
            .collect::<Result<_, _>>()
            .map_err(|e| format!("Failed to decode audio samples: {}", e))?;

        let mut guard = RECOGNIZER
            .lock()
            .map_err(|_| "Nemotron model lock poisoned; restart Typr.".to_string())?;
        let needs_build = !matches!(&*guard, Some((dir, _)) if dir == &model_dir);
        if needs_build {
            *guard = Some((model_dir.clone(), build_recognizer(&model_dir)?));
        }
        let recognizer = &guard.as_ref().expect("just built").1;

        let chunks = audio_chunker::split_into_chunks(&samples, sample_rate);
        println!(
            "[Typr] Nemotron transcribing {:.1}s in {} chunk(s), model {}",
            samples.len() as f32 / sample_rate as f32,
            chunks.len(),
            model_dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        );
        let started = std::time::Instant::now();
        let mut parts: Vec<(String, bool)> = Vec::new();
        for chunk in &chunks {
            let stream = recognizer.create_stream();
            stream.accept_waveform(sample_rate as i32, chunk.samples);
            recognizer.decode(&stream);
            let Some(result) = stream.get_result() else { continue };

            let text = result.text.trim().to_string();
            if !text.is_empty() {
                parts.push((text, chunk.overlaps_previous));
            }
        }
        let merged = audio_chunker::merge_chunk_texts(&parts);
        println!("[Typr] Nemotron completed in {:?}", started.elapsed());
        Ok(merged)
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
}
