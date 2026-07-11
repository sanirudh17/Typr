use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Append a timestamped line to `typr.log` in the app config dir, and also echo it to
/// stdout (visible in `tauri dev`). Best-effort: any file error is ignored so logging can
/// never disturb the dictation pipeline. The installed (windowed) build has no console, so
/// the file is the only place these diagnostics survive — this is "the logs of the app".
pub fn log(app_dir: &Path, msg: &str) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!("[{}] {}", secs, msg);
    println!("[Typr] {}", msg);
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(app_dir.join("typr.log"))
    {
        let _ = writeln!(f, "{}", line);
    }
}
