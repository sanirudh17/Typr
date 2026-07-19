//! Assemble spoken email addresses into real ones.
//!
//! Whisper transcribes a dictated address as ordinary words — "sanirudh1017 at gmail dot com"
//! — and when a user spells the local part out letter by letter it arrives hyphenated
//! ("r-a-s-a-n-i-y-a"). Neither reaches the LLM as an address, so the AI pass has nothing to
//! preserve and the entity guard has nothing to check. This runs BEFORE both, so downstream
//! stages see a real address.
//!
//! The whole module is deliberately conservative: a false negative just leaves the user's
//! words alone, but a false positive silently mangles prose into an address. Every rule below
//! exists to make the second case hard.

/// Recognized top-level domains. An unknown TLD means we do not treat the phrase as an
/// address at all, which is what keeps "meet at reception dot something" out of scope.
const KNOWN_TLDS: [&str; 16] = [
    "com", "org", "net", "edu", "io", "dev", "co", "in", "uk", "me", "gov", "ai", "app", "mail",
    "info", "xyz",
];

/// Words that routinely precede "at" in ordinary speech. If the candidate local part is one of
/// these, the phrase is prose ("look at example dot com" is a site, not an address) and we
/// leave it alone. This is the single most important false-positive guard.
/// Pronouns are in the list because "reach me at …", "contact you at …" and "email them at …"
/// put an innocent pronoun directly before "at", where the local part would otherwise be read.
const LOCAL_STOPWORDS: [&str; 44] = [
    "look", "looks", "looking", "go", "going", "goes", "come", "comes", "meet", "meets",
    "meeting", "arrive", "arrives", "stay", "be", "is", "are", "was", "were", "get", "gets",
    "back", "here", "there", "home", "work", "it", "this", "that", "us", "available", "live",
    "start", "starts", "me", "you", "him", "her", "them", "anyone", "everyone", "someone",
    "myself", "yourself",
];

/// Lowercase a token and strip punctuation that belongs to the sentence around it.
///
/// Interior dots are kept (they are part of "gmail.com"), but an OUTER dot is sentence
/// punctuation and must go: a dictated address usually lands at the end of a sentence, so the
/// transcriber hands us "gmail.com." — and leaving that trailing dot on makes the final domain
/// label empty, which failed to parse as an address at all.
fn norm(token: &str) -> String {
    token
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '_' && c != '-' && c != '+')
        .trim_matches('.')
        .to_lowercase()
}

/// Trailing sentence punctuation on a token, so "gmail.com?" keeps its "?" after assembly.
fn trailing_punct(token: &str) -> &str {
    let end = token
        .rfind(|c: char| c.is_alphanumeric())
        .map(|i| i + token[i..].chars().next().map_or(1, |c| c.len_utf8()))
        .unwrap_or(0);
    &token[end..]
}

fn is_tld(s: &str) -> bool {
    KNOWN_TLDS.contains(&s)
}

/// A single label — one dot-free piece of a host or local part.
fn is_wordish(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '+')
}

/// A plausible local part. Unlike a single label this may contain interior dots, because the
/// transcriber often pre-joins them: "first dot last" can arrive as one "first.last" token.
fn is_local_part(s: &str) -> bool {
    !s.is_empty() && !s.contains("..") && s.split('.').all(is_wordish)
}

/// "r-a-s-a-n-i-y-a" or "r.a.s" -> "rasaniya" / "ras". Requires at least three single letters
/// so ordinary hyphenated words ("well-known", "co-op", "e-mail") never match.
fn join_spelled_letters(token: &str) -> Option<String> {
    let parts: Vec<&str> = token.split(['-', '.']).filter(|p| !p.is_empty()).collect();
    if parts.len() >= 3 && parts.iter().all(|p| p.len() == 1 && p.chars().all(|c| c.is_ascii_alphabetic())) {
        Some(parts.concat().to_lowercase())
    } else {
        None
    }
}

/// Parse the domain that follows "at", starting at token index `j`.
/// Returns the domain and how many tokens it consumed.
///
/// Accepts both "gmail.com" (already joined by the transcriber) and "gmail dot com", including
/// multi-label forms like "example dot co dot uk".
fn parse_domain(tokens: &[&str], j: usize) -> Option<(String, usize)> {
    let head = norm(tokens.get(j)?);
    if head.is_empty() {
        return None;
    }

    // Already joined: "gmail.com".
    if head.contains('.') {
        let labels: Vec<&str> = head.split('.').collect();
        if labels.len() >= 2 && labels.iter().all(|l| is_wordish(l)) && is_tld(labels[labels.len() - 1]) {
            return Some((head, 1));
        }
        return None;
    }

    if !is_wordish(&head) {
        return None;
    }

    // Spoken: "gmail dot com", "example dot co dot uk".
    let mut domain = head;
    let mut used = 1;
    let mut matched_tld = false;
    while norm(tokens.get(j + used).unwrap_or(&"")) == "dot" {
        let label = norm(tokens.get(j + used + 1).unwrap_or(&""));
        if !is_wordish(&label) {
            break;
        }
        domain.push('.');
        domain.push_str(&label);
        used += 2;
        matched_tld = is_tld(&label);
    }

    if matched_tld {
        Some((domain, used))
    } else {
        None
    }
}

/// Take the local part from the already-emitted tokens, popping what it consumes.
///
/// Letter-run joining happens ONLY here, immediately before a confirmed "at + domain" — doing
/// it globally would rewrite prose like "the grades were A B C" into "abc".
fn take_local(out: &mut Vec<String>) -> Option<String> {
    let last = norm(out.last()?);
    if last.is_empty() {
        return None;
    }

    // "r-a-s-a-n-i-y-a" arriving as one hyphenated token.
    if let Some(joined) = join_spelled_letters(&last) {
        out.pop();
        return Some(joined);
    }

    // "r a s a n i y a" arriving as separate single-letter tokens.
    let letter_run = out
        .iter()
        .rev()
        .take_while(|t| {
            let n = norm(t);
            n.len() == 1 && n.chars().all(|c| c.is_ascii_alphabetic())
        })
        .count();
    if letter_run >= 3 {
        let start = out.len() - letter_run;
        let joined: String = out.drain(start..).map(|t| norm(&t)).collect();
        return Some(joined);
    }

    if !is_local_part(&last) || LOCAL_STOPWORDS.contains(&last.as_str()) {
        return None;
    }
    out.pop();
    let mut local = last;

    // Dotted local parts: "first dot last at ...".
    while out.len() >= 2 && norm(&out[out.len() - 1]) == "dot" {
        let candidate = norm(&out[out.len() - 2]);
        if !is_wordish(&candidate) || LOCAL_STOPWORDS.contains(&candidate.as_str()) {
            break;
        }
        out.pop();
        out.pop();
        local = format!("{}.{}", candidate, local);
    }

    Some(local)
}

/// Rewrite spoken email addresses in `text` into real ones. Text with no "at + known-TLD
/// domain" pattern is returned unchanged.
pub fn assemble_emails(text: &str) -> String {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut i = 0;

    while i < tokens.len() {
        if norm(tokens[i]) == "at" && !out.is_empty() {
            if let Some((domain, used)) = parse_domain(&tokens, i + 1) {
                // Snapshot: take_local mutates `out`, and the domain may still be rejected.
                let snapshot = out.clone();
                if let Some(local) = take_local(&mut out) {
                    let tail = trailing_punct(tokens[i + used]);
                    out.push(format!("{}@{}{}", local, domain, tail));
                    i += used + 1;
                    continue;
                }
                out = snapshot;
            }
        }
        out.push(tokens[i].to_string());
        i += 1;
    }

    out.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spoken_address_is_assembled() {
        assert_eq!(
            assemble_emails("send this to sanirudh1017 at gmail dot com"),
            "send this to sanirudh1017@gmail.com"
        );
    }

    #[test]
    fn test_letter_by_letter_local_part() {
        // The exact shape Whisper produced when the user spelled the name out.
        assert_eq!(
            assemble_emails("forward this to r-a-s-a-n-i-y-a at gmail.com?"),
            "forward this to rasaniya@gmail.com?"
        );
        // And the same spelling arriving as separate tokens.
        assert_eq!(
            assemble_emails("mail r a s a n i y a at gmail dot com"),
            "mail rasaniya@gmail.com"
        );
    }

    /// Both of these are real transcripts from a hands-on run that did NOT assemble.
    #[test]
    fn test_sentence_final_period_does_not_block_assembly() {
        // "gmail.com." — the trailing dot made the last domain label empty.
        assert_eq!(
            assemble_emails("Please forward this to r-a-s-a-n-i-y-a at gmail.com."),
            "Please forward this to rasaniya@gmail.com."
        );
    }

    #[test]
    fn test_prejoined_dotted_local_part() {
        // The transcriber pre-joined "first dot last" into one token, which the label check
        // then rejected because it contained a dot.
        assert_eq!(
            assemble_emails("Ping first.last at example.org"),
            "Ping first.last@example.org"
        );
    }

    #[test]
    fn test_dotted_local_part() {
        assert_eq!(
            assemble_emails("ping first dot last at example dot org"),
            "ping first.last@example.org"
        );
    }

    #[test]
    fn test_multi_label_domain() {
        assert_eq!(
            assemble_emails("ping billing at example dot co dot uk"),
            "ping billing@example.co.uk"
        );
    }

    #[test]
    fn test_pronoun_before_at_is_prose() {
        // "reach me at <address>" — the pronoun is not the local part.
        assert_eq!(
            assemble_emails("reach me at support dot example dot com"),
            "reach me at support dot example dot com"
        );
    }

    #[test]
    fn test_prose_is_left_alone() {
        // "look"/"go" are stopwords: these are sites and sentences, not addresses.
        assert_eq!(assemble_emails("look at example dot com"), "look at example dot com");
        assert_eq!(
            assemble_emails("can you just please go to www.youtube.com"),
            "can you just please go to www.youtube.com"
        );
        // Unknown TLD: not an address.
        assert_eq!(assemble_emails("we met at reception dot something"), "we met at reception dot something");
        // No domain at all.
        assert_eq!(assemble_emails("let us meet at noon"), "let us meet at noon");
    }

    /// Exact transcripts from the hands-on run. With the sentence-final-period fix these now
    /// parse a valid domain, so the stopword list is the ONLY thing keeping them prose —
    /// worth pinning in the form the transcriber actually produces.
    #[test]
    fn test_prose_with_joined_domain_still_left_alone() {
        assert_eq!(assemble_emails("Look at example.com."), "Look at example.com.");
        assert_eq!(
            assemble_emails("Please reach me at support.example.com."),
            "Please reach me at support.example.com."
        );
    }

    #[test]
    fn test_existing_address_untouched() {
        assert_eq!(
            assemble_emails("send this to my sanirudh1017@gmail.com?"),
            "send this to my sanirudh1017@gmail.com?"
        );
    }

    #[test]
    fn test_hyphenated_words_are_not_letter_runs() {
        // "well-known" and "e-mail" must not be collapsed; neither is 3+ single letters.
        assert_eq!(
            assemble_emails("the well-known e-mail at example dot com"),
            "the well-known e-mail@example.com"
        );
    }
}
