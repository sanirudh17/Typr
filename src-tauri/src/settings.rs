use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::context_detector::AppRule;

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
    #[serde(rename = "parakeetModel", default = "default_parakeet_model")]
    pub parakeet_model: String,
    #[serde(rename = "cloudModel", default = "default_cloud_model")]
    pub cloud_model: String,
    #[serde(rename = "aiEnabled", default)]
    pub ai_enabled: bool,
    #[serde(rename = "aiModel", default = "default_ai_model")]
    pub ai_model: String,
    #[serde(rename = "aiProfile", default = "default_ai_profile")]
    pub ai_profile: String,
    #[serde(rename = "aiPromptFormat", default = "default_ai_prompt_format")]
    pub ai_prompt_format: String,
    #[serde(rename = "aiTone", default = "default_ai_style")]
    pub ai_tone: String,
    #[serde(rename = "aiFormat", default = "default_ai_style")]
    pub ai_format: String,
    #[serde(rename = "aiCustomInstructions", default)]
    pub ai_custom_instructions: String,
    #[serde(rename = "theme", default = "default_theme")]
    pub theme: String,
    #[serde(rename = "backgroundMode", default)]
    pub background_mode: bool,
    #[serde(rename = "autostart", default)]
    pub autostart: bool,
    #[serde(rename = "hotkeySecondary", default)]
    pub hotkey_secondary: String,
    /// Write Mode hotkey: rewrites the current selection in place. Empty by
    /// default — opt-in, so it can never surprise or conflict out of the box.
    #[serde(rename = "hotkeyWrite", default)]
    pub hotkey_write: String,
    #[serde(rename = "secondaryProfile", default = "default_secondary_profile")]
    pub secondary_profile: String,
    #[serde(rename = "appRules", default)]
    pub app_rules: Vec<AppRule>,
    #[serde(rename = "autoContextOverride", default = "default_auto_context_override")]
    pub auto_context_override: String,
    /// The update version the user closed the banner on, so it is never offered
    /// unprompted again. Empty means nothing has been dismissed. Only the banner
    /// respects this — an explicit check from Settings always offers the update.
    #[serde(rename = "dismissedUpdateVersion", default)]
    pub dismissed_update_version: String,
    /// Strip filler words (um/uh/er/you know/...) in Developer/Terminal
    /// when AI cleanup is on. Exposed in AI → Advanced → Developer so the
    /// user can keep them if preferred. Defaults to true (strip).
    #[serde(rename = "developerStripFiller", default = "default_true")]
    pub developer_strip_filler: bool,
}

fn default_parakeet_model() -> String {
    "v3".to_string()
}

fn default_cloud_model() -> String {
    "accurate".to_string()
}

fn default_ai_model() -> String {
    "qwen/qwen3.8-27b".to_string()
}

/// New and legacy configs alike follow the OS until the user picks otherwise.
/// On a stock Windows 11 install that resolves to light, matching what other
/// apps show; forcing light instead would fight dark-mode OS users.
fn default_theme() -> String {
    "system".to_string()
}

fn default_ai_profile() -> String {
    "cleanup".to_string()
}

fn default_ai_prompt_format() -> String {
    "natural".to_string()
}

fn default_ai_style() -> String {
    "default".to_string()
}

fn default_secondary_profile() -> String {
    "prompt".to_string()
}

fn default_auto_context_override() -> String {
    "auto".to_string()
}

fn default_true() -> bool {
    true
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
            parakeet_model: "v3".to_string(),
            cloud_model: "accurate".to_string(),
            ai_enabled: false,
            ai_model: "qwen/qwen3.8-27b".to_string(),
            ai_profile: "cleanup".to_string(),
            ai_prompt_format: "natural".to_string(),
            ai_tone: "default".to_string(),
            ai_format: "default".to_string(),
            ai_custom_instructions: String::new(),
            theme: "system".to_string(),
            background_mode: false,
            autostart: false,
            hotkey_secondary: String::new(),
            hotkey_write: String::new(),
            secondary_profile: "prompt".to_string(),
            app_rules: Vec::new(),
            auto_context_override: "auto".to_string(),
            dismissed_update_version: String::new(),
            developer_strip_filler: true,
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

    /// Auto-start on login implies background mode — a login launch that quits on
    /// window close would be pointless. Enabling autostart forces background on;
    /// disabling autostart leaves background as the user set it.
    pub fn normalize_startup(&mut self) {
        if self.autostart {
            self.background_mode = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;

    #[test]
    fn defaults_have_background_and_autostart_off() {
        let s = Settings::default();
        assert!(!s.background_mode);
        assert!(!s.autostart);
    }

    #[test]
    fn old_config_missing_new_keys_loads_as_false() {
        // A config.json written before Phase 5 has neither key.
        let json = r#"{
            "microphone": "default",
            "engine": "local",
            "whisperModel": "medium.en-q5_0",
            "groqApiKey": "",
            "recordingMode": "toggle",
            "hotkey": "CmdOrCtrl+Shift+Space"
        }"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert!(!s.background_mode);
        assert!(!s.autostart);
    }

    #[test]
    fn background_and_autostart_round_trip() {
        let json = serde_json::to_string(&Settings {
            background_mode: true,
            autostart: true,
            ..Settings::default()
        })
        .unwrap();
        let loaded: Settings = serde_json::from_str(&json).unwrap();
        assert!(loaded.background_mode);
        assert!(loaded.autostart);
        // Confirm the wire names are camelCase.
        assert!(json.contains("\"backgroundMode\":true"));
        assert!(json.contains("\"autostart\":true"));
    }

    #[test]
    fn normalize_startup_forces_background_when_autostart_on() {
        let mut s = Settings { autostart: true, background_mode: false, ..Settings::default() };
        s.normalize_startup();
        assert!(s.background_mode);
    }

    #[test]
    fn normalize_startup_leaves_background_when_autostart_off() {
        let mut s = Settings { autostart: false, background_mode: false, ..Settings::default() };
        s.normalize_startup();
        assert!(!s.background_mode);

        let mut s2 = Settings { autostart: false, background_mode: true, ..Settings::default() };
        s2.normalize_startup();
        assert!(s2.background_mode); // untouched
    }

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
    fn test_parakeet_engine_roundtrips() {
        let mut s = Settings::default();
        s.engine = "parakeet".to_string();
        s.parakeet_model = "v2".to_string();
        let json = serde_json::to_string(&s).unwrap();
        let loaded: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.engine, "parakeet");
        assert_eq!(loaded.parakeet_model, "v2");
    }

    /// A config.json written before this phase must still load.
    #[test]
    fn test_config_without_parakeet_field_defaults_to_v3() {
        let json = r#"{"microphone":"default","engine":"local","whisperModel":"small","groqApiKey":"k","recordingMode":"toggle","hotkey":"CmdOrCtrl+Shift+Space"}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.parakeet_model, "v3");
    }

    #[test]
    fn test_default_ai_settings() {
        let s = Settings::default();
        assert_eq!(s.ai_enabled, false);
        assert_eq!(s.ai_model, "qwen/qwen3.8-27b");
    }

    #[test]
    fn test_legacy_config_without_ai_fields_defaults() {
        // A config.json written before the AI fields existed must still load.
        let json = r#"{"microphone":"default","engine":"cloud","whisperModel":"small","groqApiKey":"k","recordingMode":"toggle","hotkey":"CmdOrCtrl+Shift+Space","cloudModel":"accurate"}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.ai_enabled, false);
        assert_eq!(s.ai_model, "qwen/qwen3.8-27b");
    }

    #[test]
    fn test_ai_fields_roundtrip() {
        let dir = temp_dir().join("typr_test_ai_fields");
        let _ = fs::remove_dir_all(&dir);
        let mut s = Settings::default();
        s.ai_enabled = true;
        s.ai_model = "openai/gpt-oss-120b".to_string();
        s.save(&dir).unwrap();
        let loaded = Settings::load(&dir);
        assert_eq!(loaded.ai_enabled, true);
        assert_eq!(loaded.ai_model, "openai/gpt-oss-120b");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_default_prompt_profile_settings() {
        let s = Settings::default();
        assert_eq!(s.ai_profile, "cleanup");
        assert_eq!(s.ai_prompt_format, "natural");
    }

    #[test]
    fn test_legacy_config_without_profile_fields_defaults() {
        // A Slice A config.json (has aiEnabled/aiModel but not the profile fields) must load.
        let json = r#"{"microphone":"default","engine":"cloud","whisperModel":"small","groqApiKey":"k","recordingMode":"toggle","hotkey":"CmdOrCtrl+Shift+Space","cloudModel":"accurate","aiEnabled":true,"aiModel":"openai/gpt-oss-20b"}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.ai_profile, "cleanup");
        assert_eq!(s.ai_prompt_format, "natural");
    }

    #[test]
    fn test_profile_fields_roundtrip() {
        let dir = temp_dir().join("typr_test_profile_fields");
        let _ = fs::remove_dir_all(&dir);
        let mut s = Settings::default();
        s.ai_profile = "prompt".to_string();
        s.ai_prompt_format = "structured".to_string();
        s.save(&dir).unwrap();
        let loaded = Settings::load(&dir);
        assert_eq!(loaded.ai_profile, "prompt");
        assert_eq!(loaded.ai_prompt_format, "structured");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_default_style_settings() {
        let s = Settings::default();
        assert_eq!(s.ai_tone, "default");
        assert_eq!(s.ai_format, "default");
        assert_eq!(s.ai_custom_instructions, "");
    }

    #[test]
    fn test_legacy_config_without_style_fields_defaults() {
        // A Slice A–D config (no style fields yet) must still load.
        let json = r#"{"microphone":"default","engine":"cloud","whisperModel":"small","groqApiKey":"k","recordingMode":"toggle","hotkey":"CmdOrCtrl+Shift+Space","cloudModel":"accurate","aiEnabled":true,"aiModel":"openai/gpt-oss-20b","aiProfile":"cleanup","aiPromptFormat":"natural"}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.ai_tone, "default");
        assert_eq!(s.ai_format, "default");
        assert_eq!(s.ai_custom_instructions, "");
    }

    #[test]
    fn test_style_fields_roundtrip() {
        let dir = temp_dir().join("typr_test_style_fields");
        let _ = fs::remove_dir_all(&dir);
        let mut s = Settings::default();
        s.ai_tone = "formal".to_string();
        s.ai_format = "bullets".to_string();
        s.ai_custom_instructions = "Use British spelling.".to_string();
        s.save(&dir).unwrap();
        let loaded = Settings::load(&dir);
        assert_eq!(loaded.ai_tone, "formal");
        assert_eq!(loaded.ai_format, "bullets");
        assert_eq!(loaded.ai_custom_instructions, "Use British spelling.");
        let _ = fs::remove_dir_all(&dir);
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

    #[test]
    fn test_write_hotkey_defaults_unset() {
        // Opt-in: a fresh config must not claim any accelerator.
        let s = Settings::default();
        assert_eq!(s.hotkey_write, "");
        // And a config written before Write Mode existed still loads.
        let json = r#"{"microphone":"default","engine":"local","whisperModel":"small","groqApiKey":"k","recordingMode":"toggle","hotkey":"CmdOrCtrl+Shift+Space"}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.hotkey_write, "");
    }

    #[test]
    fn test_write_hotkey_roundtrips() {
        let mut s = Settings::default();
        s.hotkey_write = "CmdOrCtrl+Alt+W".to_string();
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"hotkeyWrite\":"));
        let loaded: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.hotkey_write, "CmdOrCtrl+Alt+W");
    }

    #[test]
    fn test_secondary_hotkey_defaults() {
        let s = Settings::default();
        assert_eq!(s.hotkey_secondary, "");
        assert_eq!(s.secondary_profile, "prompt");
    }

    #[test]
    fn test_secondary_fields_roundtrip() {
        let dir = temp_dir().join("typr_test_secondary_fields");
        let _ = fs::remove_dir_all(&dir);
        let mut s = Settings::default();
        s.hotkey_secondary = "CmdOrCtrl+Alt+P".to_string();
        s.secondary_profile = "auto".to_string();
        s.save(&dir).unwrap();
        let loaded = Settings::load(&dir);
        assert_eq!(loaded.hotkey_secondary, "CmdOrCtrl+Alt+P");
        assert_eq!(loaded.secondary_profile, "auto");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_default_context_fields() {
        let s = Settings::default();
        assert!(s.app_rules.is_empty());
        assert_eq!(s.auto_context_override, "auto");
    }

    #[test]
    fn test_legacy_config_without_context_fields_defaults() {
        // A config.json written before this slice has neither key and must still load.
        let json = r#"{"microphone":"default","engine":"cloud","whisperModel":"small","groqApiKey":"k","recordingMode":"toggle","hotkey":"CmdOrCtrl+Shift+Space"}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert!(s.app_rules.is_empty());
        assert_eq!(s.auto_context_override, "auto");
    }

    #[test]
    fn test_context_fields_roundtrip() {
        use crate::context_detector::{AppRule, ContextCategory};
        let dir = temp_dir().join("typr_test_context_fields");
        let _ = fs::remove_dir_all(&dir);
        let mut s = Settings::default();
        s.auto_context_override = "developer".to_string();
        s.app_rules = vec![AppRule {
            process_name: "obsidian.exe".to_string(),
            title_contains: None,
            category: ContextCategory::Professional,
        }];
        s.save(&dir).unwrap();
        let loaded = Settings::load(&dir);
        assert_eq!(loaded.auto_context_override, "developer");
        assert_eq!(loaded.app_rules.len(), 1);
        assert_eq!(loaded.app_rules[0].process_name, "obsidian.exe");
        assert_eq!(loaded.app_rules[0].category, ContextCategory::Professional);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_default_theme_follows_system() {
        // Follow-the-OS is the default, per platform convention (Apple HIG,
        // Material, Windows guidance) — never a forced light or dark.
        assert_eq!(Settings::default().theme, "system");
    }

    #[test]
    fn test_legacy_config_without_theme_defaults_dark() {
        // A config.json written before the Appearance setting must still load.
        let json = r#"{"microphone":"default","engine":"cloud","whisperModel":"small","groqApiKey":"k","recordingMode":"toggle","hotkey":"CmdOrCtrl+Shift+Space"}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.theme, "system");
    }

    #[test]
    fn test_theme_roundtrips() {
        for theme in ["light", "dark", "system"] {
            let mut s = Settings::default();
            s.theme = theme.to_string();
            let json = serde_json::to_string(&s).unwrap();
            let loaded: Settings = serde_json::from_str(&json).unwrap();
            assert_eq!(loaded.theme, theme);
            assert!(json.contains("\"theme\":"));
        }
    }

    #[test]
    fn test_pre_slice_b_config_defaults_secondary() {
        // A config.json written before Slice B has neither key.
        let json = r#"{"microphone":"default","engine":"cloud","whisperModel":"small","groqApiKey":"k","recordingMode":"toggle","hotkey":"CmdOrCtrl+Shift+Space"}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.hotkey_secondary, "");
        assert_eq!(s.secondary_profile, "prompt");
    }
}
