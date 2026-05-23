use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use regex::{RegexBuilder, escape};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ReplacementEntry {
    pub find: String,
    pub replace: String,
    pub case_sensitive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Dictionary {
    #[serde(default)]
    pub vocabulary_hints: Vec<String>,
    
    #[serde(default)]
    pub replacements: Vec<ReplacementEntry>,
    
    // Backwards compatibility migration field:
    #[serde(default)]
    pub words: Option<Vec<String>>,
}

impl Dictionary {
    pub fn config_path(app_dir: &PathBuf) -> PathBuf {
        app_dir.join("dictionary.json")
    }

    pub fn load(app_dir: &PathBuf) -> Self {
        let path = Self::config_path(app_dir);
        let mut dict = match fs::read_to_string(&path) {
            Ok(contents) => serde_json::from_str::<Dictionary>(&contents).unwrap_or_default(),
            Err(_) => Self::default(),
        };
        
        // Automatic migration of old 'words' array to 'vocabulary_hints'
        if let Some(old_words) = dict.words.take() {
            if !old_words.is_empty() && dict.vocabulary_hints.is_empty() {
                dict.vocabulary_hints = old_words;
                let _ = dict.save(app_dir);
            }
        }
        
        dict
    }

    pub fn save(&self, app_dir: &PathBuf) -> Result<(), String> {
        let path = Self::config_path(app_dir);
        fs::create_dir_all(app_dir).map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(&path, json).map_err(|e| e.to_string())
    }

    // --- Vocabulary Hints ---

    pub fn add_vocabulary_hint(&mut self, word: String, app_dir: &PathBuf) -> Result<(), String> {
        let trimmed = word.trim().to_string();
        if trimmed.is_empty() {
            return Err("Word cannot be empty.".to_string());
        }
        if !self.vocabulary_hints.contains(&trimmed) {
            self.vocabulary_hints.push(trimmed);
            self.save(app_dir)?;
        }
        Ok(())
    }

    pub fn remove_vocabulary_hint(&mut self, index: usize, app_dir: &PathBuf) -> Result<(), String> {
        if index >= self.vocabulary_hints.len() {
            return Err("Invalid dictionary index.".to_string());
        }
        self.vocabulary_hints.remove(index);
        self.save(app_dir)
    }

    // Backwards compatible aliases
    pub fn add_word(&mut self, word: String, app_dir: &PathBuf) -> Result<(), String> {
        self.add_vocabulary_hint(word, app_dir)
    }

    pub fn remove_word(&mut self, index: usize, app_dir: &PathBuf) -> Result<(), String> {
        self.remove_vocabulary_hint(index, app_dir)
    }

    // --- Text Replacements ---

    pub fn add_replacement(
        &mut self,
        find: String,
        replace: String,
        case_sensitive: bool,
        app_dir: &PathBuf,
    ) -> Result<(), String> {
        let find_normalized = find.split_whitespace().collect::<Vec<&str>>().join(" ");
        let replace_trimmed = replace.trim().to_string();
        if find_normalized.is_empty() {
            return Err("Find field cannot be empty.".to_string());
        }
        
        // Prevent duplicate find fields
        self.replacements.retain(|entry| entry.find != find_normalized);
        
        self.replacements.push(ReplacementEntry {
            find: find_normalized,
            replace: replace_trimmed,
            case_sensitive,
        });
        
        self.save(app_dir)
    }

    pub fn remove_replacement(&mut self, index: usize, app_dir: &PathBuf) -> Result<(), String> {
        if index >= self.replacements.len() {
            return Err("Invalid replacement index.".to_string());
        }
        self.replacements.remove(index);
        self.save(app_dir)
    }

    // --- Core Operations ---

    pub fn get_prompt(&self) -> String {
        self.vocabulary_hints.join(", ")
    }

    pub fn apply_replacements(&self, text: &str) -> String {
        // Normalize multiple spaces in the input text to a single space
        let normalized_text = text.split_whitespace().collect::<Vec<&str>>().join(" ");
        let mut result = normalized_text;

        for entry in &self.replacements {
            // Normalize spaces in the search term too, just in case
            let find_normalized = entry.find.split_whitespace().collect::<Vec<&str>>().join(" ");
            if find_normalized.is_empty() {
                continue;
            }
            let escaped_find = escape(&find_normalized);
            
            // Build regex with smart word boundaries:
            // - If starting with alphanumeric, prepend \b boundary
            // - If ending with alphanumeric, append \b boundary
            // This ensures punctuation shortcuts work seamlessly while words don't replace inside substrings.
            let mut regex_str = String::new();
            if let Some(c) = find_normalized.chars().next() {
                if c.is_alphanumeric() {
                    regex_str.push_str(r"\b");
                }
            }
            regex_str.push_str(&escaped_find);
            if let Some(c) = find_normalized.chars().last() {
                if c.is_alphanumeric() {
                    regex_str.push_str(r"\b");
                }
            }

            if let Ok(re) = RegexBuilder::new(&regex_str)
                .case_insensitive(!entry.case_sensitive)
                .build()
            {
                result = re.replace_all(&result, &entry.replace).to_string();
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;

    #[test]
    fn test_migration_and_save() {
        let dir = temp_dir().join("typr_test_dict_migration");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Write old structure manually
        let old_json = r#"{"words": ["Tauri", "Rust"]}"#;
        fs::write(dir.join("dictionary.json"), old_json).unwrap();

        // Load - should auto migrate
        let dict = Dictionary::load(&dir);
        assert_eq!(dict.vocabulary_hints.len(), 2);
        assert_eq!(dict.vocabulary_hints[0], "Tauri");
        assert_eq!(dict.vocabulary_hints[1], "Rust");
        assert!(dict.words.is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_apply_replacements() {
        let mut dict = Dictionary::default();
        dict.replacements.push(ReplacementEntry {
            find: "tory".to_string(),
            replace: "Tauri".to_string(),
            case_sensitive: false,
        });
        dict.replacements.push(ReplacementEntry {
            find: "brb".to_string(),
            replace: "be right back".to_string(),
            case_sensitive: false,
        });
        dict.replacements.push(ReplacementEntry {
            find: ":)".to_string(),
            replace: "😊".to_string(),
            case_sensitive: true,
        });

        // Test basic word replace
        assert_eq!(dict.apply_replacements("The tory framework is great."), "The Tauri framework is great.");
        assert_eq!(dict.apply_replacements("The Tory framework."), "The Tauri framework.");
        
        // Test no substring replacing (e.g. story should not become sTauriy)
        assert_eq!(dict.apply_replacements("This is a lovely story."), "This is a lovely story.");

        // Test phrase/shortcut expansion
        assert_eq!(dict.apply_replacements("I will be brb."), "I will be be right back.");

        // Test punctuation / non-alphanumeric replacement
        assert_eq!(dict.apply_replacements("Hello :)"), "Hello 😊");
    }
}

