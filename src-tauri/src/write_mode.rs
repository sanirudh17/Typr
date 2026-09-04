//! Write Mode: dictate the change, keep the selection.
//!
//! Select text in any app, press the Write Mode hotkey, dictate what to do with
//! it ("make this formal", "fix the spelling", "turn this into bullets"), press
//! the hotkey again (or release it in Push to Talk) — the overlay records and
//! transcribes exactly like a dictation, then the AI applies the spoken
//! instruction to the selection and the result replaces it.
//!
//! Timing is the whole design: opening the mic touches nothing clipboard-
//! related, so the overlay pops the instant the hotkey is pressed. All
//! clipboard work (capture, paste) waits until after key release — running
//! the Ctrl+C dance while the hotkey's modifiers are still held injects
//! stray keystrokes into the target app. The selection is therefore captured
//! at finish time, and pasting replaces exactly what was just captured.
//!
//! Safety rules:
//! - Errors never paste: a failed transcription, capture, or rewrite leaves
//!   the selection untouched and toasts why.
//! - Dictionary replacements fix speech mis-hearings — the *instruction* is
//!   transcribed with the dictionary bias like any dictation, but the *selection*
//!   goes to the model verbatim so code and identifiers survive.
//! - Undo is the host app's own Ctrl+Z — nothing custom, documented in Commands.

use std::time::Duration;
use tauri::Emitter;

use crate::ai_postprocess;
use crate::paste::paste_text;
use crate::settings::Settings;

/// Reported to the main window when Write Mode acts, so it can toast the outcome.
#[derive(Clone, serde::Serialize)]
pub struct WriteModeResult {
    pub ok: bool,
    pub message: String,
}

/// Read the user's current selection via the clipboard dance.
///
/// Stashes the current clipboard text, CLEARS the clipboard, sends Ctrl+C to the
/// focused app, then polls until non-empty text appears (or ~1.2s passes), and
/// finally restores the stash. Clearing first is what makes this deterministic:
/// a fixed sleep races slow apps and returns stale clipboard content as if it
/// were the selection — polling for the post-clear arrival cannot misread.
/// A non-text clipboard cannot be stashed through arboard's text API — in that
/// case the selection is left on the clipboard (exactly where a manual Ctrl+C
/// would have put it) and the flow still proceeds.
pub fn capture_selection() -> Result<String, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    // Best-effort stash: a non-text clipboard (image, files) simply has no stash.
    let stash = clipboard.get_text().ok();
    // Best-effort clear: even if it fails, the poll below still distinguishes a
    // fresh copy from the stash by value.
    let _ = clipboard.clear();

    send_copy()?;
    let selected = poll_clipboard_text(&mut clipboard, stash.as_deref())?;

    // Restore what was there before we borrowed the clipboard.
    if let Some(text) = stash {
        let _ = clipboard.set_text(text);
    }

    if selected.trim().is_empty() {
        return Err(
            "No text selected — select some text first, then press the Write Mode hotkey."
                .to_string(),
        );
    }
    Ok(selected)
}

/// Wait for the focused app to answer our Ctrl+C: any non-empty clipboard content
/// that differs from the pre-copy stash is the fresh selection. Times out with a
/// guidance error instead of returning stale text.
fn poll_clipboard_text(
    clipboard: &mut arboard::Clipboard,
    stash: Option<&str>,
) -> Result<String, String> {
    for _ in 0..24 {
        if let Ok(text) = clipboard.get_text() {
            if !text.is_empty() && Some(text.as_str()) != stash {
                return Ok(text);
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // One last read: the app may have answered between the final poll and now.
    if let Ok(text) = clipboard.get_text() {
        if !text.is_empty() && Some(text.as_str()) != stash {
            return Ok(text);
        }
    }
    Err("Could not grab the selection — select some text first, then press the Write Mode hotkey.".to_string())
}

/// Send Ctrl+C to the focused app. Windows-only for now, matching paste_text —
/// the macOS arm would need the osascript path paste_text already uses.
fn send_copy() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use enigo::{Direction, Enigo, Key, Keyboard, Settings};
        let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
        enigo.key(Key::Control, Direction::Press).map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_millis(15));
        enigo.key(Key::C, Direction::Press).map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_millis(15));
        enigo.key(Key::C, Direction::Release).map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_millis(15));
        enigo.key(Key::Control, Direction::Release).map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("Write Mode is not supported on this platform yet.".to_string())
    }
}

/// The model input for one rewrite: the stashed selection plus the transcribed
/// instruction, in fixed labeled blocks so the two can never bleed together.
pub fn write_mode_user_content(selected: &str, instruction: &str) -> String {
    format!(
        "Selected text:\n<<<{}>>>\n\nSpoken instruction:\n<<<{}>>>",
        selected, instruction
    )
}

/// Press one: validate, open the mic. The overlay pops instantly because this
/// touches nothing clipboard-related — safe the moment the hotkey is pressed,
/// even with the combo's modifiers still physically held. The selection is
/// captured at finish time instead (see below), once the keys are released.
pub fn begin_write_mode(
    app: &tauri::AppHandle,
    recorder: &crate::recorder::Recorder,
    settings: &Settings,
) -> Result<(), String> {
    if !settings.ai_enabled {
        return Err("Write Mode needs AI cleanup — turn it on in Settings → AI first.".to_string());
    }
    if settings.groq_api_key.trim().is_empty() {
        return Err("Write Mode needs a Groq API key — add one in Settings → Engine or AI.".to_string());
    }
    recorder.start_write_session(app, &settings.microphone)?;
    Ok(())
}

/// Finish: stop the audio, transcribe the instruction, capture the selection
/// fresh (keys are released by now), apply, paste. Claiming the session first
/// makes a duplicate finisher go quiet instead of transcribing twice.
/// Reports success on the write-mode-result event; every failure toasts and
/// leaves the user's text untouched.
pub async fn finish_write_mode(
    app: &tauri::AppHandle,
    recorder: &crate::recorder::Recorder,
    settings: &Settings,
    app_dir: &std::path::Path,
    bias_prompt: &str,
) -> Result<String, String> {
    if !recorder.take_write_session() {
        return Ok(String::new());
    }
    recorder.clear_write_finish_arm();
    let (wav_path, _duration) = recorder.stop_write_audio(app, &app_dir.to_path_buf())?;
    // The instruction transcribes like any dictation — dictionary bias included.
    let transcribed =
        crate::recorder::transcribe_audio(app, settings, &app_dir.to_path_buf(), &wav_path, bias_prompt).await;
    let _ = std::fs::remove_file(&wav_path);
    let instruction = transcribed
        .map_err(|e| format!("Did not catch that ({}). Your text was left untouched.", e))?;
    if instruction.trim().is_empty() {
        recorder.complete_write_session(app);
        return Err("Did not catch that — dictate the change again. Your text was left untouched.".to_string());
    }

    // Captured now — not at press time — so the Ctrl+C dance never collides
    // with the hotkey's own held modifiers (that collision injected stray
    // keystrokes into the target app). Whatever is selected at this moment is
    // what the user wants rewritten.
    let selected = capture_selection().map_err(|e| {
        recorder.complete_write_session(app);
        e
    })?;

    crate::recorder::Recorder::set_overlay_processing(app, true);
    let system_prompt = ai_postprocess::build_system_prompt(
        ai_postprocess::write_mode_system_prompt(),
        &settings.ai_tone,
        &settings.ai_format,
        &settings.ai_custom_instructions,
    );
    // Output scales with the selection, which can be paragraphs — the generous
    // prompt budget, not the short cleanup one.
    let budget = ai_postprocess::budget_ms("prompt");
    let user_content = write_mode_user_content(&selected, &instruction);
    let rewritten = match tokio::time::timeout(
        Duration::from_millis(budget),
        ai_postprocess::postprocess_with_fallback(
            &settings.groq_api_key,
            &user_content,
            &settings.ai_model,
            &system_prompt,
        ),
    )
    .await
    {
        Ok(Ok((clean, used_model))) => {
            // Metadata only: timing/model/size, never the text itself.
            crate::debug_log::log(
                app_dir,
                &format!("Write Mode ok model={} chars={}", used_model, clean.len()),
            );
            clean
        }
        Ok(Err(e)) => {
            recorder.complete_write_session(app);
            return Err(format!("Rewrite failed ({}). Your text was left untouched.", e));
        }
        Err(_) => {
            recorder.complete_write_session(app);
            return Err(format!(
                "Rewrite timed out after {}s. Your text was left untouched.",
                budget / 1000
            ));
        }
    };

    if rewritten.trim().is_empty() {
        recorder.complete_write_session(app);
        return Err("The rewrite came back empty. Your text was left untouched.".to_string());
    }
    crate::recorder::Recorder::set_overlay_processing(app, false);

    // The AI call took seconds, and the user may have clicked away since the
    // capture — collapsing the selection. Pasting then would INSERT at the
    // cursor instead of replacing (duplicating text on every run), so
    // re-capture and compare first: paste only on match, otherwise park the
    // rewrite on the clipboard where nothing is lost and nothing is doubled.
    match capture_selection() {
        Ok(now) if now == selected => {
            paste_text(&rewritten).map_err(|e| format!("Rewrite ready but paste failed: {}", e))?;
            let _ = app.emit(
                "write-mode-result",
                WriteModeResult { ok: true, message: format!("Rewrote {} characters.", rewritten.chars().count()) },
            );
            Ok(rewritten)
        }
        Ok(_) => {
            eprintln!("[Typr] Write Mode: selection changed mid-flight — parking rewrite on clipboard");
            park_on_clipboard(&rewritten)?;
            let _ = app.emit(
                "write-mode-result",
                WriteModeResult {
                    ok: true,
                    message: "Selection changed — rewrite copied to the clipboard, paste it where you want it.".to_string(),
                },
            );
            Ok(rewritten)
        }
        Err(_) => {
            eprintln!("[Typr] Write Mode: selection lost before paste — parking rewrite on clipboard");
            park_on_clipboard(&rewritten)?;
            let _ = app.emit(
                "write-mode-result",
                WriteModeResult {
                    ok: true,
                    message: "Selection was lost — rewrite copied to the clipboard, paste it where you want it.".to_string(),
                },
            );
            Ok(rewritten)
        }
    }
}

/// Park text on the clipboard when pasting would land it wrong. Infallible from
/// the caller's view only in that a clipboard failure is reported, never silent.
fn park_on_clipboard(text: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard.set_text(text).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_content_keeps_blocks_labeled_and_separate() {
        let u = write_mode_user_content("hello wrold", "fix the spelling");
        assert!(u.contains("Selected text:"));
        assert!(u.contains("Spoken instruction:"));
        assert!(u.contains("hello wrold"));
        assert!(u.contains("fix the spelling"));
        // Selection first: the material precedes the direction.
        assert!(u.find("hello wrold") < u.find("fix the spelling"));
    }
}

