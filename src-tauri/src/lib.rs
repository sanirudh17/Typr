pub mod settings;
pub mod audio;
pub mod transcribe_local;
pub mod transcribe_groq;
pub mod cleanup;
pub mod paste;
pub mod recorder;
pub mod downloader;
pub mod history;
pub mod dictionary;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_shell::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
