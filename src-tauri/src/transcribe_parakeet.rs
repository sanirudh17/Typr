//! Local Parakeet transcription on the CPU.
//!
//! Unlike the Whisper path this runs IN-PROCESS: no sidecar binary, no HTTP server, no CUDA.
//! That is the point of this engine — it works on machines with no NVIDIA GPU, at the cost of
//! being slower than Whisper on machines that have one.
//!
//! Two behaviours here were forced by measurement rather than chosen:
//!
//! - **The recognizer is cached.** Loading the model takes 4-6 seconds and is not disk-cache
//!   (identical on repeat runs), so building one per dictation would add that to every single
//!   transcription. It is built once and reused until the model variant changes.
//! - **Long audio is chunked.** A single decode of an 82-second recording silently dropped a
//!   whole clause while the same audio in 26-second pieces transcribed it correctly. See
//!   `audio_chunker`.

use crate::audio_chunker;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Directory name for a model variant, under the app data dir.
///
/// v3 is multilingual (25 European languages, auto-detected); v2 is English-only and faster.
/// Anything unrecognized falls back to v3 so a stale setting cannot point at a directory that
/// will never exist.
pub fn model_dir_name(variant: &str) -> &'static str {
    match variant {
        "v2" => "parakeet-tdt-0.6b-v2-int8",
        _ => "parakeet-tdt-0.6b-v3-int8",
    }
}

/// Release archive for a variant, from the sherpa-onnx model zoo.
pub fn model_download_url(variant: &str) -> String {
    format!(
        "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemo-{}.tar.bz2",
        model_dir_name(variant)
    )
}

/// The four files a sherpa-onnx offline transducer needs. Checked up front so a missing
/// download surfaces as a clear error instead of a failure deep inside the C API.
pub fn model_files_present(model_dir: &Path) -> bool {
    ["encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx", "tokens.txt"]
        .iter()
        .all(|f| model_dir.join(f).is_file())
}

/// The loaded model, kept alive between dictations.
///
/// Keyed by directory so switching variants rebuilds rather than silently transcribing with
/// the previous model. `sherpa_onnx::OfflineRecognizer` owns C resources and is not `Sync`,
/// so access is serialized — transcriptions are sequential anyway, one per dictation.
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
    // Two threads, matching the reference implementations. More does not help a 0.6B model on
    // a typical laptop and competes with whatever the user is actually doing.
    config.model_config.num_threads = 2;
    // Beam search instead of the library-default greedy decode. Greedy commits to the single
    // highest-probability token at every step; beam search keeps several hypotheses and picks
    // the best-scoring whole sequence, which is the correct decode for a transducer.
    //
    // Measured on the three real 62s dictations (v3-int8, chunking held constant): greedy
    // scored 41/48, beam width 8 scored 42/48 and was never worse than greedy on any single
    // sample, at no measurable time cost (~10.1s vs ~9.9s, inside run-to-run variance). Width 4
    // and a blank penalty both scored *below* greedy, so 8 is the width, not 4. The gain is
    // small — the stubborn errors ("multi-level"->"multiple", "dining"->"dynamic") are int8
    // acoustic mishearings that no decoder setting recovers — but it is real and free.
    config.decoding_method = Some("modified_beam_search".to_string());
    config.max_active_paths = 8;
    sherpa_onnx::OfflineRecognizer::create(&config)
        .ok_or_else(|| "Failed to load the Parakeet model.".to_string())
}

/// Drop the cached model, freeing ~640 MB. Called when switching away from this engine.
pub fn release_model() {
    if let Ok(mut guard) = RECOGNIZER.lock() {
        *guard = None;
    }
}

/// Transcribe a 16 kHz mono WAV. `audio.rs` already produces exactly that format.
pub async fn transcribe_parakeet(
    model_dir: &PathBuf,
    audio_path: &PathBuf,
) -> Result<String, String> {
    if !model_files_present(model_dir) {
        return Err(format!(
            "Parakeet model not found in {}. Download it from the Engine tab.",
            model_dir.display()
        ));
    }

    let model_dir = model_dir.clone();
    let audio_path = audio_path.clone();

    // sherpa-onnx is synchronous and CPU-bound; keeping it off the async workers stops a long
    // dictation from stalling the runtime.
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
            .map_err(|_| "Parakeet model lock poisoned; restart Typr.".to_string())?;
        let needs_build = !matches!(&*guard, Some((dir, _)) if dir == &model_dir);
        if needs_build {
            *guard = Some((model_dir.clone(), build_recognizer(&model_dir)?));
        }
        let recognizer = &guard.as_ref().expect("just built").1;

        let chunks = audio_chunker::split_into_chunks(&samples, sample_rate);
        // Logged because the Whisper path announces itself loudly and this one did not, which
        // made a run on the wrong engine impossible to spot in the console.
        println!(
            "[Typr] Parakeet transcribing {:.1}s in {} chunk(s), model {}",
            samples.len() as f32 / sample_rate as f32,
            chunks.len(),
            model_dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        );
        let started = std::time::Instant::now();
        let mut pieces: Vec<String> = Vec::new();
        for chunk in &chunks {
            let stream = recognizer.create_stream();
            stream.accept_waveform(sample_rate as i32, chunk.samples);
            recognizer.decode(&stream);
            let Some(result) = stream.get_result() else { continue };

            let overlap = if chunk.overlaps_previous { audio_chunker::OVERLAP_SECS } else { 0.0 };
            let text = audio_chunker::trim_overlap_tokens(
                &result.tokens,
                result.timestamps.as_deref(),
                &result.text,
                overlap,
            );
            if !text.is_empty() {
                pieces.push(text);
            }
        }
        println!("[Typr] Parakeet completed in {:?}", started.elapsed());
        Ok(pieces.join(" "))
    })
    .await
    .map_err(|e| format!("Parakeet task panicked: {}", e))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_dir_name_maps_variants() {
        assert_eq!(model_dir_name("v3"), "parakeet-tdt-0.6b-v3-int8");
        assert_eq!(model_dir_name("v2"), "parakeet-tdt-0.6b-v2-int8");
        // Unknown or empty falls back to the default variant, so a stale setting can never
        // point at a directory that will never exist.
        assert_eq!(model_dir_name(""), "parakeet-tdt-0.6b-v3-int8");
        assert_eq!(model_dir_name("bogus"), "parakeet-tdt-0.6b-v3-int8");
    }

    #[test]
    fn test_model_download_url_per_variant() {
        assert!(model_download_url("v3").ends_with("parakeet-tdt-0.6b-v3-int8.tar.bz2"));
        assert!(model_download_url("v2").ends_with("parakeet-tdt-0.6b-v2-int8.tar.bz2"));
        assert!(model_download_url("v3").starts_with("https://"));
        assert!(model_download_url("bogus").ends_with("parakeet-tdt-0.6b-v3-int8.tar.bz2"));
    }

    #[test]
    fn test_model_files_present_false_for_missing_dir() {
        assert!(!model_files_present(Path::new("does-not-exist-anywhere")));
    }

    #[tokio::test]
    async fn test_missing_model_errors_clearly() {
        let r = transcribe_parakeet(
            &PathBuf::from("does-not-exist-anywhere"),
            &PathBuf::from("nope.wav"),
        )
        .await;
        assert!(r.is_err());
        let e = r.unwrap_err();
        assert!(e.contains("model"), "error should name the model problem, got: {}", e);
    }
}
