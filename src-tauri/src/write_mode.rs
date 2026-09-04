//! Write Mode: rewrite the user's current text selection in place.
//!
//! Triggered by the dedicated Write Mode hotkey — never by recording. The flow:
//! 1. Read the selection with the clipboard dance (stash clipboard → Ctrl+C →
//!    read → restore), which behaves exactly like the user pressing Ctrl+C.
//! 2. Rewrite it with the dedicated WRITE_MODE prompt (an AI feature: without
//!    AI enabled and a key there is nothing to rewrite with, so that is an
//!    error, not a silent no-op).
//! 3. Paste over the selection, which is still active, so the rewrite replaces it.
//!
//! Errors never paste: overwriting a selection with a mangled fallback would
//! destroy the user's text, and only the host app's undo could save it. Undo
//! itself is the host app's own Ctrl+Z — nothing custom, documented in Commands.

use std::time::Duration;
use tauri::Emitter;

use crate::ai_postprocess;
use crate::paste::paste_text;
use crate::settings::Settings;

/// Reported to the main window when a rewrite finishes, so it can toast the outcome.
#[derive(Clone, serde::Serialize)]
pub struct WriteModeResult {
    pub ok: bool,
    pub message: String,
}

/// Read the user's current selection via the clipboard dance.
///
/// Stashes the current clipboard text, sends Ctrl+C to the focused app, reads back
/// the selection, then restores the stash. A non-text clipboard cannot be stashed
/// through arboard's text API — in that case the selection is left on the clipboard
/// (exactly where a manual Ctrl+C would have put it) and the rewrite still proceeds.
pub fn capture_selection() -> Result<String, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    // Best-effort stash: a non-text clipboard (image, files) simply has no stash.
    let stash = clipboard.get_text().ok();

    send_copy()?;
    // Give the focused app a beat to populate the clipboard, mirroring the
    // propagation delay in paste_text.
    std::thread::sleep(Duration::from_millis(200));
    let selected = clipboard.get_text().map_err(|_| {
        "No text selected — select some text first, then press the Write Mode hotkey.".to_string()
    })?;

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

/// Run one Write Mode pass: capture → rewrite → replace. Returns the rewritten
/// text on success. Shows the processing spinner around the AI call and reports
/// the outcome on the write-mode-result event for the main window to toast.
pub async fn run_write_mode(
    app: &tauri::AppHandle,
    settings: &Settings,
    app_dir: &std::path::Path,
) -> Result<String, String> {
    // Dictionary replacements fix speech mis-hearings — applying them to text the
    // user wrote themselves could corrupt code and identifiers, so the selection
    // goes to the model verbatim.

    if !settings.ai_enabled {
        return Err("Write Mode needs AI cleanup — turn it on in Settings → AI first.".to_string());
    }
    if settings.groq_api_key.trim().is_empty() {
        return Err("Write Mode needs a Groq API key — add one in Settings → Engine or AI.".to_string());
    }

    let selected = capture_selection()?;
    crate::recorder::Recorder::set_overlay_processing(app, true);

    let system_prompt = ai_postprocess::build_system_prompt(
        ai_postprocess::write_mode_system_prompt(),
        &settings.ai_tone,
        &settings.ai_format,
        &settings.ai_custom_instructions,
    );
    // Rewrite output scales with the selection, which can be paragraphs — use the
    // generous prompt budget, not the short cleanup one.
    let budget = ai_postprocess::budget_ms("prompt");
    let rewritten = match tokio::time::timeout(
        Duration::from_millis(budget),
        ai_postprocess::postprocess_with_fallback(
            &settings.groq_api_key,
            &selected,
            &settings.ai_model,
            &system_prompt,
        ),
    )
    .await
    {
        Ok(Ok((clean, used_model))) => {
            // Metadata only: timing/model/size, never the selected text itself.
            crate::debug_log::log(
                app_dir,
                &format!("Write Mode ok model={} chars={}", used_model, clean.len()),
            );
            clean
        }
        Ok(Err(e)) => {
            crate::recorder::Recorder::set_overlay_processing(app, false);
            return Err(format!("Rewrite failed ({}). Your text was left untouched.", e));
        }
        Err(_) => {
            crate::recorder::Recorder::set_overlay_processing(app, false);
            return Err(format!(
                "Rewrite timed out after {}s. Your text was left untouched.",
                budget / 1000
            ));
        }
    };

    if rewritten.trim().is_empty() {
        crate::recorder::Recorder::set_overlay_processing(app, false);
        return Err("The rewrite came back empty. Your text was left untouched.".to_string());
    }

    // The selection is still active, so pasting replaces it. From here a failure
    // still reports — but the clipboard already holds the rewrite, so nothing is lost.
    paste_text(&rewritten).map_err(|e| format!("Rewrite ready but paste failed: {}", e))?;
    crate::recorder::Recorder::set_overlay_processing(app, false);
    let _ = app.emit(
        "write-mode-result",
        WriteModeResult { ok: true, message: format!("Rewrote {} characters.", rewritten.chars().count()) },
    );
    Ok(rewritten)
}

