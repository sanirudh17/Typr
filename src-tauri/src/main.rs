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

struct AppState {
    recorder: Recorder,
    settings: Mutex<Settings>,
    history: Mutex<History>,
    app_dir: PathBuf,
}

fn get_app_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.typr.app")
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

            println!("[Typr] Registering global shortcut: {}", initial_hotkey);

            match app.global_shortcut().on_shortcut(
                initial_hotkey.as_str(),
                move |_app, shortcut, event| {
                    println!("[Typr] Hotkey event: {:?} state={:?}", shortcut, event.state);
                    let _ = tx.try_send(event.state);
                },
            ) {
                Ok(_) => println!("[Typr] Global shortcut registered successfully"),
                Err(e) => eprintln!("[Typr] ERROR: Failed to register global shortcut: {}", e),
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
