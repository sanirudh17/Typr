use std::collections::HashSet;
use std::sync::OnceLock;

fn common_words() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| {
        include_str!("../assets/common_words.txt")
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect()
    })
}

/// True when `lower` (already lowercased) is an ordinary English word we must never
/// overwrite with a correction.
pub(crate) fn is_common_word(lower: &str) -> bool {
    common_words().contains(lower)
}

// These proper names have no ordinary English reading. Keep this list separate
// from common words such as Whisper and Parakeet, which can mean something else.
const BUILTIN_NAMES: &[&str] = &["Nemotron", "FluidVoice"];

/// American Soundex phonetic key: the first letter (uppercased) followed by three
/// digits encoding the following consonants, zero-padded. Vowels separate repeated
/// consonant codes; H and W do not. Returns "" when there are no ASCII letters.
pub fn soundex(word: &str) -> String {
    fn code(c: char) -> Option<char> {
        match c {
            'B' | 'F' | 'P' | 'V' => Some('1'),
            'C' | 'G' | 'J' | 'K' | 'Q' | 'S' | 'X' | 'Z' => Some('2'),
            'D' | 'T' => Some('3'),
            'L' => Some('4'),
            'M' | 'N' => Some('5'),
            'R' => Some('6'),
            _ => None, // vowels, H, W, Y
        }
    }

    let letters: Vec<char> = word
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if letters.is_empty() {
        return String::new();
    }

    let mut result = String::new();
    result.push(letters[0]);
    let mut prev = code(letters[0]);

    for &c in &letters[1..] {
        let cur = code(c);
        if let Some(d) = cur {
            if Some(d) != prev {
                result.push(d);
                if result.len() == 4 {
                    break;
                }
            }
        }
        // Vowels reset the previous code (so a repeated consonant across a vowel is
        // coded twice); H and W are transparent and leave `prev` unchanged.
        if c != 'H' && c != 'W' {
            prev = cur;
        }
    }

    while result.len() < 4 {
        result.push('0');
    }
    result
}

/// Max Levenshtein distance allowed between a token and a hint, by hint length.
/// Short hints get a tighter bound to avoid loose matches.
fn distance_threshold(hint_len: usize) -> usize {
    if hint_len <= 4 { 1 } else { 2 }
}

/// Decide whether `token_lower` (lowercased, punctuation already stripped) should be
/// corrected to `hint`. Requires BOTH a Soundex match AND a tight edit distance, and
/// refuses to touch ordinary English words.
fn hint_matches(token_lower: &str, hint: &str) -> bool {
    let hint_lower = hint.to_lowercase();
    if token_lower.is_empty() || hint_lower.is_empty() {
        return false;
    }
    if token_lower == hint_lower {
        return false; // already correct (case handled by the caller)
    }
    if is_common_word(token_lower) {
        return false; // never overwrite an ordinary word
    }
    // A contraction is ordinary English, and `common_words.txt` carries no apostrophes at
    // all, so the check above can never protect one. Dictionary hints are technical terms
    // and names, which essentially never contain an apostrophe — so a contraction matching
    // a non-contraction hint is a false positive, not a mis-hearing.
    //
    // This is not hypothetical: "let's" was rewritten to the hint "notes" mid-dictation.
    // Both key to the same phonetic skeleton (L320 / N320 -> "320"), the first-letter
    // mismatch widened the budget to 3 edits, and levenshtein("let's", "notes") is exactly
    // 3 — so a five-letter word matched with three of five characters different.
    if token_lower.contains('\'') && !hint_lower.contains('\'') {
        return false;
    }
    // Phonetic gate: the token must key to the same Soundex sound as the hint (a
    // first-letter drop earns no extra tolerance — that was the "a syllable of a
    // dictionary entry replaces the whole word" regression: "ori"/"hori" wiped out
    // ordinary words because a skeleton-only match widened the edit budget to 3).
    let token_key = soundex(token_lower);
    let hint_key = soundex(&hint_lower);
    if token_key.is_empty() || hint_key.is_empty() || token_key != hint_key {
        return false;
    }
    // Tight edit budget only: a genuine mis-hearing differs by a letter or two
    // ("kubernetis" -> "Kubernetes"), not by half the word.
    if strsim::levenshtein(token_lower, &hint_lower) > distance_threshold(hint_lower.len()) {
        return false;
    }
    // Fragment guard: the token must not be much shorter than the hint. A 3-letter
    // fragment of ordinary speech ("tur" for "turn") keys and edits close to a longer
    // hint ("Tauri") and would be snapped into a confidently-wrong word — worse than
    // leaving the fragment for the AI pass ("tur on dark theme" -> "turn on ...").
    // Genuine mis-hearings ("tori", "sonet") are at most one character short.
    if token_lower.chars().count() + 1 < hint_lower.chars().count() {
        return false;
    }
    true
}

/// Split a token into (leading non-alphanumerics, core, trailing non-alphanumerics),
/// so we match on the core word but re-attach surrounding punctuation.
fn split_affixes(token: &str) -> (&str, &str, &str) {
    let start = token.find(|c: char| c.is_alphanumeric()).unwrap_or(token.len());
    let end = token
        .rfind(|c: char| c.is_alphanumeric())
        .map(|i| i + token[i..].chars().next().unwrap().len_utf8())
        .unwrap_or(start);
    (&token[..start], &token[start..end], &token[end..])
}

/// Re-apply the casing of `original` to `replacement`: if the original core started with
/// an uppercase letter, capitalize the replacement's first letter; otherwise use the hint
/// spelling as-is (hints carry their own intended casing, e.g. "Kubernetes").
fn apply_casing(original_core: &str, replacement: &str) -> String {
    let original_capitalized = original_core
        .chars()
        .next()
        .map(|c| c.is_uppercase())
        .unwrap_or(false);
    if original_capitalized {
        let mut chars = replacement.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        }
    } else {
        replacement.to_string()
    }
}

/// Correct transcript words to their exact dictionary spellings. Whitespace-preserving:
/// each whitespace-separated token is examined; its core is matched against the hints and,
/// on a confident match, replaced while keeping surrounding punctuation and leading case.
/// Applies only the confirmed built-in spellings when `hints` is empty.
pub fn correct_vocabulary(text: &str, hints: &[String]) -> String {
    let text = correct_compound_names(text, hints);
    static TOKENS: OnceLock<regex::Regex> = OnceLock::new();
    let tokens = TOKENS.get_or_init(|| regex::Regex::new(r"\S+").unwrap());

    tokens.replace_all(&text, |caps: &regex::Captures| {
            let token = &caps[0];
            let (lead, core, trail) = split_affixes(token);
            if core.is_empty() {
                return token.to_string();
            }
            // Handles and separated path punctuation can sit in the affixes
            // rather than the core. Keep them out of prose spelling correction.
            if lead.ends_with(['@', '/', '\\', ':', '_'])
                || trail.starts_with(['@', '/', '\\', ':', '_'])
                || core.contains(['@', '/', '\\', ':', '_', '.']) {
                return token.to_string();
            }
            let core_lower = core.to_lowercase();
            for hint in hints {
                // Case-only enforcement: the engine (or the AI pass) rendered the hint with
                // the wrong case — "herdr"/"HERDR" for "Herdr". The spelling already matches,
                // so snapping is safe; all-lowercase hints never rewrite (hint "claude" must
                // not lowercase a proper "Claude").
                if core != hint
                    && core_lower == hint.to_lowercase()
                    && hint.chars().any(|c| c.is_uppercase())
                {
                    return format!("{}{}{}", lead, hint, trail);
                }
                if hint_matches(&core_lower, hint) {
                    return format!("{}{}{}", lead, apply_casing(core, hint), trail);
                }
            }
            // An explicit dictionary spelling takes precedence over built-ins,
            // including a lowercase spelling the user wants kept verbatim.
            if hints.iter().any(|h| h.eq_ignore_ascii_case(core)) {
                return token.to_string();
            }
            for &name in BUILTIN_NAMES {
                let lower = name.to_lowercase();
                // A close phonetic spelling or a single spurious leading letter
                // (mnemotron) can be normalized without rewriting valid words.
                let extra_initial = !is_common_word(&core_lower)
                    && core_lower.chars().count() == lower.chars().count() + 1
                    && core_lower.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                    && core_lower.get(1..) == Some(lower.as_str());
                // Confirmed whole-name errors in the retained microphone WAVs,
                // also reproduced by both Whisper engines. Keep these explicit;
                // do not reopen the broad first-letter Soundex matching that
                // previously replaced ordinary words and short fragments.
                let confirmed_alias = name == "Nemotron"
                    && matches!(core_lower.as_str(), "lemotron" | "limotron");
                if core_lower == lower || hint_matches(&core_lower, name) || extra_initial || confirmed_alias {
                    return format!("{}{}{}", lead, name, trail);
                }
            }
            // A confirmed invalid-word error from the user's export, shared by
            // all engines. Never spell-check arbitrary ordinary words or names.
            if matches!(core_lower.as_str(), "accuries" | "acuracy")
                && !hints.iter().any(|h| h.eq_ignore_ascii_case(core)) {
                return format!("{}{}{}", lead, apply_casing(core, "accuracy"), trail);
            }
            token.to_string()
        })
        .into_owned()
}

/// Rejoin exact spoken parts of established product names or the user's explicit
/// compound dictionary spellings. This is spelling normalization, not fuzzy
/// phrase rewriting; ordinary phrases and line boundaries are left alone.
fn correct_compound_names(text: &str, hints: &[String]) -> String {
    let mut result = text.to_string();
    for hint in std::iter::once("FluidVoice").chain(hints.iter().map(String::as_str)) {
        if !hint.chars().all(|c| c.is_ascii_alphabetic()) { continue; }
        let mut parts = Vec::new();
        let mut start = 0;
        for (i, c) in hint.char_indices().skip(1) {
            if c.is_uppercase() && hint.as_bytes()[i - 1].is_ascii_lowercase() {
                parts.push(&hint[start..i]);
                start = i;
            }
        }
        parts.push(&hint[start..]);
        if parts.len() < 2 { continue; }
        let pattern = format!(r"(?i)\b{}\b", parts.iter().map(|p| regex::escape(p)).collect::<Vec<_>>().join(r"[^\S\r\n]+"));
        let Ok(re) = regex::Regex::new(&pattern) else { continue; };
        result = re.replace_all(&result, |caps: &regex::Captures| {
            let matched = caps.get(0).unwrap();
            let structural = |c: char| matches!(c, '@' | '/' | '\\' | ':' | '_' | '.');
            let before = result[..matched.start()].chars().next_back();
            let after = result[matched.end()..].chars().next();
            let filename_suffix = after == Some('.') && result[matched.end() + 1..]
                .chars().next().is_some_and(|c| c.is_alphanumeric());
            if before.is_some_and(structural) || filename_suffix || after.is_some_and(|c| structural(c) && c != '.') {
                matched.as_str().to_string()
            } else { hint.to_string() }
        }).into_owned();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_names_are_corrected_without_ai_or_user_hints() {
        assert_eq!(correct_vocabulary("compare fluid voice with nimotron", &[]),
            "compare FluidVoice with Nemotron");
        assert_eq!(correct_vocabulary("mnemotron and nemotron", &[]), "Nemotron and Nemotron");
        assert_eq!(correct_vocabulary("Lemotron and limotron", &[]), "Nemotron and Nemotron");
        assert_eq!(correct_vocabulary("limotron", &["Limotron".into()]), "Limotron");
        assert_eq!(correct_vocabulary("new tron, motron, limotron.rs and @lemotron", &[]),
            "new tron, motron, limotron.rs and @lemotron");
        assert_eq!(correct_vocabulary("fluent voice and fluid wise with a parakeet", &[]),
            "fluent voice and fluid wise with a parakeet");
        assert_eq!(correct_vocabulary("nimotron.rs @nimotron src/nimotron fluidvoice.c nemotron.h", &[]),
            "nimotron.rs @nimotron src/nimotron fluidvoice.c nemotron.h");
        assert_eq!(correct_vocabulary("nimotron", &["nimotron".into()]), "nimotron");
        assert_eq!(correct_vocabulary("nimotron", &["Nimotron".into()]), "Nimotron");
    }

    #[test]
    fn confirmed_spellings_work_without_ai_or_dictionary_configuration() {
        assert_eq!(correct_vocabulary("Accuries improvisations and general fixes.", &[]), "Accuracy improvisations and general fixes.");
        assert_eq!(correct_vocabulary("Acuracy improvisations.", &[]), "Accuracy improvisations.");
        assert_eq!(correct_vocabulary("acuracy", &["Acuracy".into()]), "Acuracy");
        assert_eq!(correct_vocabulary("acuracy.rs and @acuracy", &[]), "acuracy.rs and @acuracy");
        assert_eq!(correct_vocabulary("compare fluid voice with Nemotron", &[]), "compare FluidVoice with Nemotron");
        assert_eq!(correct_vocabulary("accuries.rs and me@accuries.dev", &[]), "accuries.rs and me@accuries.dev");
        assert_eq!(correct_vocabulary("@accuries /accuries :accuries", &[]), "@accuries /accuries :accuries");
        assert_eq!(correct_vocabulary("accuries/ accuries\\ accuries:", &[]), "accuries/ accuries\\ accuries:");
        assert_eq!(correct_vocabulary("a picture of the team taking a shot", &[]), "a picture of the team taking a shot");
    }

    #[test]
    fn dictionary_names_preserve_line_boundaries_and_only_join_exact_spelling() {
        let hints = vec!["GitHub".into(), "Kubernetes".into(), "CAT".into()];
        assert_eq!(correct_vocabulary("git hub\n\nkubernetis\tCAT", &hints), "GitHub\n\nKubernetes\tCAT");
        assert_eq!(correct_vocabulary("git\nhub", &hints), "git\nhub");
        assert_eq!(correct_vocabulary("quick overlay button", &hints), "quick overlay button");
        assert_eq!(correct_vocabulary("src/fluid voice.txt", &hints), "src/fluid voice.txt");
        assert_eq!(correct_vocabulary("fluid voice.rs", &hints), "fluid voice.rs");
        assert_eq!(correct_vocabulary("fluid voice.", &hints), "FluidVoice.");
    }

    #[test]
    fn test_soundex_known_values() {
        assert_eq!(soundex("Robert"), "R163");
        assert_eq!(soundex("Rupert"), "R163");
        assert_eq!(soundex("Tymczak"), "T522");
        // The mis-hearing and the intended word share a key.
        assert_eq!(soundex("warring"), soundex("whoring"));
        assert_eq!(soundex("sonet"), soundex("Sonnet"));
        // Empty / non-alpha input.
        assert_eq!(soundex(""), "");
        assert_eq!(soundex("123"), "");
    }

    #[test]
    fn test_contraction_never_corrected_to_hint() {
        // The real regression: "let's" was rewritten to the hint "notes". Both key to the
        // same phonetic skeleton and the edit distance sat exactly on the widened budget.
        assert!(!hint_matches("let's", "notes"));
        // The same shape for other everyday contractions, none of which appear in
        // common_words.txt (it contains no apostrophes at all).
        assert!(!hint_matches("that's", "types"));
        assert!(!hint_matches("it's", "its"));
        // The apostrophe-less spellings are already covered: "dont", "its" and "thats" are
        // all listed in common_words.txt, so `is_common_word` rejects them before this guard
        // is reached. It is specifically the apostrophe forms the word list cannot protect.
        assert!(is_common_word("dont") && is_common_word("its") && is_common_word("thats"));

        // The guard is one-directional. A hint that IS a contraction can still be matched
        // from a token that dropped the apostrophe — a genuine mis-transcription, and not
        // an ordinary word, so nothing rejects it earlier.
        assert!(hint_matches("oreilly", "O'Reilly"));

        // End to end, against the exact hint list that caused the regression.
        let hints: Vec<String> = ["Tauri", "Rust", "notes", "claude", "Typr", "sanir"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            correct_vocabulary("Okay, let's move on to the branch", &hints),
            "Okay, let's move on to the branch"
        );
        // And the correction it exists to do still happens.
        assert_eq!(correct_vocabulary("the tori framework", &hints), "the Tauri framework");
        // A bare syllable of a hint is left alone (over-replacement regression guard).
        assert_eq!(correct_vocabulary("the ori framework", &hints), "the ori framework");
        // The observed failure: "tur on dark theme" kept its fragment for the AI pass
        // instead of becoming the confidently-wrong "Tauri on dark theme".
        assert_eq!(
            correct_vocabulary("tur on dark theme", &hints),
            "tur on dark theme"
        );
    }

    #[test]
    fn test_is_common_word() {
        assert!(is_common_word("the"));
        assert!(is_common_word("story"));
        assert!(is_common_word("boring"));
        assert!(!is_common_word("kubernetes"));
        assert!(!is_common_word("tauri"));
    }

    #[test]
    fn test_hint_matches() {
        // Genuine mis-hearings (non-common tokens) → correct.
        assert!(hint_matches("kubernetis", "Kubernetes"));
        assert!(hint_matches("sonet", "Sonnet"));
        assert!(hint_matches("tori", "Tauri"));
        // Common words are never overwritten, even if phonetically near a hint.
        assert!(!hint_matches("story", "Tauri"));
        assert!(!hint_matches("boring", "whoring"));
        // Too far by edit distance, even if Soundex-equal → no correction.
        assert!(!hint_matches("clod", "Claude"));
        // Same word (case-insensitively) is already correct → no match, no self-loop.
        assert!(!hint_matches("tauri", "Tauri"));
        // A bare syllable of the hint must never replace a whole word: skeleton-only
        // matches earn no extra edit budget ("ori"/"hori" -> "Tauri" was the reported
        // over-replacement).
        assert!(!hint_matches("ori", "Tauri"));
        assert!(!hint_matches("hori", "Tauri"));
        // A short fragment of ordinary speech must not snap to a longer hint even when
        // it keys and edits close: "tur" (for "turn") became "Tauri" mid-dictation.
        assert!(!hint_matches("tur", "Tauri"));
        assert!(hint_matches("tori", "Tauri")); // one character short still corrects
        // Still safely blocked:
        assert!(!hint_matches("rory", "Tauri"));   // edit distance 4 — too far
        assert!(!hint_matches("ruse", "Rust"));    // skeletons differ (R200 vs R230)
        assert!(!hint_matches("wording", "whoring")); // common English word, never overwritten
    }

    #[test]
    fn test_case_only_enforcement() {
        // The exact "Herdr renders properly <25% of the time" symptom: the engine emits the
        // right spelling with the wrong case, and the dictionary must snap it back.
        let hints = vec!["Herdr".to_string(), "claude".to_string()];
        assert_eq!(correct_vocabulary("the herdr agent docs", &hints), "the Herdr agent docs");
        assert_eq!(correct_vocabulary("the HERDR agent docs", &hints), "the Herdr agent docs");
        assert_eq!(correct_vocabulary("herdr", &hints), "Herdr");
        // All-lowercase hints never rewrite casing — "Claude" stays a proper noun.
        assert_eq!(correct_vocabulary("I love claude code", &hints), "I love claude code");
        // Exact match untouched.
        assert_eq!(correct_vocabulary("Herdr docs", &hints), "Herdr docs");
    }

    #[test]
    fn test_correct_vocabulary() {
        let hints = vec!["Kubernetes".to_string(), "Tauri".to_string()];

        // Corrects a mis-hearing, preserving trailing punctuation.
        assert_eq!(
            correct_vocabulary("we deployed kubernetis.", &hints),
            "we deployed Kubernetes."
        );
        // Preserves a leading capital at sentence start.
        assert_eq!(
            correct_vocabulary("Tori is a framework.", &hints),
            "Tauri is a framework."
        );
        // Leaves ordinary words alone (guard).
        assert_eq!(
            correct_vocabulary("what a boring story.", &hints),
            "what a boring story."
        );
        // Empty hints = passthrough.
        assert_eq!(correct_vocabulary("anything at all", &[]), "anything at all");
        // Already-correct word untouched.
        assert_eq!(
            correct_vocabulary("Kubernetes rocks", &hints),
            "Kubernetes rocks"
        );
    }
}
