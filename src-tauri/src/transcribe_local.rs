use std::path::PathBuf;
use std::time::Instant;
use tauri::{AppHandle, Manager};
use tauri_plugin_shell::ShellExt;

pub async fn transcribe_local(
    app: &AppHandle,
    model_path: &PathBuf,
    audio_path: &PathBuf,
    prompt: &str,
) -> Result<String, String> {
    if !model_path.exists() {
        return Err("Whisper model not found. Please download a model first.".to_string());
    }

    let threads = std::thread::available_parallelism()
        .map(|count| {
            // Whisper.cpp is highly memory bandwidth bound on consumer CPUs.
            // User requested testing with exactly 8 threads. (Bypassing physical core check)
            let optimal_threads = count.get().min(8);
            optimal_threads.to_string()
        })
        .unwrap_or_else(|_| "8".to_string());
    let started_at = Instant::now();

    println!(
        "[Typr] Running whisper.cpp sidecar with model {:?} using {} threads",
        model_path, threads
    );

    let resource_path = app.path().resource_dir().unwrap().join("binaries");
    let current_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{};{}", resource_path.to_str().unwrap(), current_path);

    let mut cmd_args = vec![
        "-m".to_string(),
        model_path.to_str().unwrap().to_string(),
        "-f".to_string(),
        audio_path.to_str().unwrap().to_string(),
        "--no-timestamps".to_string(),
        "-t".to_string(),
        threads.clone(),
        "-bs".to_string(),
        "1".to_string(),
        "-mc".to_string(),
        "0".to_string(),
        "-nf".to_string(),
        "-l".to_string(),
        "en".to_string(),
    ];

    if !prompt.is_empty() {
        cmd_args.push("--prompt".to_string());
        cmd_args.push(prompt.to_string());
    }

    let output = app
        .shell()
        .sidecar("whisper-cpp")
        .map_err(|e| format!("Failed to create sidecar command: {}", e))?
        .env("PATH", new_path)
        .args(cmd_args)
        .output()
        .await
        .map_err(|e| format!("Failed to run whisper.cpp: {}", e))?;

    if output.status.code() != Some(0) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("whisper.cpp failed: {}", stderr));
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    println!(
        "[Typr] Whisper completed in {:?}. Output: {}",
        started_at.elapsed(),
        text
    );
    Ok(text)
}

pub fn model_filename(model_size: &str) -> String {
    format!("ggml-{}.bin", model_size)
}

pub fn model_download_url(model_size: &str) -> String {
    format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{}.bin",
        model_size
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_filename() {
        assert_eq!(model_filename("small"), "ggml-small.bin");
        assert_eq!(model_filename("medium"), "ggml-medium.bin");
    }

    #[test]
    fn test_model_download_url() {
        assert_eq!(
            model_download_url("small"),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin"
        );
    }
}
