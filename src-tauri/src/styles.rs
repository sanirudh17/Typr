use serde::{Serialize, Deserialize};
use std::path::Path;

use crate::context_detector::{ContextCategory, AppRule};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum StyleLevel {
    Formal,
    Casual,
    VeryCasual,
    Developer,
}

impl std::fmt::Display for StyleLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StyleLevel::Formal => write!(f, "Formal"),
            StyleLevel::Casual => write!(f, "Casual"),
            StyleLevel::VeryCasual => write!(f, "Very Casual"),
            StyleLevel::Developer => write!(f, "Developer"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextStyle {
    pub style_level: StyleLevel,
    pub auto_bullet_points: bool,
    pub add_trailing_period: bool,
    pub capitalize_sentences: bool,
    pub remove_filler_words: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StyleConfig {
    pub messaging: ContextStyle,
    pub email: ContextStyle,
    pub professional: ContextStyle,
    pub developer: ContextStyle,
    pub general: ContextStyle,
    pub custom_app_rules: Vec<AppRule>,
    pub enabled: bool,
}

impl Default for StyleConfig {
    fn default() -> Self {
        Self {
            messaging: ContextStyle {
                style_level: StyleLevel::Casual,
                auto_bullet_points: false,
                add_trailing_period: false,
                capitalize_sentences: true,
                remove_filler_words: true,
            },
            email: ContextStyle {
                style_level: StyleLevel::Formal,
                auto_bullet_points: true,
                add_trailing_period: true,
                capitalize_sentences: true,
                remove_filler_words: true,
            },
            professional: ContextStyle {
                style_level: StyleLevel::Formal,
                auto_bullet_points: true,
                add_trailing_period: true,
                capitalize_sentences: true,
                remove_filler_words: true,
            },
            developer: ContextStyle {
                style_level: StyleLevel::Developer,
                auto_bullet_points: true,
                add_trailing_period: false,
                capitalize_sentences: true,
                remove_filler_words: true,
            },
            general: ContextStyle {
                style_level: StyleLevel::Casual,
                auto_bullet_points: false,
                add_trailing_period: true,
                capitalize_sentences: true,
                remove_filler_words: true,
            },
            custom_app_rules: Vec::new(),
            enabled: true,
        }
    }
}

impl StyleConfig {
    pub fn load(app_dir: &Path) -> Self {
        let path = app_dir.join("styles.json");
        if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(data) => match serde_json::from_str(&data) {
                    Ok(config) => return config,
                    Err(e) => eprintln!("[Typr] Failed to parse styles.json: {}", e),
                },
                Err(e) => eprintln!("[Typr] Failed to read styles.json: {}", e),
            }
        }
        Self::default()
    }

    pub fn save(&self, app_dir: &Path) -> Result<(), String> {
        let path = app_dir.join("styles.json");
        let data = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize styles: {}", e))?;
        std::fs::write(&path, data)
            .map_err(|e| format!("Failed to write styles.json: {}", e))?;
        Ok(())
    }

    pub fn get_style_for_category(&self, category: &ContextCategory) -> &ContextStyle {
        match category {
            ContextCategory::Messaging => &self.messaging,
            ContextCategory::Email => &self.email,
            ContextCategory::Professional => &self.professional,
            ContextCategory::Developer => &self.developer,
            ContextCategory::General => &self.general,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_default_config() {
        let config = StyleConfig::default();
        assert!(config.enabled);
        assert_eq!(config.messaging.style_level, StyleLevel::Casual);
        assert_eq!(config.email.style_level, StyleLevel::Formal);
        assert_eq!(config.developer.style_level, StyleLevel::Developer);
    }

    #[test]
    fn test_load_missing_file() {
        let config = StyleConfig::load(&PathBuf::from("nonexistent_dir_12345"));
        assert!(config.enabled);
        assert_eq!(config.general.style_level, StyleLevel::Casual);
    }

    #[test]
    fn test_get_style_for_category() {
        let config = StyleConfig::default();
        let style = config.get_style_for_category(&ContextCategory::Email);
        assert_eq!(style.style_level, StyleLevel::Formal);
        assert!(style.add_trailing_period);
    }
}
