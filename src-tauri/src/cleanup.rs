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
    // Line-by-line so AI-produced bullets and paragraph breaks survive this pass — a
    // whole-text split_whitespace() would flatten every newline to a single space and
    // destroy the layout the AI pass just produced.
    if text.contains('\n') {
        return text
            .split('\n')
            .map(strip_filler_line)
            .collect::<Vec<_>>()
            .join("\n");
    }
    strip_filler_line(text)
}

fn strip_filler_line(text: &str) -> String {
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
            let followed_by_what = i + 2 < raw_tokens.len() && {
                let third = raw_tokens[i + 2].to_lowercase();
                let t3 = third.trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | ')' | '[' | ']' | '<' | '>' | ',' | ';' | ':' | '!' | '?' | '`' | '.'));
                t3 == "what"
            };
            let preceded_by_verb = matches!(
                prev_kept_lower.as_deref(),
                Some("do") | Some("did") | Some("does") | Some("dont") | Some("don't") | Some("let") | Some("if") | Some("to") | Some("as") | Some("would")
            );
            let is_you_know = trimmed == "you" && next_trimmed == "know" && !followed_by_what && !preceded_by_verb;
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

/// Remove consecutive repeated words and short repeated phrases deterministically.
/// Always applied (even when AI is off) to fix Whisper/Parakeet stutters and
/// chunk-join artefacts like "hello hello" or "fix the login fix the login".
/// Keeps entities verbatim; checks 1-, 2-, and 3-word repeats with a small
/// window so "the the" is collapsed but "the cat sat on the mat" is not.
/// Preserves legitimate grammatical repetition like "had had" and "that that".
/// Pure; never touches punctuation beyond token boundaries.
pub fn deduplicate_text(text: &str) -> String {
    if text.trim().is_empty() {
        return text.to_string();
    }
    // Line-by-line: newline structure from the AI pass must survive, and a stutter never
    // spans a line break anyway.
    if text.contains('\n') {
        return text
            .split('\n')
            .map(deduplicate_line)
            .collect::<Vec<_>>()
            .join("\n");
    }
    deduplicate_line(text)
}

fn deduplicate_line(text: &str) -> String {
    if text.trim().is_empty() {
        return text.to_string();
    }
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.len() <= 1 {
        return text.to_string();
    }
    let lowered: Vec<String> = tokens
        .iter()
        .map(|t| {
            t.to_lowercase()
                .trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | ')' | '[' | ']' | '<' | '>' | ',' | ';' | ':' | '!' | '?' | '`' | '.'))
                .to_string()
        })
        .collect();
    let is_entity = |tok: &str| {
        let t = tok.trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | ')' | '[' | ']' | '<' | '>' | ',' | ';' | ':' | '!' | '?' | '`'));
        let tl = t.trim_end_matches(['.', '/']);
        tl.contains('@') || tl.contains("://") || (tl.contains('/') && tl.contains('.'))
    };
    let mut kept: Vec<String> = Vec::new();
    let mut kept_lower: Vec<String> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        // Check for 3-, 2-, then 1-word repeats: if the next 3 words equal the last 3 kept, skip them.
        let mut skipped = false;
        for n in (1..=3).rev() {
            if i + n <= tokens.len() && kept.len() >= n {
                // Don't dedupe if any token in the window is an entity (email/path) — those
                // must survive verbatim, and repeating an email is not a stutter.
                let window_tokens = &tokens[i..i + n];
                let tail_tokens = &kept[kept.len() - n..];
                if window_tokens.iter().any(|t| is_entity(t)) || tail_tokens.iter().any(|t| is_entity(t)) {
                    continue;
                }
                let window_lower = &lowered[i..i + n];
                let tail_lower = &kept_lower[kept_lower.len() - n..];
                // Legitimate English grammar permits "had had" and "that that"; do not collapse them.
                if n == 1 && (window_lower[0] == "had" || window_lower[0] == "that") {
                    continue;
                }
                if window_lower == tail_lower {
                    // Skip the repeated window; do not update kept, so consecutive repeats of
                    // the same window (e.g., "hello hello hello") collapse to one.
                    i += n;
                    skipped = true;
                    break;
                }
            }
        }
        if skipped {
            continue;
        }
        kept.push(tokens[i].to_string());
        kept_lower.push(lowered[i].clone());
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

/// Drop a trailing close/stop marker from the finished transcript. When the user whispers
/// an end-of-session cue ("stop", "end session") right as they release the hotkey, that
/// audio still reaches the engine — it is a signal to the recorder, not content, and must
/// never be pasted into the target window. Markers are matched on the very end of the text
/// (with optional trailing sentence punctuation); anything else is left untouched.
pub fn strip_trailing_stop_phrase(text: &str) -> String {
    const SESSION_MARKERS: &[&str] = &[
        "stop recording",
        "stop dictation",
        "stop listening",
        "end recording",
        "end a session",
        "end session",
        "close a session",
        "close session",
    ];
    // Ordinary English phrases that double as a whispered cue. They are stripped only from
    // a very short utterance ("stop", "okay stop") — never from dictated content that merely
    // happens to end with them ("...We should stop." must keep its "stop"; silently deleting
    // content is worse than pasting one stray cue word).
    const AMBIGUOUS_MARKERS: &[&str] = &["that's all", "that's it", "stop"];
    const MAX_WORDS_FOR_AMBIGUOUS: usize = 2;
    let trimmed = text.trim_end();
    if trimmed.is_empty() {
        return text.to_string();
    }
    // ASCII lowercasing preserves byte offsets, so cuts on `lower` index `trimmed` safely.
    let lower = trimmed.to_ascii_lowercase();
    let body = lower.trim_end_matches(|c: char| matches!(c, '.' | '!' | '?' | ',' | ';' | ':'));
    let try_strip = |markers: &[&str]| -> Option<String> {
        for marker in markers {
            if let Some(rest) = body.strip_suffix(marker) {
                if rest.is_empty() || rest.ends_with(' ') || rest.ends_with('\n') {
                    return Some(body[..rest.len()].trim_end().to_string());
                }
            }
        }
        None
    };
    if let Some(stripped) = try_strip(SESSION_MARKERS) {
        return stripped;
    }
    if body.split_whitespace().count() <= MAX_WORDS_FOR_AMBIGUOUS {
        if let Some(stripped) = try_strip(AMBIGUOUS_MARKERS) {
            return stripped;
        }
    }
    text.to_string()
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

    /// Without-AI verification for "no text left out": a developer-style
    /// instruction carries no filler and no stutters, so the deterministic chain
    /// (filler-strip → dedupe → prose cleanup) must preserve every content word —
    /// only casing and the trailing period may change.
    #[test]
    fn test_developer_instruction_loses_no_words_without_ai() {
        let said = "check the agents folder and use the design skills from the docs folder";
        let stripped = strip_filler_words(said);
        for w in ["check", "agents", "folder", "use", "design", "skills", "docs"] {
            assert!(stripped.contains(w), "filler-strip lost: {w}");
        }
        let deduped = deduplicate_text(&stripped);
        for w in ["check", "agents", "folder", "use", "design", "skills", "docs"] {
            assert!(deduped.contains(w), "dedupe lost: {w}");
        }
        // The repeated-but-not-consecutive "folder" is content, not a stutter.
        assert_eq!(deduped.matches("folder").count(), 2);
        let cleaned = cleanup_text(&deduped).to_lowercase();
        for w in ["check", "agents", "folder", "design", "skills", "docs"] {
            assert!(cleaned.contains(w), "cleanup lost: {w}");
        }
    }

    /// Control: only actual stutter goes — "the the" collapses here, "um" goes
    /// in the filler pass, content words survive both.
    #[test]
    fn test_stutter_collapses_but_content_survives_without_ai() {
        assert_eq!(deduplicate_text("check the the agents folder"), "check the agents folder");
        assert_eq!(strip_filler_words("check the agents folder um"), "check the agents folder");
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

    #[test]
    fn test_had_had_and_that_that_preserved() {
        assert_eq!(deduplicate_text("I had had enough"), "I had had enough");
        assert_eq!(deduplicate_text("he said that that was fine"), "he said that that was fine");
    }

    #[test]
    fn test_you_know_what_preserved() {
        assert_eq!(
            strip_filler_words("you know what we should do"),
            "you know what we should do"
        );
        assert_eq!(
            strip_filler_words("do you know what happened"),
            "do you know what happened"
        );
        // Ordinary standalone filler "you know" still stripped:
        assert_eq!(
            strip_filler_words("it was you know great"),
            "it was great"
        );
    }

    /// AI-produced bullets and paragraphs must survive the deterministic passes — the whole
    /// point of "make it a list" / Auto-mode formatting is lost if newlines are flattened.
    #[test]
    fn test_filler_and_dedup_preserve_newlines() {
        let bullets = "- um buy milk\n- buy eggs";
        assert_eq!(strip_filler_words(bullets), "- buy milk\n- buy eggs");
        assert_eq!(
            deduplicate_text("hello hello\nworld world"),
            "hello\nworld"
        );
        // Dedup never crosses a line boundary.
        assert_eq!(deduplicate_text("hello\nhello"), "hello\nhello");
        // Single-line behavior is unchanged.
        assert_eq!(strip_filler_words("um buy milk"), "buy milk");
    }

    #[test]
    fn test_strip_trailing_stop_phrase() {
        // The whispered close cue never reaches the target window (short utterance).
        assert_eq!(strip_trailing_stop_phrase("hello stop."), "hello");
        assert_eq!(strip_trailing_stop_phrase("okay stop"), "okay");
        assert_eq!(strip_trailing_stop_phrase("stop"), "");
        assert_eq!(strip_trailing_stop_phrase("fix the bug end session"), "fix the bug");
        // The bug report's own wording: "when I whisper to close a session".
        assert_eq!(strip_trailing_stop_phrase("okay close a session"), "okay");
        assert_eq!(strip_trailing_stop_phrase("done, end a session"), "done,");
        // Data-loss guard: real dictated content that merely ends with an ambiguous cue word
        // must keep it. (Manual test run: "This is a test running plus 12 decimals. We should
        // stop." was pasted as "...We should" — the strip ate the content word "stop".)
        assert_eq!(
            strip_trailing_stop_phrase("This is a test running plus 12 decimals. We should stop."),
            "This is a test running plus 12 decimals. We should stop."
        );
        assert_eq!(
            strip_trailing_stop_phrase("close the modal stop"),
            "close the modal stop"
        );
        // Markers in the middle or followed by content are ordinary words.
        assert_eq!(
            strip_trailing_stop_phrase("press the stop button"),
            "press the stop button"
        );
        assert_eq!(
            strip_trailing_stop_phrase("we will stop soon"),
            "we will stop soon"
        );
        // Nothing to strip.
        assert_eq!(
            strip_trailing_stop_phrase("ship the release"),
            "ship the release"
        );
    }
}
