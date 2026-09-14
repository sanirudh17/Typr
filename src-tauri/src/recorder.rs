use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

use crate::ai_postprocess;
use crate::audio::AudioRecorder;
use crate::cleanup::cleanup_text;
use crate::commands;
use crate::dictionary::Dictionary;
use crate::paste::paste_text;
use crate::settings::Settings;
use crate::transcribe_local;
use crate::transcribe_groq;
use crate::transcribe_parakeet;
use crate::transcribe_nemotron;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum RecordingState {
    Ready,
    Recording,
    Transcribing,
}

fn update_overlay(app: &AppHandle, state: &RecordingState, show_pill: bool) {
    crate::overlay::update_overlay(app, state, show_pill);
}

#[derive(Clone)]
pub struct Recorder {
    state: Arc<Mutex<RecordingState>>,
    audio_recorder: Arc<Mutex<AudioRecorder>>,
    // Per-session AI profile override (Secondary hotkey). Set when a recording starts and
    // taken when it stops, so it lives and dies with one session and can never leak forward
    // to the next dictation the way a shared slot could.
    session_override: Arc<Mutex<Option<String>>>,
    // Focused foreground app and window class captured at the moment recording begins.
    // Preserves the user's active window context even if focus shifts during speech or processing.
    session_context: Arc<Mutex<Option<(crate::context_detector::ForegroundApp, String)>>>,
    // Live streaming session for Nemotron transducer to ingest and decode chunks during speech
    nemotron_session: Arc<Mutex<Option<crate::transcribe_nemotron::NemotronLiveSession>>>,
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(RecordingState::Ready)),
            audio_recorder: Arc::new(Mutex::new(AudioRecorder::new())),
            session_override: Arc::new(Mutex::new(None)),
            session_context: Arc::new(Mutex::new(None)),
            nemotron_session: Arc::new(Mutex::new(None)),
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
        engine: &str,
        nemotron_model_dir: Option<&std::path::Path>,
        input_gain_db: f32,
    ) -> Result<(), String> {
        // 1. Transition state to Recording while holding the state lock briefly
        {
            let mut state = self.state.lock().unwrap();
            if *state != RecordingState::Ready {
                return Err("Already recording or transcribing".to_string());
            }

            // Eagerly update the UI to eliminate perceived delay
            *state = RecordingState::Recording;
            // Bind the override to this session while we hold the state lock, so it is set
            // exactly once per recording and paired with the matching stop.
            *self.session_override.lock().unwrap() = session_override;
            // Snapshot the focused window at the moment of hotkey trigger
            let init_fg = crate::context_detector::ForegroundApp::detect();
            let init_class = crate::context_detector::focused_child_class();
            *self.session_context.lock().unwrap() = Some((init_fg, init_class));
            let _ = app.emit("recording-state", RecordingState::Recording);
            update_overlay(app, &RecordingState::Recording, true);
        }

        // 2. Start audio stream holding the audio recorder lock briefly
        let start_res = {
            let mut recorder = self.audio_recorder.lock().unwrap();
            recorder.start(mic_name)
        };

        let info = match start_res {
            Ok(info) => info,
            Err(e) => {
                // Revert state if starting failed, and drop the override so a session that
                // never actually recorded can't apply its profile to a later dictation.
                let mut state = self.state.lock().unwrap();
                *state = RecordingState::Ready;
                *self.session_override.lock().unwrap() = None;
                *self.session_context.lock().unwrap() = None;
                *self.nemotron_session.lock().unwrap() = None;
                let _ = app.emit("recording-state", RecordingState::Ready);
                update_overlay(app, &RecordingState::Ready, false);
                return Err(e);
            }
        };

        if info.fell_back || info.changed {
            let _ = app.emit("mic-changed", serde_json::json!({
                "device": info.active_device,
                "fellBack": info.fell_back,
            }));
        }

        // 3. If engine is Nemotron, initiate live streaming ingestion with NO state or recorder locks held
        if engine == "nemotron" {
            if let Some(model_dir) = nemotron_model_dir {
                match crate::transcribe_nemotron::start_live_session(model_dir, self.audio_recorder.clone(), input_gain_db) {
                    Ok(session) => {
                        *self.nemotron_session.lock().unwrap() = Some(session);
                        println!("[Typr] Started Nemotron live streaming session");
                    }
                    Err(e) => {
                        eprintln!("[Typr] Could not start Nemotron live streaming session: {}", e);
                        *self.nemotron_session.lock().unwrap() = None;
                    }
                }
            }
        } else {
            *self.nemotron_session.lock().unwrap() = None;
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

        // Stop recording, taking this session's profile override and initial context snapshot
        // in the same locked step that transitions out of Recording — so exactly one stop consumes them.
        let (session_override, session_context) = {
            let mut state = self.state.lock().unwrap();
            if *state != RecordingState::Recording {
                return Err("Not currently recording".to_string());
            }
            *state = RecordingState::Transcribing;
            let _ = app.emit("recording-state", RecordingState::Transcribing);
            update_overlay(app, &RecordingState::Transcribing, true);
            (
                self.session_override.lock().unwrap().take(),
                self.session_context.lock().unwrap().take(),
            )
        };

        // Apply the override (if any) to a local copy of the live settings. The caller passes
        // the base settings; the override forces AI on with the chosen profile for this
        // dictation only.
        let effective_settings = apply_session_override(settings, session_override);
        let settings = &effective_settings;

        let temp_path = app_dir.join("temp_recording.wav");

        // Buffer drain: wait 100ms so in-flight audio buffers in the OS/WASAPI driver
        // are delivered to the cpal stream callback before the stream is paused.
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Save audio
        let save_started_at = Instant::now();
        let save_result = {
            let mut recorder = self.audio_recorder.lock().unwrap();
            recorder.stop_and_save(&temp_path, settings.input_gain_db, Some(app))
        };
        
        let duration_secs = match save_result {
            Ok((_, duration)) => duration,
            Err(e) => {
                let mut state = self.state.lock().unwrap();
                *state = RecordingState::Ready;
                *self.nemotron_session.lock().unwrap() = None;
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
            "parakeet" => {
                let model_dir = app_dir
                    .join(transcribe_parakeet::model_dir_name(&settings.parakeet_model));
                // No prompt: transducer models have no equivalent of Whisper's prompt window,
                // so the dictionary bias the other two engines receive has nowhere to go here.
                transcribe_parakeet::transcribe_parakeet(&model_dir, &temp_path).await
            }
            "nemotron" => {
                let live_session = self.nemotron_session.lock().unwrap().take();
                let live_text = if let Some(session) = live_session {
                    match session.finish() {
                        Ok(text) if !text.trim().is_empty() => Some(text),
                        Ok(_) => None,
                        Err(e) => {
                            eprintln!("[Typr] Nemotron live streaming error: {}, falling back to batch", e);
                            None
                        }
                    }
                } else {
                    None
                };

                let live_wps = live_text
                    .as_ref()
                    .map(|t| t.split_whitespace().count() as f32 / duration_secs.max(0.1))
                    .unwrap_or(0.0);

                if let Some(text) = live_text {
                    if duration_secs > 5.0 && live_wps < 0.6 {
                        println!(
                            "[Typr] Nemotron live transcript sparse ({:.2} wps for {:.1}s), checking batch fallback",
                            live_wps, duration_secs
                        );
                        let model_dir = app_dir
                            .join(transcribe_nemotron::model_dir_name(&settings.nemotron_model));
                        match transcribe_nemotron::transcribe_nemotron(&model_dir, &temp_path).await {
                            Ok(batch_text) if batch_text.split_whitespace().count() > text.split_whitespace().count() => {
                                Ok(batch_text)
                            }
                            _ => Ok(text),
                        }
                    } else {
                        Ok(text)
                    }
                } else {
                    let model_dir = app_dir
                        .join(transcribe_nemotron::model_dir_name(&settings.nemotron_model));
                    transcribe_nemotron::transcribe_nemotron(&model_dir, &temp_path).await
                }
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

        // Transcript integrity check: detect truncation on long audio recordings.
        let word_count = raw_text.split_whitespace().count();
        let words_per_second = word_count as f64 / (duration_secs.max(0.001) as f64);
        if duration_secs > 15.0 && words_per_second < 1.5 {
            let warn_msg = format!(
                "WARNING: Transcript integrity check failed: {:.1}s audio yielded {} words ({:.2} wps < 1.5 wps threshold)",
                duration_secs, word_count, words_per_second
            );
            crate::debug_log::log(app_dir, &warn_msg);
            let toast_msg = format!(
                "Transcript looks incomplete — you dictated {:.0}s but got only {} words. Re-dictate or check the Engine tab.",
                duration_secs, word_count
            );
            let _ = app.emit("show-toast", toast_msg);
        }

        crate::debug_log::log(app_dir, &format!("STAGE 1 [engine return]: {}", raw_text));

        // Dictionary vocabulary correction (snap close mis-hearings to exact hint
        // spellings), then snippet/email replacements — both before the LLM.
        let replaced = {
            let dict = dictionary.lock().unwrap();
            let corrected = crate::vocab_correct::correct_vocabulary(&raw_text, &dict.vocabulary_hints);
            dict.apply_replacements(&corrected)
        };
        crate::debug_log::log(app_dir, &format!("STAGE 2 [dictionary]: {}", replaced));

        // Assemble spoken email addresses ("name at gmail dot com") into real ones. Must run
        // before both the AI pass and the deterministic cleanup: the LLM only promises to
        // preserve addresses it can recognize, and the entity guard can only protect one that
        // already looks like an address.
        let replaced = crate::email_assemble::assemble_emails(&replaced);
        crate::debug_log::log(app_dir, &format!("STAGE 3 [email assembly]: {}", replaced));

        // Deterministic cleanup is the always-available fallback.
        let deterministic = cleanup_text(&replaced);
        // Filler safety net: when AI is on and the user opted to strip filler in
        // Developer/Terminal, ensure filler never survives a bypass or an LLM miss.
        // The AI prompt already asks to strip filler; this is the deterministic
        // guarantee for the fallback paths where the LLM is not in the loop.
        // Always strip filler when AI is on — the former Developer toggle has been removed per user request; filler is never intentional when AI cleanup is enabled.
        let strip_filler = settings.ai_enabled;
        let replaced_stripped = if strip_filler {
            crate::cleanup::strip_filler_words(&replaced)
        } else {
            replaced.clone()
        };
        let deterministic_stripped = if strip_filler {
            crate::cleanup::strip_filler_words(&deterministic)
        } else {
            deterministic.clone()
        };

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
            // When strip_filler is on, use the filler-stripped raw so "um git status"
            // does not keep the "um".
            if strip_filler { replaced_stripped.clone() } else { replaced.clone() }
        } else if settings.ai_enabled {
            // Set when the foreground surface is a terminal: even the AI *fallback* must be
            // raw, never the prose-formatted deterministic cleanup — a command pasted with
            // sentence capitalization/periods is a corrupted command.
            let mut terminal_focus_once = false;
            let base_prompt = if settings.ai_profile == "auto" {
                // Prefer the foreground app captured at recording start (when user triggered dictation)
                let (fg, focused_class) = match session_context {
                    Some((f, c)) if !f.process_name.is_empty() => (f, c),
                    _ => (
                        crate::context_detector::ForegroundApp::detect(),
                        crate::context_detector::focused_child_class(),
                    ),
                };
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
                if is_terminal_focus(&category, &fg.process_name, &focused_class) {
                    // A real terminal surface within the Developer context: the dictation is
                    // usually a command to run, so the pass must return the literal text to
                    // type (spoken symbols converted, filler stripped) instead of the general
                    // Developer restyle that condenses commands into commit-message prose.
                    crate::debug_log::log(
                        app_dir,
                        "terminal focus -> literal-transcription AI prompt",
                    );
                    terminal_focus_once = true;
                    ai_postprocess::terminal_system_prompt()
                } else {
                    ai_postprocess::context_system_prompt(&category)
                }
            } else {
                ai_postprocess::resolve_system_prompt(
                    &settings.ai_profile,
                    &settings.ai_prompt_format,
                )
            };
            let vocab_hints = {
                let dict = dictionary.lock().unwrap();
                dict.vocabulary_hints.clone()
            };
            let base_with_vocab = append_vocabulary_hints(base_prompt, &vocab_hints);

            // Base prompt + the never-refuse contract + the user's cross-profile style
            // modifiers (Tone / Formatting / Custom Instructions). The modifiers land last so an
            // explicit setting overrides the profile's built-in style, but they can never
            // loosen the contract.
            let system_prompt = ai_postprocess::build_system_prompt(
                &base_with_vocab,
                &settings.ai_tone,
                &settings.ai_format,
                &settings.ai_custom_instructions,
            );
            let budget = ai_postprocess::budget_ms(&settings.ai_profile);
            let ai_started_at = Instant::now();
            let llm = match tokio::time::timeout(
                Duration::from_millis(budget),
                ai_postprocess::postprocess_with_fallback(
                    &settings.groq_api_key,
                    &replaced,
                    &settings.ai_model,
                    &system_prompt,
                ),
            )
            .await
            {
                Ok(Ok((clean, used_model))) => {
                    // Metadata only: timing/model/profile, never the dictated text itself.
                    // `used_model` is the model that produced this text, which is not always the
                    // one selected in settings — a fallback retry must be visible in the log.
                    crate::debug_log::log(
                        app_dir,
                        &format!(
                            "AI ok {}ms model={} profile={}",
                            ai_started_at.elapsed().as_millis(),
                            used_model,
                            settings.ai_profile,
                        ),
                    );
                    guard_ai_output(clean, &replaced, &settings.ai_profile, app_dir)
                }
                Ok(Err(e)) => {
                    crate::debug_log::log(app_dir, &format!("AI skipped (error): {}", e));
                    None
                }
                Err(_) => {
                    crate::debug_log::log(
                        app_dir,
                        &format!(
                            "AI skipped (exceeded {}ms budget){}",
                            budget,
                            if terminal_focus_once {
                                " -> terminal focus: raw fallback"
                            } else {
                                " -> using deterministic cleanup"
                            }
                        ),
                    );
                    None
                }
            };
            // Terminal surface: on any LLM miss (slow/offline/error/empty) paste the raw
            // transcription rather than the prose-formatted cleanup. Everything else keeps
            // the deterministic result. When strip_filler is on, the raw fallback is the
            // filler-stripped version so filler never leaks.
            let mut final_text_inner = if terminal_focus_once {
                choose_final(llm, if strip_filler { replaced_stripped.clone() } else { replaced.clone() })
            } else {
                choose_final(llm, if strip_filler { deterministic_stripped.clone() } else { deterministic.clone() })
            };
            // Safety net: if the LLM did return text but left filler in (model didn't obey),
            // strip it deterministically when the user opted in. Applies to both terminal
            // and IDE developer surfaces; the filler list is the unambiguous one (um/uh/etc.)
            // so it never mangles code identifiers.
            if strip_filler {
                final_text_inner = crate::cleanup::strip_filler_words(&final_text_inner);
            }
            final_text_inner
        } else {
            deterministic
        };
        crate::debug_log::log(app_dir, &format!("STAGE 4 [AI postprocess]: {}", final_text));

        // Deterministic de-duplication: collapse consecutive repeated words/phrases
        // (1-3 word window) that Whisper/Parakeet and chunk joins sometimes emit even
        // when AI post-processing is off or the AI kept the stutter. This fixes "words
        // being repeated despite said only once" without touching non-consecutive repeats.
        // Runs before voice commands so "hello hello" dedupes to one hello before any
        // casing/layout pass. Always on — stutters are never intentional.
        let final_text = crate::cleanup::deduplicate_text(&final_text);
        crate::debug_log::log(app_dir, &format!("STAGE 5 [dedup]: {}", final_text));

        // Final deterministic pass: apply always-on voice commands (casing / layout / symbols).
        // Runs after cleanup and any AI pass so nothing downstream can undo it; identical
        // behavior whether AI is on or off.
        let final_text = commands::apply_commands(&final_text);
        crate::debug_log::log(app_dir, &format!("STAGE 6 [commands]: {}", final_text));

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
        *self.nemotron_session.lock().unwrap() = None;
        let _ = app.emit("recording-state", RecordingState::Ready);
        update_overlay(app, &RecordingState::Ready, false);
    }
}

/// True when the foreground surface is a terminal inside the Developer context: the Auto
/// profile resolved Developer AND the surface is a terminal (by process or by focused
/// child window class). Such dictation gets the literal-transcription AI prompt (commands
/// typed back verbatim, spoken symbols converted) instead of the general Developer
/// restyle. Pure.
fn is_terminal_focus(
    category: &crate::context_detector::ContextCategory,
    process_name: &str,
    focused_class: &str,
) -> bool {
    *category == crate::context_detector::ContextCategory::Developer
        && (crate::context_detector::is_terminal_process(process_name)
            || crate::context_detector::is_native_terminal_class(focused_class))
}

/// Pick the final text to paste: the LLM output when it produced non-empty text, else the
/// deterministic fallback (LLM off, offline, slow, errored, or returned nothing).
fn choose_final(llm: Option<String>, fallback: String) -> String {
    match llm {
        Some(s) if !s.trim().is_empty() => s,
        _ => fallback,
    }
}

/// Inject user dictionary hints and built-in speech-to-text / developer terms into the base prompt
/// so that phonetic mis-hearings (e.g. "separate, and this bar" -> "Parakeet and Whisper",
/// "hand of" -> "handoff", "open code" -> "OpenCode", "Shift plus Pix" -> "Shift+A") are accurately corrected by the LLM.
pub fn append_vocabulary_hints(base_prompt: &str, user_hints: &[String]) -> String {
    let mut terms: Vec<String> = vec![
        "Parakeet".into(),
        "Whisper".into(),
        "Nemotron".into(),
        "OpenCode".into(),
        "Orca".into(),
        "handoff".into(),
        "screenshot".into(),
    ];
    for hint in user_hints {
        let trimmed = hint.trim();
        if !trimmed.is_empty() && !terms.iter().any(|t| t.eq_ignore_ascii_case(trimmed)) {
            terms.push(trimmed.to_string());
        }
    }
    if !terms.is_empty() {
        format!(
            "{}\n\nRecognized Vocabulary & Technical Terms:\nThe user and codebase frequently use the following terms. When speech-to-text produces phonetically similar words or garbled fragments, prioritize matching against these terms:\n{}",
            base_prompt,
            terms.join(", ")
        )
    } else {
        base_prompt.to_string()
    }
}

/// Tokenize a text into lowercase alphanumeric words for multiset comparison.
pub fn tokenize_words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect()
}

/// Compute case-insensitive vocabulary grounding (fraction of AI content words originating
/// from the transcript), expansion deviation, and refusal detection.
pub fn check_ai_divergence(pre_text: &str, ai_text: &str) -> (f32, f32, bool) {
    const REFUSAL_PATTERNS: &[&str] = &[
        "i'm sorry", "i am sorry", "i cannot", "i can't",
        "as an ai", "as a language model", "i am unable",
    ];
    let ai_lower = ai_text.to_lowercase();
    let pre_lower = pre_text.to_lowercase();
    for pat in REFUSAL_PATTERNS {
        if ai_lower.contains(pat) && !pre_lower.contains(pat) {
            return (0.0, 0.0, true);
        }
    }

    let pre_tokens = tokenize_words(pre_text);
    let ai_tokens = tokenize_words(ai_text);

    if pre_tokens.is_empty() && ai_tokens.is_empty() {
        return (1.0, 0.0, false);
    }
    if pre_tokens.is_empty() || ai_tokens.is_empty() {
        return (0.0, 1.0, true);
    }

    let pre_len = pre_tokens.len() as f32;
    let ai_len = ai_tokens.len() as f32;
    let len_delta = if ai_len > pre_len {
        (ai_len - pre_len) / pre_len
    } else {
        0.0
    };

    let pre_set: std::collections::HashSet<&str> = pre_tokens.iter().map(|s| s.as_str()).collect();
    let ai_unique: std::collections::HashSet<&str> = ai_tokens.iter().map(|s| s.as_str()).collect();

    // Filter out 1-letter words for grounding calculation unless that's all there is
    let ai_content: std::collections::HashSet<&str> = ai_unique
        .iter()
        .copied()
        .filter(|w| w.len() > 1)
        .collect();
    let (target_len, matched_count) = if ai_content.is_empty() {
        let matched = ai_unique.iter().filter(|w| pre_set.contains(*w)).count();
        (ai_unique.len(), matched)
    } else {
        let matched = ai_content.iter().filter(|w| pre_set.contains(*w)).count();
        (ai_content.len(), matched)
    };

    let grounding = matched_count as f32 / target_len.max(1) as f32;

    let diverged = if pre_len <= 5.0 {
        matched_count == 0
    } else {
        grounding < 0.25 || (ai_len > 15.0 && ai_len > pre_len * 1.7)
    };

    (grounding, len_delta, diverged)
}

/// Guard against AI hallucination or text expansion by checking divergence against
/// the pre-AI transcript. Returns Some(clean) if valid, or None if diverged (triggering
/// deterministic fallback in choose_final). Prompt mode is exempt.
pub fn guard_ai_output(clean: String, pre_text: &str, ai_profile: &str, app_dir: &std::path::Path) -> Option<String> {
    if ai_profile == "prompt" {
        return Some(clean);
    }
    let (overlap, len_delta, diverged) = check_ai_divergence(pre_text, &clean);
    if diverged {
        crate::debug_log::log(
            app_dir,
            &format!(
                "AI output diverged from transcript (overlap={:.2}, lenΔ={:.2}) -> deterministic fallback",
                overlap, len_delta
            ),
        );
        None
    } else {
        Some(clean)
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
    fn test_terminal_focus_gets_literal_transcription_prompt() {
        use crate::context_detector::ForegroundApp;
        // Windows Terminal is UWP-hosted: its focused child class is a generic input-site
        // window, so the process map is what resolves Developer. It must still be detected.
        let wt = ForegroundApp { process_name: "WindowsTerminal.exe".into(), window_title: String::new() };
        let wt_cat = crate::context_detector::resolve_category(&wt, &[], "", "");
        assert_eq!(wt_cat, crate::context_detector::ContextCategory::Developer);
        assert!(is_terminal_focus(&wt_cat, "WindowsTerminal.exe", ""));
        // Native console class alone (generic host) is also a terminal surface.
        let con_cat = crate::context_detector::resolve_category(&wt, &[], "ConsoleWindowClass", "");
        assert!(is_terminal_focus(&con_cat, "randomhost.exe", "ConsoleWindowClass"));
        // The AI pass stays on: the terminal gets its own literal-transcription prompt.
        assert_eq!(ai_postprocess::terminal_system_prompt(), ai_postprocess::terminal_system_prompt());
        assert_ne!(ai_postprocess::terminal_system_prompt(), ai_postprocess::context_system_prompt(&wt_cat));

        // IDEs resolve Developer too but are NOT terminals: general Developer restyling.
        let code = crate::context_detector::resolve_category(
            &ForegroundApp { process_name: "Code.exe".into(), window_title: String::new() },
            &[], "", "",
        );
        assert_eq!(code, crate::context_detector::ContextCategory::Developer);
        assert!(!is_terminal_focus(&code, "Code.exe", ""));
        assert!(!is_terminal_focus(&code, "Code.exe", "Chrome_WidgetWin_1"));

        // Non-terminal processes and non-Developer categories never take the terminal prompt.
        assert!(!is_terminal_focus(
            &crate::context_detector::ContextCategory::General,
            "comet.exe",
            ""
        ));
        assert!(!is_terminal_focus(&wt_cat, "", ""));
    }

    #[test]
    fn test_terminal_focus_fallback_is_raw_not_prose() {
        use crate::context_detector::ForegroundApp;
        let wt = ForegroundApp { process_name: "WindowsTerminal.exe".into(), window_title: String::new() };
        let wt_cat = crate::context_detector::resolve_category(&wt, &[], "", "");
        assert!(is_terminal_focus(&wt_cat, "WindowsTerminal.exe", ""));

        // On an LLM miss the terminal chooses the raw dictation, not the prose-formatted
        // cleanup — a command must never be sentence-capitalized or given a trailing period.
        let raw = "git commit dash m fix login bug";
        let prose = "Git commit dash m fix login bug.";
        let on_miss = choose_final(None, raw.to_string());
        assert_eq!(on_miss, raw);
        assert_ne!(on_miss, prose);

        // Non-terminal surfaces still fall back to the deterministic cleanup.
        assert_eq!(choose_final(Some("cleaned".to_string()), String::new()), "cleaned");
        assert_eq!(choose_final(None, prose.to_string()), prose);
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

    #[test]
    fn test_ai_divergence_identical_text() {
        let pre = "Let me know whether there are some changes that you would like to make.";
        let ai = "Let me know whether there are some changes that you would like to make.";
        let (overlap, len_delta, diverged) = check_ai_divergence(pre, ai);
        assert_eq!(overlap, 1.0);
        assert_eq!(len_delta, 0.0);
        assert!(!diverged);

        let temp_dir = std::env::temp_dir();
        let guarded = guard_ai_output(ai.to_string(), pre, "cleanup", &temp_dir);
        assert_eq!(guarded, Some(ai.to_string()));
    }

    #[test]
    fn test_ai_divergence_minor_cleanup() {
        let pre = "um let me know whether there are some changes that you would like to make";
        let ai = "Let me know whether there are some changes that you would like to make.";
        let (overlap, len_delta, diverged) = check_ai_divergence(pre, ai);
        assert!(overlap > 0.85, "overlap was {}", overlap);
        assert!(len_delta <= 0.40, "len_delta was {}", len_delta);
        assert!(!diverged);

        let temp_dir = std::env::temp_dir();
        let guarded = guard_ai_output(ai.to_string(), pre, "cleanup", &temp_dir);
        assert_eq!(guarded, Some(ai.to_string()));
    }

    #[test]
    fn test_ai_divergence_total_hallucination() {
        let pre = "let me know if there are changes";
        let ai = "Today we will discuss the implications of quantum computing on distributed ledger systems across modern enterprise software architectures.";
        let (overlap, _len_delta, diverged) = check_ai_divergence(pre, ai);
        assert!(overlap < 0.30, "overlap was {}", overlap);
        assert!(diverged);

        let temp_dir = std::env::temp_dir();
        let guarded = guard_ai_output(ai.to_string(), pre, "cleanup", &temp_dir);
        assert_eq!(guarded, None);
    }

    #[test]
    fn test_ai_divergence_length_blowout() {
        let pre = "one two three four five six seven eight nine ten";
        let ai = "one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty twenty-one twenty-two twenty-three twenty-four twenty-five";
        let (_overlap, len_delta, diverged) = check_ai_divergence(pre, ai);
        assert!(len_delta > 0.40, "len_delta was {}", len_delta);
        assert!(diverged);

        let temp_dir = std::env::temp_dir();
        let guarded = guard_ai_output(ai.to_string(), pre, "cleanup", &temp_dir);
        assert_eq!(guarded, None);
    }

    #[test]
    fn test_ai_divergence_prompt_mode_exempt() {
        let pre = "summarize the following email into three bullet points";
        let ai = "- Point one\n- Point two\n- Point three";
        let (overlap, _len_delta, diverged) = check_ai_divergence(pre, ai);
        assert!(diverged || overlap < 0.60);

        let temp_dir = std::env::temp_dir();
        let guarded = guard_ai_output(ai.to_string(), pre, "prompt", &temp_dir);
        assert_eq!(guarded, Some(ai.to_string()));
    }

    #[test]
    fn test_ai_divergence_stutter_and_filler_reduction() {
        let pre = "he was connected he connected the slop via a keyboard in the bouc and why he brought the shortcuts were configured to control shift one two four, but the problem is when you press Control Shift and one in his keyboard or any other shortcut nothing responded, then suddenly when he pressed it again for some time when he tried, it automatically popped up and it stopped working perfectly so I didn't know what happened the problem is not the hot key, but sometimes when the input is pressed, the hotkey is pressed sometimes we didn't respond and sometimes it is responding";
        let ai = "My friend was using the app and connected the laptop via a keyboard and mouse. The shortcuts were configured to Ctrl+Shift+1 through 4. However, when he pressed Ctrl+Shift+1 or any other shortcut, nothing responded. Then suddenly, when he pressed it again after some time, it automatically popped up and worked perfectly. I don't know what happened. The problem isn't the hotkey, but sometimes when the hotkey is pressed it doesn't respond, and sometimes it does.";
        let (grounding, len_delta, diverged) = check_ai_divergence(pre, ai);
        assert!(grounding >= 0.50, "grounding was {}", grounding);
        assert_eq!(len_delta, 0.0);
        assert!(!diverged, "Stutter and filler cleanup should not be flagged as diverged");

        let temp_dir = std::env::temp_dir();
        let guarded = guard_ai_output(ai.to_string(), pre, "cleanup", &temp_dir);
        assert_eq!(guarded, Some(ai.to_string()));
    }

    #[test]
    fn test_ai_divergence_detects_refusal() {
        let pre = "explain how the audio buffering algorithm works";
        let ai = "I am sorry, but as an AI I cannot assist with this request.";
        let (_grounding, _len_delta, diverged) = check_ai_divergence(pre, ai);
        assert!(diverged, "AI refusal must trigger divergence fallback");

        let temp_dir = std::env::temp_dir();
        let guarded = guard_ai_output(ai.to_string(), pre, "cleanup", &temp_dir);
        assert_eq!(guarded, None);
    }

    #[test]
    fn test_append_vocabulary_hints() {
        let base = "Base prompt content.";
        let user_hints = vec!["customTerm".to_string(), "whisper".to_string(), "  ".to_string()];
        let augmented = append_vocabulary_hints(base, &user_hints);

        assert!(augmented.starts_with("Base prompt content."));
        assert!(augmented.contains("Recognized Vocabulary & Technical Terms:"));
        assert!(augmented.contains("Parakeet"));
        assert!(augmented.contains("Whisper"));
        assert!(augmented.contains("Nemotron"));
        assert!(augmented.contains("OpenCode"));
        assert!(augmented.contains("Orca"));
        assert!(augmented.contains("handoff"));
        assert!(augmented.contains("screenshot"));
        assert!(augmented.contains("customTerm"));

        // Case-insensitive de-duplication: "whisper" in user_hints should not duplicate "Whisper"
        let count_whisper = augmented.to_lowercase().matches("whisper").count();
        assert_eq!(count_whisper, 1);
    }
}
