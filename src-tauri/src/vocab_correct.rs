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
fn is_common_word(lower: &str) -> bool {
    common_words().contains(lower)
}

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
    if soundex(token_lower) != soundex(&hint_lower) {
        return false; // phonetic gate
    }
    strsim::levenshtein(token_lower, &hint_lower) <= distance_threshold(hint_lower.len())
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
/// A no-op when `hints` is empty.
pub fn correct_vocabulary(text: &str, hints: &[String]) -> String {
    if hints.is_empty() {
        return text.to_string();
    }

    text.split(' ')
        .map(|token| {
            let (lead, core, trail) = split_affixes(token);
            if core.is_empty() {
                return token.to_string();
            }
            let core_lower = core.to_lowercase();
            for hint in hints {
                if hint_matches(&core_lower, hint) {
                    return format!("{}{}{}", lead, apply_casing(core, hint), trail);
                }
            }
            token.to_string()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

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
