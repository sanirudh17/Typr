use reqwest::multipart;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Instant;

fn groq_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

pub async fn transcribe_groq(api_key: &str, audio_path: &PathBuf, prompt: &str) -> Result<String, String> {
    if api_key.is_empty() {
        return Err("Groq API key not set. Please enter your API key in settings.".to_string());
    }

    let started_at = Instant::now();

    let audio_bytes = std::fs::read(audio_path)
        .map_err(|e| format!("Failed to read audio file: {}", e))?;

    let file_part = multipart::Part::bytes(audio_bytes)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| e.to_string())?;

    let mut form = multipart::Form::new()
        .text("model", "whisper-large-v3-turbo")
        .text("language", "en")
        .text("response_format", "json")
        .part("file", file_part);

    if !prompt.is_empty() {
        form = form.text("prompt", prompt.to_string());
    }

    let response = groq_client()
        .post("https://api.groq.com/openai/v1/audio/transcriptions")
        .header("Authorization", format!("Bearer {}", api_key))
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("Groq API request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Groq API error ({}): {}", status, body));
    }

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Groq response: {}", e))?;

    let text = json["text"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or("No 'text' field in Groq response".to_string())?;

    println!(
        "[Typr] Groq transcription completed in {:?}",
        started_at.elapsed()
    );

    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_empty_api_key() {
        let path = PathBuf::from("/tmp/test.wav");
        let result = transcribe_groq("", &path, "").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("API key not set"));
    }
}
