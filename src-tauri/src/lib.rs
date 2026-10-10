pub mod settings;
pub mod audio;
pub mod audio_chunker;
pub mod transcribe_local;
pub mod transcribe_groq;
pub mod transcribe_parakeet;
pub mod transcribe_nemotron;
pub mod cleanup;
pub mod paste;
pub mod recorder;
pub mod downloader;
pub mod history;
pub mod data_cleanup;
pub mod dictionary;
pub mod vocabulary;
pub mod learning;
pub mod whisper_server;
pub mod ai_postprocess;
pub mod ai_models;
pub mod context_detector;
pub mod debug_log;
pub mod commands;
pub mod spelling;
pub mod hotkey;
pub mod vocab_correct;
pub mod email_assemble;
pub mod overlay;
pub mod write_mode;
pub mod selection;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_shell::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
