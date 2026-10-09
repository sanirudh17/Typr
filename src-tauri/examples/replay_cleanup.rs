//! Explicit text-only checks of configured cleanup models and every profile.
//! Sends the fixed regression passage (or an explicitly supplied case) to Groq.
//! No audio uploads or app writes. `--model ID --case-json PATH professional`
//! runs one focused replay instead of the complete profile matrix.
use typr_lib::{ai_postprocess as ai, context_detector::ContextCategory as Category};
#[tokio::main]
async fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut requested = Vec::new();
    let mut model_filter = None;
    let mut case_path = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--model" => model_filter = Some(args.next().ok_or("--model requires an ID")?),
            "--case-json" => case_path = Some(args.next().ok_or("--case-json requires a path")?),
            _ => requested.push(arg),
        }
    }
    let app_dir = dirs::config_dir().ok_or("No app directory")?.join("com.typr.app");
    let config: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(app_dir.join("config.json"))
        .map_err(|_| "Could not read configured credentials")?).map_err(|_| "Invalid settings")?;
    let key = config["groqApiKey"].as_str().filter(|s| !s.is_empty()).ok_or("Groq key is not configured")?;
    let source = if let Some(path) = case_path {
        let case: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        case["source"].as_str().ok_or("Case needs a source string")?.to_string()
    } else {
        "Acuracy improvisations and general fixes. Compare FluidVoice with Nemotron. The picture shows our team taking a shot. Add a quick overlay button. The model does not work without post processing. Keep all final words, including robust decoding algorithms.".to_string()
    };
    let profiles = [
        ("cleanup", ai::resolve_system_prompt("cleanup", "natural")),
        ("messaging", ai::context_system_prompt(&Category::Messaging)),
        ("email", ai::context_system_prompt(&Category::Email)),
        ("professional", ai::context_system_prompt(&Category::Professional)),
        ("developer", ai::context_system_prompt(&Category::Developer)),
        ("terminal", ai::terminal_system_prompt()),
        ("prompt_natural", ai::resolve_system_prompt("prompt", "natural")),
        ("prompt_structured", ai::resolve_system_prompt("prompt", "structured")),
    ];
    for model in ["qwen/qwen3.8-27b", "openai/gpt-oss-120b"] {
        if model_filter.as_ref().is_some_and(|requested| requested != model) { continue; }
        for (profile, base) in profiles {
            if !requested.is_empty() && !requested.iter().any(|p| p == profile) { continue; }
            let prompt = ai::build_system_prompt(base, "default", "default", "");
            let started = std::time::Instant::now();
            let result = ai::postprocess_with_fallback(key, &source, model, &prompt).await;
            let row = match result {
                Ok(result) => serde_json::json!({"model":result.model,"requested_model":model,"profile":profile,"ms":started.elapsed().as_millis(),"accepted":true,
                    "recovered":result.recovered,"retried":result.retried,"text":result.text}),
                Err(error) => serde_json::json!({"model":model,"profile":profile,"ms":started.elapsed().as_millis(),"accepted":false,
                    "error": if error.starts_with("Groq chat error") { error.split(':').next().unwrap_or("Cloud error") } else { &error }}),
            };
            println!("CLEANUP REPLAY {row}");
            // Evaluation pacing only, not an app delay or a retry policy.
            if model_filter.is_none() || requested.len() != 1 {
                tokio::time::sleep(std::time::Duration::from_millis(10_000)).await;
            }
        }
    }
    Ok(())
}
