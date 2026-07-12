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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Correction {
    pub find: String,
    pub replace: String,
}

/// Word-level diff: strip the common leading and trailing runs of equal words;
/// the differing middle of `old` becomes `find`, the middle of `new` becomes `replace`.
/// Returns `None` when there is no meaningful word-level change.
pub fn propose_correction(old: &str, new: &str) -> Option<Correction> {
    let old_words: Vec<&str> = old.split_whitespace().collect();
    let new_words: Vec<&str> = new.split_whitespace().collect();

    let mut start = 0;
    while start < old_words.len()
        && start < new_words.len()
        && old_words[start] == new_words[start]
    {
        start += 1;
    }

    let mut end_old = old_words.len();
    let mut end_new = new_words.len();
    while end_old > start && end_new > start && old_words[end_old - 1] == new_words[end_new - 1] {
        end_old -= 1;
        end_new -= 1;
    }

    let find = old_words[start..end_old].join(" ");
    let replace = new_words[start..end_new].join(" ");

    if find.is_empty() && replace.is_empty() {
        return None;
    }
    Some(Correction { find, replace })
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

        self.save(app_dir)
    }

    pub fn delete_item(&mut self, id: &str, app_dir: &PathBuf) -> Result<(), String> {
        let before = self.items.len();
        self.items.retain(|it| it.id != id);
        if self.items.len() == before {
            return Err(format!("No transcription with id {}", id));
        }
        self.save(app_dir)
    }

    pub fn update_item(&mut self, id: &str, new_text: String, app_dir: &PathBuf) -> Result<(), String> {
        let item = self
            .items
            .iter_mut()
            .find(|it| it.id == id)
            .ok_or_else(|| format!("No transcription with id {}", id))?;
        item.word_count = new_text.split_whitespace().count();
        item.text = new_text;
        self.save(app_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_app_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("typr_hist_test_{}", nanos))
    }

    fn item(id: &str, text: &str) -> TranscriptionItem {
        TranscriptionItem {
            id: id.to_string(),
            timestamp: 0,
            text: text.to_string(),
            duration_secs: 1.0,
            word_count: text.split_whitespace().count(),
        }
    }

    #[test]
    fn test_delete_item_removes_by_id() {
        let dir = temp_app_dir();
        let mut h = History { items: vec![item("a", "one"), item("b", "two"), item("c", "three")] };
        h.delete_item("b", &dir).unwrap();
        let ids: Vec<&str> = h.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "c"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_delete_item_missing_id_errs() {
        let dir = temp_app_dir();
        let mut h = History { items: vec![item("a", "one")] };
        assert!(h.delete_item("zzz", &dir).is_err());
        assert_eq!(h.items.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_update_item_replaces_text_and_recounts() {
        let dir = temp_app_dir();
        let mut it = item("a", "one two");
        it.timestamp = 42;
        it.duration_secs = 3.5;
        let mut h = History { items: vec![it] };
        h.update_item("a", "one two three four".to_string(), &dir).unwrap();
        let got = &h.items[0];
        assert_eq!(got.text, "one two three four");
        assert_eq!(got.word_count, 4);
        assert_eq!(got.timestamp, 42);
        assert_eq!(got.duration_secs, 3.5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_update_item_missing_id_errs() {
        let dir = temp_app_dir();
        let mut h = History { items: vec![item("a", "one")] };
        assert!(h.update_item("zzz", "x".to_string(), &dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_propose_correction_single_word() {
        assert_eq!(
            propose_correction("college male", "college mail"),
            Some(Correction { find: "male".to_string(), replace: "mail".to_string() })
        );
    }

    #[test]
    fn test_propose_correction_prefix_and_suffix() {
        assert_eq!(
            propose_correction("the api is down", "the API is down"),
            Some(Correction { find: "api".to_string(), replace: "API".to_string() })
        );
    }

    #[test]
    fn test_propose_correction_multiword_middle() {
        assert_eq!(
            propose_correction("meet me at noon today", "meet me at ten today"),
            Some(Correction { find: "noon".to_string(), replace: "ten".to_string() })
        );
    }

    #[test]
    fn test_propose_correction_identical_is_none() {
        assert_eq!(propose_correction("same text", "same text"), None);
    }

    #[test]
    fn test_propose_correction_whitespace_only_is_none() {
        assert_eq!(propose_correction("a  b", "a b"), None);
    }

    #[test]
    fn test_propose_correction_pure_insertion() {
        assert_eq!(
            propose_correction("call me", "call me now"),
            Some(Correction { find: String::new(), replace: "now".to_string() })
        );
    }

    #[test]
    fn test_propose_correction_pure_deletion() {
        assert_eq!(
            propose_correction("call me now", "call me"),
            Some(Correction { find: "now".to_string(), replace: String::new() })
        );
    }
}
