use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    pub microphone: String,
    pub engine: String,
    #[serde(rename = "whisperModel")]
    pub whisper_model: String,
    #[serde(rename = "groqApiKey")]
    pub groq_api_key: String,
    #[serde(rename = "recordingMode")]
    pub recording_mode: String,
    pub hotkey: String,
    #[serde(rename = "cloudModel", default = "default_cloud_model")]
    pub cloud_model: String,
}

fn default_cloud_model() -> String {
    "accurate".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            microphone: "default".to_string(),
            engine: "local".to_string(),
            whisper_model: "medium.en-q5_0".to_string(),
            groq_api_key: String::new(),
            recording_mode: "toggle".to_string(),
            hotkey: "CmdOrCtrl+Shift+Space".to_string(),
            cloud_model: "accurate".to_string(),
        }
    }
}

impl Settings {
    pub fn config_path(app_dir: &PathBuf) -> PathBuf {
        app_dir.join("config.json")
    }

    pub fn load(app_dir: &PathBuf) -> Self {
        let path = Self::config_path(app_dir);
        match fs::read_to_string(&path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, app_dir: &PathBuf) -> Result<(), String> {
        let path = Self::config_path(app_dir);
        fs::create_dir_all(app_dir).map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(&path, json).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;

    #[test]
    fn test_default_settings() {
        let settings = Settings::default();
        assert_eq!(settings.microphone, "default");
        assert_eq!(settings.engine, "local");
        assert_eq!(settings.whisper_model, "medium.en-q5_0");
        assert_eq!(settings.groq_api_key, "");
        assert_eq!(settings.recording_mode, "toggle");
        assert_eq!(settings.hotkey, "CmdOrCtrl+Shift+Space");
    }

    #[test]
    fn test_save_and_load() {
        let dir = temp_dir().join("typr_test_settings");
        let _ = fs::remove_dir_all(&dir);

        let mut settings = Settings::default();
        settings.engine = "cloud".to_string();
        settings.groq_api_key = "test-key-123".to_string();

        settings.save(&dir).unwrap();
        let loaded = Settings::load(&dir);

        assert_eq!(loaded.engine, "cloud");
        assert_eq!(loaded.groq_api_key, "test-key-123");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_missing_file_returns_default() {
        let dir = temp_dir().join("typr_test_missing");
        let _ = fs::remove_dir_all(&dir);
        let settings = Settings::load(&dir);
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn test_load_corrupt_json_returns_default() {
        let dir = temp_dir().join("typr_test_corrupt");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.json"), "not json").unwrap();

        let settings = Settings::load(&dir);
        assert_eq!(settings, Settings::default());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_default_cloud_model_is_accurate() {
        assert_eq!(Settings::default().cloud_model, "accurate");
    }

    #[test]
    fn test_legacy_config_without_cloud_model_defaults_accurate() {
        // A config.json written before this field existed must still load.
        let json = r#"{"microphone":"default","engine":"cloud","whisperModel":"small","groqApiKey":"k","recordingMode":"toggle","hotkey":"CmdOrCtrl+Shift+Space"}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.cloud_model, "accurate");
    }

    #[test]
    fn test_cloud_model_roundtrips() {
        let dir = temp_dir().join("typr_test_cloud_model");
        let _ = fs::remove_dir_all(&dir);
        let mut s = Settings::default();
        s.cloud_model = "fast".to_string();
        s.save(&dir).unwrap();
        assert_eq!(Settings::load(&dir).cloud_model, "fast");
        let _ = fs::remove_dir_all(&dir);
    }
}
