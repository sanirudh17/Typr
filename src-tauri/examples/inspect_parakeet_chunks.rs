//! Inspect local Parakeet chunk text and alignment. No app writes or uploads.
use typr_lib::{audio, audio_chunker, transcribe_parakeet};
fn main() -> Result<(), String> {
    let root = dirs::config_dir().ok_or("No config directory")?.join("com.typr.app")
        .join(transcribe_parakeet::model_dir_name("v3"));
    let mut config = sherpa_onnx::OfflineRecognizerConfig::default();
    config.model_config.transducer.encoder = Some(root.join("encoder.int8.onnx").to_string_lossy().into_owned());
    config.model_config.transducer.decoder = Some(root.join("decoder.int8.onnx").to_string_lossy().into_owned());
    config.model_config.transducer.joiner = Some(root.join("joiner.int8.onnx").to_string_lossy().into_owned());
    config.model_config.tokens = Some(root.join("tokens.txt").to_string_lossy().into_owned());
    config.model_config.model_type = Some("nemo_transducer".into());
    config.model_config.num_threads = 6;
    config.decoding_method = Some("greedy_search".into());
    let recognizer = sherpa_onnx::OfflineRecognizer::create(&config).ok_or("Cannot load Parakeet")?;
    for path in std::env::args_os().skip(1).map(std::path::PathBuf::from) {
        let (mut samples, rate) = audio_chunker::read_wav_samples(&path)?;
        audio::process_speech_audio_chain(&mut samples, rate, audio::FIXED_INPUT_GAIN_DB);
        audio::normalize_target_rms(&mut samples, rate, 0.20, 2.5);
        audio::apply_tanh_limiter(&mut samples, 0.75, 0.95);
        let mut parts = Vec::new();
        for (index, chunk) in audio_chunker::split_into_chunks(&samples, rate).iter().enumerate() {
            let stream = recognizer.create_stream();
            stream.accept_waveform(rate as i32, chunk.samples);
            recognizer.decode(&stream);
            let result = stream.get_result().ok_or("Missing result")?;
            println!("CHUNK {}", serde_json::json!({"file":path.file_name().unwrap_or_default().to_string_lossy(),
                "index":index,"overlap":chunk.overlaps_previous,"seconds":chunk.samples.len() as f32 / rate as f32,
                "text":result.text,"tokens":result.tokens,"timestamps":result.timestamps,"durations":result.durations}));
            parts.push((result.text, chunk.overlaps_previous));
        }
        println!("MERGED {}", audio_chunker::merge_chunk_texts(&parts));
    }
    Ok(())
}
