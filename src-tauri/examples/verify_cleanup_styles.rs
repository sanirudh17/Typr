//! Three serial live cleanup requests with disposable text; no user document.
use typr_lib::{ai_postprocess, context_detector::ContextCategory, settings::Settings};
#[tokio::main]
async fn main() -> Result<(), String> {
    if std::env::var("TYPR_LIVE_STYLE_TEST").as_deref() != Ok("1") {
        return Err("Set TYPR_LIVE_STYLE_TEST=1 to permit three live requests.".into());
    }
    let dir = dirs::config_dir().ok_or("No config directory")?.join("com.typr.app");
    let settings = Settings::load(&dir);
    let source = "Review the invoice.\nSchedule the meeting.\nKeep the final attachment.";
    for (tone, format) in [("default", "bullets"), ("formal", "paragraphs"), ("default", "raw")] {
        let prompt = ai_postprocess::build_system_prompt(
            ai_postprocess::context_system_prompt(&ContextCategory::Professional), tone, format, "");
        let started = std::time::Instant::now();
        let result = ai_postprocess::postprocess_with_fallback(&settings.groq_api_key, source, &settings.ai_model, &prompt).await?;
        let text = result.text.trim();
        match format {
            "bullets" => assert!(text.lines().all(|line|line.starts_with("- ")) && text.lines().count()==3, "Expected three bullets: {text}"),
            "paragraphs" => assert!(!text.contains("- "), "Paragraph style created bullets"),
            _ => assert_eq!(text, source, "Raw style changed structure"),
        }
        for sentence in ["Review the invoice.", "Schedule the meeting.", "Keep the final attachment."] {
            assert!(text.contains(sentence), "A clause changed: {text}");
        }
        println!("PASS style={format} tone={tone} model={} elapsed_ms={}",result.model,started.elapsed().as_millis());
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    Ok(())
}
