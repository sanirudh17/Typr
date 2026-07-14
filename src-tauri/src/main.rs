#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
#[cfg(not(windows))]
use tauri::image::Image;
use tauri::{Manager, State, WebviewUrl, WebviewWindowBuilder};
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
    app.global_shortcut()
        .on_shortcut(accelerator, move |_app, shortcut, event| {
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
}

struct AppState {
    recorder: Recorder,
    settings: Mutex<Settings>,
    history: Mutex<History>,
    dictionary: Mutex<Dictionary>,
    app_dir: PathBuf,
    hotkey_tx: Sender<HotkeyEvent>,
    pending_profile_override: Mutex<Option<String>>,
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

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
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

    // Reconcile the OS auto-start registry entry with the toggle.
    let autolaunch = app.autolaunch();
    let currently_enabled = autolaunch.is_enabled().unwrap_or(false);
    if settings.autostart && !currently_enabled {
        let _ = autolaunch.enable();
    } else if !settings.autostart && currently_enabled {
        let _ = autolaunch.disable();
    }

    let old_settings = state.settings.lock().unwrap().clone();
    let mic_changed = old_settings.microphone != settings.microphone;
    let engine_changed = old_settings.engine != settings.engine;
    let model_changed = old_settings.whisper_model != settings.whisper_model;

    settings.save(&state.app_dir)?;
    *state.settings.lock().unwrap() = settings.clone();

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
        if settings_clone.engine == "local" {
            // Start/restart if switched from cloud, or if model changed
            if engine_changed || model_changed {
                let model_path = app_dir_clone.join(transcribe_local::model_filename(&settings_clone.whisper_model));
                println!("[Typr] On-demand engine/model change: ensuring Whisper server is running with {:?}", model_path);
                if let Err(e) = typr_lib::whisper_server::ensure_running(&app_clone, &model_path).await {
                    eprintln!("[Typr] Failed to start local Whisper HTTP server: {}", e);
                }
            }
        } else if old_settings.engine == "local" && settings_clone.engine == "cloud" {
            // Switched from local to cloud: stop the server immediately to free VRAM
            println!("[Typr] Switching to Cloud mode: stopping Whisper HTTP server to free VRAM...");
            typr_lib::whisper_server::stop_server().await;
        }
    });

    Ok(())
}

#[tauri::command]
async fn set_hotkey(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    accelerator: String,
) -> Result<String, String> {
    // 1. Validate the shape before touching the OS registration.
    typr_lib::hotkey::validate_accelerator(&accelerator)?;

    let old = state.settings.lock().unwrap().hotkey.clone();

    // 2. Clear the old primary (harmless no-op if hotkeys are suspended), then
    //    try the new one. This always ends with `accelerator` registered on
    //    success — even when it equals `old` — so it is safe to call while
    //    global shortcuts are suspended for capture.
    let _ = app.global_shortcut().unregister(old.as_str());
    match register_hotkey(&app, &accelerator, HotkeySource::Primary, &state.hotkey_tx) {
        Ok(_) => {
            // 3. Persist only when it actually changed.
            if accelerator != old {
                let mut settings = state.settings.lock().unwrap().clone();
                settings.hotkey = accelerator.clone();
                settings.save(&state.app_dir)?;
                *state.settings.lock().unwrap() = settings;
                println!("[Typr] Hotkey rebound to {}", accelerator);
            }
            // Capture suspended everything — re-arm the secondary too.
            register_secondary_if_set(&app, &state);
            Ok(accelerator)
        }
        Err(e) => {
            // 4. Best-effort restore so we never end with no hotkey.
            let _ = register_hotkey(&app, &old, HotkeySource::Primary, &state.hotkey_tx);
            register_secondary_if_set(&app, &state);
            eprintln!("[Typr] Rebind to {} failed ({}); kept {}", accelerator, e, old);
            Err(format!(
                "`{}` is unavailable — it may be in use by Windows or another app.",
                accelerator
            ))
        }
    }
}

/// Unregister all global shortcuts while the UI captures a new combo, so the
/// keystrokes reach the webview instead of being swallowed (or firing a
/// recording) by the currently-registered hotkey.
#[tauri::command]
fn suspend_hotkeys(app: tauri::AppHandle) -> Result<(), String> {
    app.global_shortcut()
        .unregister_all()
        .map_err(|e| e.to_string())
}

/// Re-register the saved hotkeys after a capture is cancelled without a
/// successful rebind — both the primary and (if set) the secondary, since
/// capture suspends all global shortcuts.
#[tauri::command]
fn resume_hotkeys(app: tauri::AppHandle, state: State<AppState>) -> Result<(), String> {
    let hotkey = state.settings.lock().unwrap().hotkey.clone();
    let res = register_hotkey(&app, &hotkey, HotkeySource::Primary, &state.hotkey_tx);
    register_secondary_if_set(&app, &state);
    res
}

#[tauri::command]
async fn set_secondary_hotkey(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    accelerator: String,
) -> Result<String, String> {
    typr_lib::hotkey::validate_accelerator(&accelerator)?;

    let (primary, old_secondary) = {
        let s = state.settings.lock().unwrap();
        (s.hotkey.clone(), s.hotkey_secondary.clone())
    };
    if accelerator == primary {
        // Restore whatever was suspended for capture before returning.
        let _ = register_hotkey(&app, &primary, HotkeySource::Primary, &state.hotkey_tx);
        register_secondary_if_set(&app, &state);
        return Err("That's already your main hotkey — pick a different combo.".to_string());
    }

    if !old_secondary.is_empty() {
        let _ = app.global_shortcut().unregister(old_secondary.as_str());
    }
    match register_hotkey(&app, &accelerator, HotkeySource::Secondary, &state.hotkey_tx) {
        Ok(_) => {
            let mut settings = state.settings.lock().unwrap().clone();
            settings.hotkey_secondary = accelerator.clone();
            settings.save(&state.app_dir)?;
            *state.settings.lock().unwrap() = settings;
            // Capture suspended everything — re-arm the primary.
            let _ = register_hotkey(&app, &primary, HotkeySource::Primary, &state.hotkey_tx);
            println!("[Typr] Secondary hotkey set to {}", accelerator);
            Ok(accelerator)
        }
        Err(e) => {
            // Restore prior state: old secondary (if any) + primary.
            if !old_secondary.is_empty() {
                let _ = register_hotkey(&app, &old_secondary, HotkeySource::Secondary, &state.hotkey_tx);
            }
            let _ = register_hotkey(&app, &primary, HotkeySource::Primary, &state.hotkey_tx);
            eprintln!("[Typr] Secondary rebind to {} failed: {}", accelerator, e);
            Err(format!(
                "`{}` is unavailable — it may be in use by Windows or another app.",
                accelerator
            ))
        }
    }
}

#[tauri::command]
fn clear_secondary_hotkey(app: tauri::AppHandle, state: State<AppState>) -> Result<(), String> {
    let old = state.settings.lock().unwrap().hotkey_secondary.clone();
    if !old.is_empty() {
        let _ = app.global_shortcut().unregister(old.as_str());
    }
    let mut settings = state.settings.lock().unwrap().clone();
    settings.hotkey_secondary = String::new();
    settings.save(&state.app_dir)?;
    *state.settings.lock().unwrap() = settings;
    // Clear any override that may reference the now-removed secondary session.
    *state.pending_profile_override.lock().unwrap() = None;
    println!("[Typr] Secondary hotkey cleared");
    Ok(())
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
fn delete_transcription(state: State<AppState>, id: String) -> Result<(), String> {
    state.history.lock().unwrap().delete_item(&id, &state.app_dir)
}

#[tauri::command]
fn update_transcription(state: State<AppState>, id: String, text: String) -> Result<(), String> {
    state.history.lock().unwrap().update_item(&id, text, &state.app_dir)
}

#[tauri::command]
fn propose_correction(old: String, new: String) -> Option<typr_lib::history::Correction> {
    typr_lib::history::propose_correction(&old, &new)
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

/// Record which AI profile (if any) this recording session should use, based on
/// which hotkey started it. Secondary → force the configured profile; Primary → none.
fn set_session_override(state: &AppState, source: HotkeySource) {
    let ov = match source {
        HotkeySource::Secondary => Some(state.settings.lock().unwrap().secondary_profile.clone()),
        HotkeySource::Primary => None,
    };
    *state.pending_profile_override.lock().unwrap() = ov;
}

/// Live settings for the dictation about to be transcribed, with the per-session
/// override applied (and cleared) if the session was started by the secondary hotkey.
fn session_settings(state: &AppState) -> Settings {
    let mut settings = state.settings.lock().unwrap().clone();
    if let Some(profile) = state.pending_profile_override.lock().unwrap().take() {
        settings.ai_enabled = true;
        settings.ai_profile = profile;
    }
    settings
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
async fn do_toggle_recording(
    app: &tauri::AppHandle,
    state: &AppState,
    source: HotkeySource,
) -> Result<String, String> {
    let current_state = state.recorder.get_state();
    match current_state {
        RecordingState::Ready => {
            set_session_override(state, source);
            let mic = state.settings.lock().unwrap().microphone.clone();
            state.recorder.start_recording(app, &mic)?;
            Ok("recording".to_string())
        }
        RecordingState::Recording => {
            let settings = session_settings(state);
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

fn main() {
    let _single_instance = match acquire_single_instance() {
        Ok(guard) => guard,
        Err(message) => {
            eprintln!("[Typr] {}", message);
            return;
        }
    };

    println!("[Typr] Starting process PID {}", std::process::id());

    let app_dir = get_app_dir();
    let settings = Settings::load(&app_dir);
    let history = History::load(&app_dir);
    let dictionary = Dictionary::load(&app_dir);
    let initial_hotkey = settings.hotkey.clone();

    let (hotkey_tx, hotkey_rx) = tokio::sync::mpsc::channel::<HotkeyEvent>(32);

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .manage(AppState {
            recorder: Recorder::new(),
            settings: Mutex::new(settings),
            history: Mutex::new(history),
            dictionary: Mutex::new(dictionary),
            app_dir,
            hotkey_tx: hotkey_tx.clone(),
            pending_profile_override: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            list_microphones,
            get_recording_state,
            get_amplitude,
            get_frequency_bands,
            check_model_downloaded,
            download_model,
            toggle_recording,
            set_hotkey,
            suspend_hotkeys,
            resume_hotkeys,
            set_secondary_hotkey,
            clear_secondary_hotkey,
            set_secondary_profile,
            get_history,
            delete_transcription,
            update_transcription,
            propose_correction,
            write_text_file,
            get_dictionary,
            add_dictionary_word,
            remove_dictionary_word,
            add_vocabulary_hint,
            remove_vocabulary_hint,
            add_replacement,
            remove_replacement,
        ])
        .setup(move |app| {
            // Handle window close to properly exit the app
            let main_window = app.get_webview_window("main");
            if let Some(window) = main_window {
                #[cfg(not(windows))]
                match Image::from_bytes(include_bytes!("../icons/icon.png")) {
                    Ok(icon) => {
                        if let Err(e) = window.set_icon(icon) {
                            eprintln!("[Typr] Failed to set main window icon: {}", e);
                        }
                    }
                    Err(e) => eprintln!("[Typr] Failed to load main window icon: {}", e),
                }

                if launched_hidden() {
                    println!("[Typr] Launched hidden (auto-start); main window stays in tray");
                } else if let Err(e) = window.show() {
                    eprintln!("[Typr] Failed to show main window: {}", e);
                }

                let app_handle = app.handle().clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        let background = app_handle
                            .state::<AppState>()
                            .settings
                            .lock()
                            .unwrap()
                            .background_mode;
                        if background {
                            // Hide to tray, keep running so the hotkey still works.
                            api.prevent_close();
                            if let Some(w) = app_handle.get_webview_window("main") {
                                let _ = w.hide();
                            }
                            if !FIRST_HIDE_NOTIFIED.swap(true, Ordering::SeqCst) {
                                let _ = app_handle
                                    .notification()
                                    .builder()
                                    .title("Typr is still running")
                                    .body("Right-click the tray icon to quit.")
                                    .show();
                            }
                        } else {
                            println!("[Typr] Main window close requested, exiting app");
                            tauri::async_runtime::block_on(async {
                                typr_lib::whisper_server::stop_server().await;
                            });
                            std::process::exit(0);
                        }
                    }
                });
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
                            let _ = w.unminimize();
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "quit" => {
                        println!("[Typr] Quit from tray, exiting app");
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
                            let _ = w.unminimize();
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                })
                .build(app)?;

            // Create the overlay window (floating pill, bottom center, always on top)
            let monitor = app.primary_monitor().ok().flatten();
            let (x, y) = if let Some(m) = monitor {
                let size = m.size();
                let scale = m.scale_factor();
                let logical_w = size.width as f64 / scale;
                let logical_h = size.height as f64 / scale;
                ((logical_w - 300.0) as i32 / 2, (logical_h - 160.0) as i32)
            } else {
                (810, 950)
            };

            let overlay = WebviewWindowBuilder::new(
                app,
                "overlay",
                WebviewUrl::App("src/overlay.html".into()),
            )
            .title("")
            .inner_size(300.0, 120.0)
            .position(x as f64, y as f64)
            .resizable(false)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .shadow(false)
            .build();

            match overlay {
                Ok(window) => {
                    println!("[Typr] Overlay window created");
                    let _ = window.set_ignore_cursor_events(true);
                }
                Err(e) => eprintln!("[Typr] Failed to create overlay: {}", e),
            }

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
                                        set_session_override(state.inner(), hotkey_event.source);
                                        let mic = state.settings.lock().unwrap().microphone.clone();
                                        match state.recorder.start_recording(&rx_handle, &mic) {
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
                                    let settings = session_settings(state.inner());
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
