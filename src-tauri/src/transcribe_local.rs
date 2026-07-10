use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use tauri_plugin_shell::ShellExt;

static LOCAL_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
fn local_client() -> &'static reqwest::Client {
    LOCAL_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            // Safety cap only. The real timeout is set per-request, scaled to audio length
            // (see `post_timeout` below) so a legitimately-slow decode on a battery-throttled
            // GPU is NOT killed and forced down the cold-sidecar fallback.
            .timeout(Duration::from_secs(180))
            .no_proxy() // Disables system proxy detection to prevent WPAD lookup stalls when offline
            .build()
            .unwrap_or_default()
    })
}

/// Estimate recorded audio length (seconds) from a 16 kHz mono s16 WAV's byte length.
/// Used to scale the inference timeout; not exact, but well within the safety margin.
fn estimate_wav_seconds(byte_len: usize) -> f64 {
    // 16000 samples/s * 2 bytes/sample = 32000 B/s of PCM, past the 44-byte header.
    byte_len.saturating_sub(44) as f64 / 32_000.0
}

/// Timeout for a warm-server inference POST, scaled to the clip length. Floor keeps short
/// clips from failing fast into a cold restart on a slow GPU; cap bounds a genuine hang.
fn post_timeout_for(audio_secs: f64) -> Duration {
    Duration::from_secs_f64((audio_secs * 3.0 + 15.0).clamp(20.0, 180.0))
}

pub async fn transcribe_local(
    app: &AppHandle,
    model_path: &PathBuf,
    audio_path: &PathBuf,
    prompt: &str,
) -> Result<String, String> {
    if !model_path.exists() {
        return Err("Whisper model not found. Please download a model first.".to_string());
    }

    let cuda_threads = std::thread::available_parallelism()
        .map(|count| count.get().min(4).to_string())
        .unwrap_or_else(|_| "4".to_string());

    let cpu_threads = std::thread::available_parallelism()
        .map(|count| count.get().min(12).to_string())
        .unwrap_or_else(|_| "12".to_string());

    let resource_path = app.path().resource_dir().unwrap().join("binaries");
    let current_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{};{}", resource_path.to_str().unwrap(), current_path);

    // 1. Try persistent HTTP server first
    println!(
        "[Typr] Attempting persistent local Whisper HTTP server execution with model {:?}",
        model_path
    );
    let http_start = Instant::now();

    // Read audio bytes once
    let file_bytes_result = std::fs::read(audio_path);
    if let Ok(file_bytes) = file_bytes_result {
        // Scale the inference timeout to the clip length so a slow-but-fine decode on a
        // battery-throttled GPU stays on the warm server instead of tripping the fallback.
        let est_audio_secs = estimate_wav_seconds(file_bytes.len());
        let post_timeout = post_timeout_for(est_audio_secs);
        println!(
            "[Typr] Local inference: ~{:.1}s audio; warm-server POST timeout {:.0}s",
            est_audio_secs,
            post_timeout.as_secs_f64()
        );

        // Build the request body helper
        let make_form = |bytes: Vec<u8>| {
            let part = reqwest::multipart::Part::bytes(bytes)
                .file_name("audio.wav")
                .mime_str("audio/wav")
                .unwrap();
            let mut form = reqwest::multipart::Form::new()
                .part("file", part)
                .text("temperature", "0.0")
                .text("response_format", "json");
            if !prompt.is_empty() {
                form = form.text("prompt", prompt.to_string());
            }
            form
        };

        // Guard the hot path on model identity: reuse the warm server only if it is
        // already serving the selected model; otherwise (re)start it first so we never
        // transcribe with a stale model (e.g. right after downloading a new model).
        let intended_key = model_path.to_string_lossy().to_string();
        let current = crate::whisper_server::current_model_key();
        if !crate::whisper_server::warm_server_matches(current.as_deref(), &intended_key) {
            println!(
                "[Typr] Warm server model mismatch (have {:?}, want {}). Ensuring correct model...",
                current, intended_key
            );
            if let Err(e) = crate::whisper_server::ensure_running(app, model_path).await {
                println!(
                    "[Typr] Failed to ensure correct model server: {}. Will still try POST then sidecars...",
                    e
                );
            }
        }

        // POST to the (now correct) warm server.
        let mut http_result = local_client()
            .post("http://127.0.0.1:8080/inference")
            .timeout(post_timeout)
            .multipart(make_form(file_bytes.clone()))
            .send()
            .await;

        // If direct post fails (e.g. connection refused), ensure the server is running, and retry
        if http_result.is_err() {
            println!("[Typr] Direct POST failed. Ensuring persistent server is running...");
            match crate::whisper_server::ensure_running(app, model_path).await {
                Ok(_) => {
                    println!("[Typr] Persistent server ensured healthy. Retrying inference POST...");
                    http_result = local_client()
                        .post("http://127.0.0.1:8080/inference")
                        .timeout(post_timeout)
                        .multipart(make_form(file_bytes))
                        .send()
                        .await;
                }
                Err(e) => {
                    println!("[Typr] Failed to ensure persistent server: {}. Falling back to sidecars...", e);
                }
            }
        }

        // Process the final HTTP result if we have a successful connection
        if let Ok(response) = http_result {
            if response.status().is_success() {
                #[derive(serde::Deserialize)]
                struct InferenceResponse {
                    text: String,
                }
                if let Ok(inf_res) = response.json::<InferenceResponse>().await {
                    let text = inf_res.text.trim().to_string();
                    let elapsed = http_start.elapsed();
                    println!(
                        "[Typr] Persistent HTTP Whisper completed in {:?} (~{:.2}x realtime, {:.1}s audio). Output: {}",
                        elapsed,
                        elapsed.as_secs_f64() / est_audio_secs.max(0.1),
                        est_audio_secs,
                        text
                    );
                    return Ok(text);
                }
            } else {
                println!("[Typr] HTTP server returned error: {}. Falling back to sidecars...", response.status());
            }
        } else {
            println!("[Typr] Persistent HTTP server failed. Falling back to sidecars...");
        }
    } else {
        println!("[Typr] Failed to read audio file. Falling back to sidecars...");
    }

    // 2. Try GPU (CUDA) execution as fallback
    println!(
        "[Typr] Attempting whisper.cpp GPU (CUDA) execution with model {:?} using {} threads",
        model_path, cuda_threads
    );
    let started_gpu = Instant::now();

    let mut cuda_cmd_args = vec![
        "-m".to_string(),
        model_path.to_str().unwrap().to_string(),
        "-f".to_string(),
        audio_path.to_str().unwrap().to_string(),
        "--no-timestamps".to_string(),
        "-t".to_string(),
        cuda_threads,
        "-bs".to_string(),
        "1".to_string(),
        "-mc".to_string(),
        "0".to_string(),
        "-nf".to_string(),
        "-l".to_string(),
        "en".to_string(),
    ];

    if !prompt.is_empty() {
        cuda_cmd_args.push("--prompt".to_string());
        cuda_cmd_args.push(prompt.to_string());
    }

    let gpu_result = app
        .shell()
        .sidecar("whisper-cpp-cuda")
        .map_err(|e| format!("Failed to create sidecar command: {}", e))?
        .env("PATH", &new_path)
        .args(cuda_cmd_args)
        .output()
        .await;

    match gpu_result {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            println!(
                "[Typr] GPU (CUDA) Whisper completed in {:?}. Output: {}",
                started_gpu.elapsed(),
                text
            );
            return Ok(text);
        }
        other => {
            let error_details = match &other {
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    format!("Exit status: {:?}, Stderr: {}", output.status, stderr)
                }
                Err(e) => e.to_string(),
            };
            println!(
                "[Typr] GPU (CUDA) execution failed or not available. Error: {}. Falling back to CPU...",
                error_details
            );
        }
    }

    // 2. CPU Fallback Path
    println!(
        "[Typr] Running CPU fallback sidecar with model {:?} using {} threads",
        model_path, cpu_threads
    );
    let started_cpu = Instant::now();

    let mut cpu_cmd_args = vec![
        "-m".to_string(),
        model_path.to_str().unwrap().to_string(),
        "-f".to_string(),
        audio_path.to_str().unwrap().to_string(),
        "--no-timestamps".to_string(),
        "-t".to_string(),
        cpu_threads,
        "-bs".to_string(),
        "1".to_string(),
        "-mc".to_string(),
        "0".to_string(),
        "-nf".to_string(),
        "-l".to_string(),
        "en".to_string(),
    ];

    if !prompt.is_empty() {
        cpu_cmd_args.push("--prompt".to_string());
        cpu_cmd_args.push(prompt.to_string());
    }

    let cpu_output = app
        .shell()
        .sidecar("whisper-cpp")
        .map_err(|e| format!("Failed to create sidecar command: {}", e))?
        .env("PATH", &new_path)
        .args(cpu_cmd_args)
        .output()
        .await;

    match cpu_output {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            println!(
                "[Typr] CPU Fallback Whisper completed in {:?}. Output: {}",
                started_cpu.elapsed(),
                text
            );
            Ok(text)
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("whisper.cpp CPU fallback failed with exit status: {:?}. Stderr: {}", output.status, stderr))
        }
        Err(e) => {
            Err(format!("Failed to run whisper.cpp CPU fallback: {}", e))
        }
    }
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
        // English quantized IDs (the current defaults) must map to the real HF filenames.
        assert_eq!(model_filename("small.en-q5_1"), "ggml-small.en-q5_1.bin");
        assert_eq!(model_filename("medium.en-q5_0"), "ggml-medium.en-q5_0.bin");
    }

    #[test]
    fn test_model_download_url() {
        assert_eq!(
            model_download_url("small"),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin"
        );
        assert_eq!(
            model_download_url("medium.en-q5_0"),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.en-q5_0.bin"
        );
    }

    #[test]
    fn test_estimate_wav_seconds() {
        // 32000 B/s of PCM after a 44-byte header.
        assert_eq!(estimate_wav_seconds(44), 0.0);
        assert_eq!(estimate_wav_seconds(44 + 32_000), 1.0);
        assert_eq!(estimate_wav_seconds(44 + 320_000), 10.0);
        // Undersized/garbage input never underflows.
        assert_eq!(estimate_wav_seconds(0), 0.0);
    }

    #[test]
    fn test_post_timeout_for() {
        // Short clips get the floor, not a fail-fast 5s.
        assert_eq!(post_timeout_for(0.0), Duration::from_secs(20));
        assert_eq!(post_timeout_for(1.0), Duration::from_secs(20));
        // Mid-length scales up (5s -> 30s).
        assert_eq!(post_timeout_for(5.0), Duration::from_secs(30));
        // Long clips scale further (34s -> 117s), well past a slow decode.
        assert_eq!(post_timeout_for(34.0), Duration::from_secs(117));
        // Cap bounds a genuine hang.
        assert_eq!(post_timeout_for(1000.0), Duration::from_secs(180));
    }
}
