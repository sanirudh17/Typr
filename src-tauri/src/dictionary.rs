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
    #[serde(default)]
    pub entries: Vec<crate::vocabulary::Entry>,
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
        
        let mut migrated=false;
        for term in dict.vocabulary_hints.clone() {
            if !dict.entries.iter().any(|e| e.canonical_term.eq_ignore_ascii_case(&term)) {
                let mut entry = crate::vocabulary::Entry::manual(term);
                if entry.validate().is_ok() { dict.entries.push(entry); migrated=true; }
            }
        }
        dict.sync_hints();
        if migrated { let _=dict.save(app_dir); }
        dict
    }

    pub fn save(&self, app_dir: &PathBuf) -> Result<(), String> {
        let path = Self::config_path(app_dir);
        fs::create_dir_all(app_dir).map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let temporary = app_dir.join("dictionary.json.tmp");
        fs::write(&temporary, json).map_err(|e| e.to_string())?;
        // rename replaces atomically on Windows and Unix; a failed write leaves
        // the last good dictionary untouched.
        fs::rename(&temporary, &path).map_err(|e| e.to_string())
    }

    fn sync_hints(&mut self) {
        self.vocabulary_hints = self.entries.iter().filter(|e| e.enabled).map(|e| e.canonical_term.clone()).collect();
    }

    pub fn upsert_entry(&mut self, mut entry: crate::vocabulary::Entry, app_dir: &PathBuf) -> Result<(), String> {
        entry.validate()?;
        let mut next = self.clone();
        if let Some(existing) = next.entries.iter_mut().find(|e| e.id == entry.id) {
            entry.created_at = existing.created_at;
            entry.hit_count = existing.hit_count;
            entry.source = existing.source.clone();
            *existing = entry;
        } else {
            if next.entries.len() >= crate::vocabulary::MAX_ENTRIES { return Err("Dictionary limit reached (2,000 terms).".into()); }
            entry.id = crate::vocabulary::new_id();
            entry.created_at = crate::vocabulary::now_ms();
            next.entries.push(entry);
        }
        let mut terms = std::collections::HashSet::new();
        if next.entries.iter().any(|e| !terms.insert(e.canonical_term.to_lowercase())) {
            return Err("This preferred spelling is already in your dictionary. Edit its aliases instead.".into());
        }
        for e in &mut next.entries { if self.entries.iter().find(|old| old.id == e.id) != Some(e) { e.updated_at = crate::vocabulary::now_ms(); } }
        next.sync_hints(); next.save(app_dir)?; *self = next; Ok(())
    }

    pub fn delete_entry(&mut self, id: &str, app_dir: &PathBuf) -> Result<(), String> {
        let mut next = self.clone();
        if !next.entries.iter().any(|e| e.id == id) { return Err("Dictionary entry no longer exists.".into()); }
        next.entries.retain(|e| e.id != id); next.sync_hints(); next.save(app_dir)?; *self = next; Ok(())
    }

    pub fn export_entries(&self) -> Result<String, String> {
        serde_json::to_string_pretty(&serde_json::json!({"version":1,"entries":self.entries})).map_err(|e| e.to_string())
    }

    /// Validate the whole import before changing any data. Duplicate spellings
    /// merge aliases without overwriting local metadata or disabled status.
    pub fn import_entries(&mut self, json: &str, app_dir: &PathBuf) -> Result<usize, String> {
        #[derive(serde::Deserialize)] struct Document { version: u32, entries: Vec<crate::vocabulary::Entry> }
        if json.len() > 2_000_000 { return Err("Choose a dictionary JSON smaller than 2 MB.".into()); }
        let mut doc: Document = serde_json::from_str(json).map_err(|_| "Invalid dictionary JSON.".to_string())?;
        if doc.version != 1 || doc.entries.len() > crate::vocabulary::MAX_ENTRIES { return Err("Unsupported dictionary version or too many entries.".into()); }
        for e in &mut doc.entries { e.validate()?; }
        let count = doc.entries.len(); let mut next = self.clone();
        for mut e in doc.entries {
            if let Some(old) = next.entries.iter_mut().find(|old| old.canonical_term.eq_ignore_ascii_case(&e.canonical_term)) {
                old.aliases.extend(e.aliases); old.validate()?; old.updated_at=crate::vocabulary::now_ms();
            } else {
                // Imported IDs are untrusted; always allocate a local identity.
                e.id=crate::vocabulary::new_id(); next.entries.push(e);
            }
        }
        if next.entries.len() > crate::vocabulary::MAX_ENTRIES { return Err("Import would exceed 2,000 terms.".into()); }
        next.sync_hints(); next.save(app_dir)?; *self=next; Ok(count)
    }

    pub fn record_hits(&mut self, transcript: &str, app_dir: &PathBuf) -> Result<(), String> {
        let mut next=self.clone(); let mut changed=false;
        for e in &mut next.entries {
            if e.enabled && crate::vocabulary::contains_term(transcript, &e.canonical_term) {
                e.hit_count=e.hit_count.saturating_add(1); changed=true;
            }
        }
        if changed { next.save(app_dir)?; *self=next; } Ok(())
    }

    pub fn engine_prompt(&self, engine: &str, cloud_opt_in: bool) -> String {
        if engine == "cloud" && !cloud_opt_in || !matches!(engine,"local"|"cloud") { return String::new(); }
        let mut prompt=crate::vocabulary::hotwords(&self.entries,500);
        for r in &self.replacements {
            let term=r.find.trim();
            if term.is_empty() || crate::vocabulary::contains_term(&prompt,term) { continue; }
            let addition=format!("{}{}",if prompt.is_empty(){""}else{", "},term);
            if prompt.chars().count()+addition.chars().count()<=500 { prompt.push_str(&addition); }
        }
        prompt
    }

    // --- Vocabulary Hints ---

    pub fn add_vocabulary_hint(&mut self, word: String, app_dir: &PathBuf) -> Result<(), String> {
        if self.entries.iter().any(|e| e.canonical_term.eq_ignore_ascii_case(word.trim())) { return Ok(()); }
        self.upsert_entry(crate::vocabulary::Entry::manual(word), app_dir)
    }

    pub fn remove_vocabulary_hint(&mut self, index: usize, app_dir: &PathBuf) -> Result<(), String> {
        if index >= self.vocabulary_hints.len() {
            return Err("Invalid dictionary index.".to_string());
        }
        let term=&self.vocabulary_hints[index];
        let id=self.entries.iter().find(|e| &e.canonical_term==term).map(|e|e.id.clone()).ok_or("Dictionary entry no longer exists.")?;
        self.delete_entry(&id,app_dir)
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

    /// Bias prompt for the transcription engine: vocabulary hints plus every
    /// snippet trigger phrase, so the engine is more likely to actually *hear*
    /// a trigger (so the snippet fires). De-duplicated case-insensitively and
    /// capped at MAX_BIAS_CHARS, truncated at an entry boundary.
    pub fn get_bias_prompt(&self) -> String {
        let mut seen: Vec<String> = Vec::new();
        let mut terms: Vec<&str> = Vec::new();

        let hints = self.vocabulary_hints.iter().map(|s| s.as_str());
        let triggers = self.replacements.iter().map(|r| r.find.as_str());
        for raw in hints.chain(triggers) {
            let term = raw.trim();
            if term.is_empty() {
                continue;
            }
            let lower = term.to_lowercase();
            if seen.iter().any(|s| s == &lower) {
                continue;
            }
            seen.push(lower);
            terms.push(term);
        }

        let mut out = String::new();
        for term in terms {
            let addition = if out.is_empty() {
                term.to_string()
            } else {
                format!(", {}", term)
            };
            if out.len() + addition.len() > MAX_BIAS_CHARS {
                break;
            }
            out.push_str(&addition);
        }
        out
    }

    pub fn apply_replacements(&self, text: &str) -> String {
        // Preserve paragraphs, list rows, and literal spacing from the AI/command
        // passes. Normalize the trigger pattern, never the whole transcript.
        let mut result = text.to_string();

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
            let escaped_find = find_normalized
                .split(' ')
                .map(escape)
                .collect::<Vec<_>>()
                .join(r"[^\S\r\n]+");
            
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
                // A snippet may contain its own trigger ("check" -> "check the
                // folder"). The post-AI reassertion must not expand it twice.
                let protected: Vec<_> = if entry.replace.is_empty() { Vec::new() }
                    else { RegexBuilder::new(&escape(&entry.replace))
                        .case_insensitive(!entry.case_sensitive).build().unwrap()
                        .find_iter(&result).map(|m| m.range()).collect() };
                result = re.replace_all(&result, |caps: &regex::Captures| {
                    let matched = caps.get(0).unwrap();
                    if protected.iter().any(|range| range.start <= matched.start() && range.end >= matched.end()) {
                        matched.as_str().to_string()
                    } else if entry.case_sensitive {
                        entry.replace.clone()
                    } else {
                        preserve_case(matched.as_str(), &entry.replace)
                    }
                }).to_string();
            }
        }
        result
    }

    /// A shortcut dictated by itself is an explicit request for its saved text.
    /// Ignore only surrounding whitespace and ASR sentence punctuation; keep
    /// the entry's case setting and never infer a match from similar words.
    pub fn standalone_snippet(&self, text: &str) -> Option<String> {
        let mut entries: Vec<_> = self.replacements.iter().collect();
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.find.len()));
        for entry in entries {
            let words: Vec<_> = entry.find.split_whitespace().map(escape).collect();
            if words.is_empty() { continue; }
            let pattern = format!(r"\A(?:{})[.!?]*\z", words.join(r"[^\S\r\n]+"));
            if let Ok(re) = RegexBuilder::new(&pattern)
                .case_insensitive(!entry.case_sensitive).build()
            {
                if re.is_match(text.trim()) { return Some(entry.replace.clone()); }
            }
        }
        None
    }
}

/// Character budget for the engine bias prompt (Whisper's prompt window is ~224 tokens;
/// keep it conservative and truncate at an entry boundary).
///
/// Sized at ~2.2 chars/token, not English prose's ~4. This budget is spent almost entirely on
/// proper nouns and unusual names — exactly the tokens a BPE vocabulary splits hardest — so
/// 800 chars of hints could exceed 224 tokens and be silently truncated by Whisper, dropping
/// the tail of the user's dictionary without any error.
const MAX_BIAS_CHARS: usize = 500;

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
    #[test]
    fn rich_dictionary_roundtrip_and_no_alias_replacement() {
        let dir=std::env::temp_dir().join(crate::vocabulary::new_id());
        let mut dict=Dictionary::default();
        let mut entry=crate::vocabulary::Entry::manual("Supabase".into());
        entry.aliases=vec!["supper base".into()]; entry.category=crate::vocabulary::Category::Product;
        entry.source=crate::vocabulary::Source::Learned; entry.confidence=0.85; entry.notes="Database platform".into();
        dict.upsert_entry(entry,&dir).unwrap(); dict.record_hits("Deploy to Supabase.",&dir).unwrap();
        assert_eq!(dict.entries[0].hit_count,1);
        assert_eq!(dict.apply_replacements("supper base"),"supper base");
        let mut imported=Dictionary::default(); imported.import_entries(&dict.export_entries().unwrap(),&dir).unwrap();
        let mut original=dict.entries[0].clone(); original.id=imported.entries[0].id.clone();
        assert_eq!(original,imported.entries[0]);
        assert_eq!(Dictionary::load(&dir).entries,imported.entries);
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn import_is_atomic_and_merges_without_enabling_disabled_entries() {
        let dir=std::env::temp_dir().join(crate::vocabulary::new_id());
        let mut dict=Dictionary::default(); let mut e=crate::vocabulary::Entry::manual("Tauri".into());
        e.enabled=false; dict.upsert_entry(e.clone(),&dir).unwrap();
        e.aliases=vec!["tory".into()];
        let good=serde_json::json!({"version":1,"entries":[e.clone()]});
        dict.import_entries(&good.to_string(),&dir).unwrap();
        assert!(!dict.entries[0].enabled); assert_eq!(dict.entries[0].aliases,vec!["tory"]);
        assert!(Dictionary::load(&dir).vocabulary_hints.is_empty());
        let before=dict.export_entries().unwrap(); e.canonical_term="".into();
        assert!(dict.import_entries(&serde_json::json!({"version":1,"entries":[e]}).to_string(),&dir).is_err());
        assert_eq!(dict.export_entries().unwrap(),before);
        assert!(dict.import_entries(r#"{"version":2,"entries":[]}"#,&dir).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn cloud_opt_in_and_engine_capability_are_enforced() {
        let mut dict=Dictionary::default(); dict.entries.push(crate::vocabulary::Entry::manual("Tauri".into()));
        assert_eq!(dict.engine_prompt("local",false),"Tauri");
        assert!(dict.engine_prompt("cloud",false).is_empty());
        assert_eq!(dict.engine_prompt("cloud",true),"Tauri");
        assert!(dict.engine_prompt("parakeet",true).is_empty()); assert!(dict.engine_prompt("nemotron",true).is_empty());
    }

    #[test]
    fn replacements_preserve_layout_and_literal_snippets() {
        let mut dict = Dictionary::default();
        let text = "First paragraph.\n\n1. my  app\n2. set env\n";
        assert_eq!(dict.apply_replacements(text), text);
        dict.replacements = vec![
            ReplacementEntry { find: "my app".into(), replace: "FluidVoice".into(), case_sensitive: false },
            ReplacementEntry { find: "set env".into(), replace: "$env:PATH = $PATH".into(), case_sensitive: true },
        ];
        assert_eq!(dict.apply_replacements(text), "First paragraph.\n\n1. FluidVoice\n2. $env:PATH = $PATH\n");
        assert_eq!(dict.apply_replacements("my\napp"), "my\napp");
    }

    #[test]
    fn dictionary_reassertion_does_not_repeat_an_expansion() {
        let mut dict = Dictionary::default();
        dict.replacements.push(ReplacementEntry { find: "check".into(), replace: "check the folder".into(), case_sensitive: false });
        let first = dict.apply_replacements("Check\n\ncheck");
        assert_eq!(first, "Check the folder\n\ncheck the folder");
        assert_eq!(dict.apply_replacements(&first), first);
    }
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
    fn standalone_email_shortcuts_match_sentence_case_and_asr_punctuation() {
        let mut dict = Dictionary::default();
        dict.replacements.push(ReplacementEntry {
            find: "personal email".into(), replace: "voice@example.com".into(), case_sensitive: false,
        });
        for transcript in ["personal email", "Personal email.", "PERSONAL EMAIL!", " Personal  email? "] {
            assert_eq!(dict.standalone_snippet(transcript), Some("voice@example.com".into()));
            assert_eq!(crate::cleanup::cleanup_text(&dict.apply_replacements(transcript)), "voice@example.com");
        }
        assert_eq!(dict.standalone_snippet("Please use my personal email."), None);
        assert_eq!(dict.apply_replacements("Please use my personal email."), "Please use my voice@example.com.");
        for unrelated in ["Personal emails.", "Personal e-mail.", "Personal\nemail", "Personal email and name."] {
            assert_eq!(dict.standalone_snippet(unrelated), None);
        }
    }

    #[test]
    fn standalone_shortcuts_respect_explicit_case_matching() {
        let mut dict = Dictionary::default();
        dict.replacements.push(ReplacementEntry {
            find: "personal email".into(), replace: "voice@example.com".into(), case_sensitive: true,
        });
        assert_eq!(dict.standalone_snippet("personal email."), Some("voice@example.com".into()));
        assert_eq!(dict.standalone_snippet("Personal email."), None);
        assert_eq!(dict.apply_replacements("Personal email."), "Personal email.");
    }

    #[test]
    fn standalone_shortcuts_keep_literal_content_and_prefer_longer_triggers() {
        let mut dict = Dictionary::default();
        dict.replacements = vec![
            ReplacementEntry { find: "contact".into(), replace: "short".into(), case_sensitive: false },
            ReplacementEntry { find: "contact.".into(), replace: "uh new line stop\n$PATH".into(), case_sensitive: false },
            ReplacementEntry { find: "clear token".into(), replace: String::new(), case_sensitive: false },
        ];
        assert_eq!(dict.standalone_snippet("Contact."), Some("uh new line stop\n$PATH".into()));
        assert_eq!(dict.standalone_snippet("Clear token."), Some(String::new()));
        assert_eq!(dict.standalone_snippet("We need contact."), None);
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
    fn test_bias_prompt_merges_hints_and_triggers() {
        let mut dict = Dictionary::default();
        dict.vocabulary_hints = vec!["Tauri".to_string()];
        dict.replacements.push(ReplacementEntry {
            find: "college mail".to_string(),
            replace: "a@b.edu".to_string(),
            case_sensitive: false,
        });
        let p = dict.get_bias_prompt();
        assert!(p.contains("Tauri"));
        assert!(p.contains("college mail"));
    }

    #[test]
    fn test_bias_prompt_dedupes_case_insensitively_and_drops_empties() {
        let mut dict = Dictionary::default();
        dict.vocabulary_hints = vec!["Claude".to_string(), "  ".to_string()];
        dict.replacements.push(ReplacementEntry {
            find: "claude".to_string(),
            replace: "x".to_string(),
            case_sensitive: false,
        });
        let p = dict.get_bias_prompt();
        assert_eq!(p.to_lowercase().matches("claude").count(), 1);
        assert!(!p.contains(",  ,"));
    }

    #[test]
    fn test_bias_prompt_truncates_at_entry_boundary_within_budget() {
        let mut dict = Dictionary::default();
        // 40 hints of ~27 chars each far exceeds the 800-char budget.
        dict.vocabulary_hints = (0..40).map(|i| format!("term{:02}_padding_padding_word", i)).collect();
        let p = dict.get_bias_prompt();
        assert!(p.len() <= 800, "len was {}", p.len());
        // No entry is cut mid-term: the string never ends with a partial ", ".
        assert!(!p.ends_with(','));
        assert!(!p.ends_with(", "));
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

