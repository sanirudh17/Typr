use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionItem {
    pub id: String,
    pub timestamp: u64,
    pub text: String,
    pub duration_secs: f32,
    pub word_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct History {
    pub items: Vec<TranscriptionItem>,
}

impl History {
    pub fn config_path(app_dir: &PathBuf) -> PathBuf {
        app_dir.join("history.json")
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

    pub fn add_item(&mut self, text: String, duration_secs: f32, app_dir: &PathBuf) -> Result<(), String> {
        let word_count = text.split_whitespace().count();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        let id = format!("tx_{}", timestamp);
        
        let item = TranscriptionItem {
            id,
            timestamp,
            text,
            duration_secs,
            word_count,
        };
        
        self.items.insert(0, item);
        
        // Keep max 100 items to avoid large files
        if self.items.len() > 100 {
            self.items.truncate(100);
        }
        
        self.save(app_dir)
    }
}
