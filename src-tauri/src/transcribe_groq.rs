use reqwest::multipart;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

fn groq_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            // Guard the *connection*, not the whole exchange. `timeout` covers uploading the
            // audio too, so a 5s total ceiling aborted healthy long dictations mid-upload and
            // burned all three retries on them — a ~1 minute recording is ~2 MB of WAV. The
            // total ceiling stays generous; the caller decides when to give up.
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_default()
    })
}

/// Map the friendly cloud model value to a Groq model id.
/// "fast" -> turbo (speed); anything else -> full large-v3 (accuracy).
fn groq_model_id(model: &str) -> &'static str {
    match model {
        "fast" => "whisper-large-v3-turbo",
        _ => "whisper-large-v3",
    }
}

/// Transcribe audio with Groq's cloud Whisper API.
///
/// # The `_prompt` argument is accepted and deliberately NOT used
///
/// Mirroring commit 189f677 for local Whisper: passing dictionary bias hints to Whisper
/// causes the model to silently drop speech on longer dictations (e.g. omitting entire
/// clauses). Vocabulary correction runs deterministically post-transcription via `vocab_correct`.
///
/// Long audio is chunked via `audio_chunker::split_into_chunks` and reconciled with
/// `audio_chunker::merge_chunk_texts` to prevent tail-truncation on long recordings.
pub async fn transcribe_groq(
    api_key: &str,
    audio_path: &PathBuf,
    _prompt: &str,
    model: &str,
) -> Result<String, String> {
    if api_key.is_empty() {
        return Err("Groq API key not set. Please enter your API key in settings.".to_string());
    }

    let started_at = Instant::now();

    let (samples, sample_rate) = match crate::audio_chunker::read_wav_samples(audio_path) {
        Ok(res) => res,
        Err(_) => {
            let audio_bytes = std::fs::read(audio_path)
                .map_err(|e| format!("Failed to read audio file: {}", e))?;
            let text = post_groq_chunk(api_key, audio_bytes, model).await?;
            return Ok(text);
        }
    };

    let chunks = crate::audio_chunker::split_into_chunks(&samples, sample_rate);
    println!(
        "[Typr] Groq transcribing {:.1}s in {} chunk(s), model {}",
        samples.len() as f32 / sample_rate as f32,
        chunks.len(),
        groq_model_id(model)
    );

    let mut parts: Vec<(String, bool)> = Vec::with_capacity(chunks.len());
    for chunk in &chunks {
        let chunk_bytes = crate::audio_chunker::samples_to_wav_bytes(chunk.samples, sample_rate)?;
        let chunk_text = post_groq_chunk(api_key, chunk_bytes, model).await?;
        let trimmed = chunk_text.trim().to_string();
        if !trimmed.is_empty() {
            parts.push((trimmed, chunk.overlaps_previous));
        }
    }

    let merged = crate::audio_chunker::merge_chunk_texts(&parts);
    println!(
        "[Typr] Groq transcription completed successfully in {:?}",
        started_at.elapsed()
    );
    Ok(merged)
}

async fn post_groq_chunk(
    api_key: &str,
    audio_bytes: Vec<u8>,
    model: &str,
) -> Result<String, String> {
    let max_retries = 3;
    let mut last_error = String::new();

    for attempt in 1..=max_retries {
        let file_part = multipart::Part::bytes(audio_bytes.clone())
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| e.to_string())?;

        // NO PROMPT. See documentation on transcribe_groq — passing dictionary hints
        // as an initial prompt causes Whisper to silently truncate speech on longer audio.
        let form = multipart::Form::new()
            .text("model", groq_model_id(model))
            .text("language", "en")
            .text("temperature", "0")
            .text("response_format", "json")
            .part("file", file_part);

        println!("[Typr] Sending Groq transcription attempt {}/{}", attempt, max_retries);

        let response_result = groq_client()
            .post("https://api.groq.com/openai/v1/audio/transcriptions")
            .header("Authorization", format!("Bearer {}", api_key))
            .multipart(form)
            .send()
            .await;

        match response_result {
            Ok(response) => {
                if response.status().is_success() {
                    let json: serde_json::Value = response
                        .json()
                        .await
                        .map_err(|e| format!("Failed to parse Groq response: {}", e))?;

                    let text = json["text"]
                        .as_str()
                        .map(|s| s.to_string())
                        .ok_or("No 'text' field in Groq response".to_string())?;

                    return Ok(text);
                } else {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    last_error = format!("Groq API error ({}): {}", status, body);

                    if status.is_client_error()
                        && status != reqwest::StatusCode::TOO_MANY_REQUESTS
                        && status != reqwest::StatusCode::REQUEST_TIMEOUT
                    {
                        println!("[Typr] Non-retryable Groq client error: {}. Aborting retries.", status);
                        break;
                    }
                }
            }
            Err(e) => {
                last_error = format!("Groq API request failed: {}", e);
            }
        }

        if attempt < max_retries {
            let delay = Duration::from_millis(300 * attempt as u64);
            println!(
                "[Typr] Groq transcription attempt {} failed: {}. Retrying in {:?}...",
                attempt, last_error, delay
            );
            tokio::time::sleep(delay).await;
        }
    }

    Err(format!(
        "Groq transcription failed after {} attempts. Last error: {}",
        max_retries, last_error
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_empty_api_key() {
        let path = PathBuf::from("/tmp/test.wav");
        let result = transcribe_groq("", &path, "", "accurate").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("API key not set"));
    }

    #[test]
    fn test_groq_model_id_mapping() {
        assert_eq!(groq_model_id("fast"), "whisper-large-v3-turbo");
        assert_eq!(groq_model_id("accurate"), "whisper-large-v3");
        assert_eq!(groq_model_id(""), "whisper-large-v3");
        assert_eq!(groq_model_id("anything-else"), "whisper-large-v3");
    }

    #[test]
    fn test_cloud_chunking_recovers_from_single_pass_truncation() {
        // Simulating the cloud truncation bug:
        // A single 70s audio pass with prompt would drop the tail.
        // With chunking + merge_chunk_texts:
        // Chunk 1 returns: "Seven item list. Item 1 is first. Item 2 is second. Item 3 is third."
        // Chunk 2 (overlap) returns: "Item 3 is third. Item 4 is fourth. Let me know whether there are some changes that you would like to make."
        let parts = vec![
            ("Seven item list. Item 1 is first. Item 2 is second. Item 3 is third.".to_string(), false),
            ("Item 3 is third. Item 4 is fourth. Let me know whether there are some changes that you would like to make.".to_string(), true),
        ];
        let merged = crate::audio_chunker::merge_chunk_texts(&parts);
        assert_eq!(
            merged,
            "Seven item list. Item 1 is first. Item 2 is second. Item 3 is third. Item 4 is fourth. Let me know whether there are some changes that you would like to make."
        );
    }
}
