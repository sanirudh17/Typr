//! Native decoder A/B on prepared WAVs. No microphone, uploads, or app writes.
use std::path::PathBuf;
use typr_lib::{audio_chunker, transcribe_parakeet};

fn main() -> Result<(), String> {
    let clips: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if clips.is_empty() { return Err("Expected prepared WAV paths".into()); }
    let folder = dirs::config_dir().ok_or("No app directory")?.join("com.typr.app")
        .join(transcribe_parakeet::model_dir_name("v3"));
    for decoder in ["modified_beam_search", "greedy_search"] {
        let mut config = sherpa_onnx::OfflineRecognizerConfig::default();
        config.model_config.transducer.encoder = Some(folder.join("encoder.int8.onnx").to_string_lossy().into_owned());
        config.model_config.transducer.decoder = Some(folder.join("decoder.int8.onnx").to_string_lossy().into_owned());
        config.model_config.transducer.joiner = Some(folder.join("joiner.int8.onnx").to_string_lossy().into_owned());
        config.model_config.tokens = Some(folder.join("tokens.txt").to_string_lossy().into_owned());
        config.model_config.num_threads = 6;
        config.model_config.model_type = Some("nemo_transducer".into());
        config.decoding_method = Some(decoder.into());
        config.max_active_paths = 8;
        let recognizer = sherpa_onnx::OfflineRecognizer::create(&config).ok_or("Could not load Parakeet")?;
        for path in &clips {
            let (samples, rate) = audio_chunker::read_wav_samples(path)?;
            for scale in [1.0f32, 2.5] {
                let scaled: Vec<f32> = samples.iter().map(|s| (s * scale).clamp(-1.0, 1.0)).collect();
                let started = std::time::Instant::now();
                let mut parts = Vec::new();
                for chunk in audio_chunker::split_into_chunks(&scaled, rate) {
                    let stream = recognizer.create_stream();
                    stream.accept_waveform(rate as i32, chunk.samples);
                    recognizer.decode(&stream);
                    let text = stream.get_result().ok_or("No decoder result")?.text;
                    parts.push((text, chunk.overlaps_previous));
                }
                println!("LOCAL COMPARE {}", serde_json::json!({
                    "file": path.file_name().unwrap_or_default().to_string_lossy(), "decoder": decoder,
                    "scale": scale, "decode_ms": started.elapsed().as_millis(),
                    "text": audio_chunker::merge_chunk_texts(&parts)}));
            }
        }
    }
    Ok(())
}
