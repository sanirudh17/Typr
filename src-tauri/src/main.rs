#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
#[cfg(not(windows))]
use tauri::image::Image;
use tauri::{Listener, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_autostart::{ManagerExt, MacosLauncher};
use tokio::sync::mpsc::Sender;

use typr_lib::audio;
use typr_lib::downloader;
use typr_lib::recorder::{Recorder, RecordingState};
use typr_lib::settings::Settings;
use typr_lib::context_detector::{parse_category, AppRule, RunningApp};
use typr_lib::transcribe_local;
use typr_lib::dictionary::Dictionary;

use typr_lib::history::History;

#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::CreateMutexW;

/// True when launched with `--hidden` (Windows login auto-start). The main window
/// stays hidden in the tray; the hotkey still works.
fn launched_hidden() -> bool {
    std::env::args().any(|a| a == "--hidden")
}

/// Which configured hotkey fired. `Secondary` routes the dictation through the
/// configured AI profile for that one session.
#[derive(Clone, Copy, Debug, PartialEq)]
enum HotkeySource {
    Primary,
    Secondary,
    Write,
    CancelWrite,
}

/// A global-shortcut press/release tagged with the hotkey it came from.
#[derive(Clone, Copy, Debug)]
struct HotkeyEvent {
    source: HotkeySource,
    state: ShortcutState,
}

/// Register one accelerator, forwarding its press/release events (tagged with
/// `source`) over `tx`. Returns `Err` if the OS/another app already owns it.
fn register_hotkey(
    app: &tauri::AppHandle,
    accelerator: &str,
    source: HotkeySource,
    tx: &Sender<HotkeyEvent>,
) -> Result<(), String> {
    let tx = tx.clone();
    let held = AtomicBool::new(false);
    app.global_shortcut()
        .on_shortcut(accelerator, move |app, shortcut, event| {
            if matches!(source, HotkeySource::Primary | HotkeySource::Secondary) && app.state::<AppState>().recorder.is_write_active() {
                return;
            }
            if matches!(source, HotkeySource::Write) {
                if event.state == ShortcutState::Released { held.store(false, Ordering::SeqCst); }
                else if held.swap(true, Ordering::SeqCst) { return; }
                if event.state == ShortcutState::Pressed && app.state::<AppState>().recorder.get_state() == RecordingState::Transcribing {
                    use tauri::Emitter;
                    let _ = app.emit("show-toast", "An operation is in progress. Wait for it to finish, or press Escape to cancel Write Mode.");
                    return;
                }
            }
            println!("[Typr] Hotkey event: {:?} state={:?}", shortcut, event.state);
            let _ = tx.try_send(HotkeyEvent { source, state: event.state });
        })
        .map_err(|e| e.to_string())
}

/// Best-effort registration of the secondary hotkey when one is configured.
fn register_secondary_if_set(app: &tauri::AppHandle, state: &AppState) {
    let secondary = state.settings.lock().unwrap().hotkey_secondary.clone();
    if !secondary.is_empty() {
        match register_hotkey(app, &secondary, HotkeySource::Secondary, &state.hotkey_tx) {
            Ok(_) => println!("[Typr] Secondary hotkey registered: {}", secondary),
            Err(e) => eprintln!("[Typr] Secondary hotkey unavailable ({}): {}", secondary, e),
        }
    }
    register_write_if_enabled(app, state);
}

fn register_write_if_enabled(app: &tauri::AppHandle, state: &AppState) {
    let settings = state.settings.lock().unwrap().clone();
    if !settings.write_enabled { return; }
    for (key, source) in [(&settings.write_hotkey, HotkeySource::Write)] {
        if app.global_shortcut().is_registered(key.as_str()) { continue; }
        if let Err(e) = register_hotkey(app, key, source, &state.hotkey_tx) {
            use tauri::Emitter;
            let _ = app.emit("show-notification", format!("Write Mode shortcut {key} is unavailable. Change it in Recording settings."));
            eprintln!("[Typr] Write Mode shortcut unavailable: {e}");
        }
    }
}

fn validate_shortcuts(settings: &Settings) -> Result<(), String> {
    typr_lib::hotkey::validate_accelerator(&settings.write_hotkey)?;
    let mut keys = vec![settings.hotkey.as_str()];
    if !settings.hotkey_secondary.is_empty() { keys.push(&settings.hotkey_secondary); }
    if settings.write_enabled { keys.push(settings.write_hotkey.as_str()); }
    let mut parsed = Vec::new();
    for key in keys {
        typr_lib::hotkey::validate_accelerator(key)?;
        let shortcut = key.parse::<tauri_plugin_global_shortcut::Shortcut>().map_err(|e| e.to_string())?;
        if parsed.contains(&shortcut) { return Err("Each shortcut needs a different key combination.".into()); }
        parsed.push(shortcut);
    }
    Ok(())
}

fn register_saved_shortcuts(app: &tauri::AppHandle, state: &AppState, settings: &Settings) -> Result<(), String> {
    validate_shortcuts(settings)?;
    register_hotkey(app, &settings.hotkey, HotkeySource::Primary, &state.hotkey_tx)?;
    if !settings.hotkey_secondary.is_empty() { register_hotkey(app, &settings.hotkey_secondary, HotkeySource::Secondary, &state.hotkey_tx)?; }
    if settings.write_enabled {
        register_hotkey(app, &settings.write_hotkey, HotkeySource::Write, &state.hotkey_tx)?;
    }
    Ok(())
}

fn change_shortcuts(app: &tauri::AppHandle, state: &AppState, edit: impl FnOnce(&mut Settings)) -> Result<Settings, String> {
    state.recorder.run_when_idle(|| {
        let old = state.settings.lock().unwrap().clone();
        let mut new = old.clone(); edit(&mut new);
        let result: Result<(), String> = (|| {
            validate_shortcuts(&new)?;
            app.global_shortcut().unregister_all().map_err(|e| e.to_string())?;
            register_saved_shortcuts(app, state, &new)?;
            new.save(&state.app_dir)?;
            Ok(())
        })();
        if let Err(e) = result {
            let _ = app.global_shortcut().unregister_all();
            let _ = register_saved_shortcuts(app, state, &old);
            return Err(format!("Could not set the shortcut: {e}. Your settings are unchanged."));
        }
        *state.settings.lock().unwrap() = new.clone();
        Ok(new)
    })
}

#[tauri::command]
fn set_write_enabled(app: tauri::AppHandle, state: State<AppState>, enabled: bool) -> Result<(), String> {
    change_shortcuts(&app, &state, |s| s.write_enabled = enabled)?; Ok(())
}
#[tauri::command]
fn set_write_hotkey(app: tauri::AppHandle, state: State<AppState>, accelerator: String) -> Result<String, String> {
    Ok(change_shortcuts(&app, &state, |s| s.write_hotkey = accelerator)?.write_hotkey)
}
struct AppState {
    recorder: Recorder,
    settings: Mutex<Settings>,
    history: Mutex<History>,
    dictionary: Mutex<Dictionary>,
    app_dir: PathBuf,
    hotkey_tx: Sender<HotkeyEvent>,
}

#[cfg(windows)]
struct SingleInstanceGuard(HANDLE);

#[cfg(windows)]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

#[cfg(windows)]
fn acquire_single_instance() -> Result<SingleInstanceGuard, String> {
    let mutex_name: Vec<u16> = "Local\\TyprSingleInstanceMutex"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };
    if handle.is_null() {
        return Err("Failed to create single-instance mutex".to_string());
    }

    let last_error = unsafe { GetLastError() };
    if last_error == ERROR_ALREADY_EXISTS {
        unsafe {
            let _ = CloseHandle(handle);
        }
        return Err("Another Typr instance is already running".to_string());
    }

    Ok(SingleInstanceGuard(handle))
}

#[cfg(not(windows))]
struct SingleInstanceGuard;

#[cfg(not(windows))]
fn acquire_single_instance() -> Result<SingleInstanceGuard, String> {
    Ok(SingleInstanceGuard)
}

/// When a second process hits the single-instance mutex, try to bring the
/// already-running window to the front and center it, so a Start-menu /
/// shortcut launch while the app is hidden to tray (background mode) still
/// "opens" the window in the middle of the screen instead of silently
/// exiting. Best-effort: failure is silently ignored and the process just
/// exits as before.
#[cfg(windows)]
fn try_focus_existing_window() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetSystemMetrics, SetForegroundWindow, SetWindowPos, ShowWindow,
        SM_CXSCREEN, SM_CYSCREEN, SW_RESTORE, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    };
    unsafe {
        let title: Vec<u16> = "Typr".encode_utf16().chain(std::iter::once(0)).collect();
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        if hwnd.is_null() {
            return;
        }
        // Center on the primary monitor using the configured window size.
        // 1160×720 is the default in tauri.conf.json; centering via the OS
        // metrics keeps the second-launch behaviour consistent with the
        // `center: true` / `window.center()` path in the main instance.
        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);
        let win_w = 1160;
        let win_h = 720;
        let x = ((screen_w - win_w) / 2).max(0);
        let y = ((screen_h - win_h) / 2).max(0);
        let _ = ShowWindow(hwnd, SW_RESTORE);
        let _ = SetWindowPos(hwnd, std::ptr::null_mut(), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        let _ = SetForegroundWindow(hwnd);
    }
}

fn get_app_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.typr.app")
}

fn hotkey_candidates(preferred: &str) -> Vec<String> {
    let mut candidates = vec![preferred.to_string()];
    for fallback in [
        "CmdOrCtrl+Alt+Space",
        "CmdOrCtrl+Shift+D",
        "CmdOrCtrl+Alt+D",
        "CmdOrCtrl+Shift+V",
    ] {
        if fallback != preferred {
            candidates.push(fallback.to_string());
        }
    }
    candidates
}

/// Re-sync tao's cached visibility flag with the real window (bug #12).
///
/// tao tracks visibility in an internal flag and only calls ShowWindow when that flag
/// *changes* — `apply_diff` early-returns on an empty diff. A second process re-showing
/// the window with raw Win32 (`try_focus_existing_window`) leaves the window really
/// visible while tao's flag still says hidden. From then on:
/// - close (`set_visible(false)`) no-ops — the button "flashes red" but nothing hides;
/// - any *other* flag change (maximize/minimize/style) reaches the
///   `!new.contains(VISIBLE) → ShowWindow(SW_HIDE)` branch and the window vanishes.
/// Re-asserting `set_visible(true)` while the window is really visible forces tao's
/// flag back in sync; when already in sync it is a cheap no-op.
fn resync_visibility(w: &tauri::WebviewWindow) {
    if w.is_visible().unwrap_or(false) {
        let _ = w.show();
    }
}

/// Shows the main window once the frontend DOM has committed and painted its initial frame.
/// Keeps the window invisible during cold-start WebView2 initialization to avoid the black/blank flash.
#[tauri::command]
fn show_main_window(app: tauri::AppHandle) {
    if launched_hidden() {
        return;
    }
    if let Some(w) = app.get_webview_window("main") {
        resync_visibility(&w);
        if !w.is_visible().unwrap_or(false) {
            let _ = w.center();
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
        }
    }
}

/// Frontend maximize button. Resyncs visibility first: without it, toggling maximize on a
/// window whose cached VISIBLE flag is stale hides the window instead (bug #12).
#[tauri::command]
fn toggle_app_maximize(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        resync_visibility(&w);
        if w.is_maximized().unwrap_or(false) {
            let _ = w.unmaximize();
        } else {
            let _ = w.maximize();
        }
    }
}

/// Disables the Windows DWM open/show transition animation so the frameless window
/// snaps in cleanly without an OS fade/black-frame animation.
#[cfg(windows)]
fn disable_transitions(window: &tauri::WebviewWindow) {
    if let Ok(hwnd) = window.hwnd() {
        #[link(name = "dwmapi")]
        extern "system" {
            fn DwmSetWindowAttribute(
                hwnd: *mut std::ffi::c_void,
                dwAttribute: u32,
                pvAttribute: *const std::ffi::c_void,
                cbAttribute: u32,
            ) -> i32;
        }
        const DWMWA_TRANSITIONS_FORCEDISABLED: u32 = 3;
        let on: i32 = 1;
        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd.0 as *mut std::ffi::c_void,
                DWMWA_TRANSITIONS_FORCEDISABLED,
                &on as *const i32 as *const std::ffi::c_void,
                std::mem::size_of::<i32>() as u32,
            );
        }
    }
}

#[cfg(not(windows))]
fn disable_transitions(_window: &tauri::WebviewWindow) {}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
async fn list_ai_models(state: State<'_, AppState>, force_refresh: bool, expected_key: String) -> Result<typr_lib::ai_models::Catalog, String> {
    let key = state.settings.lock().unwrap().groq_api_key.clone();
    // Clicking Refresh can race the key field's blur/save. Do not return the
    // previous account's catalog under the new key's UI request. Neither key
    // nor service error bodies are included in this error.
    if key.trim() != expected_key.trim() {
        return Err("The API key is still changing. Refresh again after saving it.".into());
    }
    Ok(typr_lib::ai_models::discover(&key, &state.app_dir, force_refresh).await)
}

#[tauri::command]
fn get_history(state: State<AppState>) -> History {
    state.history.lock().unwrap().clone()
}

#[tauri::command]
async fn save_settings(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    mut settings: Settings,
) -> Result<(), String> {
    settings.normalize_startup();
    state.recorder.learning.cancel(&app);

    // Reconcile the OS auto-start registry entry with the toggle.
    let autolaunch = app.autolaunch();
    let currently_enabled = autolaunch.is_enabled().unwrap_or(false);
    if settings.autostart && !currently_enabled {
        let _ = autolaunch.enable();
    } else if !settings.autostart && currently_enabled {
        let _ = autolaunch.disable();
    }

    let old_settings = state.settings.lock().unwrap().clone();
    settings.write_enabled = old_settings.write_enabled;
    settings.write_hotkey = old_settings.write_hotkey.clone();
    let mic_changed = old_settings.microphone != settings.microphone;
    let engine_changed = old_settings.engine != settings.engine;
    let model_changed = old_settings.whisper_model != settings.whisper_model;
    let parakeet_model_changed = old_settings.parakeet_model != settings.parakeet_model;
    let nemotron_model_changed = old_settings.nemotron_model != settings.nemotron_model;
    let acceleration_changed = old_settings.whisper_acceleration != settings.whisper_acceleration;

    settings.save(&state.app_dir)?;
    *app.state::<typr_lib::settings::RuntimeSettings>().acceleration.lock().unwrap() = settings.whisper_acceleration.clone();
    *state.settings.lock().unwrap() = settings.clone();
    state.recorder.learning.set_enabled(settings.learn_from_corrections);

    if old_settings.theme != settings.theme {
        if let Some(win) = app.get_webview_window("main") {
            let light = match settings.theme.as_str() {
                "light" => true,
                "dark" => false,
                _ => !matches!(dark_light::detect(), Ok(dark_light::Mode::Dark)),
            };
            let bg = if light {
                tauri::window::Color(0xee, 0xf0, 0xf3, 255)
            } else {
                tauri::window::Color(0x09, 0x09, 0x0b, 255)
            };
            let _ = win.set_background_color(Some(bg));
        }
    }

    if mic_changed {
        let recorder = state.recorder.clone();
        let mic = settings.microphone.clone();
        tauri::async_runtime::spawn(async move {
            println!("[Typr] Re-initializing audio stream in background for new mic: {}", mic);
            if let Err(e) = recorder.pre_initialize(&mic) {
                eprintln!("[Typr] Failed to pre-initialize new mic: {}", e);
                return;
            }
            // Warm the newly selected device so its first record captures instantly.
            recorder.begin_warm();
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            recorder.end_warm();
            println!("[Typr] New mic warm-up complete (mic idle)");
        });
    }

    // Handle on-demand server management
    let app_clone = app.clone();
    let app_dir_clone = state.app_dir.clone();
    let settings_clone = settings.clone();
    tauri::async_runtime::spawn(async move {
        // Settings saves can arrive faster than a model loads. Drop stale work
        // before it can reload an engine the user has already left.
        let current = app_clone.state::<AppState>().settings.lock().unwrap().clone();
        if current.engine != settings_clone.engine || current.whisper_model != settings_clone.whisper_model
            || current.parakeet_model != settings_clone.parakeet_model || current.nemotron_model != settings_clone.nemotron_model
            || current.whisper_acceleration != settings_clone.whisper_acceleration { return; }
        let recorder = app_clone.state::<AppState>().recorder.clone();
        let next_engine = settings_clone.engine.clone();
        let _ = tokio::task::spawn_blocking(move || recorder.run_when_idle(|| {
            if next_engine != "parakeet" { typr_lib::transcribe_parakeet::try_release_model(); }
            if next_engine != "nemotron" { typr_lib::transcribe_nemotron::try_release_model(); }
            Ok(())
        })).await;
        // If we just left the Whisper engine (for Parakeet or Cloud), stop the CUDA server now
        // to free VRAM instead of waiting on the idle reaper. Previously only the Cloud switch
        // did this, so Whisper -> Parakeet kept the dGPU engaged for ~3 minutes.
        if typr_lib::whisper_server::should_stop_on_engine_switch(
            &old_settings.engine,
            &settings_clone.engine,
        ) {
            println!(
                "[Typr] Leaving Whisper for '{}': stopping Whisper HTTP server to free VRAM...",
                settings_clone.engine
            );
            typr_lib::whisper_server::stop_server().await;
        }

        // Warm the engine we switched TO, so the first dictation isn't stalled by a cold load.
        match settings_clone.engine.as_str() {
            "local" => {
                if engine_changed || model_changed || acceleration_changed {
                    let model_path = app_dir_clone
                        .join(transcribe_local::model_filename(&settings_clone.whisper_model));
                    println!("[Typr] On-demand engine/model change: ensuring Whisper server is running with {:?}", model_path);
                    if let Err(e) = typr_lib::whisper_server::ensure_running(&app_clone, &model_path).await {
                        eprintln!("[Typr] Failed to start local Whisper HTTP server: {}", e);
                    }
                }
            }
            "parakeet" => {
                if engine_changed || parakeet_model_changed {
                    // Parakeet loads its ONNX model (~4-6s) into a cached recognizer on first
                    // use. Prewarm it now — in a blocking task, since it is CPU-bound — so the
                    // first dictation after selecting Parakeet isn't stalled by that build. This
                    // is Parakeet's analogue of the Whisper server pre-start above.
                    let model_dir = app_dir_clone
                        .join(typr_lib::transcribe_parakeet::model_dir_name(&settings_clone.parakeet_model));
                    println!("[Typr] Engine/model change to Parakeet: prewarming {:?}", model_dir);
                    tauri::async_runtime::spawn_blocking(move || {
                        if let Err(e) = typr_lib::transcribe_parakeet::prewarm(&model_dir) {
                            eprintln!("[Typr] Parakeet prewarm failed: {}", e);
                        }
                    });
                }
            }
            "nemotron" => {
                if engine_changed || nemotron_model_changed {
                    let model_dir = app_dir_clone
                        .join(typr_lib::transcribe_nemotron::model_dir_name(&settings_clone.nemotron_model));
                    println!("[Typr] Engine/model change to Nemotron: prewarming {:?}", model_dir);
                    tauri::async_runtime::spawn_blocking(move || {
                        if let Err(e) = typr_lib::transcribe_nemotron::prewarm(&model_dir) {
                            eprintln!("[Typr] Nemotron prewarm failed: {}", e);
                        }
                    });
                }
            }
            _ => {}
        }
    });

    Ok(())
}

#[tauri::command]
async fn set_hotkey(app: tauri::AppHandle, state: State<'_, AppState>, accelerator: String) -> Result<String, String> {
    Ok(change_shortcuts(&app, &state, |s| s.hotkey = accelerator)?.hotkey)
}

#[tauri::command]
fn suspend_hotkeys(app: tauri::AppHandle, state: State<AppState>) -> Result<(), String> {
    if state.recorder.get_state() != RecordingState::Ready { return Err("Finish your recording before changing shortcuts.".into()); }
    app.global_shortcut().unregister_all().map_err(|e| e.to_string())
}

#[tauri::command]
fn resume_hotkeys(app: tauri::AppHandle, state: State<AppState>) -> Result<(), String> {
    let settings = state.settings.lock().unwrap().clone();
    register_saved_shortcuts(&app, &state, &settings)
}

#[tauri::command]
async fn set_secondary_hotkey(app: tauri::AppHandle, state: State<'_, AppState>, accelerator: String) -> Result<String, String> {
    Ok(change_shortcuts(&app, &state, |s| s.hotkey_secondary = accelerator)?.hotkey_secondary)
}

#[tauri::command]
fn clear_secondary_hotkey(app: tauri::AppHandle, state: State<AppState>) -> Result<(), String> {
    change_shortcuts(&app, &state, |s| s.hotkey_secondary.clear())?; Ok(())
}

#[tauri::command]
fn set_secondary_profile(state: State<AppState>, profile: String) -> Result<(), String> {
    if !matches!(profile.as_str(), "cleanup" | "prompt" | "auto") {
        return Err(format!("Unknown profile: {}", profile));
    }
    let mut settings = state.settings.lock().unwrap().clone();
    settings.secondary_profile = profile.clone();
    settings.save(&state.app_dir)?;
    *state.settings.lock().unwrap() = settings;
    println!("[Typr] Secondary profile set to {}", profile);
    Ok(())
}

#[tauri::command]
fn get_dictionary(state: State<AppState>) -> Dictionary {
    state.dictionary.lock().unwrap().clone()
}
#[tauri::command]
fn save_dictionary_entry(
    app: tauri::AppHandle,
    state: State<AppState>,
    entry: typr_lib::vocabulary::Entry,
) -> Result<(), String> {
    state
        .dictionary
        .lock()
        .unwrap()
        .upsert_entry(entry, &state.app_dir)?;
    use tauri::Emitter;
    let _ = app.emit("dictionary-updated", ());
    Ok(())
}
#[tauri::command]
fn delete_dictionary_entry(
    app: tauri::AppHandle,
    state: State<AppState>,
    id: String,
) -> Result<(), String> {
    state
        .dictionary
        .lock()
        .unwrap()
        .delete_entry(&id, &state.app_dir)?;
    use tauri::Emitter;
    let _ = app.emit("dictionary-updated", ());
    Ok(())
}
#[tauri::command]
fn import_dictionary(
    app: tauri::AppHandle,
    state: State<AppState>,
    json: String,
) -> Result<usize, String> {
    let count = state
        .dictionary
        .lock()
        .unwrap()
        .import_entries(&json, &state.app_dir)?;
    use tauri::Emitter;
    let _ = app.emit("dictionary-updated", ());
    Ok(count)
}
#[tauri::command]
fn export_dictionary(state: State<AppState>) -> Result<String, String> {
    state.dictionary.lock().unwrap().export_entries()
}
#[tauri::command]
fn get_learning_suggestion(state: State<AppState>) -> Option<typr_lib::learning::Suggestion> {
    state.recorder.learning.pending()
}
#[tauri::command]
fn accept_learning_suggestion(
    app: tauri::AppHandle,
    state: State<AppState>,
    id: String,
) -> Result<(), String> {
    let settings = state.settings.lock().unwrap();
    if !settings.learn_from_corrections {
        return Err("Learn from corrections is off.".into());
    }
    state.recorder.learning.accept_with(&id, |candidate| {
        let mut dict = state.dictionary.lock().unwrap();
        let mut entry = dict
            .entries
            .iter()
            .find(|e| e.canonical_term.eq_ignore_ascii_case(&candidate.correct))
            .cloned()
            .unwrap_or_else(|| {
                let mut e = typr_lib::vocabulary::Entry::manual(candidate.correct.clone());
                e.source = typr_lib::vocabulary::Source::Learned;
                e.confidence = candidate.confidence;
                e
            });
        entry.aliases.push(candidate.wrong.clone());
        dict.upsert_entry(entry, &state.app_dir)
    })?;
    drop(settings);
    state.recorder.learning.cancel(&app);
    use tauri::Emitter;
    let _ = app.emit("dictionary-updated", ());
    Ok(())
}
#[tauri::command]
fn dismiss_learning_suggestion(app: tauri::AppHandle, state: State<AppState>, id: String) {
    state.recorder.learning.dismiss(&id, &app);
}

#[tauri::command]
fn add_dictionary_word(state: State<AppState>, word: String) -> Result<(), String> {
    state.dictionary.lock().unwrap().add_word(word, &state.app_dir)
}

#[tauri::command]
fn remove_dictionary_word(state: State<AppState>, index: usize) -> Result<(), String> {
    state.dictionary.lock().unwrap().remove_word(index, &state.app_dir)
}

#[tauri::command]
fn add_vocabulary_hint(state: State<AppState>, word: String) -> Result<(), String> {
    state.dictionary.lock().unwrap().add_vocabulary_hint(word, &state.app_dir)
}

#[tauri::command]
fn remove_vocabulary_hint(state: State<AppState>, index: usize) -> Result<(), String> {
    state.dictionary.lock().unwrap().remove_vocabulary_hint(index, &state.app_dir)
}

#[tauri::command]
fn add_replacement(
    state: State<AppState>,
    find: String,
    replace: String,
    case_sensitive: bool,
) -> Result<(), String> {
    state.dictionary.lock().unwrap().add_replacement(find, replace, case_sensitive, &state.app_dir)
}

#[tauri::command]
fn remove_replacement(state: State<AppState>, index: usize) -> Result<(), String> {
    state.dictionary.lock().unwrap().remove_replacement(index, &state.app_dir)
}

#[tauri::command]
fn get_app_rules(state: State<AppState>) -> Vec<AppRule> {
    state.settings.lock().unwrap().app_rules.clone()
}

#[tauri::command]
fn list_running_apps() -> Vec<RunningApp> {
    typr_lib::context_detector::list_running_apps()
}

#[tauri::command]
fn add_app_rule(
    state: State<AppState>,
    process_name: String,
    title_contains: Option<String>,
    category: String,
) -> Result<(), String> {
    let category = parse_category(&category)?;
    let process_name = process_name.trim().to_string();
    let title_contains = title_contains
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if process_name.is_empty() && title_contains.is_none() {
        return Err("enter an app, a title filter, or both".to_string());
    }
    let mut settings = state.settings.lock().unwrap();
    settings.app_rules.push(AppRule { process_name, title_contains, category });
    settings.save(&state.app_dir)
}

#[tauri::command]
fn remove_app_rule(state: State<AppState>, index: usize) -> Result<(), String> {
    let mut settings = state.settings.lock().unwrap();
    if index < settings.app_rules.len() {
        settings.app_rules.remove(index);
    }
    settings.save(&state.app_dir)
}

#[tauri::command]
fn delete_transcription(state: State<AppState>, id: String) -> Result<(), String> {
    state.history.lock().unwrap().delete_item(&id, &state.app_dir)
}

#[tauri::command]
async fn clear_local_data(app: tauri::AppHandle) -> Result<typr_lib::data_cleanup::ClearDataReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let report = state.recorder.run_when_idle(|| {
            state.recorder.learning.cancel(&app);
            let mut history = state.history.lock().unwrap();
            typr_lib::data_cleanup::clear_local_data(&state.app_dir, &mut history)
        })?;
        use tauri::Emitter;
        let _ = app.emit("history-updated", ());
        Ok(report)
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
fn update_transcription(state: State<AppState>, id: String, text: String) -> Result<(), String> {
    state.history.lock().unwrap().update_item(&id, text, &state.app_dir)
}

#[tauri::command]
fn write_text_file(path: String, contents: String) -> Result<(), String> {
    std::fs::write(&path, contents).map_err(|e| e.to_string())
}

#[tauri::command]
fn list_microphones() -> Vec<audio::MicDevice> {
    audio::list_microphones()
}

#[tauri::command]
fn check_cuda_runtime(app: tauri::AppHandle) -> bool {
    typr_lib::whisper_runtime::cuda_installed(&app)
}
#[tauri::command]
fn show_write_preview(window: tauri::WebviewWindow) -> Result<(), String> {
    if window.label() != "write-preview" { return Err("Unavailable outside the rewrite preview.".into()); }
    window.center().map_err(|e| e.to_string())?;
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
}
#[tauri::command]
fn copy_write_preview(window: tauri::WebviewWindow, text: String) -> Result<(), String> {
    if window.label() != "write-preview" || text.chars().count() > 16000 || text.contains('\0') { return Err("The rewrite could not be copied.".into()); }
    arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text)).map_err(|_| "The clipboard is busy. Try copying again.".into())
}
#[tauri::command]
async fn download_cuda_runtime(app: tauri::AppHandle) -> Result<(), String> {
    typr_lib::whisper_runtime::download_cuda(app).await
}

#[tauri::command]
fn get_recording_state(state: State<AppState>) -> RecordingState {
    state.recorder.get_state()
}

#[tauri::command]
fn get_amplitude(state: State<AppState>) -> Vec<f32> {
    state.recorder.get_amplitude()
}

#[tauri::command]
fn get_frequency_bands(state: State<AppState>) -> Vec<f32> {
    state.recorder.get_frequency_bands()
}

#[tauri::command]
fn check_model_downloaded(state: State<AppState>, model_size: String) -> bool {
    let model_file = transcribe_local::model_filename(&model_size);
    state.app_dir.join(&model_file).exists()
}

#[tauri::command]
fn check_parakeet_downloaded(state: State<AppState>, variant: String) -> bool {
    let dir = state.app_dir.join(typr_lib::transcribe_parakeet::model_dir_name(&variant));
    typr_lib::transcribe_parakeet::model_files_present(&dir)
}

/// Fetch and unpack a Parakeet model.
///
/// Parakeet ships as a bz2 archive of four files where Whisper ships one .bin, so the existing
/// downloader handles the fetch and extraction is added on top. Extraction runs in-process
/// rather than shelling out to tar, which a Windows machine may not have.
#[tauri::command]
async fn download_parakeet_model(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    variant: String,
) -> Result<(), String> {
    use std::io::Read;

    let url = typr_lib::transcribe_parakeet::model_download_url(&variant);
    let target = state
        .app_dir
        .join(typr_lib::transcribe_parakeet::model_dir_name(&variant));
    let archive = state.app_dir.join("parakeet-download.tar.bz2");

    downloader::download_model(app.clone(), &url, &archive).await?;

    std::fs::create_dir_all(&target).map_err(|e| format!("Could not create model dir: {}", e))?;
    let file = std::fs::File::open(&archive).map_err(|e| format!("Could not open archive: {}", e))?;
    let mut tar = tar::Archive::new(bzip2::read::BzDecoder::new(file));

    // Entries are re-rooted by filename: the archive's top-level directory name is not
    // guaranteed to match ours, and only the four files the recognizer needs are kept — the
    // bundled sample WAVs are skipped.
    let wanted = ["encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx", "tokens.txt"];
    for entry in tar.entries().map_err(|e| format!("Could not read archive: {}", e))? {
        let mut entry = entry.map_err(|e| format!("Corrupt archive entry: {}", e))?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        if !wanted.contains(&name.as_str()) {
            continue;
        }
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).map_err(|e| format!("Could not extract {}: {}", name, e))?;
        std::fs::write(target.join(&name), buf)
            .map_err(|e| format!("Could not write {}: {}", name, e))?;
    }
    let _ = std::fs::remove_file(&archive);

    // A partial extract would otherwise surface much later as an opaque failure inside the
    // C API, so it is caught here where the cause is still obvious.
    if !typr_lib::transcribe_parakeet::model_files_present(&target) {
        return Err("Download finished but the model files are incomplete.".to_string());
    }
    Ok(())
}

#[tauri::command]
fn check_nemotron_downloaded(state: State<AppState>, variant: String) -> bool {
    let dir = state.app_dir.join(typr_lib::transcribe_nemotron::model_dir_name(&variant));
    typr_lib::transcribe_nemotron::model_files_present(&dir)
}

#[tauri::command]
async fn download_nemotron_model(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    variant: String,
) -> Result<(), String> {
    let url = typr_lib::transcribe_nemotron::model_download_url(&variant);
    let target = state
        .app_dir
        .join(typr_lib::transcribe_nemotron::model_dir_name(&variant));
    std::fs::create_dir_all(&target).map_err(|e| format!("Could not create model dir: {}", e))?;

    if url.ends_with(".tar.bz2") {
        use std::io::Read;
        let archive = state.app_dir.join("nemotron-download.tar.bz2");
        downloader::download_model(app.clone(), &url, &archive).await?;
        let file = std::fs::File::open(&archive).map_err(|e| format!("Could not open archive: {}", e))?;
        let mut tar = tar::Archive::new(bzip2::read::BzDecoder::new(file));
        let wanted = ["encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx", "tokens.txt"];
        for entry in tar.entries().map_err(|e| format!("Could not read archive: {}", e))? {
            let mut entry = entry.map_err(|e| format!("Corrupt archive entry: {}", e))?;
            let path = entry.path().map_err(|e| e.to_string())?.into_owned();
            let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                continue;
            };
            if !wanted.contains(&name.as_str()) {
                continue;
            }
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).map_err(|e| format!("Could not extract {}: {}", name, e))?;
            std::fs::write(target.join(&name), buf)
                .map_err(|e| format!("Could not write {}: {}", name, e))?;
        }
        let _ = std::fs::remove_file(&archive);
    } else {
        let wanted = ["tokens.txt", "joiner.int8.onnx", "decoder.int8.onnx", "encoder.int8.onnx"];
        let items: Vec<downloader::MultiFileItem> = wanted
            .iter()
            .map(|&name| downloader::MultiFileItem {
                url: format!("{}/{}", url.trim_end_matches('/'), name),
                dest: target.join(name),
            })
            .collect();
        downloader::download_multiple_files(app.clone(), &items).await?;
    }

    if !typr_lib::transcribe_nemotron::model_files_present(&target) {
        return Err("Download finished but the model files are incomplete.".to_string());
    }
    Ok(())
}

#[tauri::command]
async fn delete_whisper_model(
    state: State<'_, AppState>,
    model_size: String,
) -> Result<(), String> {
    typr_lib::whisper_server::stop_server().await;
    let model_file = transcribe_local::model_filename(&model_size);
    let dest = state.app_dir.join(&model_file);
    if dest.exists() {
        std::fs::remove_file(&dest).map_err(|e| format!("Failed to delete model file: {}", e))?;
    }
    Ok(())
}

#[tauri::command]
fn delete_parakeet_model(
    state: State<'_, AppState>,
    variant: String,
) -> Result<(), String> {
    typr_lib::transcribe_parakeet::release_model();
    let target = state
        .app_dir
        .join(typr_lib::transcribe_parakeet::model_dir_name(&variant));
    if target.exists() {
        std::fs::remove_dir_all(&target).map_err(|e| format!("Failed to delete model directory: {}", e))?;
    }
    let archive = state.app_dir.join("parakeet-download.tar.bz2");
    if archive.exists() {
        let _ = std::fs::remove_file(archive);
    }
    Ok(())
}

#[tauri::command]
fn delete_nemotron_model(
    state: State<'_, AppState>,
    variant: String,
) -> Result<(), String> {
    typr_lib::transcribe_nemotron::release_model();
    let target = state
        .app_dir
        .join(typr_lib::transcribe_nemotron::model_dir_name(&variant));
    if target.exists() {
        std::fs::remove_dir_all(&target).map_err(|e| format!("Failed to delete model directory: {}", e))?;
    }
    let archive = state.app_dir.join("nemotron-download.tar.bz2");
    if archive.exists() {
        let _ = std::fs::remove_file(archive);
    }
    Ok(())
}

#[tauri::command]
async fn download_model(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    model_size: String,
) -> Result<(), String> {
    let url = transcribe_local::model_download_url(&model_size);
    let model_file = transcribe_local::model_filename(&model_size);
    let dest = state.app_dir.join(&model_file);
    downloader::download_model(app.clone(), &url, &dest).await?;

    // Proactively warm the server if the just-downloaded model is the one in use,
    // so the first local dictation isn't stalled ~15s by a cold model load.
    let (engine, selected) = {
        let s = state.settings.lock().unwrap();
        (s.engine.clone(), s.whisper_model.clone())
    };
    if typr_lib::whisper_server::should_warm_after_download(&engine, &selected, &model_size) {
        let app_clone = app.clone();
        let model_path = dest.clone();
        tauri::async_runtime::spawn(async move {
            println!("[Typr] Warming local Whisper server after download of {:?}", model_path);
            if let Err(e) = typr_lib::whisper_server::ensure_running(&app_clone, &model_path).await {
                eprintln!("[Typr] Post-download warm-up failed: {}", e);
            }
        });
    }

    Ok(())
}

/// The AI profile override this recording should carry, based on which hotkey started it.
/// Secondary → force the configured profile; Primary → none. The value travels into
/// `start_recording` and is consumed by the matching `stop_and_transcribe`, so it is scoped
/// to a single session and cannot leak to the next dictation.
fn profile_override_for(state: &AppState, source: HotkeySource) -> Option<String> {
    match source {
        HotkeySource::Secondary => Some(state.settings.lock().unwrap().secondary_profile.clone()),
        _ => None,
    }
}

#[tauri::command]
async fn toggle_recording(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    // UI button always acts as the primary (no AI override).
    do_toggle_recording(&app, &state, HotkeySource::Primary).await
}

/// Shared logic for toggle recording, used by both the Tauri command and hotkey handler.
/// Fire-and-forget pre-warm of the local Whisper server at record-start. If the local
/// engine is selected, mark activity (resets the idle reaper), and if the warm server
/// isn't already serving the chosen model, spawn a background `ensure_running` so the
/// server is hot by the time the user stops speaking. Never awaited — record-start is
/// never delayed.
fn prewarm_local(app: &tauri::AppHandle) {
    let (engine, model) = {
        let state = app.state::<AppState>();
        let settings = state.settings.lock().unwrap();
        (settings.engine.clone(), settings.whisper_model.clone())
    };
    if engine == "parakeet" {
        let state = app.state::<AppState>();
        let variant = state.settings.lock().unwrap().parakeet_model.clone();
        let directory = state.app_dir.join(typr_lib::transcribe_parakeet::model_dir_name(&variant));
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(error) = typr_lib::transcribe_parakeet::prewarm(&directory) { eprintln!("[Typr] Parakeet prewarm: {error}"); }
        });
        return;
    }
    if engine != "local" {
        return;
    }
    // Record-start counts as activity even when the server is already warm.
    typr_lib::whisper_server::note_activity();

    let model_path = app
        .state::<AppState>()
        .app_dir
        .join(transcribe_local::model_filename(&model));
    let warm_matches = typr_lib::whisper_server::warm_server_matches(
        typr_lib::whisper_server::current_model_key().as_deref(),
        &model_path.to_string_lossy(),
    ) && typr_lib::whisper_server::current_backend_is_gpu() == typr_lib::whisper_runtime::uses_cuda(app);
    if !typr_lib::whisper_server::should_prewarm(&engine, warm_matches) {
        return;
    }

    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = typr_lib::whisper_server::ensure_running(&app_clone, &model_path).await {
            eprintln!("[Typr] Prewarm ensure_running failed: {}", e);
        }
    });
}

async fn do_write(app: &tauri::AppHandle, state: &AppState) -> Result<(), String> {
    match state.recorder.get_state() {
        RecordingState::Ready => {
            let settings = state.settings.lock().unwrap().clone();
            let tx = state.hotkey_tx.clone();
            // The callback signals directly: the receiver may be awaiting ASR/AI.
            app.global_shortcut().on_shortcut("Escape", move |app, _, event| {
                if event.state == ShortcutState::Pressed {
                    app.state::<AppState>().recorder.cancel_write();
                    let _ = tx.try_send(HotkeyEvent { source: HotkeySource::CancelWrite, state: ShortcutState::Pressed });
                }
            }).map_err(|_| "Escape is unavailable for cancellation. Close the app using it and retry Write Mode.".to_string())?;
            prewarm_local(app);
            let result = state.recorder.start_write(app, &settings, &state.app_dir).await;
            if result.is_err() || state.recorder.get_state() == RecordingState::Ready { let _ = app.global_shortcut().unregister("Escape"); }
            result
        }
        RecordingState::Recording if state.recorder.is_write_active() => {
            let settings = state.settings.lock().unwrap().clone();
            let result = state.recorder.stop_and_transcribe(app, &settings, &state.history, &state.dictionary, &state.app_dir).await.map(|_| ());
            let _ = app.global_shortcut().unregister("Escape");
            result
        }
        _ => Err("Finish your current recording before starting Write Mode.".into()),
    }
}

async fn do_toggle_recording(
    app: &tauri::AppHandle,
    state: &AppState,
    source: HotkeySource,
) -> Result<String, String> {
    if state.recorder.is_write_active() { return Err("Finish Write Mode with its hotkey, or press Escape to cancel.".into()); }
    let current_state = state.recorder.get_state();
    match current_state {
        RecordingState::Ready => {
            let session_override = profile_override_for(state, source);
            prewarm_local(app);
            let settings = state.settings.lock().unwrap().clone();
            let mic = settings.microphone.clone();
            let nemotron_dir = state
                .app_dir
                .join(typr_lib::transcribe_nemotron::model_dir_name(&settings.nemotron_model));
            let parakeet_dir = state.app_dir.join(typr_lib::transcribe_parakeet::model_dir_name(&settings.parakeet_model));
            state
                .recorder
                .start_recording(app, &mic, session_override, &settings.engine, Some(&nemotron_dir), Some(&parakeet_dir))?;
            Ok("recording".to_string())
        }
        RecordingState::Recording => {
            let settings = state.settings.lock().unwrap().clone();
            let result = state
                .recorder
                .stop_and_transcribe(app, &settings, &state.history, &state.dictionary, &state.app_dir)
                .await?;
            Ok(result)
        }
        RecordingState::Transcribing => {
            Err("Currently transcribing, please wait".to_string())
        }
    }
}

/// Set once the window has hidden to tray this run, so the "still running" hint shows
/// only the first time per process, not on every close.
static FIRST_HIDE_NOTIFIED: AtomicBool = AtomicBool::new(false);

async fn handle_close_requested(app: &tauri::AppHandle) {
    let (background, app_dir) = {
        let state = app.state::<AppState>();
        let bg = state.settings.lock().unwrap().background_mode;
        let dir = state.app_dir.clone();
        (bg, dir)
    };
    typr_lib::debug_log::log(
        &app_dir,
        &format!("[Typr] Close requested (background_mode: {})", background),
    );
    if background {
        // Hide to tray, keep running so the hotkey still works.
        if let Some(w) = app.get_webview_window("main") {
            // Re-sync tao's cached VISIBLE flag first — otherwise hide() silently no-ops
            // on a stale flag and the window never closes (bug #12).
            resync_visibility(&w);
            let _ = w.hide();
        }
        if !FIRST_HIDE_NOTIFIED.swap(true, Ordering::SeqCst) {
            let _ = app
                .notification()
                .builder()
                .title("Typr is still running")
                .body("Right-click the tray icon to quit.")
                .show();
        }
    } else {
        println!("[Typr] Main window close requested, exiting app");
        typr_lib::debug_log::log(&app_dir, "[Typr] Exiting app cleanly, stopping whisper server");
        typr_lib::whisper_server::stop_server().await;
        std::process::exit(0);
    }
}

#[tauri::command]
async fn request_app_close(app: tauri::AppHandle) {
    handle_close_requested(&app).await;
}

fn main() {
    let _single_instance = match acquire_single_instance() {
        Ok(guard) => guard,
        Err(message) => {
            eprintln!("[Typr] {}", message);
            #[cfg(windows)]
            try_focus_existing_window();
            return;
        }
    };

    println!("[Typr] Starting process PID {}", std::process::id());

    let app_dir = get_app_dir();
    let settings = Settings::load(&app_dir);
    let history = History::load(&app_dir);
    let dictionary = Dictionary::load(&app_dir);
    let initial_hotkey = settings.hotkey.clone();
    let runtime_settings = typr_lib::settings::RuntimeSettings {
        acceleration: Mutex::new(settings.whisper_acceleration.clone()),
    };
    let recorder=Recorder::new();
    recorder.learning.set_enabled(settings.learn_from_corrections);

    let (hotkey_tx, hotkey_rx) = tokio::sync::mpsc::channel::<HotkeyEvent>(32);

    tauri::Builder::default()
        .manage(runtime_settings)
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        // In-app updates. The updater fetches latest.json from the GitHub release, verifies
        // the bundle against the public key in tauri.conf.json, and runs the NSIS installer
        // in passive mode. `process` provides the relaunch after it finishes.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(AppState {
            recorder,
            settings: Mutex::new(settings),
            history: Mutex::new(history),
            dictionary: Mutex::new(dictionary),
            app_dir,
            hotkey_tx: hotkey_tx.clone(),
        })
        .invoke_handler(tauri::generate_handler![
            check_parakeet_downloaded,
            download_parakeet_model,
            delete_parakeet_model,
            check_nemotron_downloaded,
            download_nemotron_model,
            delete_nemotron_model,
            get_settings,
            list_ai_models,
            save_settings,
            list_microphones,
            check_cuda_runtime,
            show_write_preview,
            copy_write_preview,
            download_cuda_runtime,
            get_recording_state,
            get_amplitude,
            get_frequency_bands,
            check_model_downloaded,
            download_model,
            delete_whisper_model,
            toggle_recording,
            set_hotkey,
            set_write_enabled,
            set_write_hotkey,
            suspend_hotkeys,
            resume_hotkeys,
            set_secondary_hotkey,
            clear_secondary_hotkey,
            set_secondary_profile,
            get_history,
            delete_transcription,
            clear_local_data,
            update_transcription,
            write_text_file,
            get_dictionary,
            save_dictionary_entry,
            delete_dictionary_entry,
            import_dictionary,
            export_dictionary,
            get_learning_suggestion,
            accept_learning_suggestion,
            dismiss_learning_suggestion,
            add_dictionary_word,
            remove_dictionary_word,
            add_vocabulary_hint,
            remove_vocabulary_hint,
            add_replacement,
            remove_replacement,
            get_app_rules,
            list_running_apps,
            add_app_rule,
            remove_app_rule,
            show_main_window,
            request_app_close,
            toggle_app_maximize,
        ])
        .setup(move |app| {
            // Keep model availability current even during tray-only use.
            // This worker shares discovery's cache/deduplication with the UI,
            // and never holds settings or recorder locks during a request.
            let catalog_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    let (key, dir) = {
                        let state = catalog_handle.state::<AppState>();
                        let key = state.settings.lock().unwrap().groq_api_key.clone();
                        (key, state.app_dir.clone())
                    };
                    if !key.trim().is_empty() {
                        typr_lib::ai_models::discover(&key, &dir, false).await;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                }
            });
            // Build the main window here, not in tauri.conf: only the builder can
            // carry a per-launch native background and a Rust-truth init script.
            // A post-hoc set_background_color races first show and loses, leaving
            // a black native surface for the webview's first second — the
            // cold-start flash no in-page script can cover (no page exists yet).
            // The persisted theme seeds both: bg paints the OS surface instantly,
            // __TYPR_BOOT__ beats every page script (see index.html head).
            let theme_pref = app
                .state::<AppState>()
                .settings
                .lock()
                .unwrap()
                .theme
                .clone();
            let light = match theme_pref.as_str() {
                "light" => true,
                "dark" => false,
                // Detection failure falls back to light (Windows factory default).
                _ => !matches!(dark_light::detect(), Ok(dark_light::Mode::Dark)),
            };
            // Must match style.css --bg per theme exactly.
            let bg = if light {
                tauri::window::Color(0xee, 0xf0, 0xf3, 255)
            } else {
                tauri::window::Color(0x09, 0x09, 0x0b, 255)
            };
            // Never interpolate the raw setting: a hand-edited config could break
            // out of the string. Only the three known values pass through.
            let theme_init = match theme_pref.as_str() {
                "light" | "dark" | "system" => theme_pref.as_str(),
                _ => "system",
            };
            let init_script =
                format!(r#"window.__TYPR_BOOT__ = {{ theme: "{}" }};"#, theme_init);
            match WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                .title("Typr")
                .inner_size(1160.0, 720.0)
                .min_inner_size(700.0, 500.0)
                .decorations(false)
                .transparent(false)
                .resizable(true)
                .center()
                .visible(false)
                .initialization_script(&init_script)
                .background_color(bg)
                .build()
            {
                Ok(window) => {
                    #[cfg(not(windows))]
                    match Image::from_bytes(include_bytes!("../icons/icon.png")) {
                        Ok(icon) => {
                            if let Err(e) = window.set_icon(icon) {
                                eprintln!("[Typr] Failed to set main window icon: {}", e);
                            }
                        }
                        Err(e) => eprintln!("[Typr] Failed to load main window icon: {}", e),
                    }

                    disable_transitions(&window);

                    // Always launch centered on the current monitor, whether this is a fresh
                    // process or the window is being re-shown from the tray. The builder
                    // .center() handles initial positioning, but the explicit call
                    // guarantees the same result when opening or restoring.
                    let _ = window.center();

                    if launched_hidden() {
                        println!("[Typr] Launched hidden (auto-start); main window stays in tray");
                    } else {
                        // The window is created with visible(false) so WebView2 has time to boot,
                        // parse HTML/CSS, run loadSettings(), and paint its initial frame before
                        // appearing on screen. Once ready, main.ts calls `show_main_window` which
                        // centers and reveals the window in one smooth step with zero blank/black flash.
                        //
                        // Safety valve: ensure the main window is shown even if webview script fails.
                        let h = app.handle().clone();
                        tauri::async_runtime::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                            if let Some(win) = h.get_webview_window("main") {
                                if !win.is_visible().unwrap_or(true) {
                                    println!("[Typr] Safety valve: showing main window after timeout");
                                    let _ = win.center();
                                    let _ = win.unminimize();
                                    let _ = win.show();
                                    let _ = win.set_focus();
                                }
                            }
                        });
                    }

                    let app_handle = app.handle().clone();
                    window.on_window_event(move |event| {
                        match event {
                            tauri::WindowEvent::CloseRequested { api, .. } => {
                                api.prevent_close();
                                let h = app_handle.clone();
                                tauri::async_runtime::spawn(async move {
                                    handle_close_requested(&h).await;
                                });
                            }
                            // Heal the moment a window re-gains focus (e.g. it was restored
                            // behind Tauri's back by a second process): re-syncing here means
                            // close/maximize work no matter how the window got re-shown.
                            tauri::WindowEvent::Focused(true) => {
                                if let Some(w) = app_handle.get_webview_window("main") {
                                    resync_visibility(&w);
                                }
                            }
                            _ => {}
                        }
                    });
                }
                Err(e) => {
                    eprintln!("[Typr] CRITICAL: main window failed to build ({}). Continuing tray-only.", e);
                }
            }

            // System tray — the only true exit lives here, so it is always created.
            let show_item = MenuItem::with_id(app, "show", "Show Typr", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&show_item, &quit_item])?;

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Typr")
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            // Center every time the user re-opens from the tray so the
                            // window never reappears where it was hidden (top-left drift).
                            let _ = w.center();
                            let _ = w.unminimize();
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "quit" => {
                        println!("[Typr] Quit from tray, exiting app");
                        let app_dir = app.state::<AppState>().app_dir.clone();
                        typr_lib::debug_log::log(&app_dir, "[Typr] Quit from tray, exiting app");
                        tauri::async_runtime::block_on(async {
                            typr_lib::whisper_server::stop_server().await;
                        });
                        std::process::exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.center();
                            let _ = w.unminimize();
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                })
                .build(app)?;

            // Create the overlay window (floating pill, bottom center, always on top)
            if let Err(e) = typr_lib::overlay::create_overlay_window(app.handle()) {
                eprintln!("[Typr] Failed to create overlay: {}", e);
            }

            let handle_for_overlay = app.handle().clone();
            app.listen_any("overlay-ready", move |_event| {
                typr_lib::overlay::mark_overlay_ready(&handle_for_overlay);
            });

            // Mirror dictation-failure messages to a system notification: the in-app toast
            // renders in the main window, which is hidden while dictating from the tray.
            // This lives in the binary (not the lib): linking tauri-plugin-notification
            // into the lib breaks the unit-test binary at load (STATUS_ENTRYPOINT_NOT_FOUND).
            let handle_for_notify = app.handle().clone();
            app.listen_any("show-notification", move |event| {
                if let Ok(msg) = serde_json::from_str::<String>(event.payload()) {
                    let _ = handle_for_notify
                        .notification()
                        .builder()
                        .title("Typr")
                        .body(msg)
                        .show();
                }
            });


            let handle = app.handle().clone();

            // Pre-initialize the microphone stream in the background
            let state = handle.state::<AppState>();
            let mic = state.settings.lock().unwrap().microphone.clone();
            let handle_for_warmup = handle.clone();
            tauri::async_runtime::spawn(async move {
                // Clone the Recorder out so we don't hold `State` across the await below.
                let recorder = handle_for_warmup.state::<AppState>().recorder.clone();
                println!("[Typr] Pre-initializing audio stream in background for mic: {}", mic);
                if let Err(e) = recorder.pre_initialize(&mic) {
                    eprintln!("[Typr] Failed to pre-initialize audio stream on startup: {}", e);
                    return;
                }
                // One-time device warm-up: pay the cold activation up front so even the
                // first record captures instantly, then settle the mic back to idle (off).
                recorder.begin_warm();
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                recorder.end_warm();
                println!("[Typr] Audio device warm-up complete (mic idle)");
            });

            // Pre-initialize the Whisper HTTP Server if running in local mode
            let handle_for_server = handle.clone();
            tauri::async_runtime::spawn(async move {
                let state_clone = handle_for_server.state::<AppState>();
                let settings = state_clone.settings.lock().unwrap().clone();
                if settings.engine == "local" {
                    let model_path = state_clone.app_dir.join(transcribe_local::model_filename(&settings.whisper_model));
                    println!("[Typr] Pre-starting local Whisper HTTP server on startup with model {:?}", model_path);
                    typr_lib::whisper_server::note_activity();
                    if let Err(e) = typr_lib::whisper_server::ensure_running(&handle_for_server, &model_path).await {
                        eprintln!("[Typr] Failed to pre-start local Whisper HTTP server on startup: {}", e);
                    }
                } else if settings.engine == "parakeet" {
                    // Same reasoning as the Whisper pre-start: load the Parakeet model now so the
                    // first dictation isn't stalled ~4-6s by a cold build. Blocking/CPU-bound.
                    let model_dir = state_clone.app_dir.join(typr_lib::transcribe_parakeet::model_dir_name(&settings.parakeet_model));
                    println!("[Typr] Pre-warming Parakeet model on startup: {:?}", model_dir);
                    tauri::async_runtime::spawn_blocking(move || {
                        if let Err(e) = typr_lib::transcribe_parakeet::prewarm(&model_dir) {
                            eprintln!("[Typr] Failed to pre-warm Parakeet model on startup: {}", e);
                        }
                    });
                } else if settings.engine == "nemotron" {
                    let model_dir = state_clone.app_dir.join(typr_lib::transcribe_nemotron::model_dir_name(&settings.nemotron_model));
                    println!("[Typr] Pre-warming Nemotron model on startup: {:?}", model_dir);
                    tauri::async_runtime::spawn_blocking(move || {
                        if let Err(e) = typr_lib::transcribe_nemotron::prewarm(&model_dir) {
                            eprintln!("[Typr] Failed to pre-warm Nemotron model on startup: {}", e);
                        }
                    });
                }
            });

            // Idle reaper: spin the warm local server down after IDLE_TIMEOUT of no
            // dictation so the dGPU can power-gate (battery). Never fires mid-recording
            // (guarded on recorder == Ready); the next record-start re-warms it.
            let handle_for_reaper = handle.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    let state = handle_for_reaper.state::<AppState>();
                    let recorder_ready =
                        state.recorder.get_state() == RecordingState::Ready;
                    state.recorder.reap_write_context();
                    let engine = state.settings.lock().unwrap().engine.clone();
                    let recorder = state.recorder.clone();
                    let _ = tokio::task::spawn_blocking(move || recorder.run_when_idle(|| {
                        typr_lib::model_cache::reap(&engine); Ok(())
                    })).await;
                    let server_running = typr_lib::whisper_server::is_server_running();
                    let idle = typr_lib::whisper_server::time_since_activity()
                        .unwrap_or(std::time::Duration::ZERO);
                    if typr_lib::whisper_server::should_spin_down(
                        recorder_ready,
                        server_running,
                        idle,
                        typr_lib::whisper_server::IDLE_TIMEOUT,
                    ) {
                        println!(
                            "[Typr] Idle {}s > {}s — spinning down warm Whisper server to save battery",
                            idle.as_secs(),
                            typr_lib::whisper_server::IDLE_TIMEOUT.as_secs()
                        );
                        typr_lib::whisper_server::stop_server().await;
                    }
                }
            });

            let rx_handle = handle.clone();
            let mut hotkey_rx = hotkey_rx;
            tauri::async_runtime::spawn(async move {
                while let Some(hotkey_event) = hotkey_rx.recv().await {
                    let state = rx_handle.state::<AppState>();
                    if matches!(hotkey_event.source, HotkeySource::Write | HotkeySource::CancelWrite) {
                        if hotkey_event.state == ShortcutState::Pressed {
                            let result = if hotkey_event.source == HotkeySource::CancelWrite {
                                if state.recorder.is_write_active() && state.recorder.get_state() == RecordingState::Recording {
                                    state.recorder.cancel_write();
                                    let settings = state.settings.lock().unwrap().clone();
                                    let result = state.recorder.stop_and_transcribe(&rx_handle, &settings, &state.history, &state.dictionary, &state.app_dir).await.map(|_| ());
                                    let _ = rx_handle.global_shortcut().unregister("Escape");
                                    result
                                } else { Ok(()) }
                            } else { do_write(&rx_handle, state.inner()).await };
                            if let Err(message) = result {
                                use tauri::Emitter;
                                let _ = rx_handle.emit("show-toast", &message);
                                let _ = rx_handle.emit("show-notification", &message);
                            }
                        }
                        continue;
                    }
                    if state.recorder.is_write_active() { continue; }
                    let mode = state.settings.lock().unwrap().recording_mode.clone();
                    match hotkey_event.state {
                        ShortcutState::Pressed => {
                            match mode.as_str() {
                                "toggle" => {
                                    println!("[Typr] Toggle mode: calling do_toggle_recording");
                                    match do_toggle_recording(&rx_handle, state.inner(), hotkey_event.source).await {
                                        Ok(result) => println!("[Typr] Toggle result: {}", result),
                                        Err(e) => eprintln!("[Typr] Toggle error: {}", e),
                                    }
                                }
                                "push-to-talk" => {
                                    let current = state.recorder.get_state();
                                    println!("[Typr] PTT mode, current state: {:?}", current);
                                    if current == RecordingState::Ready {
                                        let session_override = profile_override_for(state.inner(), hotkey_event.source);
                                        prewarm_local(&rx_handle);
                                        let settings = state.settings.lock().unwrap().clone();
                                        let mic = settings.microphone.clone();
                                        let nemotron_dir = state
                                            .app_dir
                                            .join(typr_lib::transcribe_nemotron::model_dir_name(&settings.nemotron_model));
                                        let parakeet_dir = state.app_dir.join(typr_lib::transcribe_parakeet::model_dir_name(&settings.parakeet_model));
                                        match state.recorder.start_recording(
                                            &rx_handle,
                                            &mic,
                                            session_override,
                                            &settings.engine,
                                            Some(&nemotron_dir),
                                            Some(&parakeet_dir),
                                        ) {
                                            Ok(_) => println!("[Typr] Recording started"),
                                            Err(e) => eprintln!("[Typr] Start recording error: {}", e),
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        ShortcutState::Released => {
                            if mode == "push-to-talk" {
                                let current = state.recorder.get_state();
                                if current == RecordingState::Recording {
                                    let settings = state.settings.lock().unwrap().clone();
                                    match state.recorder.stop_and_transcribe(
                                        &rx_handle,
                                        &settings,
                                        &state.history,
                                        &state.dictionary,
                                        &state.app_dir,
                                    ).await {
                                        Ok(result) => println!("[Typr] Transcription: {}", result),
                                        Err(e) => eprintln!("[Typr] Transcription error: {}", e),
                                    }
                                }
                            }
                        }
                    }
                }
            });

            let mut registered_hotkey = None;
            for candidate in hotkey_candidates(&initial_hotkey) {
                println!("[Typr] Registering global shortcut: {}", candidate);
                match register_hotkey(&handle, &candidate, HotkeySource::Primary, &hotkey_tx) {
                    Ok(_) => {
                        registered_hotkey = Some(candidate);
                        break;
                    }
                    Err(e) => {
                        eprintln!("[Typr] Hotkey unavailable: {}", e);
                    }
                }
            }

            match registered_hotkey {
                Some(active_hotkey) => {
                    println!("[Typr] Global shortcut registered successfully: {}", active_hotkey);
                    if active_hotkey != initial_hotkey {
                        let state = app.state::<AppState>();
                        let mut settings = state.settings.lock().unwrap();
                        settings.hotkey = active_hotkey.clone();
                        if let Err(e) = settings.save(&state.app_dir) {
                            eprintln!("[Typr] Failed to persist fallback hotkey: {}", e);
                        } else {
                            println!("[Typr] Saved fallback hotkey: {}", active_hotkey);
                        }
                    }
                }
                None => {
                    eprintln!("[Typr] ERROR: No available global shortcut could be registered.");
                    app.handle().exit(1);
                }
            }

            register_secondary_if_set(&handle, app.state::<AppState>().inner());

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
