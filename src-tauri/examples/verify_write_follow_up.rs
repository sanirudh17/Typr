//! Two explicitly opted-in, serial AI requests using disposable text only.
//! No microphone, user document, input injection, or settings writes.
use typr_lib::{settings::Settings, write_mode::{self, Cancellation}};

#[tokio::main]
async fn main() -> Result<(), String> {
    if std::env::var("TYPR_LIVE_WRITE_TEST").as_deref() != Ok("1") {
        return Err("Set TYPR_LIVE_WRITE_TEST=1 to allow two live AI requests.".into());
    }
    let directory = dirs::config_dir().ok_or("Configuration directory unavailable.")?.join("com.typr.app");
    let settings = Settings::load(&directory);
    if settings.groq_api_key.trim().is_empty() { return Err("Configure your Groq key in Typr first.".into()); }
    let original = "Send the report today. Ask Jamie for two time slots tomorrow.";
    let first_instruction = "Format these as two hyphen bullets, keeping every word unchanged.";
    let first = write_mode::rewrite(original, first_instruction, None, &settings, &Cancellation::default()).await?;
    let expected_first = "- Send the report today.\n- Ask Jamie for two time slots tomorrow.";
    if first.trim() != expected_first { return Err("The selected AI model did not preserve the requested first edit exactly.".into()); }
    let latest = "In the second bullet, replace 'two time slots' with 'one time slot'. Leave the first bullet unchanged.";
    let second = write_mode::rewrite(&first, latest, Some(first_instruction), &settings, &Cancellation::default()).await?;
    if second.trim() != "- Send the report today.\n- Ask Jamie for one time slot tomorrow." {
        return Err("The selected AI model did not apply the latest instruction to the current passage exactly.".into());
    }
    println!("PASS: selected AI model produced two bullets and applied a follow-up change without reverting the first edit");
    Ok(())
}
