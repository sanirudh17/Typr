use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

use crate::ai_postprocess;
use crate::audio::AudioRecorder;
use crate::cleanup::cleanup_text;
use crate::commands;
use crate::dictionary::Dictionary;
use crate::paste::paste_text;
use crate::settings::Settings;
use crate::transcribe_local;
use crate::transcribe_groq;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum RecordingState {
    Ready,
    Recording,
    Transcribing,
}

fn update_overlay(app: &AppHandle, state: &RecordingState, show_pill: bool) {
    if let Some(overlay) = app.get_webview_window("overlay") {
        let pill_state = match state {
            RecordingState::Ready => "ready",
            RecordingState::Recording => "recording",
            RecordingState::Transcribing => "processing",
        };
        let js = format!(
            "if (window.__setPillState) window.__setPillState('{}'); else document.getElementById('pill').style.display = '{}';",
            pill_state,
            if show_pill { "flex" } else { "none" }
        );
        let _ = overlay.eval(&js);
    }
}

#[derive(Clone)]
pub struct Recorder {
    state: Arc<Mutex<RecordingState>>,
    audio_recorder: Arc<Mutex<AudioRecorder>>,
    // Per-session AI profile override (Secondary hotkey). Set when a recording starts and
    // taken when it stops, so it lives and dies with one session and can never leak forward
    // to the next dictation the way a shared slot could.
    session_override: Arc<Mutex<Option<String>>>,
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(RecordingState::Ready)),
            audio_recorder: Arc::new(Mutex::new(AudioRecorder::new())),
            session_override: Arc::new(Mutex::new(None)),
        }
    }

    pub fn pre_initialize(&self, mic_name: &str) -> Result<(), String> {
        let mut recorder = self.audio_recorder.lock().unwrap();
        recorder.ensure_initialized(mic_name).map(|_| ())
    }

    /// Begin the one-time device warm-up: play the pre-built stream so the audio device
    /// activates (paying the cold ~1-2s cost up front, off the record path).
    pub fn begin_warm(&self) {
        self.audio_recorder.lock().unwrap().device_play();
    }

    /// End the warm-up: settle the stream back to idle (mic off) — but never interrupt an
    /// active recording, so only pause if we're still Ready.
    pub fn end_warm(&self) {
        if *self.state.lock().unwrap() == RecordingState::Ready {
            self.audio_recorder.lock().unwrap().device_pause_idle();
        }
    }

    pub fn get_state(&self) -> RecordingState {
        self.state.lock().unwrap().clone()
    }

    pub fn get_amplitude(&self) -> Vec<f32> {
        self.audio_recorder.lock().unwrap().get_amplitude_ring()
    }
    
    pub fn get_frequency_bands(&self) -> Vec<f32> {
        self.audio_recorder.lock().unwrap().get_frequency_bands()
    }

    pub fn start_recording(
        &self,
        app: &AppHandle,
        mic_name: &str,
        session_override: Option<String>,
    ) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        if *state != RecordingState::Ready {
            return Err("Already recording or transcribing".to_string());
        }

        // Eagerly update the UI to eliminate perceived delay
        *state = RecordingState::Recording;
        // Bind the override to this session while we hold the state lock, so it is set
        // exactly once per recording and paired with the matching stop.
        *self.session_override.lock().unwrap() = session_override;
        let _ = app.emit("recording-state", RecordingState::Recording);
        update_overlay(app, &RecordingState::Recording, true);

        // Now start the audio recording
        let mut recorder = self.audio_recorder.lock().unwrap();
        match recorder.start(mic_name) {
            Ok(info) => {
                if info.fell_back || info.changed {
                    let _ = app.emit("mic-changed", serde_json::json!({
                        "device": info.active_device,
                        "fellBack": info.fell_back,
                    }));
                }
            }
            Err(e) => {
                // Revert state if starting failed, and drop the override so a session that
                // never actually recorded can't apply its profile to a later dictation.
                *state = RecordingState::Ready;
                *self.session_override.lock().unwrap() = None;
                let _ = app.emit("recording-state", RecordingState::Ready);
                update_overlay(app, &RecordingState::Ready, false);
                return Err(e);
            }
        }

        Ok(())
    }

    pub async fn stop_and_transcribe(
        &self,
        app: &AppHandle,
        settings: &Settings,
        history: &std::sync::Mutex<crate::history::History>,
        dictionary: &std::sync::Mutex<Dictionary>,
        app_dir: &PathBuf,
    ) -> Result<String, String> {
        let transcription_started_at = Instant::now();

        // Stop recording, taking this session's profile override in the same locked step
        // that transitions out of Recording — so exactly one stop consumes it.
        let session_override = {
            let mut state = self.state.lock().unwrap();
            if *state != RecordingState::Recording {
                return Err("Not currently recording".to_string());
            }
            *state = RecordingState::Transcribing;
            let _ = app.emit("recording-state", RecordingState::Transcribing);
            update_overlay(app, &RecordingState::Transcribing, true);
            self.session_override.lock().unwrap().take()
        };

        // Apply the override (if any) to a local copy of the live settings. The caller passes
        // the base settings; the override forces AI on with the chosen profile for this
        // dictation only.
        let effective_settings = apply_session_override(settings, session_override);
        let settings = &effective_settings;

        let temp_path = app_dir.join("temp_recording.wav");

        // Save audio
        let save_started_at = Instant::now();
        let save_result = {
            let mut recorder = self.audio_recorder.lock().unwrap();
            recorder.stop_and_save(&temp_path)
        };
        
        let duration_secs = match save_result {
            Ok((_, duration)) => duration,
            Err(e) => {
                let mut state = self.state.lock().unwrap();
                *state = RecordingState::Ready;
                let _ = app.emit("recording-state", RecordingState::Ready);
                update_overlay(app, &RecordingState::Ready, false);
                return Err(e);
            }
        };
        println!(
            "[Typr] Audio save and preprocessing completed in {:?}",
            save_started_at.elapsed()
        );

        // Transcribe
        let prompt = {
            let dict = dictionary.lock().unwrap();
            dict.get_bias_prompt()
        };

        let transcribe_result = match settings.engine.as_str() {
            "local" => {
                let model_path = app_dir.join(transcribe_local::model_filename(&settings.whisper_model));
                transcribe_local::transcribe_local(app, &model_path, &temp_path, &prompt).await
            }
            "cloud" => {
                transcribe_groq::transcribe_groq(&settings.groq_api_key, &temp_path, &prompt, &settings.cloud_model).await
            }
            _ => Err(format!("Unknown engine: {}", settings.engine)),
        };

        // Cleanup temp file
        let _ = std::fs::remove_file(&temp_path);

        // Keep the overlay in its "processing" state through dictionary replacement and the
        // AI cleanup pass below; we only reset to Ready just before pasting, so the spinner
        // covers the AI latency instead of clearing ~1-2s early. Every exit path resets.
        let raw_text = match transcribe_result {
            Ok(text) => text,
            Err(e) => {
                self.reset_ready(app);
                return Err(e);
            }
        };

        // Dictionary vocabulary correction (snap close mis-hearings to exact hint
        // spellings), then snippet/email replacements — both before the LLM.
        let replaced = {
            let dict = dictionary.lock().unwrap();
            let corrected = crate::vocab_correct::correct_vocabulary(&raw_text, &dict.vocabulary_hints);
            dict.apply_replacements(&corrected)
        };

        // Assemble spoken email addresses ("name at gmail dot com") into real ones. Must run
        // before both the AI pass and the deterministic cleanup: the LLM only promises to
        // preserve addresses it can recognize, and the entity guard can only protect one that
        // already looks like an address.
        let replaced = crate::email_assemble::assemble_emails(&replaced);

        // Deterministic cleanup is the always-available fallback.
        let deterministic = cleanup_text(&replaced);

        // Command-bearing dictations (casing/layout/symbols) go straight to the deterministic
        // path: the LLM would otherwise reword or half-apply the command phrases before the
        // command pass runs. Prompt Mode is exempt — its whole job is to rewrite the utterance.
        let bypass_ai_for_commands =
            settings.ai_profile != "prompt" && commands::contains_command(&replaced);
        if bypass_ai_for_commands {
            crate::debug_log::log(app_dir, "commands present -> raw text + command pass (skipping AI & prose cleanup)");
        }

        // Optional Groq LLM cleanup with a hard 2.5s budget. On off/offline/slow/error we
        // paste the deterministic result instead, so a dictation is never blocked.
        let final_text = if bypass_ai_for_commands {
            // Command/code dictation: skip prose cleanup too (no forced capitalization or
            // trailing period) so literal input like "claude --dangerously-skip-permissions"
            // is not sentence-formatted. The command pass below does the real work.
            replaced.clone()
        } else if settings.ai_enabled {
            let base_prompt = if settings.ai_profile == "auto" {
                let fg = crate::context_detector::ForegroundApp::detect();
                let focused_class = crate::context_detector::focused_child_class();
                let category = crate::context_detector::resolve_category(
                    &fg,
                    &settings.app_rules,
                    &focused_class,
                    &settings.auto_context_override,
                );
                // Metadata only: log the process name, focused window class, and resolved
                // category — never the window title (it can contain the user's content,
                // email address, etc.). Class names are generic and safe to log.
                crate::debug_log::log(
                    app_dir,
                    &format!(
                        "AUTO proc=\"{}\" class=\"{}\" -> {}",
                        fg.process_name, focused_class, category
                    ),
                );
                ai_postprocess::context_system_prompt(&category)
            } else {
                ai_postprocess::resolve_system_prompt(
                    &settings.ai_profile,
                    &settings.ai_prompt_format,
                )
            };
            // Base prompt + the never-refuse contract + the user's cross-profile style
            // modifiers (Tone / Formatting / Custom Instructions). The modifiers land last so an
            // explicit setting overrides the profile's built-in style, but they can never
            // loosen the contract.
            let system_prompt = ai_postprocess::build_system_prompt(
                base_prompt,
                &settings.ai_tone,
                &settings.ai_format,
                &settings.ai_custom_instructions,
            );
            let budget = ai_postprocess::budget_ms(&settings.ai_profile);
            let ai_started_at = Instant::now();
            let llm = match tokio::time::timeout(
                Duration::from_millis(budget),
                ai_postprocess::postprocess(
                    &settings.groq_api_key,
                    &replaced,
                    &settings.ai_model,
                    &system_prompt,
                ),
            )
            .await
            {
                Ok(Ok(clean)) => {
                    // Metadata only: timing/model/profile, never the dictated text itself.
                    crate::debug_log::log(
                        app_dir,
                        &format!(
                            "AI ok {}ms model={} profile={}",
                            ai_started_at.elapsed().as_millis(),
                            ai_postprocess::resolve_model(&settings.ai_model),
                            settings.ai_profile,
                        ),
                    );
                    Some(clean)
                }
                Ok(Err(e)) => {
                    crate::debug_log::log(app_dir, &format!("AI skipped (error): {}", e));
                    None
                }
                Err(_) => {
                    crate::debug_log::log(
                        app_dir,
                        &format!(
                            "AI skipped (exceeded {}ms budget) -> using deterministic cleanup",
                            budget
                        ),
                    );
                    None
                }
            };
            choose_final(llm, deterministic)
        } else {
            deterministic
        };

        // Final deterministic pass: apply always-on voice commands (casing / layout / symbols).
        // Runs after cleanup and any AI pass so nothing downstream can undo it; identical
        // behavior whether AI is on or off.
        let final_text = commands::apply_commands(&final_text);

        // Transcription + AI cleanup are done; clear the spinner now, then paste so the text
        // appears right as the overlay disappears. Resetting before paste also guarantees the
        // overlay clears even if paste_text errors below.
        self.reset_ready(app);

        // Auto-paste and record history
        if !final_text.is_empty() {
            paste_text(&final_text)?;
            let _ = history.lock().unwrap().add_item(final_text.clone(), duration_secs, app_dir);
            let _ = app.emit("history-updated", ());
        }

        println!(
            "[Typr] Full stop-to-text pipeline completed in {:?}",
            transcription_started_at.elapsed()
        );

        Ok(final_text)
    }

    /// Reset the recorder to Ready and clear the processing overlay. Called once the
    /// stop-to-text pipeline finishes (or on transcription error), so the spinner clears
    /// only after the AI cleanup pass rather than ~1-2s before the paste lands.
    fn reset_ready(&self, app: &AppHandle) {
        let mut state = self.state.lock().unwrap();
        *state = RecordingState::Ready;
        let _ = app.emit("recording-state", RecordingState::Ready);
        update_overlay(app, &RecordingState::Ready, false);
    }
}

/// Pick the final text to paste: the LLM output when it produced non-empty text, else the
/// deterministic fallback (LLM off, offline, slow, errored, or returned nothing).
fn choose_final(llm: Option<String>, fallback: String) -> String {
    match llm {
        Some(s) if !s.trim().is_empty() => s,
        _ => fallback,
    }
}

/// Produce the effective settings for a dictation given the session's profile override.
/// `Some(profile)` (a Secondary-hotkey session) forces AI on with that profile; `None`
/// (Primary hotkey) leaves the live settings untouched. Pure so the override semantics are
/// unit-testable without an audio device or Tauri handle.
pub fn apply_session_override(base: &Settings, session_override: Option<String>) -> Settings {
    let mut settings = base.clone();
    if let Some(profile) = session_override {
        settings.ai_enabled = true;
        settings.ai_profile = profile;
    }
    settings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state_is_ready() {
        let recorder = Recorder::new();
        assert_eq!(recorder.get_state(), RecordingState::Ready);
    }

    #[test]
    fn test_apply_session_override_none_is_passthrough() {
        // Primary hotkey (no override): settings unchanged, including the user's AI toggle.
        let mut base = Settings::default();
        base.ai_enabled = false;
        base.ai_profile = "cleanup".into();
        let effective = apply_session_override(&base, None);
        assert_eq!(effective.ai_enabled, false);
        assert_eq!(effective.ai_profile, "cleanup");
    }

    #[test]
    fn test_apply_session_override_forces_profile_and_enables_ai() {
        // Secondary hotkey: forces AI on with the chosen profile even if AI was off.
        let mut base = Settings::default();
        base.ai_enabled = false;
        base.ai_profile = "cleanup".into();
        let effective = apply_session_override(&base, Some("prompt".into()));
        assert_eq!(effective.ai_enabled, true);
        assert_eq!(effective.ai_profile, "prompt");
    }

    #[test]
    fn test_apply_session_override_does_not_mutate_base() {
        // The override applies to a copy; a subsequent no-override call sees the original,
        // so one session's override can never bleed into the next.
        let mut base = Settings::default();
        base.ai_enabled = false;
        base.ai_profile = "cleanup".into();
        let _forced = apply_session_override(&base, Some("prompt".into()));
        let next = apply_session_override(&base, None);
        assert_eq!(next.ai_enabled, false);
        assert_eq!(next.ai_profile, "cleanup");
    }

    #[test]
    fn test_choose_final_prefers_nonempty_llm() {
        assert_eq!(choose_final(Some("clean text".to_string()), "fallback".to_string()), "clean text");
    }

    #[test]
    fn test_choose_final_falls_back_on_empty_llm() {
        assert_eq!(choose_final(Some("   ".to_string()), "fallback".to_string()), "fallback");
    }

    #[test]
    fn test_choose_final_falls_back_on_none() {
        assert_eq!(choose_final(None, "fallback".to_string()), "fallback");
    }
}
