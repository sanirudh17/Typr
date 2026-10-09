//! Compare an installed or temporary Nemotron model on prepared review WAVs.
//! Local inference only; no app configuration, microphone, or history changes.
use std::path::PathBuf;
use typr_lib::transcribe_nemotron;
#[tokio::main]
async fn main() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let folder = PathBuf::from(args.next().ok_or("Expected model directory and prepared WAV paths")?);
    let clips: Vec<PathBuf> = args.map(PathBuf::from).collect();
    if clips.is_empty() { return Err("Expected at least one prepared WAV".into()); }
    transcribe_nemotron::prewarm(&folder)?;
    for path in clips {
        let started = std::time::Instant::now();
        let text = transcribe_nemotron::transcribe_nemotron(&folder, &path).await?;
        println!("VARIANT REPLAY {}", serde_json::json!({
            "file": path.file_name().unwrap_or_default().to_string_lossy(),
            "decode_ms": started.elapsed().as_millis(), "text": text}));
    }
    Ok(())
}
