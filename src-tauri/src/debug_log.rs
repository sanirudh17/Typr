use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use std::sync::Mutex;

static LOG_WRITE: Mutex<()> = Mutex::new(());

/// Append a timestamped line to `typr.log` in the app config dir, and also echo it to
/// stdout (visible in `tauri dev`). Best-effort: any file error is ignored so logging can
/// never disturb the dictation pipeline. The installed (windowed) build has no console, so
/// the file is the only place these diagnostics survive — this is "the logs of the app".
pub fn log(app_dir: &Path, msg: &str) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!("[{}] {}\n", secs, msg);
    println!("[Typr] {}", msg);
    // Review saves now finish in the background. Keep a complete log record
    // together so a worker cannot splice its output into a stage/PERF line.
    let Ok(_write_guard) = LOG_WRITE.lock() else { return; };
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(app_dir.join("typr.log"))
    {
        let _ = f.write_all(line.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn background_log_records_do_not_interleave() {
        let id = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("typr-log-write-{}-{id}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let threads: Vec<_> = (0..32).map(|i| {
            let dir = dir.clone();
            std::thread::spawn(move || log(&dir, &format!("message-{i}")))
        }).collect();
        for thread in threads { thread.join().unwrap(); }
        let contents = std::fs::read_to_string(dir.join("typr.log")).unwrap();
        assert_eq!(contents.lines().count(), 32);
        for i in 0..32 { assert!(contents.lines().any(|line| line.ends_with(&format!("] message-{i}")))); }
        std::fs::remove_file(dir.join("typr.log")).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }
}
