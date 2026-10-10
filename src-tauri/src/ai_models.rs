//! Groq model discovery, caching and request compatibility. No transcript is
//! sent by discovery; only the existing API key authenticates GET /models.
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::{Mutex, OnceLock}, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};

pub const DEFAULT_MODEL: &str = "qwen/qwen3.8-27b";
const BOOTSTRAP_MODELS: [&str; 3] = [DEFAULT_MODEL, "openai/gpt-oss-120b", "openai/gpt-oss-20b"];
const CACHE_FILE: &str = "groq-models.json";
const MAX_RESPONSE_BYTES: usize = 512 * 1024;
const CACHE_TTL: Duration = Duration::from_secs(3600);

#[derive(Clone, Deserialize, Serialize)]
pub struct Model {
    pub id: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Catalog {
    pub models: Vec<Model>,
    pub fetched_at: Option<u64>,
    pub source: String,
    pub warning: Option<String>,
}

#[derive(Default)]
struct Cache {
    // Kept only in memory to isolate accounts. Never serialized or logged.
    key: String,
    catalog: Option<Catalog>,
    verified: Option<Instant>,
    retry_after: Option<Instant>,
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache::default()))
}

pub fn valid_model_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 160 && id.bytes().all(|c|
        c.is_ascii_alphanumeric() || matches!(c, b'/' | b'-' | b'_' | b'.' | b':'))
}

/// Groq's list has no endpoint/capability field. Exclude known non-editor
/// families, and allow new chat families without adding an app allowlist.
pub fn is_cleanup_model(id: &str) -> bool {
    if !valid_model_id(id) { return false; }
    let lower = id.to_ascii_lowercase();
    !["whisper", "distil-whisper", "canopylabs/", "playai/", "orpheus", "tts", "speech",
        "prompt-guard", "safeguard", "moderation", "embedding", "groq/compound"]
        .iter().any(|family| lower.contains(family))
}

fn parse_catalog(bytes: &[u8]) -> Result<Catalog, String> {
    let json: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|_| "Groq returned an unreadable model list. Try Refresh again.".to_string())?;
    let rows = json["data"].as_array()
        .filter(|rows| rows.len() <= 500)
        .ok_or("Groq returned an invalid model list. Try Refresh again.")?;
    let mut ids: Vec<String> = rows.iter()
        .filter(|row| row["active"].as_bool().unwrap_or(true))
        .filter_map(|row| row["id"].as_str())
        .filter(|id| is_cleanup_model(id)).map(str::to_owned).collect();
    ids.sort(); ids.dedup();
    if ids.is_empty() {
        return Err("Groq did not list any text cleanup models for this key. Your selection was kept.".into());
    }
    Ok(Catalog {
        models: ids.into_iter().map(|id| Model { id }).collect(),
        fetched_at: Some(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()),
        source: "live".into(), warning: None,
    })
}

fn offline_catalog(app_dir: &Path) -> Catalog {
    // Disk contains public model IDs and a timestamp only, never a key, account
    // identifier, user text or a successful-authentication claim.
    let path = app_dir.join(CACHE_FILE);
    if std::fs::metadata(&path).map(|m| m.len() <= MAX_RESPONSE_BYTES as u64).unwrap_or(false) {
        if let Ok(bytes) = std::fs::read(path) {
            if let Ok(mut saved) = serde_json::from_slice::<Catalog>(&bytes) {
                saved.models.retain(|model| is_cleanup_model(&model.id));
                saved.models.truncate(500);
                if !saved.models.is_empty() {
                    saved.source = "cached".into(); saved.warning = None;
                    return saved;
                }
            }
        }
    }
    Catalog { models: BOOTSTRAP_MODELS.iter().map(|id| Model { id: (*id).into() }).collect(),
        fetched_at: None, source: "starter".into(), warning: None }
}

fn error_for_status(status: u16) -> String {
    match status {
        401 => "Groq rejected this API key. Check the key and try Refresh again.".into(),
        403 => "This key cannot list Groq models. Check its permissions and try Refresh again.".into(),
        429 => "Groq's model list is rate limited. Wait a minute before refreshing.".into(),
        _ => format!("Could not refresh Groq models (HTTP {status}). Your selection was kept."),
    }
}

async fn fetch(api_key: &str) -> Result<Catalog, (String, Duration)> {
    let fail = |message: String| (message, Duration::from_secs(60));
    let client = reqwest::Client::builder().no_proxy()
        .redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(8)).build()
        .map_err(|_| fail("Could not start model discovery. Try Refresh again.".into()))?;
    let mut response = client.get("https://api.groq.com/openai/v1/models")
        .bearer_auth(api_key).send().await
        .map_err(|_| fail("Could not reach Groq. Your saved models and selection are still available.".into()))?;
    if !response.status().is_success() {
        let delay = response.headers().get("retry-after").and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok()).unwrap_or(60).clamp(60, 3600);
        return Err((error_for_status(response.status().as_u16()), Duration::from_secs(delay)));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await
        .map_err(|_| fail("Groq's model list was interrupted. Try Refresh again.".into()))? {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(fail("Groq's model list was too large. Your selection was kept.".into()));
        }
        bytes.extend_from_slice(&chunk);
    }
    parse_catalog(&bytes).map_err(fail)
}

/// Serialized discovery avoids duplicate startup/key-change/manual requests.
/// The refresh lock is never used by dictation or editing requests.
pub async fn discover(api_key: &str, app_dir: &Path, force: bool) -> Catalog {
    static REFRESH: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    let _refresh = REFRESH.get_or_init(|| tokio::sync::Mutex::new(())).lock().await;
    let key = api_key.trim();
    {
        let mut state = cache().lock().unwrap();
        if state.catalog.is_none() { state.catalog = Some(offline_catalog(app_dir)); }
        if state.key != key {
            state.key = key.into(); state.verified = None; state.retry_after = None;
            if let Some(catalog) = &mut state.catalog { catalog.source = "cached".into(); catalog.warning = None; }
        }
        if key.is_empty() {
            let mut catalog = state.catalog.clone().unwrap();
            catalog.warning = Some("Add your Groq API key to discover available models.".into());
            return catalog;
        }
        if let Some(retry) = state.retry_after.filter(|retry| *retry > Instant::now()) {
            let mut catalog = state.catalog.clone().unwrap();
            if force { catalog.warning = Some(format!("Try Refresh again in {} seconds.",
                retry.saturating_duration_since(Instant::now()).as_secs() + 1)); }
            return catalog;
        }
        if !force && state.verified.is_some_and(|time| time.elapsed() < CACHE_TTL) {
            return state.catalog.clone().unwrap();
        }
    }
    match fetch(key).await {
        Ok(catalog) => {
            // Failure to cache on disk must not block a valid model list.
            if let Ok(bytes) = serde_json::to_vec(&catalog) {
                let _ = std::fs::create_dir_all(app_dir);
                let _ = std::fs::write(app_dir.join(CACHE_FILE), bytes);
            }
            let mut state = cache().lock().unwrap();
            state.catalog = Some(catalog.clone()); state.verified = Some(Instant::now());
            state.retry_after = Some(Instant::now() + Duration::from_secs(30));
            catalog
        }
        Err((warning, delay)) => {
            let mut state = cache().lock().unwrap();
            state.retry_after = Some(Instant::now() + delay);
            let catalog = state.catalog.as_mut().unwrap();
            if catalog.source == "live" { catalog.source = "cached".into(); }
            catalog.warning = Some(warning);
            catalog.clone()
        }
    }
}

fn known_models(api_key: &str) -> Option<Vec<String>> {
    let state = cache().lock().unwrap();
    if state.key != api_key.trim() || state.verified.is_none() { return None; }
    Some(state.catalog.as_ref()?.models.iter().map(|model| model.id.clone()).collect())
}

/// Prefer the selected family on retirement, then established cleanup families.
pub fn replacement(selected: &str, models: &[String]) -> Option<String> {
    models.iter().filter(|id| id.as_str() != selected).min_by_key(|id| {
        let family = if selected.starts_with("openai/gpt-oss-") { "openai/gpt-oss-" }
            else if selected.starts_with("qwen/") { "qwen/" } else { "" };
        let rank = if !family.is_empty() && id.starts_with(family) { 0 }
            else if id.as_str() == DEFAULT_MODEL { 1 }
            else if id.starts_with("qwen/qwen3") { 2 }
            else if id.as_str() == "openai/gpt-oss-120b" { 3 }
            else if id.as_str() == "openai/gpt-oss-20b" { 4 } else { 5 };
        (rank, *id)
    }).cloned()
}

pub fn effective_model(api_key: &str, selected: &str) -> String {
    let selected = selected.trim();
    let selected = if is_cleanup_model(selected) { selected } else { DEFAULT_MODEL };
    if let Some(models) = known_models(api_key) {
        if !models.iter().any(|id| id == selected) {
            return replacement(selected, &models).unwrap_or_else(|| selected.into());
        }
    }
    selected.into()
}

pub fn fallback_model(api_key: &str, selected: &str) -> Option<String> {
    if let Some(models) = known_models(api_key) {
        // Prefer a different family for reasoning starvation or refusal.
        let different: Vec<_> = models.iter().filter(|id|
            id.starts_with("qwen/") != selected.starts_with("qwen/")).cloned().collect();
        return replacement(selected, if different.is_empty() { &models } else { &different });
    }
    Some(if selected.starts_with("qwen/") { "openai/gpt-oss-120b" } else { DEFAULT_MODEL }.into())
        .filter(|id| id != selected)
}

/// Optional parameters are sent only to documented compatible families. A new
/// chat model can use standard Chat Completions without a release of Typr.
pub fn reasoning_effort_for(model: &str) -> Option<&'static str> {
    if model.starts_with("qwen/qwen3") { Some("none") }
        else if matches!(model, "openai/gpt-oss-20b" | "openai/gpt-oss-120b") { Some("medium") }
        else { None }
}

pub fn apply_reasoning_options(body: &mut serde_json::Value, model: &str) {
    if let Some(effort) = reasoning_effort_for(model) { body["reasoning_effort"] = effort.into(); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_future_chat_models_without_an_allowlist() {
        let catalog = parse_catalog(br#"{"data":[{"id":"vendor/new-editor-42b"},{"id":"qwen/qwen3.9-28b"},{"id":"vendor/new-editor-42b"},{"id":"retired-chat","active":false}]}"#).unwrap();
        assert_eq!(catalog.models.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(),
            vec!["qwen/qwen3.9-28b", "vendor/new-editor-42b"]);
        assert_eq!(catalog.source, "live");
        assert!(catalog.fetched_at.is_some());
    }

    #[test]
    fn excludes_speech_safety_and_tool_agents() {
        for id in ["whisper-large-v3", "whisper-large-v3-turbo", "distil-whisper-large-v3-en",
            "canopylabs/orpheus-v1-english", "playai/play-tts", "meta-llama/llama-prompt-guard-2-86m",
            "openai/gpt-oss-safeguard-20b", "groq/compound", "groq/compound-mini", "vendor/text-embedding"] {
            assert!(!is_cleanup_model(id), "{id}");
        }
        for id in ["llama-3.3-70b-versatile", "openai/gpt-oss-120b", "qwen/qwen3.8-27b", "vendor/future-chat"] {
            assert!(is_cleanup_model(id), "{id}");
        }
    }

    #[test]
    fn rejects_malformed_empty_and_oversized_lists() {
        for body in [br#"{}"#.as_slice(), br#"{"data":[]}"#, br#"{"data":[{"id":"whisper-large-v3"}]}"#, b"not json"] {
            assert!(parse_catalog(body).is_err());
        }
        let body = serde_json::json!({"data":vec![serde_json::json!({"id":"llama-chat"});501]});
        assert!(parse_catalog(&serde_json::to_vec(&body).unwrap()).is_err());
        for id in ["", "bad model", "<script>", "../ model", "https://host?secret", "model\nnew"] {
            assert!(!valid_model_id(id));
        }
        assert!(!valid_model_id(&"a".repeat(161)));
    }

    #[test]
    fn generic_models_never_receive_unsupported_reasoning_options() {
        let mut body = serde_json::json!({"model":"new-vendor/chat"});
        apply_reasoning_options(&mut body, "new-vendor/chat");
        assert!(body.get("reasoning_effort").is_none());
        apply_reasoning_options(&mut body, DEFAULT_MODEL);
        assert_eq!(body["reasoning_effort"], "none");
        apply_reasoning_options(&mut body, "openai/gpt-oss-120b");
        assert_eq!(body["reasoning_effort"], "medium");
        assert_eq!(reasoning_effort_for("openai/gpt-oss-new"), None);
    }

    #[test]
    fn retirement_uses_available_models_and_never_repeats_failed_model() {
        let ids = vec!["vendor/new-chat".into(), "qwen/qwen3.9-28b".into(), "openai/gpt-oss-20b".into()];
        assert_eq!(replacement("qwen/retired", &ids).as_deref(), Some("qwen/qwen3.9-28b"));
        assert_eq!(replacement("openai/gpt-oss-retired", &ids).as_deref(), Some("openai/gpt-oss-20b"));
        assert_eq!(replacement("retired", &["vendor/new-chat".into()]).as_deref(), Some("vendor/new-chat"));
        assert!(replacement("only-chat", &["only-chat".into()]).is_none());
        assert!(replacement("retired", &[]).is_none());
    }

    #[test]
    fn saved_catalog_contains_only_public_metadata() {
        let catalog = parse_catalog(br#"{"data":[{"id":"chat-model","private":"not persisted"}]}"#).unwrap();
        let value = serde_json::to_value(catalog).unwrap();
        assert_eq!(value["models"][0], serde_json::json!({"id":"chat-model"}));
        assert!(value.get("key").is_none());
        assert!(!value.to_string().contains("not persisted"));
    }

    #[test]
    fn errors_are_actionable_without_echoing_credentials_or_response_bodies() {
        assert!(error_for_status(401).contains("key"));
        assert!(error_for_status(403).contains("permissions"));
        assert!(error_for_status(429).contains("Wait"));
        assert!(error_for_status(503).contains("503"));
    }
}
