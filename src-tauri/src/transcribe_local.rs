use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Instant;
use tauri::{AppHandle, Manager};
use tauri_plugin_shell::ShellExt;

static LOCAL_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
fn local_client() -> &'static reqwest::Client {
    LOCAL_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5)) // Bounded 5-second timeout to fail-fast and fallback if server hangs
            .no_proxy() // Disables system proxy detection to prevent WPAD lookup stalls when offline
            .build()
            .unwrap_or_default()
    })
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
                    println!(
                        "[Typr] Persistent HTTP Whisper completed in {:?}. Output: {}",
                        http_start.elapsed(),
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
    }

    #[test]
    fn test_model_download_url() {
        assert_eq!(
            model_download_url("small"),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin"
        );
    }
}
