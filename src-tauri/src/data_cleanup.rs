//! Clear generated local data without touching preferences, credentials, or models.
use crate::history::History;
use serde::Serialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Default, Serialize)]
pub struct ClearDataReport {
    pub history_items: usize,
    pub files_removed: usize,
    pub bytes_freed: u64,
    pub warnings: Vec<String>,
}

fn is_link(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() { return true; }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Includes directory junctions, which are not reported as symlinks.
        if metadata.file_attributes() & 0x400 != 0 { return true; }
    }
    false
}

fn generated_wav(name: &str) -> bool {
    name == "temp_recording.wav" || name.strip_prefix("nemotron-compare-")
        .and_then(|s| s.strip_suffix(".wav"))
        .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
}

fn remove_generated_file(path: &Path, root: &Path, report: &mut ClearDataReport) {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let result = (|| {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.to_string()),
        };
        if !metadata.is_file() || is_link(&metadata) {
            return Err("not a regular local file".into());
        }
        // Canonicalize before deleting; never follow a link out of app storage.
        let resolved = path.canonicalize().map_err(|e| e.to_string())?;
        if !resolved.starts_with(root) { return Err("outside app storage".into()); }
        fs::remove_file(path).map_err(|e| e.to_string())?;
        report.files_removed += 1;
        report.bytes_freed += metadata.len();
        Ok(())
    })();
    if let Err(e) = result { report.warnings.push(format!("{name}: {e}")); }
}

/// Called while Recorder's idle lock and the history lock are held.
/// Only exact generated filenames are eligible; no recursive deletion.
pub fn clear_local_data(app_dir: &Path, history: &mut History) -> Result<ClearDataReport, String> {
    let metadata = fs::symlink_metadata(app_dir).map_err(|e| e.to_string())?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Err("App storage is not a regular local directory.".into());
    }
    let root = app_dir.canonicalize().map_err(|e| e.to_string())?;
    let history_path = app_dir.join("history.json");
    let old_size = match fs::symlink_metadata(&history_path) {
        Ok(m) if m.is_file() && !is_link(&m) => m.len(),
        Ok(_) => return Err("History is not a regular local file. No data was cleared.".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
        Err(e) => return Err(e.to_string()),
    };
    // Persist before changing memory. A failed write must retain visible history.
    History::default().save(&app_dir.to_path_buf())?;
    let mut report = ClearDataReport {
        history_items: history.items.len(),
        bytes_freed: old_size.saturating_sub(fs::metadata(&history_path).map(|m| m.len()).unwrap_or(0)),
        ..Default::default()
    };
    *history = History::default();
    remove_generated_file(&app_dir.join("typr.log"), &root, &mut report);
    match fs::read_dir(app_dir) {
        Ok(entries) => for entry in entries {
            match entry {
                Ok(entry) if generated_wav(&entry.file_name().to_string_lossy()) =>
                    remove_generated_file(&entry.path(), &root, &mut report),
                Ok(_) => {},
                Err(e) => report.warnings.push(e.to_string()),
            }
        },
        Err(e) => report.warnings.push(format!("Temporary recordings: {e}")),
    }
    let review_dir = app_dir.join("review-audio");
    match fs::symlink_metadata(&review_dir) {
        Ok(m) if m.is_dir() && !is_link(&m) => match fs::read_dir(&review_dir) {
            Ok(entries) => for entry in entries {
                match entry {
                    Ok(entry) => {
                        let name = entry.file_name();
                        let name = name.to_string_lossy();
                        if name.strip_suffix(".wav").is_some_and(|id|
                            !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())) {
                            remove_generated_file(&entry.path(), &root, &mut report);
                        }
                    },
                    Err(e) => report.warnings.push(e.to_string()),
                }
            },
            Err(e) => report.warnings.push(format!("Saved review recordings: {e}")),
        },
        Ok(_) => report.warnings.push("Saved review recordings: not a regular local directory".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
        Err(e) => report.warnings.push(format!("Saved review recordings: {e}")),
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let path = std::env::temp_dir().join(format!("typr-data-cleanup-{}-{id}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }

    #[test]
    fn clears_history_and_generated_files_preserving_preferences_and_models() {
        let dir = Fixture::new();
        let mut history = History::default();
        history.add_item("private transcript".into(), 10.0, &dir.0).unwrap();
        for name in ["config.json", "dictionary.json", "styles.json", "ggml-model.bin", "personal.wav"] {
            fs::write(dir.0.join(name), b"keep").unwrap();
        }
        fs::create_dir(dir.0.join("nemotron-model")).unwrap();
        fs::write(dir.0.join("nemotron-model/model.onnx"), b"keep").unwrap();
        fs::create_dir(dir.0.join("review-audio")).unwrap();
        fs::write(dir.0.join("review-audio/123.wav"), b"recording").unwrap();
        fs::write(dir.0.join("review-audio/personal.wav"), b"keep").unwrap();
        for name in ["typr.log", "temp_recording.wav", "nemotron-compare-123.wav"] {
            fs::write(dir.0.join(name), b"generated").unwrap();
        }
        let report = clear_local_data(&dir.0, &mut history).unwrap();
        assert_eq!((report.history_items, report.files_removed), (1, 4));
        assert!(report.bytes_freed > 0 && report.warnings.is_empty());
        assert!(history.items.is_empty() && History::load(&dir.0).items.is_empty());
        for name in ["config.json", "dictionary.json", "styles.json", "ggml-model.bin", "personal.wav", "nemotron-model/model.onnx", "review-audio/personal.wav"] {
            assert_eq!(fs::read(dir.0.join(name)).unwrap(), b"keep");
        }
        let again = clear_local_data(&dir.0, &mut history).unwrap();
        assert_eq!((again.history_items, again.files_removed), (0, 0));
    }

    #[test]
    fn invalid_history_target_keeps_memory_and_files() {
        let dir = Fixture::new();
        let mut history = History::default();
        history.add_item("keep transcript".into(), 1.0, &dir.0).unwrap();
        fs::remove_file(dir.0.join("history.json")).unwrap();
        fs::create_dir(dir.0.join("history.json")).unwrap();
        fs::write(dir.0.join("typr.log"), b"keep").unwrap();
        assert!(clear_local_data(&dir.0, &mut history).is_err());
        assert_eq!(history.items.len(), 1);
        assert_eq!(fs::read(dir.0.join("typr.log")).unwrap(), b"keep");
    }

    #[test]
    fn directories_named_like_generated_files_are_never_deleted() {
        let dir = Fixture::new();
        fs::create_dir(dir.0.join("temp_recording.wav")).unwrap();
        fs::write(dir.0.join("temp_recording.wav/keep"), b"keep").unwrap();
        let report = clear_local_data(&dir.0, &mut History::default()).unwrap();
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(fs::read(dir.0.join("temp_recording.wav/keep")).unwrap(), b"keep");
    }
}
