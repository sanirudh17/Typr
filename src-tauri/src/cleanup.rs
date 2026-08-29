pub fn cleanup_text(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Normalize multiple spaces to single space
    let normalized: String = trimmed
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ");
    let chars: Vec<char> = normalized.chars().collect();

    // Capitalize first letter of each sentence
    let mut result = String::new();
    let mut capitalize_next = true;

    for i in 0..chars.len() {
        let ch = chars[i];
        if capitalize_next && ch.is_alphabetic() {
            // Leave email/URL tokens (e.g. a snippet-expanded address) verbatim so
            // we never turn sanirudh1017@... into Sanirudh1017@...
            if token_is_verbatim(&chars, i) {
                result.push(ch);
            } else {
                result.extend(ch.to_uppercase());
            }
            capitalize_next = false;
        } else {
            result.push(ch);
            if ch == '.' || ch == '!' || ch == '?' {
                // Only trigger sentence capitalization if the punctuation is followed
                // by whitespace or is at the end. This prevents capitalized letters
                // inside emails (gmail.com), numbers (3.5), or file extensions (.exe).
                if i + 1 >= chars.len() || chars[i + 1].is_whitespace() {
                    capitalize_next = true;
                }
            }
        }
    }

    // Handle the ending relative to an email/URL, where a trailing sentence dot is
    // wrong (it breaks a pasted address/link). Strip one the engine glued on, and
    // never add one; otherwise ensure the sentence ends in terminal punctuation.
    if last_token_is_verbatim(&result) {
        while matches!(result.chars().last(), Some('.') | Some('!') | Some('?')) {
            result.pop();
        }
    } else if let Some(last) = result.chars().last() {
        if !matches!(last, '.' | '!' | '?') {
            result.push('.');
        }
    }

    result
}

/// Whether the whitespace-delimited token surrounding index `i` is an email or
/// URL and should be preserved exactly (no casing/punctuation changes).
fn token_is_verbatim(chars: &[char], i: usize) -> bool {
    let mut start = i;
    while start > 0 && !chars[start - 1].is_whitespace() {
        start -= 1;
    }
    let mut end = i;
    while end < chars.len() && !chars[end].is_whitespace() {
        end += 1;
    }
    let token = &chars[start..end];
    token.contains(&'@') || token.windows(3).any(|w| w == [':', '/', '/'])
}

/// Strip filler words and stutters deterministically. Used as a safety net
/// when AI cleanup is on but the LLM was skipped (voice-command bypass) or
/// failed — especially for Developer/Terminal where "um git status" must not
/// paste the "um". Keeps code identifiers, emails, URLs, and file paths
/// verbatim, and collapses consecutive repeated words.
pub fn strip_filler_words(text: &str) -> String {
    if text.trim().is_empty() {
        return text.to_string();
    }
    const SINGLE_FILLER: &[&str] = &["um", "uh", "er", "ah", "hmm", "erm", "uhm"];
    let raw_tokens: Vec<&str> = text.split_whitespace().collect();
    let mut kept: Vec<String> = Vec::new();
    let mut i = 0;
    let mut prev_kept_lower: Option<String> = None;

    while i < raw_tokens.len() {
        let tok = raw_tokens[i];
        let lower = tok.to_lowercase();
        let trimmed = lower.trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | ')' | '[' | ']' | '<' | '>' | ',' | ';' | ':' | '!' | '?' | '`' | '.'));
        let is_entity = {
            let t = tok.trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | ')' | '[' | ']' | '<' | '>' | ',' | ';' | ':' | '!' | '?' | '`'));
            let tl = t.trim_end_matches(['.', '/']);
            tl.contains('@') || tl.contains("://") || (tl.contains('/') && tl.contains('.'))
        };
        if is_entity {
            let norm = trimmed.to_string();
            let is_repeat = prev_kept_lower.as_deref() == Some(&norm) && !norm.is_empty();
            if !is_repeat {
                kept.push(tok.to_string());
                prev_kept_lower = Some(norm);
            }
            i += 1;
            continue;
        }
        // Two-word filler phrases: "you know", "i mean" (case-insensitive, punctuation-tolerant).
        if i + 1 < raw_tokens.len() {
            let next = raw_tokens[i + 1];
            let next_trimmed = next.to_lowercase().trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | ')' | '[' | ']' | '<' | '>' | ',' | ';' | ':' | '!' | '?' | '`' | '.')).to_string();
            let is_you_know = trimmed == "you" && next_trimmed == "know";
            let is_i_mean = trimmed == "i" && next_trimmed == "mean";
            if is_you_know || is_i_mean {
                i += 2;
                continue;
            }
        }
        if SINGLE_FILLER.contains(&trimmed) {
            i += 1;
            continue;
        }
        let norm = trimmed.to_string();
        let is_repeat = prev_kept_lower.as_deref() == Some(&norm) && !norm.is_empty();
        if is_repeat {
            i += 1;
            continue;
        }
        kept.push(tok.to_string());
        prev_kept_lower = Some(norm);
        i += 1;
    }
    kept.join(" ")
}

/// Whether the final token of `s` is an email or URL.
fn last_token_is_verbatim(s: &str) -> bool {
    match s.split_whitespace().last() {
        Some(tok) => tok.contains('@') || tok.contains("://"),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trim_whitespace() {
        assert_eq!(cleanup_text("  hello world  "), "Hello world.");
    }

    #[test]
    fn test_normalize_spaces() {
        assert_eq!(cleanup_text("hello    world"), "Hello world.");
    }

    #[test]
    fn test_capitalize_first_letter() {
        assert_eq!(cleanup_text("hello world"), "Hello world.");
    }

    #[test]
    fn test_capitalize_after_period() {
        assert_eq!(cleanup_text("hello. world"), "Hello. World.");
    }

    #[test]
    fn test_capitalize_after_question_mark() {
        assert_eq!(cleanup_text("hello? world"), "Hello? World.");
    }

    #[test]
    fn test_capitalize_after_exclamation() {
        assert_eq!(cleanup_text("hello! world"), "Hello! World.");
    }

    #[test]
    fn test_ensure_ending_punctuation() {
        assert_eq!(cleanup_text("hello world"), "Hello world.");
    }

    #[test]
    fn test_preserve_existing_ending_punctuation() {
        assert_eq!(cleanup_text("hello world."), "Hello world.");
        assert_eq!(cleanup_text("hello world!"), "Hello world!");
        assert_eq!(cleanup_text("hello world?"), "Hello world?");
    }

    #[test]
    fn test_empty_string() {
        assert_eq!(cleanup_text(""), "");
        assert_eq!(cleanup_text("   "), "");
    }

    #[test]
    fn test_already_clean() {
        assert_eq!(cleanup_text("Hello world."), "Hello world.");
    }

    #[test]
    fn test_email_at_sentence_start_not_capitalized() {
        // A snippet that expands to an email at the start of the utterance must
        // stay verbatim — no leading capital, no trailing period.
        assert_eq!(
            cleanup_text("sanirudh1017@gmail.com"),
            "sanirudh1017@gmail.com"
        );
    }

    #[test]
    fn test_url_at_sentence_start_not_capitalized() {
        assert_eq!(
            cleanup_text("https://github.com/sanirudh17"),
            "https://github.com/sanirudh17"
        );
    }

    #[test]
    fn test_email_mid_sentence_keeps_sentence_formatting() {
        // Normal first word still capitalizes; email token stays intact; sentence
        // still gets its period because it does not end on the email.
        assert_eq!(
            cleanup_text("my email is sanirudh1017@gmail.com and it works"),
            "My email is sanirudh1017@gmail.com and it works."
        );
    }

    #[test]
    fn test_no_trailing_period_when_ending_on_email() {
        assert_eq!(
            cleanup_text("reach me at sanirudh1017@gmail.com"),
            "Reach me at sanirudh1017@gmail.com"
        );
    }

    #[test]
    fn test_strips_engine_period_glued_to_trailing_email() {
        // The engine tends to append a period; strip it so the address stays clean.
        assert_eq!(
            cleanup_text("reach me at sanirudh1017@gmail.com."),
            "Reach me at sanirudh1017@gmail.com"
        );
        assert_eq!(
            cleanup_text("sanirudh1017@gmail.com."),
            "sanirudh1017@gmail.com"
        );
    }

    #[test]
    fn test_do_not_capitalize_emails_or_numbers() {
        assert_eq!(
            cleanup_text("please contact me@domain.com for info."),
            "Please contact me@domain.com for info."
        );
        assert_eq!(
            cleanup_text("the version is 1.2.3 and is stable."),
            "The version is 1.2.3 and is stable."
        );
        assert_eq!(
            cleanup_text("file saved as a .exe file."),
            "File saved as a .exe file."
        );
    }
}
