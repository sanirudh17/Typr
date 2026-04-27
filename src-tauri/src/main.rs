#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use typr_lib::audio;
use typr_lib::downloader;
use typr_lib::recorder::{Recorder, RecordingState};
use typr_lib::settings::Settings;
use typr_lib::transcribe_local;

use typr_lib::history::History;

#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::CreateMutexW;

struct AppState {
    recorder: Recorder,
    settings: Mutex<Settings>,
    history: Mutex<History>,
    app_dir: PathBuf,
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
fn save_settings(state: State<AppState>, settings: Settings) -> Result<(), String> {
    settings.save(&state.app_dir)?;
    *state.settings.lock().unwrap() = settings;
    Ok(())
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
    downloader::download_model(app, &url, &dest).await
}

#[tauri::command]
async fn toggle_recording(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    do_toggle_recording(&app, &state).await
}

/// Shared logic for toggle recording, used by both the Tauri command and hotkey handler.
async fn do_toggle_recording(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<String, String> {
    let current_state = state.recorder.get_state();
    match current_state {
        RecordingState::Ready => {
            let mic = state.settings.lock().unwrap().microphone.clone();
            state.recorder.start_recording(app, &mic)?;
            Ok("recording".to_string())
        }
        RecordingState::Recording => {
            let settings = state.settings.lock().unwrap().clone();
            let result = state
                .recorder
                .stop_and_transcribe(app, &settings, &state.history, &state.app_dir)
                .await?;
            Ok(result)
        }
        RecordingState::Transcribing => {
            Err("Currently transcribing, please wait".to_string())
        }
    }
}

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
    let initial_hotkey = settings.hotkey.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            recorder: Recorder::new(),
            settings: Mutex::new(settings),
            history: Mutex::new(history),
            app_dir,
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
            get_history,
        ])
        .setup(move |app| {
            // Handle window close to properly exit the app
            let main_window = app.get_webview_window("main");
            if let Some(window) = main_window {
                let _window_clone = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { .. } = event {
                        println!("[Typr] Main window close requested, exiting app");
                        std::process::exit(0);
                    }
                });
            }

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
                Ok(_) => println!("[Typr] Overlay window created"),
                Err(e) => eprintln!("[Typr] Failed to create overlay: {}", e),
            }

            let handle = app.handle().clone();

            let (tx, mut rx) = tokio::sync::mpsc::channel::<ShortcutState>(32);
            let rx_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                while let Some(event_state) = rx.recv().await {
                    let state = rx_handle.state::<AppState>();
                    let mode = state.settings.lock().unwrap().recording_mode.clone();
                    match event_state {
                        ShortcutState::Pressed => {
                            match mode.as_str() {
                                "toggle" => {
                                    println!("[Typr] Toggle mode: calling do_toggle_recording");
                                    match do_toggle_recording(&rx_handle, state.inner()).await {
                                        Ok(result) => println!("[Typr] Toggle result: {}", result),
                                        Err(e) => eprintln!("[Typr] Toggle error: {}", e),
                                    }
                                }
                                "push-to-talk" => {
                                    let current = state.recorder.get_state();
                                    println!("[Typr] PTT mode, current state: {:?}", current);
                                    if current == RecordingState::Ready {
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
                                    let settings = state.settings.lock().unwrap().clone();
                                    match state.recorder.stop_and_transcribe(
                                        &rx_handle,
                                        &settings,
                                        &state.history,
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
                let tx_for_hotkey = tx.clone();
                match app.global_shortcut().on_shortcut(
                    candidate.as_str(),
                    move |_app, shortcut, event| {
                        println!("[Typr] Hotkey event: {:?} state={:?}", shortcut, event.state);
                        let _ = tx_for_hotkey.try_send(event.state);
                    },
                ) {
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

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
