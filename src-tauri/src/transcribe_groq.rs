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

pub async fn transcribe_groq(api_key: &str, audio_path: &PathBuf, prompt: &str, model: &str) -> Result<String, String> {
    if api_key.is_empty() {
        return Err("Groq API key not set. Please enter your API key in settings.".to_string());
    }

    let started_at = Instant::now();

    let audio_bytes = std::fs::read(audio_path)
        .map_err(|e| format!("Failed to read audio file: {}", e))?;

    let max_retries = 3;
    let mut last_error = String::new();

    for attempt in 1..=max_retries {
        let file_part = multipart::Part::bytes(audio_bytes.clone())
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| e.to_string())?;

        let mut form = multipart::Form::new()
            .text("model", groq_model_id(model))
            .text("language", "en")
            .text("temperature", "0")
            .text("response_format", "json")
            .part("file", file_part);

        if !prompt.is_empty() {
            form = form.text("prompt", prompt.to_string());
        }

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

                    println!(
                        "[Typr] Groq transcription completed successfully on attempt {} in {:?}",
                        attempt,
                        started_at.elapsed()
                    );

                    return Ok(text);
                } else {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    last_error = format!("Groq API error ({}): {}", status, body);

                    // If it is a client error (e.g. 401 Unauthorized, 400 Bad Request) that is NOT a rate limit (429)
                    // or a request timeout (408), fail fast instead of retrying.
                    if status.is_client_error() && status != reqwest::StatusCode::TOO_MANY_REQUESTS && status != reqwest::StatusCode::REQUEST_TIMEOUT {
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
            println!("[Typr] Groq transcription attempt {} failed: {}. Retrying in {:?}...", attempt, last_error, delay);
            tokio::time::sleep(delay).await;
        }
    }

    Err(format!("Groq transcription failed after {} attempts. Last error: {}", max_retries, last_error))
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
}
