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

// Reads a hint list that may contain EITHER the bare-string form (`"Claude"`)
// or the structured `{ "word": ..., "enforce": ... }` form written by the
// short-lived enforce-toggle experiment. Both collapse to the plain word, so old
// dictionary.json files load cleanly and get re-saved as simple strings.
fn deserialize_hints<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Hint {
        Bare(String),
        Structured {
            word: String,
            #[serde(default)]
            #[allow(dead_code)]
            enforce: bool,
        },
    }

    let raw = Vec::<Hint>::deserialize(deserializer)?;
    Ok(raw
        .into_iter()
        .map(|h| match h {
            Hint::Bare(word) => word,
            Hint::Structured { word, .. } => word,
        })
        .collect())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Dictionary {
    // Vocabulary hints are a pure recognition aid: they are sent to the engine as
    // a bias prompt to improve how it hears these terms. They NEVER rewrite the
    // transcript, so common words are never mangled. Deterministic spelling/word
    // correction that understands context is handled by AI post-processing.
    #[serde(default, deserialize_with = "deserialize_hints")]
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
        // De-duplicate case-insensitively so "Claude"/"claude" don't both linger.
        if !self
            .vocabulary_hints
            .iter()
            .any(|w| w.eq_ignore_ascii_case(&trimmed))
        {
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
        // A comma-separated list of proper-noun spellings is the officially
        // recommended way to bias Whisper. De-duplicate case-insensitively so a
        // polluted list doesn't waste the limited prompt window.
        let mut seen: Vec<String> = Vec::new();
        let mut words: Vec<&str> = Vec::new();
        for hint in &self.vocabulary_hints {
            let word = hint.trim();
            if word.is_empty() {
                continue;
            }
            let lower = word.to_lowercase();
            if seen.iter().any(|s| s == &lower) {
                continue;
            }
            seen.push(lower);
            words.push(word);
        }
        words.join(", ")
    }

    pub fn apply_replacements(&self, text: &str) -> String {
        // Normalize multiple spaces in the input text to a single space
        let normalized_text = text.split_whitespace().collect::<Vec<&str>>().join(" ");
        let mut result = normalized_text;

        // Sort replacements by find field length in descending order,
        // so that longer phrase/context matches are evaluated and replaced first.
        let mut sorted_replacements = self.replacements.clone();
        sorted_replacements.sort_by(|a, b| b.find.len().cmp(&a.find.len()));

        for entry in &sorted_replacements {
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
                if entry.case_sensitive {
                    result = re.replace_all(&result, &entry.replace).to_string();
                } else {
                    // For case-insensitive replacements, respect the original casing context (e.g. title case or ALL CAPS)
                    result = re.replace_all(&result, |caps: &regex::Captures| {
                        preserve_case(&caps[0], &entry.replace)
                    }).to_string();
                }
            }
        }
        result
    }
}

/// True when a replacement encodes its own intentional casing/structure and must
/// NOT inherit the matched text's casing — emails, URLs, handles, code, acronyms,
/// or anything containing an uppercase letter, a digit, or a structural symbol.
fn has_intrinsic_casing(s: &str) -> bool {
    s.chars()
        .any(|c| c.is_uppercase() || c.is_ascii_digit())
        || s.contains('@')
        || s.contains('/')
        || s.contains(':')
        || s.contains('_')
}

/// Helper function to preserve the casing of a matched string when replacing it.
fn preserve_case(matched: &str, replacement: &str) -> String {
    if matched.is_empty() || replacement.is_empty() {
        return replacement.to_string();
    }

    // A replacement with its own casing (e.g. sanirudh1017@gmail.com) is emitted
    // exactly as written; borrowing the trigger's case would corrupt it.
    if has_intrinsic_casing(replacement) {
        return replacement.to_string();
    }

    // Check if matched is all uppercase (excluding non-alphabetic chars)
    let is_all_uppercase = matched.chars().any(|c| c.is_alphabetic())
        && matched.chars().filter(|c| c.is_alphabetic()).all(|c| c.is_uppercase());

    // Mirror ALL CAPS only onto a single-word replacement. A short spoken trigger
    // (e.g. "BRB") is often rendered in caps by the engine, and shouting a whole
    // multi-word expansion ("BE RIGHT BACK") is almost never intended — those fall
    // through to sentence-case capitalization below.
    if is_all_uppercase && replacement.split_whitespace().count() <= 1 {
        return replacement.to_uppercase();
    }

    // Check if matched is capitalized (first alphabetic character is uppercase)
    let is_capitalized = matched.chars().find(|c| c.is_alphabetic())
        .map_or(false, |c| c.is_uppercase());

    if is_capitalized {
        let mut chars = replacement.chars();
        match chars.next() {
            None => String::new(),
            Some(first) => {
                first.to_uppercase().collect::<String>() + chars.as_str()
            }
        }
    } else {
        replacement.to_string()
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
    fn test_bare_string_hints_load() {
        let json = r#"{"vocabulary_hints": ["Claude", "app", ".exe", "CAT"]}"#;
        let dict: Dictionary = serde_json::from_str(json).unwrap();
        assert_eq!(dict.vocabulary_hints, vec!["Claude", "app", ".exe", "CAT"]);
    }

    #[test]
    fn test_structured_hints_load_collapses_to_word() {
        // Files written by the short-lived enforce-toggle experiment still load,
        // collapsing to the plain word (the enforce flag is dropped).
        let json = r#"{"vocabulary_hints": [{"word": "claude", "enforce": true}, {"word": "Rust", "enforce": false}]}"#;
        let dict: Dictionary = serde_json::from_str(json).unwrap();
        assert_eq!(dict.vocabulary_hints, vec!["claude", "Rust"]);
    }

    #[test]
    fn test_hints_never_rewrite_transcript() {
        // Guard the core promise: hints are bias-only and must never mangle text.
        // (There is no enforcement API; the recorder pipeline applies only
        // replacements + cleanup, never the hint list.)
        let mut dict = Dictionary::default();
        dict.vocabulary_hints.push("CAT".to_string());
        dict.vocabulary_hints.push("Claude".to_string());
        // apply_replacements only touches configured replacements, not hints:
        assert_eq!(dict.apply_replacements("the cat sat with claude"), "the cat sat with claude");
    }

    #[test]
    fn test_get_prompt_dedupes_case_insensitively() {
        let mut dict = Dictionary::default();
        dict.vocabulary_hints.push("Claude".to_string());
        dict.vocabulary_hints.push("claude".to_string());
        dict.vocabulary_hints.push("Rust".to_string());
        // First-seen spelling wins; duplicates dropped.
        assert_eq!(dict.get_prompt(), "Claude, Rust");
    }

    #[test]
    fn test_add_hint_dedupes_case_insensitively() {
        let dir = temp_dir().join("typr_test_add_hint_dedupe");
        let _ = fs::remove_dir_all(&dir);
        let mut dict = Dictionary::default();
        dict.add_vocabulary_hint("claude".to_string(), &dir).unwrap();
        // Re-adding the same word in a different case does not create a duplicate.
        dict.add_vocabulary_hint("Claude".to_string(), &dir).unwrap();
        assert_eq!(dict.vocabulary_hints.len(), 1);
        assert_eq!(dict.vocabulary_hints[0], "claude");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_replacement_preserves_email_casing_at_sentence_start() {
        let mut dict = Dictionary::default();
        dict.replacements.push(ReplacementEntry {
            find: "personal email".to_string(),
            replace: "sanirudh1017@gmail.com".to_string(),
            case_sensitive: false,
        });
        // Trigger capitalized by the engine at sentence start must NOT title-case
        // the email (which would corrupt it to "Sanirudh1017@gmail.com").
        assert_eq!(
            dict.apply_replacements("Personal email"),
            "sanirudh1017@gmail.com"
        );
    }

    #[test]
    fn test_replacement_preserves_url_casing() {
        let mut dict = Dictionary::default();
        dict.replacements.push(ReplacementEntry {
            find: "my site".to_string(),
            replace: "https://GitHub.com/sanirudh17".to_string(),
            case_sensitive: false,
        });
        assert_eq!(
            dict.apply_replacements("My site"),
            "https://GitHub.com/sanirudh17"
        );
    }

    #[test]
    fn test_replacement_still_borrows_case_for_plain_words() {
        // Plain lowercase word replacements should still adapt to sentence case.
        let mut dict = Dictionary::default();
        dict.replacements.push(ReplacementEntry {
            find: "brb".to_string(),
            replace: "be right back".to_string(),
            case_sensitive: false,
        });
        assert_eq!(dict.apply_replacements("Brb"), "Be right back");
        assert_eq!(dict.apply_replacements("i said brb"), "i said be right back");
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

    #[test]
    fn test_phrase_before_word_ordering() {
        let mut dict = Dictionary::default();
        // Add shorter replacement first
        dict.replacements.push(ReplacementEntry {
            find: "water".to_string(),
            replace: "H2O".to_string(),
            case_sensitive: false,
        });
        // Add longer phrase later
        dict.replacements.push(ReplacementEntry {
            find: "power water".to_string(),
            replace: "hydroelectric energy".to_string(),
            case_sensitive: false,
        });

        // The longer phrase "power water" should be replaced with "hydroelectric energy"
        // rather than "water" being replaced first and breaking the phrase to "power H2O".
        assert_eq!(
            dict.apply_replacements("We need to produce power water from the river."),
            "We need to produce hydroelectric energy from the river."
        );
    }

    #[test]
    fn test_case_preservation() {
        let mut dict = Dictionary::default();
        // Add lowercase find and replace
        dict.replacements.push(ReplacementEntry {
            find: "whisper".to_string(),
            replace: "speech engine".to_string(),
            case_sensitive: false,
        });
        dict.replacements.push(ReplacementEntry {
            find: "groq".to_string(),
            replace: "cloud provider".to_string(),
            case_sensitive: false,
        });

        // Test Title Case capitalization preservation (e.g. start of sentence)
        assert_eq!(
            dict.apply_replacements("Whisper is a highly accurate tool."),
            "Speech engine is a highly accurate tool."
        );

        // An ALL CAPS trigger no longer shouts a multi-word expansion; it falls
        // back to sentence-case capitalization instead of "SPEECH ENGINE".
        assert_eq!(
            dict.apply_replacements("We love WHISPER for transcription."),
            "We love Speech engine for transcription."
        );

        // Test keeping custom user casing when matched text is lowercase
        assert_eq!(
            dict.apply_replacements("let's search on groq api."),
            "let's search on cloud provider api."
        );
    }

    #[test]
    fn test_all_caps_trigger_does_not_shout_multiword_expansion() {
        // The engine often renders a short spoken trigger in caps ("BRB"). A
        // multi-word expansion must not come out shouting ("BE RIGHT BACK").
        let mut dict = Dictionary::default();
        dict.replacements.push(ReplacementEntry {
            find: "brb".to_string(),
            replace: "be right back".to_string(),
            case_sensitive: false,
        });

        // ALL CAPS trigger -> sentence-case, not shouting.
        assert_eq!(dict.apply_replacements("BRB"), "Be right back");
        // Capitalized trigger -> first letter only.
        assert_eq!(dict.apply_replacements("Brb"), "Be right back");
        // Lowercase trigger -> verbatim lowercase.
        assert_eq!(dict.apply_replacements("brb"), "be right back");
        // Mid-sentence ALL CAPS trigger is not shouted either.
        assert_eq!(
            dict.apply_replacements("i said BRB and left"),
            "i said Be right back and left"
        );
    }

    #[test]
    fn test_all_caps_trigger_still_mirrors_single_word_expansion() {
        // A single-word expansion still mirrors ALL CAPS (genuine acronym use).
        let mut dict = Dictionary::default();
        dict.replacements.push(ReplacementEntry {
            find: "whisper".to_string(),
            replace: "engine".to_string(),
            case_sensitive: false,
        });
        assert_eq!(
            dict.apply_replacements("We love WHISPER here."),
            "We love ENGINE here."
        );
    }
}

