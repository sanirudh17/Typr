//! CPU runtime ships with Typr. NVIDIA support is a separately verified,
//! versioned download; no native code is loaded from an arbitrary URL/path.
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::{io::Read, path::{Path, PathBuf}, time::Duration};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_shell::{process::Command, ShellExt};

pub const RELEASE: &str = "b5454";
pub const CUDA_SHA256: &str = "afef0b881c500958921c3f5523b50e59ee2ec9b6f5cbd25b324c51ed308a957a";
const CUDA_URL: &str = "https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-win-cuda-12.4.0-x64.zip";
const MAX_ARCHIVE: u64 = 750 * 1024 * 1024;
const MAX_EXPANDED: u64 = 1536 * 1024 * 1024;
static DOWNLOAD_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const REQUIRED: &[&str] = &["whisper-cli.exe", "whisper-server.exe", "whisper.dll", "ggml.dll", "ggml-base.dll", "ggml-cpu-x64.dll", "ggml-cuda.dll", "cublas64_12.dll", "cublasLt64_12.dll", "cudart64_12.dll"];

pub fn cuda_directory(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|e| e.to_string())?
        .join("runtimes").join(format!("whisper-cuda-{RELEASE}")))
}
pub fn installed_at(directory: &Path) -> bool {
    std::fs::read_to_string(directory.join("verified.txt")).is_ok_and(|value| value == CUDA_SHA256)
        && REQUIRED.iter().all(|name| directory.join(name).metadata().is_ok_and(|m| m.is_file() && m.len() > 0))
}
pub fn cuda_installed(app: &AppHandle) -> bool {
    cuda_directory(app).is_ok_and(|directory| installed_at(&directory))
}
pub fn uses_cuda(app: &AppHandle) -> bool {
    app.state::<crate::settings::RuntimeSettings>().acceleration.lock().unwrap().as_str() == "cuda"
        && cuda_installed(app)
}
pub fn command(app: &AppHandle, server: bool, gpu: bool) -> Result<Command, String> {
    let (command, directory) = if gpu {
        let directory = cuda_directory(app)?;
        if !installed_at(&directory) { return Err("Download NVIDIA support in Engine settings first.".into()); }
        let executable = directory.join(if server { "whisper-server.exe" } else { "whisper-cli.exe" });
        (app.shell().command(executable), directory)
    } else {
        let command = app.shell().sidecar(if server { "whisper-server" } else { "whisper-cpp" })
            .map_err(|e| format!("Could not start the CPU Whisper runtime: {e}"))?;
        (command, app.path().resource_dir().map_err(|e| e.to_string())?.join("binaries/cpu"))
    };
    let path = std::env::join_paths(std::iter::once(directory.clone()).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())))
        .map_err(|e| e.to_string())?;
    Ok(command.current_dir(directory).env("PATH", path))
}

fn runtime_file(name: &str) -> bool {
    REQUIRED.contains(&name) || (name.starts_with("ggml-cpu-") && name.ends_with(".dll")
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'.'))
        || matches!(name, "nvblas64_12.dll" | "nvrtc64_120_0.dll" | "nvrtc-builtins64_124.dll")
}
fn archive_filename(path: &str) -> Option<&str> {
    let name = path.strip_prefix("Release/")?;
    if name.contains(['/', '\\']) || !runtime_file(name) { None } else { Some(name) }
}
fn digest_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 { break; }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn install_archive(archive: &Path, staging: &Path, destination: &Path) -> Result<(), String> {
    if digest_file(archive)? != CUDA_SHA256 { return Err("NVIDIA download failed its checksum check. Retry the download.".into()); }
    std::fs::create_dir(staging).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(std::fs::File::open(archive).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut expanded = 0u64;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
        let Some(name) = archive_filename(entry.name()).map(str::to_string) else { continue; };
        if entry.unix_mode().is_some_and(|mode| mode & 0o170000 == 0o120000) { return Err("Unsupported link in NVIDIA archive.".into()); }
        expanded = expanded.checked_add(entry.size()).ok_or("NVIDIA archive is too large.")?;
        if expanded > MAX_EXPANDED { return Err("NVIDIA archive is too large.".into()); }
        let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(staging.join(name)).map_err(|e| e.to_string())?;
        let expected = entry.size();
        let copied = std::io::copy(&mut entry.by_ref().take(expected + 1), &mut output).map_err(|e| e.to_string())?;
        if copied != expected { return Err("Incomplete NVIDIA runtime file.".into()); }
    }
    if !REQUIRED.iter().all(|name| staging.join(name).is_file()) { return Err("NVIDIA archive is missing runtime files.".into()); }
    // Catch missing DLL dependencies before making the runtime selectable.
    for binary in ["whisper-cli.exe", "whisper-server.exe"] {
        let mut command = std::process::Command::new(staging.join(binary));
        command.arg("-h").current_dir(staging).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        #[cfg(windows)] { use std::os::windows::process::CommandExt; command.creation_flags(0x08000000); }
        let mut child = command.spawn().map_err(|_| "NVIDIA runtime could not start. Your CPU runtime is unchanged.")?;
        let started = std::time::Instant::now();
        loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                if !status.success() { return Err("NVIDIA runtime failed its load check. Your CPU runtime is unchanged.".into()); }
                break;
            }
            if started.elapsed() > Duration::from_secs(15) { let _ = child.kill(); let _ = child.wait(); return Err("NVIDIA runtime load check timed out.".into()); }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    std::fs::write(staging.join("verified.txt"), CUDA_SHA256).map_err(|e| e.to_string())?;
    if destination.exists() {
        // Preserve an incomplete previous installation for recovery rather than
        // recursively deleting a computed path or overwriting loaded DLLs.
        std::fs::rename(destination, destination.with_extension(crate::vocabulary::new_id())).map_err(|e| e.to_string())?;
    }
    std::fs::rename(staging, destination).map_err(|e| e.to_string())
}

pub async fn download_cuda(app: AppHandle) -> Result<(), String> {
    let _lock = DOWNLOAD_LOCK.try_lock().map_err(|_| "NVIDIA support is already downloading.")?;
    let destination = cuda_directory(&app)?;
    if installed_at(&destination) { return Ok(()); }
    let root = destination.parent().ok_or("Invalid runtime directory.")?;
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let nonce = crate::vocabulary::new_id();
    let archive = root.join(format!("cuda-{nonce}.zip.part"));
    let staging = root.join(format!("cuda-{nonce}.staging"));
    let result = async {
        let client = reqwest::Client::builder().no_proxy().connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(900))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                let url = attempt.url();
                if attempt.previous().len() < 5 && url.scheme() == "https"
                    && url.host_str().is_some_and(|host| host == "github.com" || host.ends_with(".githubusercontent.com")) {
                    attempt.follow()
                } else { attempt.stop() }
            })).build().map_err(|e| e.to_string())?;
        let response = client.get(CUDA_URL).send().await.map_err(|_| "Could not download NVIDIA support. Check your connection and retry.")?;
        if !response.status().is_success() { return Err(format!("NVIDIA download failed (HTTP {}). Retry later.", response.status().as_u16())); }
        let total = response.content_length().unwrap_or(684_913_404);
        if total > MAX_ARCHIVE { return Err("NVIDIA download is larger than expected.".into()); }
        let mut file = tokio::fs::OpenOptions::new().write(true).create_new(true).open(&archive).await.map_err(|e| e.to_string())?;
        let mut stream = response.bytes_stream();
        let mut downloaded = 0u64;
        let mut last_emit = std::time::Instant::now();
        use tokio::io::AsyncWriteExt;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| "NVIDIA download was interrupted. Retry the download.")?;
            downloaded += chunk.len() as u64;
            if downloaded > MAX_ARCHIVE { return Err("NVIDIA download is larger than expected.".into()); }
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
            if last_emit.elapsed() > Duration::from_millis(150) {
                let _ = app.emit("cuda-download-progress", crate::downloader::DownloadProgress {downloaded,total,percent:(downloaded as f64 / total.max(1) as f64 * 95.0).min(95.0)});
                last_emit = std::time::Instant::now();
            }
        }
        file.flush().await.map_err(|e| e.to_string())?;
        drop(file);
        let a = archive.clone(); let s = staging.clone(); let d = destination.clone();
        tokio::task::spawn_blocking(move || install_archive(&a, &s, &d)).await.map_err(|_| "NVIDIA installation stopped unexpectedly.")??;
        let _ = app.emit("cuda-download-progress", crate::downloader::DownloadProgress {downloaded,total,percent:100.0});
        Ok(())
    }.await;
    let _ = std::fs::remove_file(&archive);
    if staging.exists() {
        // Extraction created only flat regular files. Never follow a directory link.
        if staging.symlink_metadata().is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
            for entry in std::fs::read_dir(&staging).into_iter().flatten().flatten() { let _ = std::fs::remove_file(entry.path()); }
            let _ = std::fs::remove_dir(&staging);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn checksum_failure_never_publishes_a_runtime() {
        let root = std::env::temp_dir().join(format!("typr-runtime-check-{}",crate::vocabulary::new_id()));
        std::fs::create_dir(&root).unwrap(); let archive=root.join("bad.zip");
        std::fs::write(&archive,b"untrusted archive").unwrap();
        assert!(install_archive(&archive,&root.join("staging"),&root.join("runtime")).unwrap_err().contains("checksum"));
        assert!(!root.join("staging").exists()); assert!(!root.join("runtime").exists());
        std::fs::remove_file(archive).unwrap(); std::fs::remove_dir(root).unwrap();
    }
    #[test] #[ignore = "requires the pinned CUDA archive; uses no GPU or cloud AI"]
    fn verified_cuda_archive_installs_and_both_executables_start() {
        let archive=PathBuf::from(std::env::var_os("TYPR_CUDA_TEST_ARCHIVE").expect("Set TYPR_CUDA_TEST_ARCHIVE"));
        let root=archive.parent().unwrap().join(format!("runtime-audit-{}",crate::vocabulary::new_id()));
        std::fs::create_dir(&root).unwrap(); let destination=root.join("cuda");
        install_archive(&archive,&root.join("staging"),&destination).unwrap();
        assert!(installed_at(&destination));
        assert!(!destination.join("llama.dll").exists());
        for entry in std::fs::read_dir(&destination).unwrap() { std::fs::remove_file(entry.unwrap().path()).unwrap(); }
        std::fs::remove_dir(destination).unwrap(); std::fs::remove_dir(root).unwrap();
    }
    #[test] fn only_expected_flat_runtime_files_are_extracted() {
        assert_eq!(archive_filename("Release/ggml-cuda.dll"), Some("ggml-cuda.dll"));
        assert_eq!(archive_filename("Release/ggml-cpu-alderlake.dll"), Some("ggml-cpu-alderlake.dll"));
        for name in ["Release/../evil.dll", "Release/a/b.dll", "Release/ggml-cpu-../evil.dll", "Release/ggml-cpu-evil\\.dll", "Release/llama.dll", "Release/whisper-talk-llama.exe", "Other/whisper-cli.exe"] { assert_eq!(archive_filename(name), None); }
    }
    #[test] fn incomplete_installation_cannot_enable_gpu() {
        let dir = std::env::temp_dir().join(format!("typr-runtime-{}",crate::vocabulary::new_id()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("verified.txt"), CUDA_SHA256).unwrap();
        assert!(!installed_at(&dir));
        std::fs::remove_file(dir.join("verified.txt")).unwrap(); std::fs::remove_dir(dir).unwrap();
    }
}
