use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use tauri_plugin_shell::process::CommandChild;
use tauri_plugin_shell::ShellExt;

static SERVER_CHILD: Mutex<Option<CommandChild>> = Mutex::new(None);
static CURRENT_MODEL: Mutex<Option<String>> = Mutex::new(None);

/// Ensures the local Whisper HTTP server is running with the specified model.
/// If the server is already running with a different model, it stops the old server and starts a new one.
pub async fn ensure_running(app: &AppHandle, model_path: &PathBuf) -> Result<(), String> {
    if !model_path.exists() {
        return Err("Whisper model not found. Please download a model first.".to_string());
    }

    let model_key = model_path.to_string_lossy().to_string();

    let already_running = {
        let child_guard = SERVER_CHILD.lock().unwrap();
        let model_guard = CURRENT_MODEL.lock().unwrap();
        child_guard.is_some() && model_guard.as_ref() == Some(&model_key)
    };

    if already_running {
        // Wait up to 15 seconds for the already running/starting server to become healthy
        let start_time = Instant::now();
        let timeout = Duration::from_secs(15);
        while start_time.elapsed() < timeout {
            if is_server_healthy().await {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        // If it failed to become healthy, we will stop it and start a new one
    }

    // Stop any existing server process first
    stop_server().await;

    println!(
        "[Typr] Starting persistent GPU Whisper HTTP Server with model {:?}",
        model_path
    );

    let resource_path = app.path().resource_dir().unwrap().join("binaries");
    let current_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{};{}", resource_path.to_str().unwrap(), current_path);

    let threads = std::thread::available_parallelism()
        .map(|count| count.get().min(8).to_string())
        .unwrap_or_else(|_| "4".to_string());

    let cmd_args = vec![
        "-m".to_string(),
        model_path.to_str().unwrap().to_string(),
        "--host".to_string(),
        "127.0.0.1".to_string(),
        "--port".to_string(),
        "8080".to_string(),
        "-t".to_string(),
        threads,
        "-bs".to_string(),
        "1".to_string(),
        "-mc".to_string(),
        "0".to_string(),
        "-nf".to_string(),
        "-nt".to_string(),
        "-l".to_string(),
        "en".to_string(),
    ];

    let spawn_result = app
        .shell()
        .sidecar("whisper-server-cuda")
        .map_err(|e| format!("Failed to create server sidecar: {}", e))?
        .env("PATH", new_path)
        .args(cmd_args)
        .spawn();

    match spawn_result {
        Ok((mut rx, child)) => {
            {
                let mut child_guard = SERVER_CHILD.lock().unwrap();
                *child_guard = Some(child);
            }
            {
                let mut model_guard = CURRENT_MODEL.lock().unwrap();
                *model_guard = Some(model_key);
            }

            // Spawn log monitor task
            tauri::async_runtime::spawn(async move {
                use tauri_plugin_shell::process::CommandEvent;
                while let Some(event) = rx.recv().await {
                    match event {
                        CommandEvent::Stdout(line) => {
                            let text = String::from_utf8_lossy(&line);
                            println!("[whisper-server stdout] {}", text.trim());
                        }
                        CommandEvent::Stderr(line) => {
                            let text = String::from_utf8_lossy(&line);
                            println!("[whisper-server stderr] {}", text.trim());
                        }
                        CommandEvent::Terminated(status) => {
                            println!("[Typr] whisper-server exited with code: {:?}", status.code);
                            let mut child_guard = SERVER_CHILD.lock().unwrap();
                            *child_guard = None;
                            let mut model_guard = CURRENT_MODEL.lock().unwrap();
                            *model_guard = None;
                            break;
                        }
                        _ => {}
                    }
                }
            });

            // Wait up to 15 seconds for the server to start responding
            let start_time = Instant::now();
            let timeout = Duration::from_secs(15);
            while start_time.elapsed() < timeout {
                if is_server_healthy().await {
                    println!(
                        "[Typr] Persistent Whisper Server is healthy and ready in {:?}",
                        start_time.elapsed()
                    );
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }

            Err("Whisper server started but failed health check within timeout.".to_string())
        }
        Err(e) => Err(format!("Failed to spawn whisper-server sidecar: {}", e)),
    }
}

/// Terminates the persistent Whisper server process if it is running.
pub async fn stop_server() {
    let child = {
        let mut child_guard = SERVER_CHILD.lock().unwrap();
        child_guard.take()
    };
    if let Some(child) = child {
        println!("[Typr] Terminating persistent Whisper Server...");
        let _ = child.kill();
    }
    let mut model_guard = CURRENT_MODEL.lock().unwrap();
    *model_guard = None;
}

/// Pings the server root to verify HTTP health.
async fn is_server_healthy() -> bool {
    let client = reqwest::Client::new();
    match client
        .get("http://127.0.0.1:8080/")
        .timeout(Duration::from_millis(200))
        .send()
        .await
    {
        Ok(_) => true,
        Err(_) => false,
    }
}
