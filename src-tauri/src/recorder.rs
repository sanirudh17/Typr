use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager};

use crate::audio::AudioRecorder;
use crate::cleanup::cleanup_text;
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

pub struct Recorder {
    state: Arc<Mutex<RecordingState>>,
    audio_recorder: Arc<Mutex<AudioRecorder>>,
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(RecordingState::Ready)),
            audio_recorder: Arc::new(Mutex::new(AudioRecorder::new())),
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

    pub fn start_recording(&self, app: &AppHandle, mic_name: &str) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        if *state != RecordingState::Ready {
            return Err("Already recording or transcribing".to_string());
        }

        // Eagerly update the UI to eliminate perceived delay
        *state = RecordingState::Recording;
        let _ = app.emit("recording-state", RecordingState::Recording);
        update_overlay(app, &RecordingState::Recording, true);

        // Now start the audio recording
        let mut recorder = self.audio_recorder.lock().unwrap();
        if let Err(e) = recorder.start(mic_name) {
            // Revert state if starting failed
            *state = RecordingState::Ready;
            let _ = app.emit("recording-state", RecordingState::Ready);
            update_overlay(app, &RecordingState::Ready, false);
            return Err(e);
        }

        Ok(())
    }

    pub async fn stop_and_transcribe(
        &self,
        app: &AppHandle,
        settings: &Settings,
        history: &std::sync::Mutex<crate::history::History>,
        app_dir: &PathBuf,
    ) -> Result<String, String> {
        let transcription_started_at = Instant::now();

        // Stop recording
        {
            let mut state = self.state.lock().unwrap();
            if *state != RecordingState::Recording {
                return Err("Not currently recording".to_string());
            }
            *state = RecordingState::Transcribing;
            let _ = app.emit("recording-state", RecordingState::Transcribing);
            update_overlay(app, &RecordingState::Transcribing, true);
        }

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
        let transcribe_result = match settings.engine.as_str() {
            "local" => {
                let model_path = app_dir.join(transcribe_local::model_filename(&settings.whisper_model));
                transcribe_local::transcribe_local(app, &model_path, &temp_path).await
            }
            "cloud" => {
                transcribe_groq::transcribe_groq(&settings.groq_api_key, &temp_path).await
            }
            _ => Err(format!("Unknown engine: {}", settings.engine)),
        };

        // Cleanup temp file
        let _ = std::fs::remove_file(&temp_path);

        // Reset state
        {
            let mut state = self.state.lock().unwrap();
            *state = RecordingState::Ready;
            let _ = app.emit("recording-state", RecordingState::Ready);
            update_overlay(app, &RecordingState::Ready, false);
        }

        let raw_text = transcribe_result?;

        // Clean up text
        let cleaned = cleanup_text(&raw_text);

        // Auto-paste and record history
        if !cleaned.is_empty() {
            paste_text(&cleaned)?;
            let _ = history.lock().unwrap().add_item(cleaned.clone(), duration_secs, app_dir);
            let _ = app.emit("history-updated", ());
        }

        println!(
            "[Typr] Full stop-to-text pipeline completed in {:?}",
            transcription_started_at.elapsed()
        );

        Ok(cleaned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state_is_ready() {
        let recorder = Recorder::new();
        assert_eq!(recorder.get_state(), RecordingState::Ready);
    }
}
