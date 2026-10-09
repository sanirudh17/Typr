//! Explicit cloud comparison of prepared review WAVs through the production API.
//! Uploads only the WAV paths passed on the command line to the configured Groq service.
//! Never records, pastes, changes settings, or writes transcription history.
use std::path::PathBuf;
use typr_lib::{cleanup, dictionary::Dictionary, transcribe_groq, vocab_correct};

#[tokio::main]
async fn main() -> Result<(), String> {
    let clips: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if clips.is_empty() { return Err("Usage: cargo run --example replay_cloud -- <prepared.wav> [...] (uploads audio to Groq)".into()); }
    // Validate every file before uploading any of them.
    for path in &clips {
        let reader = hound::WavReader::open(path).map_err(|e| e.to_string())?;
        let spec = reader.spec();
        if spec.channels != 1 || spec.sample_rate != 16_000 || spec.bits_per_sample != 16
            || spec.sample_format != hound::SampleFormat::Int {
            return Err("Expected prepared mono s16/16 kHz review WAVs".into());
        }
    }
    let app_dir = dirs::config_dir().ok_or("No app configuration directory")?.join("com.typr.app");
    let config: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(app_dir.join("config.json"))
        .map_err(|_| "Could not read cloud configuration")?).map_err(|_| "Invalid cloud configuration")?;
    let key = config["groqApiKey"].as_str().filter(|s| !s.is_empty()).ok_or("Groq key is not configured")?;
    let dict: Dictionary = serde_json::from_str(&std::fs::read_to_string(app_dir.join("dictionary.json"))
        .map_err(|_| "Could not read dictionary")?).map_err(|_| "Invalid dictionary")?;
    for model in ["accurate", "fast"] {
        for path in &clips {
            let started = std::time::Instant::now();
            let raw = transcribe_groq::transcribe_groq(key, path, "", model).await?;
            let decode_ms = started.elapsed().as_millis();
            let corrected = vocab_correct::correct_vocabulary(&raw, &dict.vocabulary_hints);
            let corrected = dict.apply_replacements(&corrected);
            let corrected = typr_lib::email_assemble::assemble_emails(&corrected);
            println!("CLOUD REPLAY {}", serde_json::json!({"file": path.file_name().unwrap_or_default().to_string_lossy(), "model": model,
                "decode_ms": decode_ms, "raw": raw, "ai_off": cleanup::cleanup_text(&corrected)}));
        }
    }
    Ok(())
}
