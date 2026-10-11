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

/// Report a dictation failure to the user. The in-app toast lives in the MAIN window,
/// which is hidden while dictating from the tray (background mode) — the user saw
/// nothing and only found the log line later. So every failure message is also sent as
/// a `show-notification` event, which main.rs turns into a system notification.
///
/// The lib must NOT call the notification plugin directly: linking
/// tauri-plugin-notification into the lib breaks the unit-test binary at process load
/// (STATUS_ENTRYPOINT_NOT_FOUND; bisected out 2026-10-06). The plugin stays owned by
/// the binary (main.rs), the lib only emits the event.
pub(crate) fn notify_user(app: &AppHandle, message: &str) {
    let _ = app.emit("show-toast", message);
    let _ = app.emit("show-notification", message);
}

#[derive(Clone)]
pub struct Recorder {
    pub learning: crate::learning::Learning,
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
    parakeet_session: Arc<Mutex<Option<crate::transcribe_parakeet::ParakeetLiveSession>>>,
    review_saves: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
    write_session: Arc<Mutex<Option<crate::write_mode::Session>>>,
    write_cancel: Arc<Mutex<Option<crate::write_mode::Cancellation>>>,
    write_recent: Arc<Mutex<Option<crate::write_mode::RecentEdit>>>,
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            learning: crate::learning::Learning::default(),
            state: Arc::new(Mutex::new(RecordingState::Ready)),
            audio_recorder: Arc::new(Mutex::new(AudioRecorder::new())),
            session_override: Arc::new(Mutex::new(None)),
            session_context: Arc::new(Mutex::new(None)),
            nemotron_session: Arc::new(Mutex::new(None)),
            parakeet_session: Arc::new(Mutex::new(None)),
            review_saves: Arc::new(Mutex::new(Vec::new())),
            write_session: Arc::new(Mutex::new(None)),
            write_cancel: Arc::new(Mutex::new(None)),
            write_recent: Arc::new(Mutex::new(None)),
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
    pub fn reap_write_context(&self) {
        let mut recent = self.write_recent.lock().unwrap();
        if recent.as_ref().is_some_and(|last| !last.valid()) { *recent = None; }
    }

    pub fn is_write_active(&self) -> bool { self.write_cancel.lock().unwrap().is_some() }

    pub fn cancel_write(&self) {
        if let Some(cancel) = self.write_cancel.lock().unwrap().as_ref() { cancel.cancel(); }
    }


    pub async fn start_write(&self, app: &AppHandle, settings: &Settings, app_dir: &PathBuf) -> Result<(), String> {
        if !settings.write_enabled { return Err("Enable Write Mode in Recording settings first.".into()); }
        if settings.groq_api_key.trim().is_empty() { return Err("Add a Groq API key in Write Mode settings to begin editing.".into()); }
        let cancellation = crate::write_mode::Cancellation::default();
        {
            let mut state = self.state.lock().unwrap();
            if *state != RecordingState::Ready { return Err("Finish your current recording first.".into()); }
            *state = RecordingState::Transcribing; // Reserve the recorder while reading the selection.
            *self.write_cancel.lock().unwrap() = Some(cancellation.clone());
        }
        crate::overlay::set_write_mode(app, true);
        let _ = app.emit("recording-state", RecordingState::Transcribing);
        update_overlay(app, &RecordingState::Transcribing, true);
        let recent = {
            let mut last = self.write_recent.lock().unwrap();
            if last.as_ref().is_some_and(|edit| !edit.valid()) { *last = None; }
            last.clone()
        };
        let previous = recent.as_ref().map(|last| last.selection.clone());
        let capture_cancel = cancellation.clone();
        let selection = tokio::task::spawn_blocking(move || crate::selection::capture_with_previous(previous.as_ref(), &capture_cancel)).await
            .map_err(|_| "Could not read the selected text.".to_string()).and_then(|r| r);
        let selection = match selection.and_then(|s| { cancellation.check()?; Ok(s) }) {
            Ok(selection) => selection,
            Err(e) => { self.reset_ready(app); return Err(e); }
        };
        let previous_instruction = recent.filter(|last| last.selection.same_passage(&selection)).map(|last| last.instruction);
        *self.write_session.lock().unwrap() = Some(crate::write_mode::Session { selection, settings: settings.clone(), cancellation, previous_instruction });
        let nemotron = app_dir.join(transcribe_nemotron::model_dir_name(&settings.nemotron_model));
        let parakeet = app_dir.join(transcribe_parakeet::model_dir_name(&settings.parakeet_model));
        if let Err(e) = self.start_recording(app, &settings.microphone, None, &settings.engine, Some(&nemotron), Some(&parakeet)) {
            self.reset_ready(app); return Err(e);
        }
        // Escape during model/session startup must not leave a cancelled mic running.
        let cancelled = self.write_cancel.lock().unwrap().as_ref().is_some_and(|c| c.check().is_err());
        if cancelled {
            self.abort_write_capture(app);
        }
        Ok(())
    }

    fn abort_write_capture(&self, app: &AppHandle) {
        let _ = self.audio_recorder.lock().unwrap().stop_capture();
        self.nemotron_session.lock().unwrap().take();
        self.parakeet_session.lock().unwrap().take();
        self.audio_recorder.lock().unwrap().clear_samples();
        self.session_override.lock().unwrap().take();
        self.session_context.lock().unwrap().take();
        self.reset_ready(app);
    }

    /// Prevent a hotkey from starting capture during a local-data operation.
    pub fn run_when_idle<T>(&self, operation: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let state = self.state.lock().unwrap();
        if *state != RecordingState::Ready {
            return Err("Finish your current recording first.".into());
        }
        // A completed dictation may still be saving opt-in review audio. Join
        // before deleting, so no background writer recreates data after Clear.
        for save in self.review_saves.lock().unwrap().drain(..) {
            let _ = save.join();
        }
        operation()
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
        parakeet_model_dir: Option<&std::path::Path>,
    ) -> Result<(), String> {
        crate::model_cache::note_activity();
        // 1. Transition state to Recording while holding the state lock briefly
        self.learning.cancel(app);
        {
            let mut state = self.state.lock().unwrap();
            let reserved_write = *state == RecordingState::Transcribing && self.write_session.lock().unwrap().is_some();
            if *state != RecordingState::Ready && !reserved_write {
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
            recorder.set_meter_gain(crate::audio::FIXED_INPUT_GAIN_DB);
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
                *self.parakeet_session.lock().unwrap() = None;
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
                match crate::transcribe_nemotron::start_live_session(model_dir, self.audio_recorder.clone()) {
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

        *self.parakeet_session.lock().unwrap() = None;
        // Keep the measured accuracy path as the default. Buffered streaming
        // removes most post-stop work, but the reference clip has additional
        // internal model omissions under chunk-local level normalization.
        if engine == "parakeet" && std::env::var("TYPR_PARAKEET_BUFFERED").map(|v| v == "1").unwrap_or(false) {
            if let Some(model_dir) = parakeet_model_dir.filter(|dir|
                dir.file_name().is_some_and(|name| name == transcribe_parakeet::model_dir_name("v3"))) {
                match transcribe_parakeet::start_live_session(model_dir, self.audio_recorder.clone()) {
                    Ok(session) => {
                        *self.parakeet_session.lock().unwrap() = Some(session);
                        println!("[Typr] Started Parakeet v3 buffered streaming session");
                    }
                    Err(e) => eprintln!("[Typr] Parakeet buffered start failed; retaining batch path: {e}"),
                }
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

        let cancelled = self.write_cancel.lock().unwrap().as_ref().is_some_and(|c| c.check().is_err());
        if cancelled {
            self.abort_write_capture(app);
            return Err("Write Mode cancelled. Your selected text was kept.".into());
        }

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
        let write_session = self.write_session.lock().unwrap().take();
        let effective_settings = if let Some(write) = &write_session { write.settings.clone() } else { apply_session_override(settings, session_override) };
        let settings = &effective_settings;

        // Delimit this capture for stage logs and asynchronous comparison output.
        let capture_id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default().as_nanos();
        crate::debug_log::log(app_dir, &format!(
            "PERF capture={capture_id} release engine={} ai={} ai_model={}",
            settings.engine, settings.ai_enabled, settings.ai_model));

        let temp_path = app_dir.join("temp_recording.wav");
        let parakeet_model_dir = app_dir.join(transcribe_parakeet::model_dir_name(&settings.parakeet_model));
        let parakeet_live = settings.engine == "parakeet" && self.parakeet_session.lock().unwrap()
            .as_ref().is_some_and(|session| session.model_dir == parakeet_model_dir);
        let mut wav_requested = settings.engine != "nemotron" && !parakeet_live;

        // Release grace: keep capturing for 400ms after the stop request. The cpal stream is
        // NOT paused during this sleep (stop_and_save pauses it afterwards), so every sample
        // the mic produces in this window is still recorded. That covers a hotkey released
        // mid-word (common in push-to-talk) and any audio still in flight in the driver.
        // The 400ms tail-silence padding in `stop_and_save` only helps the engine flush; it
        // cannot restore speech that was never captured. Cost: +400ms per dictation.
        // If "last word cut off" persists, check the `tail150` figure in the
        // "Audio preprocessed" log line: ~1.0x means capture still ends mid-speech.
        tokio::time::sleep(Duration::from_millis(400)).await;

        // Live Nemotron needs only the stopped raw buffer. Defer WAV preparation
        // until we know whether a batch decode is necessary.
        let save_started_at = Instant::now();
        let save_result = {
            let mut recorder = self.audio_recorder.lock().unwrap();
            if !wav_requested {
                recorder.stop_capture()
            } else {
                recorder.stop_and_save(&temp_path, Some(app)).map(|(_, duration)| duration)
            }
        };
        
        let duration_secs = match save_result {
            Ok(duration) => duration,
            Err(e) => {
                // Capture failures used to die silently (no log line, no toast): a quick
                // "D"/"hi" tap that falls under the 0.4 s minimum or the silence gate
                // pasted nothing and left no trace. Log and tell the user instead.
                crate::debug_log::log(app_dir, &format!("capture failed: {}", e));
                let toast = match e.as_str() {
                    "Audio too short" => "That was too short to transcribe — hold the hotkey while you speak.",
                    "Audio was silent" => "Nothing heard — check your microphone and speak closer to it.",
                    _ => "No audio captured — check your microphone.",
                };
                notify_user(app, toast);
                *self.nemotron_session.lock().unwrap() = None;
                *self.parakeet_session.lock().unwrap() = None;
                self.audio_recorder.lock().unwrap().clear_samples();
                if wav_requested { let _ = std::fs::remove_file(&temp_path); }
                self.reset_ready(app);
                return Err(e);
            }
        };
        crate::debug_log::log(app_dir, &format!(
            "PERF capture={capture_id} audio_ms={} capture_secs={duration_secs:.3} release_elapsed_ms={}",
            save_started_at.elapsed().as_millis(), transcription_started_at.elapsed().as_millis()));

        // Explicit local-only diagnostics. Off by default; useful when normal
        // and fast speech need replaying through exactly the same engines.
        if review_audio_enabled() {
            let review_started_at = Instant::now();
            let capture = self.audio_recorder.lock().unwrap().review_capture();
            crate::debug_log::log(app_dir, &format!(
                "PERF capture={capture_id} review_snapshot_ms={}", review_started_at.elapsed().as_millis()));
            let review_dir = app_dir.join("review-audio");
            let path = review_dir.join(format!("{capture_id}.wav"));
            let log_dir = app_dir.clone();
            let worker = std::thread::Builder::new().name("typr-review-save".into()).spawn(move || {
                let started = Instant::now();
                let result = std::fs::create_dir_all(&review_dir).map_err(|e| e.to_string())
                    .and_then(|_| capture.save(&path));
                match result {
                    Ok(()) => crate::debug_log::log(&log_dir, &format!(
                        "REVIEW AUDIO capture={capture_id} raw_wav={path:?}")),
                    Err(e) => crate::debug_log::log(&log_dir, &format!(
                        "REVIEW AUDIO capture={capture_id} save failed: {e}")),
                }
                crate::debug_log::log(&log_dir, &format!(
                    "PERF capture={capture_id} review_save_ms={} background=true", started.elapsed().as_millis()));
            });
            match worker {
                Ok(worker) => {
                    let mut saves = self.review_saves.lock().unwrap();
                    saves.retain(|save| !save.is_finished());
                    saves.push(worker);
                },
                Err(e) => crate::debug_log::log(app_dir, &format!(
                    "REVIEW AUDIO capture={capture_id} could not start save: {e}")),
            }
        }

        // Transcribe
        let prompt = {
            let dict = dictionary.lock().unwrap();
            dict.engine_prompt(&settings.engine,settings.dictionary_cloud_hints)
        };

        let engine_started_at = Instant::now();
        let transcribe_result = match settings.engine.as_str() {
            "local" => {
                let model_path = app_dir.join(transcribe_local::model_filename(&settings.whisper_model));
                transcribe_local::transcribe_local(app, &model_path, &temp_path, &prompt).await
            }
            "cloud" => {
                transcribe_groq::transcribe_groq(&settings.groq_api_key, &temp_path, &prompt, &settings.cloud_model).await
            }
            "parakeet" => {
                let model_dir = parakeet_model_dir;
                // No prompt: transducer models have no equivalent of Whisper's prompt window,
                // so the dictionary bias the other two engines receive has nowhere to go here.
                let session = self.parakeet_session.lock().unwrap().take();
                let live_result = if let Some(session) = session.filter(|s| s.model_dir == model_dir) {
                    Some(tokio::task::spawn_blocking(move || session.finish()).await
                        .map_err(|e| format!("Parakeet finish task failed: {e}")).and_then(|result| result))
                } else { None };
                match live_result {
                    Some(Ok(result)) => {
                        crate::debug_log::log(app_dir, &format!(
                            "PERF capture={capture_id} parakeet_path=Buffered completed_chunks={} tail_decode_ms={} wav_saved=false",
                            result.completed_chunks, result.tail_ms));
                        Ok(result.text)
                    }
                    other => {
                        if let Some(Err(error)) = other {
                            crate::debug_log::log(app_dir, &format!("Parakeet buffered recovery: {error}"));
                        }
                        async {
                            if !wav_requested {
                                wav_requested = true;
                                self.audio_recorder.lock().unwrap().save_capture(&temp_path, Some(app))?;
                            }
                            transcribe_parakeet::transcribe_parakeet(&model_dir, &temp_path).await
                        }.await
                    }
                }
            }
            "nemotron" => {
                let live_session = self.nemotron_session.lock().unwrap().take();
                let compare = std::env::var("TYPR_NEMOTRON_COMPARE")
                    .map(|v| v == "1").unwrap_or(false);
                let batch_primary = duration_secs <= NEMOTRON_SHORT_CLIP_SECS
                    && nemotron_batch_verify_enabled() && !compare;
                let mut short_session = None;
                let live_text = if let Some(session) = live_session {
                    if batch_primary {
                        // Join without decoding a tail we normally discard. Retain
                        // the stream in case the processed-WAV decode fails.
                        match session.pause() {
                            Ok(session) => short_session = Some(session),
                            Err(e) => eprintln!("[Typr] Nemotron worker failed: {e}; using batch"),
                        }
                        None
                    } else {
                        match session.finish() {
                            Ok(text) if !text.trim().is_empty() => Some(text),
                            Ok(_) => None,
                            Err(e) => {
                                eprintln!("[Typr] Nemotron live streaming error: {}, falling back to batch", e);
                                None
                            }
                        }
                    }
                } else {
                    None
                };

                let live_wps = live_text
                    .as_ref()
                    .map(|t| t.split_whitespace().count() as f32 / duration_secs.max(0.1))
                    .unwrap_or(0.0);
                let path = nemotron_decode_path(duration_secs, live_text.is_some(),
                    live_wps, nemotron_batch_verify_enabled());
                crate::debug_log::log(app_dir, &format!(
                    "PERF capture={capture_id} nemotron_path={path:?} live_wps={live_wps:.2} compare={compare}"));

                let wav_result = if nemotron_needs_wav(path, compare) {
                    wav_requested = true;
                    let wav_started = Instant::now();
                    let result = self.audio_recorder.lock().unwrap().save_capture(&temp_path, Some(app));
                    crate::debug_log::log(app_dir, &format!(
                        "PERF capture={capture_id} wav_ms={} wav_saved={}",
                        wav_started.elapsed().as_millis(), result.is_ok()));
                    result
                } else {
                    self.audio_recorder.lock().unwrap().notify_capture_levels(app);
                    crate::debug_log::log(app_dir, &format!(
                        "PERF capture={capture_id} wav_ms=0 wav_saved=false"));
                    Ok(())
                };
                match wav_result {
                    Err(e) => Err(e),
                    Ok(()) => {

                        // Opt-in diagnostic (TYPR_NEMOTRON_COMPARE=1): decode the SAME processed WAV with
                        // the batch path in the background and log both transcripts, so live-vs-batch
                        // accuracy can be compared on real dictations. The pasted text is unaffected.
                        // Note: the background decode takes the shared recognizer lock, so a dictation
                        // started while it is still running can be delayed. Diagnostic use only.
                        if let Some(ref live) = live_text {
                            if background_nemotron_compare(compare, path) {
                                let cmp_path = app_dir.join(format!("nemotron-compare-{capture_id}.wav"));
                                if std::fs::copy(&temp_path, &cmp_path).is_ok() {
                                    let model_dir = app_dir
                                        .join(transcribe_nemotron::model_dir_name(&settings.nemotron_model));
                                    let live_clone = live.clone();
                                    let log_dir = app_dir.clone();
                                    tokio::spawn(async move {
                                        let batch = transcribe_nemotron::transcribe_nemotron(&model_dir, &cmp_path).await;
                                        let _ = std::fs::remove_file(&cmp_path);
                                        crate::debug_log::log(
                                            &log_dir,
                                            &format!("NEMOTRON COMPARE capture={capture_id} live: {:?} | batch: {:?}", live_clone, batch),
                                        );
                                    });
                                }
                            }
                        }

                        match path {
                            NemotronDecodePath::BatchPrimary | NemotronDecodePath::BatchVerify => {
                                let model_dir = app_dir
                                    .join(transcribe_nemotron::model_dir_name(&settings.nemotron_model));
                                if path == NemotronDecodePath::BatchVerify {
                                    println!(
                                        "[Typr] Nemotron live transcript sparse ({:.2} wps for {:.1}s), checking batch fallback",
                                        live_wps, duration_secs
                                    );
                                } else {
                                    println!(
                                        "[Typr] Nemotron short dictation ({:.1}s): single batch decode instead of live + verify",
                                        duration_secs
                                    );
                                }
                                let mut batch = transcribe_nemotron::transcribe_nemotron(&model_dir, &temp_path).await;
                                let mut live = live_text.unwrap_or_default();
                                if compare {
                                    crate::debug_log::log(app_dir, &format!(
                                        "NEMOTRON COMPARE capture={capture_id} live: {live:?} | batch: {batch:?}"));
                                }
                                if !matches!(&batch, Ok(t) if !t.trim().is_empty()) {
                                    if let Some(session) = short_session.take() {
                                        match session.finish() {
                                            Ok(text) => live = text,
                                            Err(e) => eprintln!("[Typr] Nemotron recovery failed: {e}"),
                                        }
                                    }
                                }
                                // Total failure (batch empty AND live empty) pastes nothing — the
                                // "second try nothing pasted" report. One retry: a transient
                                // engine flake recovers, a deterministic empty costs one extra
                                // decode on a dictation that already failed. Never retry when we
                                // already hold a transcript — that would only add latency.
                                if should_retry_empty_batch(
                                    !matches!(&batch, Ok(t) if !t.trim().is_empty()),
                                    live.trim().is_empty(),
                                ) {
                                    println!("[Typr] Nemotron decode came back empty, retrying batch once");
                                    batch = transcribe_nemotron::transcribe_nemotron(&model_dir, &temp_path).await;
                                }
                                match (path, batch) {
                                    // Batch is the primary decode: one pass over the whole clip, so
                                    // take it unless it came back empty.
                                    (NemotronDecodePath::BatchPrimary, Ok(batch_text))
                                        if !batch_text.trim().is_empty() => Ok(batch_text),
                                    // Verify: only take batch when it recovered more words.
                                    (NemotronDecodePath::BatchVerify, Ok(batch_text))
                                        if batch_text.split_whitespace().count()
                                            > live.split_whitespace().count() => Ok(batch_text),
                                    (_, Err(e)) if live.trim().is_empty() => Err(e),
                                    _ => Ok(live),
                                }
                            }
                            NemotronDecodePath::LiveOnly => Ok(live_text.unwrap_or_default()),
                        }
                    }
                }
            }
            _ => Err(format!("Unknown engine: {}", settings.engine)),
        };
        crate::debug_log::log(app_dir, &format!(
            "PERF capture={capture_id} engine_ms={} release_elapsed_ms={}",
            engine_started_at.elapsed().as_millis(), transcription_started_at.elapsed().as_millis()));

        // Join any unused worker after an engine/model change, BEFORE clearing
        // capture samples. Workers must never read the next session's buffer.
        *self.nemotron_session.lock().unwrap() = None;
        *self.parakeet_session.lock().unwrap() = None;
        // The engine is done and live workers have been joined: release the
        // raw capture buffer instead of holding it until the next start().
        self.audio_recorder.lock().unwrap().clear_samples();

        // Cleanup temp file
        if wav_requested { let _ = std::fs::remove_file(&temp_path); }

        // Keep the overlay in its "processing" state through dictionary replacement and the
        // AI cleanup pass below; we only reset to Ready just before pasting, so the spinner
        // covers the AI latency instead of clearing ~1-2s early. Every exit path resets.
        let raw_text = match transcribe_result {
            Ok(text) => text,
            Err(e) => {
                // Transcription failures were equally invisible in typr.log.
                crate::debug_log::log(app_dir, &format!("transcribe failed: {}", e));
                self.reset_ready(app);
                return Err(e);
            }
        };

        // Punctuation/noise fragments are discarded, but an actual recognized
        // letter is meaningful even after a pause. Audio already passed the
        // capture's signal-based silence gate; duration cannot prove intent.
        if is_hallucinated_fragment(&raw_text, duration_secs) {
            crate::debug_log::log(app_dir, &format!("STAGE 1 [engine return]: {} (rejected: hallucinated fragment)", raw_text));
            notify_user(app, "Nothing heard — check your microphone and speak closer to it.");
            self.reset_ready(app);
            return Err("Audio was silent".to_string());
        }

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
            notify_user(app, &toast_msg);
        }

        crate::debug_log::log(app_dir, &format!("STAGE 1 [engine return]: {}", raw_text));

        // Only explicitly configured Snippets rewrite text deterministically.
        let (replaced, standalone_snippet) = {
            let dict = dictionary.lock().unwrap();
            let standalone = if write_session.is_none() {
                dict.standalone_snippet(&raw_text)
            } else {
                None
            };
            (if write_session.is_some() { raw_text.clone() } else { standalone.clone().unwrap_or_else(|| dict.apply_replacements(&raw_text)) }, standalone)
        };
        crate::debug_log::log(app_dir, &format!("STAGE 2 [dictionary]: {}", replaced));

        // Assemble spoken email addresses ("name at gmail dot com") into real ones. Must run
        // before both the AI pass and the deterministic cleanup: the LLM only promises to
        // preserve addresses it can recognize, and the entity guard can only protect one that
        // already looks like an address.
        let replaced = if standalone_snippet.is_some() {
            replaced
        } else {
            crate::email_assemble::assemble_emails(&replaced)
        };
        crate::debug_log::log(app_dir, &format!("STAGE 3 [email assembly]: {}", replaced));

        if let Some(write) = write_session {
            let result = async {
                write.cancellation.check()?;
                let edited = crate::write_mode::rewrite(&write.selection.text, &replaced, write.previous_instruction.as_deref(), settings, &write.cancellation).await?;
                if write.selection.terminal {
                    write.cancellation.check()?;
                    crate::write_preview::show(app, &edited, &settings.theme)?;
                    return Ok("Write Mode rewrite ready to copy".to_string());
                }
                let target = write.selection.clone();
                let text = edited.clone();
                let cancel = write.cancellation.clone();
                let warning = tokio::task::spawn_blocking(move || crate::selection::replace(&target, &text, &cancel)).await
                    .map_err(|_| "Could not replace the selection safely.".to_string())??;
                if let Some(message) = warning {
                    *self.write_recent.lock().unwrap() = None;
                    notify_user(app, &message);
                } else {
                    *self.write_recent.lock().unwrap() = Some(crate::write_mode::RecentEdit {
                        selection: write.selection.with_edited_text(edited), instruction: replaced.clone(), created: Instant::now(),
                    });
                    let _ = app.emit("show-toast", "Text updated. Use Ctrl+Z in your editor to undo.");
                }
                Ok("Write Mode edit applied".to_string())
            }.await;
            // Selected text and rewritten content never enter dictation history/logs.
            self.reset_ready(app);
            return result;
        }

        // Deterministic cleanup is the always-available fallback.
        let deterministic = cleanup_text(&replaced);
        // Filler safety net: when AI is on and the user opted to strip filler in
        // Developer/Terminal, ensure filler never survives a bypass or an LLM miss.
        // The AI prompt already asks to strip filler; this is the deterministic
        // guarantee for the fallback paths where the LLM is not in the loop.
        // Strip filler from AI-enabled dictation; a standalone snippet's saved
        // text is literal and must not be interpreted as speech.
        let strip_filler = settings.ai_enabled && standalone_snippet.is_none();
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
        // command pass runs. This includes Prompt Mode — an explicit voice command is a
        // direct instruction whose reliability outweighs the rewrite, and the log showed the
        // LLM deleting "Make it a list" before the command pass ever saw it.
        let bypass_ai_for_commands = standalone_snippet.is_none() && commands::contains_command(&replaced);
        if bypass_ai_for_commands {
            crate::debug_log::log(app_dir, "commands present -> raw text + command pass (skipping AI & prose cleanup)");
        }

        // Optional Groq LLM cleanup with a profile-specific budget. On off/offline/slow/error we
        // paste the deterministic result instead, so a dictation is never blocked.
        let final_text = if let Some(expansion) = &standalone_snippet {
            crate::debug_log::log(app_dir, "standalone snippet -> saved text (skipping AI & prose cleanup)");
            expansion.clone()
        } else if bypass_ai_for_commands {
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
            let mut base_with_vocab = append_vocabulary_hints(base_prompt, &[]);
            let vocabulary = crate::vocabulary::cleanup_hint_for_text(&dictionary.lock().unwrap().entries, &replaced_stripped);
            crate::debug_log::log(app_dir, &format!("AI vocabulary_chars={} engine={}", vocabulary.chars().count(), settings.engine));
            base_with_vocab.push_str(&vocabulary);

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
            let budget = ai_postprocess::budget_ms_for_text(&settings.ai_profile, &replaced_stripped);
            let ai_started_at = Instant::now();
            let llm = match tokio::time::timeout(
                Duration::from_millis(budget),
                ai_postprocess::postprocess_with_fallback(
                    &settings.groq_api_key,
                    &replaced_stripped,
                    &settings.ai_model,
                    &system_prompt,
                ),
            )
            .await
            {
                Ok(Ok(result)) => {
                    // Metadata only: timing/model/profile, never the dictated text itself.
                    // The result's model produced this text, which is not always the
                    // one selected in settings — a fallback retry must be visible in the log.
                    let reference=ai_postprocess::vocabulary_validation_reference(&replaced_stripped,&result.text,&system_prompt);
                    let guarded = guard_ai_output(result.text, &reference, &settings.ai_profile, app_dir);
                    if guarded.is_some() { crate::debug_log::log(
                        app_dir,
                        &format!(
                            "AI ok {}ms model={} profile={} recovered={} retried={}",
                            ai_started_at.elapsed().as_millis(),
                            result.model,
                            settings.ai_profile,
                            result.recovered,
                            result.retried,
                        ),
                    ); }
                    guarded
                }
                Ok(Err(e)) => {
                    crate::debug_log::log(app_dir, &format!("AI skipped {}ms (error): {}",
                        ai_started_at.elapsed().as_millis(), e));
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
            if llm.is_none() {
                // AI-on must not silently look like AI-off when a service or
                // preservation failure occurs. Preserve the complete dictation
                // and surface the actual outcome, including when hidden to tray.
                notify_user(app, if terminal_focus_once {
                    "AI processing couldn't be applied. Your literal dictation was kept."
                } else {
                    "AI processing couldn't be applied. Your dictation was kept with basic cleanup."
                });
            }
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
        // casing/layout pass. Standalone snippets preserve intentional repetitions.
        let final_text = if standalone_snippet.is_some() {
            final_text
        } else {
            crate::cleanup::deduplicate_text(&final_text)
        };
        crate::debug_log::log(app_dir, &format!("STAGE 5 [dedup]: {}", final_text));

        // Re-assert the dictionary after the AI pass: the LLM can silently revert a
        // corrected spelling or snippet expansion on some runs but not others — exactly
        // the "sometimes works, sometimes doesn't" dictionary symptom. Both steps are
        // deterministic and idempotent, so running them again is safe. A literal
        // standalone expansion must not trigger another snippet.
        let final_text = if standalone_snippet.is_some() { final_text } else {
            let dict = dictionary.lock().unwrap();
            dict.apply_replacements(&final_text)
        };
        crate::debug_log::log(app_dir, &format!("STAGE 5.5 [dictionary re-assert]: {}", final_text));

        // Final deterministic pass: apply always-on voice commands (casing / layout / symbols).
        // Runs after cleanup and any AI pass so nothing downstream can undo it; identical
        // behavior whether AI is on or off. Saved standalone snippet content
        // is not a voice command, even if it contains a command phrase.
        let final_text = if standalone_snippet.is_some() {
            final_text
        } else {
            commands::apply_commands(&final_text)
        };
        crate::debug_log::log(app_dir, &format!("STAGE 6 [commands]: {}", final_text));

        // A whispered trailing stop/close marker ("stop", "end session") is a signal to the
        // recorder, not content — drop it so it never reaches the target window.
        // A saved snippet's trailing words are intentional content.
        let final_text = if standalone_snippet.is_some() {
            final_text
        } else {
            crate::cleanup::strip_trailing_stop_phrase(&final_text)
        };
        // No trailing whitespace in the pasted payload: in a terminal a trailing newline
        // acts as Enter and executes the last line (bug #2: "commands executed"), and in
        // editors it leaves a stray blank line. Interior newlines from "new line" /
        // bullets are untouched.
        let final_text = final_text.trim_end().to_string();

        // Hide the overlay before paste, but remain busy until history is saved.
        // Otherwise Clear data could succeed and an in-flight paste would then
        // put the old transcript straight back into the supposedly empty history.
        let processing_ms = transcription_started_at.elapsed().as_millis();
        update_overlay(app, &RecordingState::Ready, false);

        // Auto-paste and record history
        if !final_text.is_empty() {
            self.learning.cancel(app);
            let marker=if settings.learn_from_corrections {
                tokio::time::timeout(Duration::from_millis(600),tauri::async_runtime::spawn_blocking(crate::selection::prepare_observation))
                    .await.ok().and_then(Result::ok).and_then(Result::ok)
            } else { None };
            if let Err(e) = paste_text(&final_text) {
                self.reset_ready(app);
                return Err(e);
            }
            let _ = history.lock().unwrap().add_item(final_text.clone(), duration_secs, app_dir);
            let _ = app.emit("history-updated", ());
            let known={
                let mut dict=dictionary.lock().unwrap();
                let _=dict.record_hits(&final_text,app_dir);
                dict.entries.clone()
            };
            if let Some(marker)=marker {
                self.learning.observe(app.clone(),marker,final_text.clone(),settings.clone(),known);
            }
        }

        let total_ms = transcription_started_at.elapsed().as_millis();
        crate::debug_log::log(app_dir, &format!(
            "PERF capture={capture_id} processing_ms={processing_ms} total_ms={} paste_and_history_ms={}",
            total_ms, total_ms.saturating_sub(processing_ms)));

        self.reset_ready(app);

        Ok(final_text)
    }

    /// Reset the recorder to Ready and clear the processing overlay. Called once the
    /// stop-to-text pipeline finishes (or on transcription error), so the spinner clears
    /// only after the AI cleanup pass rather than ~1-2s before the paste lands.
    fn reset_ready(&self, app: &AppHandle) {
        // Measure idle time from completion, including long recordings.
        crate::model_cache::note_activity();
        let mut state = self.state.lock().unwrap();
        *state = RecordingState::Ready;
        *self.nemotron_session.lock().unwrap() = None;
        *self.parakeet_session.lock().unwrap() = None;
        self.write_session.lock().unwrap().take();
        self.write_cancel.lock().unwrap().take();
        crate::overlay::set_write_mode(app, false);
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
        "FluidVoice".into(),
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
            "{}\n\nRecognized Vocabulary & Technical Terms:\nUse these exact spellings only when the transcript clearly refers to the term. Do not replace an ordinary word or invent a term merely because it appears in this list:\n{}",
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

/// Which decode the Nemotron engine should use for a finished recording.
///
/// Live streaming pays off on long dictations — the audio is decoded while the user is
/// still speaking, so only the tail flush remains at the hotkey release. On short clips
/// that advantage is negligible, and decoding live *and then* batch to catch a dropped
/// tail costs a second full pass (the "not instant any more" report). A single batch pass
/// over the whole clip is cheaper than live-then-batch and has no live chunk-boundary
/// seam to drop a word at, so short clips are decoded once, by batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NemotronDecodePath {
    /// Use the live transcript as-is (long, healthy live output).
    LiveOnly,
    /// Decode batch as well and keep it only when it recovered more words (long but sparse).
    BatchVerify,
    /// Decode batch once and prefer it unless it is empty (short clip, or no live text).
    BatchPrimary,
}

/// Clip length up to which a single batch decode replaces live streaming.
const NEMOTRON_SHORT_CLIP_SECS: f32 = 8.0;

fn background_nemotron_compare(compare: bool, path: NemotronDecodePath) -> bool {
    compare && path == NemotronDecodePath::LiveOnly
}

fn nemotron_needs_wav(path: NemotronDecodePath, compare: bool) -> bool {
    compare || path != NemotronDecodePath::LiveOnly
}

fn review_audio_enabled() -> bool {
    std::env::var("TYPR_REVIEW_AUDIO").map(|v| v == "1").unwrap_or(false)
}

pub fn nemotron_decode_path(
    duration_secs: f32,
    live_available: bool,
    live_wps: f32,
    batch_verify_enabled: bool,
) -> NemotronDecodePath {
    if !live_available {
        return NemotronDecodePath::BatchPrimary;
    }
    if !batch_verify_enabled {
        return NemotronDecodePath::LiveOnly;
    }
    if duration_secs <= NEMOTRON_SHORT_CLIP_SECS {
        return NemotronDecodePath::BatchPrimary;
    }
    if duration_secs > 5.0 && live_wps < 0.6 {
        return NemotronDecodePath::BatchVerify;
    }
    NemotronDecodePath::LiveOnly
}

/// Escape hatch for the Nemotron tail trade-off: `TYPR_NEMOTRON_BATCH_VERIFY=0` restores
/// the pure live-streaming path (fastest, and the behaviour before tail recovery existed).
fn nemotron_batch_verify_enabled() -> bool {
    std::env::var("TYPR_NEMOTRON_BATCH_VERIFY")
        .map(|v| v != "0")
        .unwrap_or(true)
}

/// Whether a second batch decode is worth attempting after a Nemotron decode round.
/// Only when the round left us with nothing at all: retrying a total failure can rescue
/// a transient engine flake, while retrying when we already hold a transcript would
/// only add latency for no reason.
fn should_retry_empty_batch(batch_empty: bool, live_empty: bool) -> bool {
    batch_empty && live_empty
}

/// Whether an engine transcript is a hallucinated fragment rather than dictation.
///
/// Discard empty/punctuation fragments. A single spoken letter can be held or
/// followed by a pause; never infer that it is noise solely from duration.
pub fn is_hallucinated_fragment(raw_text: &str, duration_secs: f32) -> bool {
    if duration_secs < 1.0 {
        return false;
    }
    let squashed: String = raw_text.chars().filter(|c| c.is_alphanumeric()).collect();
    if squashed.len() == 1 && squashed.as_bytes()[0].is_ascii_alphabetic() { return false; }
    squashed.chars().count() <= 1
}

/// Compute case-insensitive vocabulary grounding (fraction of AI content words originating
/// from the transcript), expansion deviation, and refusal detection.
pub fn check_ai_divergence(pre_text: &str, ai_text: &str) -> (f32, f32, bool) {
    const REFUSAL_PATTERNS: &[&str] = &[
        "i'm sorry", "i am sorry", "i cannot", "i can't",
        "as an ai", "as a language model", "i am unable",
        // Reply-shaped signatures: the model answering the user instead of transcribing
        // (e.g. "That's not the purpose of a voice…").
        "not the purpose", "i'm here to", "i am here to", "my purpose is",
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

    // Truncation: the AI returned only a prefix, dropping one or more final words. Word-set
    // grounding cannot see this (every surviving word is grounded), yet a silently dropped last
    // word is the one failure the user cannot spot and cannot recover from. A trailing filler
    // or courtesy word is a legitimate removal and is not flagged.
    const TAIL_FILLER: &[&str] = &[
        "um", "uh", "er", "ah", "hmm", "erm", "uhm", "like", "okay", "ok", "right", "so",
        "yeah", "please", "thanks", "basically", "actually", "literally", "just",
    ];
    if ai_tokens.len() < pre_tokens.len() && pre_tokens[..ai_tokens.len()] == ai_tokens[..] {
        let tail = &pre_tokens[ai_tokens.len()..];
        if tail.iter().any(|dropped| dropped.len() >= 3 && !TAIL_FILLER.contains(&dropped.as_str())) {
            return (grounding, len_delta, true);
        }
    }

    // Grounding in the output alone accepts an arbitrary subset of a long
    // transcript. Reject severe omissions as well as invented additions.
    const STOP_WORDS: &[&str] = &[
        "a", "an", "the", "and", "or", "to", "of", "in", "on", "at", "for", "with",
        "is", "are", "was", "were", "be", "been", "it", "that", "this", "i", "you",
        "we", "they", "me", "my", "our", "your", "um", "uh", "er", "please",
    ];
    let source_content = pre_tokens.iter().filter(|t| !STOP_WORDS.contains(&t.as_str())).count();
    let retained = pre_tokens.iter().filter(|t| {
        !STOP_WORDS.contains(&t.as_str()) && ai_tokens.contains(t)
    }).count();
    if source_content >= 8 && retained * 2 < source_content {
        return (grounding, len_delta, true);
    }

    // Merge/split correction: for short dictations, accept an output that is a close spelling
    // of the transcript once spaces and punctuation are ignored and the word count did not
    // grow. This admits legitimate single-token corrections of multi-word mishearings
    // ("get hub" -> "GitHub", "pie torch" -> "PyTorch", "type script" -> "TypeScript") that
    // word-set grounding would otherwise reject as divergence.
    let squash = |s: &str| -> String {
        s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
    };
    let (pre_sq, ai_sq) = (squash(pre_text), squash(ai_text));
    let max_chars = pre_sq.chars().count().max(ai_sq.chars().count()).max(1);
    let char_similarity = 1.0 - strsim::levenshtein(&pre_sq, &ai_sq) as f32 / max_chars as f32;
    let close_respelling = pre_len <= 5.0 && ai_len <= pre_len && char_similarity >= 0.7;

    let diverged = if close_respelling {
        false
    } else if pre_len <= 5.0 {
        // Short dictations were the hole an "answered instead of transcribed" reply slipped
        // through: one shared word used to defeat the guard. Require most content words to
        // originate from the transcript and reject obvious length blowouts — a reply to a
        // 5-word question ("Do you support computer use?") is both.
        matched_count == 0 || grounding < 0.5 || ai_len > (pre_len * 1.7).max(pre_len + 3.0)
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
    #[test]
    fn live_decode_skips_wav_but_batch_and_comparison_keep_it() {
        use super::{nemotron_needs_wav, NemotronDecodePath::*};
        assert!(!nemotron_needs_wav(LiveOnly, false));
        for path in [LiveOnly, BatchPrimary, BatchVerify] {
            assert!(nemotron_needs_wav(path, true));
        }
        assert!(nemotron_needs_wav(BatchPrimary, false));
        assert!(nemotron_needs_wav(BatchVerify, false));
    }
    use super::*;

    #[test]
    fn test_initial_state_is_ready() {
        let recorder = Recorder::new();
        assert_eq!(recorder.get_state(), RecordingState::Ready);
    }

    #[test]
    fn local_data_operations_require_idle_recorder() {
        let recorder = Recorder::new();
        assert_eq!(recorder.run_when_idle(|| Ok(42)), Ok(42));
        for busy in [RecordingState::Recording, RecordingState::Transcribing] {
            *recorder.state.lock().unwrap() = busy;
            let mut called = false;
            assert!(recorder.run_when_idle(|| { called = true; Ok(()) }).is_err());
            assert!(!called);
        }
    }

    #[test]
    fn local_data_operations_wait_for_background_review_save() {
        let recorder = Recorder::new();
        let saved = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_saved = saved.clone();
        let worker = std::thread::spawn(move || {
            worker_saved.store(true, std::sync::atomic::Ordering::Release);
        });
        recorder.review_saves.lock().unwrap().push(worker);
        recorder.run_when_idle(|| {
            assert!(saved.load(std::sync::atomic::Ordering::Acquire));
            Ok(())
        }).unwrap();
        assert!(recorder.review_saves.lock().unwrap().is_empty());
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
    fn test_ai_divergence_accepts_single_token_corrections_of_short_dictations() {
        for (pre, ai) in [
            ("get hub", "GitHub"),
            ("pie torch", "PyTorch"),
            ("type script", "TypeScript"),
            ("open get hub issues", "Open GitHub issues"),
        ] {
            let (_g, _d, diverged) = check_ai_divergence(pre, ai);
            assert!(!diverged, "legitimate correction rejected: {pre:?} -> {ai:?}");
        }
    }

    #[test]
    fn test_ai_divergence_still_rejects_replies_to_short_dictations() {
        for (pre, ai) in [
            ("is it working", "Yes, it is working perfectly fine today"),
            ("what is rust", "Rust is a systems programming language"),
            ("get hub", "Sure, here is a poem about roses"),
        ] {
            let (_g, _d, diverged) = check_ai_divergence(pre, ai);
            assert!(diverged, "reply not rejected: {pre:?} -> {ai:?}");
        }
    }

    #[test]
    fn test_ai_divergence_rejects_a_dropped_final_word() {
        for (pre, ai) in [
            ("this is a test", "This is a"),
            ("are you good to go right now", "Are you good to go right"),
            ("please restart the build server now", "Please restart the build server"),
        ] {
            let (_g, _d, diverged) = check_ai_divergence(pre, ai);
            assert!(diverged, "dropped last word not caught: {pre:?} -> {ai:?}");
        }
    }

    #[test]
    fn test_ai_divergence_allows_trailing_filler_removal_and_normal_cleanup() {
        for (pre, ai) in [
            ("send the file okay", "Send the file."),
            ("send the report please", "Send the report."),
            ("this is a test", "This is a test."),
            ("are you good to go right now", "Are you good to go right now?"),
        ] {
            let (_g, _d, diverged) = check_ai_divergence(pre, ai);
            assert!(!diverged, "normal cleanup rejected: {pre:?} -> {ai:?}");
        }
    }

    #[test]
    fn test_hallucinated_fragment_guard() {
        // Silence is judged from audio. A pause must not delete a spoken letter.
        assert!(!is_hallucinated_fragment("D", 2.5));
        assert!(!is_hallucinated_fragment("D.", 3.0));
        assert!(!is_hallucinated_fragment("g", 5.0));
        assert!(is_hallucinated_fragment("...", 2.0));
        assert!(is_hallucinated_fragment("", 5.0));
        // A deliberate single-letter dictation is a quick tap and must pass through.
        assert!(!is_hallucinated_fragment("D", 0.5));
        assert!(!is_hallucinated_fragment("D", 0.9));
        // Real (even tiny) dictations are never fragments.
        assert!(!is_hallucinated_fragment("hi", 2.0));
        assert!(!is_hallucinated_fragment("a", 0.4));
        assert!(!is_hallucinated_fragment("This is a test", 3.0));
    }

    #[test]
    fn test_nemotron_decode_path_policy() {
        use NemotronDecodePath::*;
        // No live text (session failed or produced nothing) -> batch, as before.
        assert_eq!(nemotron_decode_path(4.0, false, 0.0, true), BatchPrimary);
        assert_eq!(nemotron_decode_path(40.0, false, 0.0, true), BatchPrimary);
        // Short clip with live text -> ONE batch decode (not live + verify): the reported
        // "no longer instant" regression was the second full decode on every short clip.
        assert_eq!(nemotron_decode_path(4.0, true, 2.5, true), BatchPrimary);
        assert_eq!(nemotron_decode_path(8.0, true, 2.5, true), BatchPrimary);
        // Long, healthy live output -> keep the streaming result, no second decode.
        assert_eq!(nemotron_decode_path(30.0, true, 2.5, true), LiveOnly);
        // Long but sparse live output -> verify with batch and keep the richer transcript.
        assert_eq!(nemotron_decode_path(30.0, true, 0.3, true), BatchVerify);
        // Escape hatch: verification off restores pure live streaming everywhere.
        assert_eq!(nemotron_decode_path(4.0, true, 2.5, false), LiveOnly);
        assert_eq!(nemotron_decode_path(30.0, true, 0.3, false), LiveOnly);
        // With verification off and no live text, batch is still the only option.
        assert_eq!(nemotron_decode_path(30.0, false, 0.0, false), BatchPrimary);
    }

    #[test]
    fn comparison_reuses_batch_when_the_main_path_already_decodes_it() {
        assert!(background_nemotron_compare(true, NemotronDecodePath::LiveOnly));
        assert!(!background_nemotron_compare(false, NemotronDecodePath::LiveOnly));
        assert!(!background_nemotron_compare(true, NemotronDecodePath::BatchPrimary));
        assert!(!background_nemotron_compare(true, NemotronDecodePath::BatchVerify));
    }

    #[test]
    fn test_should_retry_empty_batch_only_on_total_failure() {
        // Nothing from either decode: one retry is worth it.
        assert!(should_retry_empty_batch(true, true));
        // A transcript from either side: no retry, take what we have.
        assert!(!should_retry_empty_batch(false, true));
        assert!(!should_retry_empty_batch(true, false));
        assert!(!should_retry_empty_batch(false, false));
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
