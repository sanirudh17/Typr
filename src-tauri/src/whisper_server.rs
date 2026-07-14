use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use tauri_plugin_shell::process::CommandChild;
use tauri_plugin_shell::ShellExt;

static HEALTH_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
fn health_client() -> &'static reqwest::Client {
    HEALTH_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_millis(200))
            .no_proxy() // Disables system proxy detection to prevent WPAD lookup stalls when offline
            .build()
            .unwrap_or_default()
    })
}

static SERVER_CHILD: Mutex<Option<CommandChild>> = Mutex::new(None);
static CURRENT_MODEL: Mutex<Option<String>> = Mutex::new(None);
/// PID of the server currently owning SERVER_CHILD/CURRENT_MODEL. Lets a server's
/// log-monitor task tell whether it is still the current server before it clears
/// shared state on exit — see `terminate_should_clear`.
static CURRENT_PID: Mutex<Option<u32>> = Mutex::new(None);

/// Whether a just-terminated server (`terminated_pid`) may clear the shared
/// "current server" state. Only the process that is *still* current may: a stale
/// monitor whose server was already replaced by a model switch must not, or it
/// would drop the new server's child handle (orphaning its process) and wipe
/// CURRENT_MODEL (the "have None" self-heal + leaked whisper-server processes).
fn terminate_should_clear(current_pid: Option<u32>, terminated_pid: u32) -> bool {
    current_pid == Some(terminated_pid)
}

/// The model key (path string) the persistent server is currently serving, if any.
pub fn current_model_key() -> Option<String> {
    CURRENT_MODEL.lock().unwrap().clone()
}

/// Whether the warm server (serving `current`) already has the `intended` model loaded.
pub fn warm_server_matches(current: Option<&str>, intended: &str) -> bool {
    current == Some(intended)
}

/// Whether a just-finished download should proactively (re)start the local server:
/// only when the local engine is selected AND the downloaded model is the selected one.
/// Don't spin up a local server for a model the user isn't using (or while on cloud).
pub fn should_warm_after_download(engine: &str, selected_model: &str, downloaded_model: &str) -> bool {
    engine == "local" && selected_model == downloaded_model
}

/// Whether the idle reaper should spin the warm server down now. Pure so it can be
/// unit-tested without a live server: the reaper reads live state, this decides.
pub fn should_spin_down(
    recorder_ready: bool,
    server_running: bool,
    idle: Duration,
    timeout: Duration,
) -> bool {
    recorder_ready && server_running && idle > timeout
}

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

    // Stop any existing server process first, then wait for port 8080 to actually be
    // released before spawning the replacement. Killing the child does not free the socket
    // instantly on Windows: binding too early makes the new server exit(1), and the health
    // check false-positives against the dying old process (the `exited code 1` +
    // `have None` restart churn seen when switching models).
    stop_server().await;
    wait_for_port_free().await;

    println!(
        "[Typr] Starting persistent GPU Whisper HTTP Server with model {:?}",
        model_path
    );

    let resource_path = app.path().resource_dir().unwrap().join("binaries");
    let current_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{};{}", resource_path.to_str().unwrap(), current_path);

    // Optimize threads to 4 when running on GPU to avoid driver thread contention
    let threads = std::thread::available_parallelism()
        .map(|count| count.get().min(4).to_string())
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
            // Capture the pid before moving the child so the log-monitor task can
            // identify itself and avoid clobbering a newer server's state on exit.
            let server_pid = child.pid();
            {
                let mut child_guard = SERVER_CHILD.lock().unwrap();
                *child_guard = Some(child);
            }
            {
                let mut model_guard = CURRENT_MODEL.lock().unwrap();
                *model_guard = Some(model_key);
            }
            {
                let mut pid_guard = CURRENT_PID.lock().unwrap();
                *pid_guard = Some(server_pid);
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
                            println!(
                                "[Typr] whisper-server (pid {}) exited with code: {:?}",
                                server_pid, status.code
                            );
                            // Only clear shared state if we are still the current server.
                            // After a model switch the newer server owns SERVER_CHILD/
                            // CURRENT_MODEL; clearing them here would orphan its process
                            // handle and wipe the loaded-model marker.
                            let is_current =
                                terminate_should_clear(*CURRENT_PID.lock().unwrap(), server_pid);
                            if is_current {
                                *CURRENT_PID.lock().unwrap() = None;
                                *SERVER_CHILD.lock().unwrap() = None;
                                *CURRENT_MODEL.lock().unwrap() = None;
                            } else {
                                println!(
                                    "[Typr] (stale monitor for pid {}; a newer server is current — not clearing state)",
                                    server_pid
                                );
                            }
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
    *CURRENT_MODEL.lock().unwrap() = None;
    // Clear the current-pid marker too, so the killed server's own Terminated
    // handler treats itself as stale and doesn't race a replacement's state.
    *CURRENT_PID.lock().unwrap() = None;
}

/// After stopping the server, poll until the port stops accepting connections — i.e. the old
/// process has released the socket — so the replacement can bind it. Bounded so we never hang
/// if something else holds the port; on timeout we start anyway (the post-spawn health wait
/// then surfaces any real bind failure).
async fn wait_for_port_free() {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        // is_server_healthy() returns false the moment the connection is refused (port free).
        if !is_server_healthy().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    println!("[Typr] Warning: port 8080 still busy after 5s; starting new server anyway");
}

/// Pings the server root to verify HTTP health.
async fn is_server_healthy() -> bool {
    match health_client()
        .get("http://127.0.0.1:8080/")
        .send()
        .await
    {
        Ok(_) => true,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_warm_server_matches() {
        assert!(warm_server_matches(Some("/x/ggml-medium.bin"), "/x/ggml-medium.bin"));
        assert!(!warm_server_matches(Some("/x/ggml-small.bin"), "/x/ggml-medium.bin"));
        assert!(!warm_server_matches(None, "/x/ggml-medium.bin"));
    }

    #[test]
    fn test_terminate_should_clear() {
        // The still-current server terminating clears the shared state.
        assert!(terminate_should_clear(Some(100), 100));
        // A stale monitor (an older server replaced by a model switch) must NOT
        // clear — doing so orphaned the new server's process and wiped the model
        // marker ("have None" self-heal + leaked whisper-server processes).
        assert!(!terminate_should_clear(Some(200), 100));
        // Nothing is current -> nothing to clear.
        assert!(!terminate_should_clear(None, 100));
    }

    #[test]
    fn test_should_spin_down() {
        let t = Duration::from_secs(180);
        // All conditions met -> spin down.
        assert!(should_spin_down(true, true, Duration::from_secs(200), t));
        // Recorder busy (recording/transcribing) -> never spin down.
        assert!(!should_spin_down(false, true, Duration::from_secs(200), t));
        // No server running -> nothing to do.
        assert!(!should_spin_down(true, false, Duration::from_secs(200), t));
        // Under the timeout -> stay warm.
        assert!(!should_spin_down(true, true, Duration::from_secs(100), t));
        // Exactly at the timeout is NOT past it -> stay warm.
        assert!(!should_spin_down(true, true, Duration::from_secs(180), t));
        // Just past the timeout -> spin down.
        assert!(should_spin_down(true, true, Duration::from_secs(181), t));
    }

    #[test]
    fn test_should_warm_after_download() {
        // local engine + the downloaded model is the selected one -> warm it
        assert!(should_warm_after_download("local", "medium", "medium"));
        // local engine but a different model was downloaded -> don't warm
        assert!(!should_warm_after_download("local", "small", "medium"));
        // on cloud -> never spin up the local server
        assert!(!should_warm_after_download("cloud", "medium", "medium"));
    }
}
