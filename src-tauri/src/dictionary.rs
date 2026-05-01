use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Dictionary {
    pub words: Vec<String>,
}

impl Dictionary {
    pub fn config_path(app_dir: &PathBuf) -> PathBuf {
        app_dir.join("dictionary.json")
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

    pub fn add_word(&mut self, word: String, app_dir: &PathBuf) -> Result<(), String> {
        let trimmed = word.trim().to_string();
        if trimmed.is_empty() {
            return Err("Word cannot be empty.".to_string());
        }
        if !self.words.contains(&trimmed) {
            self.words.push(trimmed);
            self.save(app_dir)?;
        }
        Ok(())
    }

    pub fn remove_word(&mut self, index: usize, app_dir: &PathBuf) -> Result<(), String> {
        if index >= self.words.len() {
            return Err("Invalid dictionary index.".to_string());
        }
        self.words.remove(index);
        self.save(app_dir)
    }

    pub fn get_prompt(&self) -> String {
        self.words.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;

    #[test]
    fn test_save_and_load() {
        let dir = temp_dir().join("typr_test_dict2");
        let _ = fs::remove_dir_all(&dir);

        let mut dict = Dictionary::default();
        dict.add_word("Tauri".to_string(), &dir).unwrap();

        let loaded = Dictionary::load(&dir);
        assert_eq!(loaded.words.len(), 1);
        assert_eq!(loaded.words[0], "Tauri");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_get_prompt() {
        let mut dict = Dictionary::default();
        dict.words.push("Tauri".to_string());
        dict.words.push("Rust".to_string());
        assert_eq!(dict.get_prompt(), "Tauri, Rust");
    }
}
