use std::io::Write;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};

#[derive(Clone, serde::Serialize)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: u64,
    pub percent: f64,
}

pub async fn download_model(
    app: AppHandle,
    url: &str,
    dest: &PathBuf,
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Download failed with status: {}", response.status()));
    }

    let total = extract_content_length(&response);

    // Ensure parent directory exists
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let mut file = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut downloaded: u64 = 0;

    let mut stream = response.bytes_stream();
    use futures_util::StreamExt;

    let mut last_emit = std::time::Instant::now();
    let mut last_percent = 0.0f64;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download stream error: {}", e))?;
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        downloaded += chunk.len() as u64;

        let percent = if total > 0 {
            ((downloaded as f64 / total as f64) * 100.0).min(99.0)
        } else {
            0.0
        };

        if percent - last_percent >= 0.5 || last_emit.elapsed() >= std::time::Duration::from_millis(50) {
            last_percent = percent;
            last_emit = std::time::Instant::now();
            let _ = app.emit("download-progress", DownloadProgress {
                downloaded,
                total,
                percent,
            });
        }
    }

    // Ensure clean 100% emission upon completion
    let _ = app.emit("download-progress", DownloadProgress {
        downloaded,
        total: total.max(downloaded),
        percent: 100.0,
    });

    Ok(())
}

fn extract_content_length_from_headers(headers: &reqwest::header::HeaderMap) -> u64 {
    headers
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
}

fn extract_content_length(resp: &reqwest::Response) -> u64 {
    let header_len = extract_content_length_from_headers(resp.headers());
    if header_len > 0 {
        return header_len;
    }
    resp.content_length().unwrap_or(0)
}

#[derive(Clone, Debug)]
pub struct MultiFileItem {
    pub url: String,
    pub dest: PathBuf,
}

pub async fn download_multiple_files(
    app: AppHandle,
    files: &[MultiFileItem],
) -> Result<(), String> {
    use futures_util::StreamExt;

    let client = reqwest::Client::new();

    // Concurrent pre-flight HEAD requests to calculate cumulative size across all files without delay
    let head_futures = files.iter().map(|item| {
        let client = client.clone();
        let url = item.url.clone();
        async move {
            if let Ok(resp) = client.head(&url).send().await {
                if resp.status().is_success() {
                    return extract_content_length(&resp);
                }
            }
            0u64
        }
    });
    let sizes = futures_util::future::join_all(head_futures).await;
    let mut total_size: u64 = sizes.into_iter().sum();

    // Fallback: If HEAD requests failed or CDN gave 0, estimate total_size
    // (Nemotron models are 682,215,474 bytes) so the progress bar
    // moves monotonically and is never stuck at 0%.
    if total_size < 500_000_000 {
        total_size = 682_215_474;
    }

    let mut cumulative_downloaded: u64 = 0;
    let mut last_emit = std::time::Instant::now();
    let mut last_percent = 0.0f64;

    for item in files {
        let response = client
            .get(&item.url)
            .send()
            .await
            .map_err(|e| format!("Download request failed for {}: {}", item.url, e))?;

        if !response.status().is_success() {
            return Err(format!("Download failed with status: {} for {}", response.status(), item.url));
        }

        if let Some(parent) = item.dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        let mut file = std::fs::File::create(&item.dest).map_err(|e| e.to_string())?;
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("Download stream error: {}", e))?;
            file.write_all(&chunk).map_err(|e| e.to_string())?;
            cumulative_downloaded += chunk.len() as u64;

            if cumulative_downloaded > total_size {
                total_size = cumulative_downloaded + 50_000_000;
            }

            let percent = if total_size > 0 {
                ((cumulative_downloaded as f64 / total_size as f64) * 100.0).min(99.0)
            } else {
                0.0
            };

            if percent - last_percent >= 0.5 || last_emit.elapsed() >= std::time::Duration::from_millis(50) {
                last_percent = percent;
                last_emit = std::time::Instant::now();
                let _ = app.emit("download-progress", DownloadProgress {
                    downloaded: cumulative_downloaded,
                    total: total_size,
                    percent,
                });
            }
        }
    }

    // Ensure clean 100% emission upon completion
    let _ = app.emit("download-progress", DownloadProgress {
        downloaded: cumulative_downloaded,
        total: cumulative_downloaded,
        percent: 100.0,
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_multi_file_item_construction() {
        let item = MultiFileItem {
            url: "https://example.com/file1.onnx".to_string(),
            dest: PathBuf::from("test/file1.onnx"),
        };
        assert_eq!(item.url, "https://example.com/file1.onnx");
        assert_eq!(item.dest, PathBuf::from("test/file1.onnx"));
    }

    #[test]
    fn test_progress_monotonic_calculation() {
        let total_size = 1000u64;
        let mut downloaded = 0u64;
        let mut last_percent = 0.0f64;

        for chunk_size in [100, 200, 300, 400] {
            downloaded += chunk_size;
            let percent = ((downloaded as f64 / total_size as f64) * 100.0).min(100.0);
            assert!(percent >= last_percent, "Progress must be monotonic: {} < {}", percent, last_percent);
            last_percent = percent;
        }
        assert_eq!(last_percent, 100.0);
    }

    #[test]
    fn test_extract_content_length_from_headers() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::CONTENT_LENGTH, "12345".parse().unwrap());
        assert_eq!(extract_content_length_from_headers(&headers), 12345);
    }

    #[test]
    fn test_progress_throttling_condition() {
        let last_percent = 50.0f64;
        let p1 = 50.2f64;
        assert!(p1 - last_percent < 0.5);
        let p2 = 50.5f64;
        assert!(p2 - last_percent >= 0.5);
    }
}
