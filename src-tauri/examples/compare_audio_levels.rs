//! Local Nemotron audio-level A/B. Temporary files only; no uploads or app writes.
use std::path::PathBuf;
use typr_lib::{audio_chunker, transcribe_nemotron};
struct Temp(PathBuf);
impl Drop for Temp { fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); } }

#[tokio::main]
async fn main() -> Result<(), String> {
    let clips: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if clips.is_empty() { return Err("Expected prepared WAV paths".into()); }
    let folder = dirs::config_dir().ok_or("No app directory")?.join("com.typr.app")
        .join(transcribe_nemotron::model_dir_name("v3.5"));
    transcribe_nemotron::prewarm(&folder)?;
    for path in clips {
        let (samples, rate) = audio_chunker::read_wav_samples(&path)?;
        for scale in [1.0f32, 2.5] {
            let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_nanos();
            let temp = Temp(std::env::temp_dir().join(format!("typr-audio-level-{}-{nonce}.wav", std::process::id())));
            let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp.0).map_err(|e| e.to_string())?;
            let scaled: Vec<f32> = samples.iter().map(|s| (s * scale).clamp(-1.0, 1.0)).collect();
            std::io::Write::write_all(&mut std::io::BufWriter::new(file), &audio_chunker::samples_to_wav_bytes(&scaled, rate)?)
                .map_err(|e| e.to_string())?;
            let started = std::time::Instant::now();
            let text = transcribe_nemotron::transcribe_nemotron(&folder, &temp.0).await?;
            println!("AUDIO LEVEL {}", serde_json::json!({"file": path.file_name().unwrap_or_default().to_string_lossy(),
                "scale": scale, "decode_ms": started.elapsed().as_millis(), "text": text}));
        }
    }
    Ok(())
}
