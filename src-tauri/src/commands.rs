//! Deterministic, always-on voice commands: casing, layout, and symbols.
//! Runs as the final pipeline pass (after cleanup and any AI pass). Offline, never errors.
//!
//! Operates on whitespace-separated words. Because it runs on already-cleaned, single-spaced
//! text, non-command runs are rejoined with single spaces (acceptable post-cleanup).

/// Apply deterministic voice commands to already-cleaned text.
pub fn apply_commands(text: &str) -> String {
    if text.trim().is_empty() {
        return text.to_string();
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut out = OutBuilder::new();
    let mut i = 0;
    while i < words.len() {
        match match_trigger(&words, i) {
            Some(Trigger { len, kind }) => match kind {
                Kind::Case(style) => {
                    let end = capture_end(&words, i + len);
                    let captured = &words[i + len..end];
                    let ident = join_case(captured, style);
                    if ident.is_empty() {
                        // No words captured: emit the trigger words literally (no-op).
                        for w in &words[i..i + len] {
                            out.push_word(w);
                        }
                        i += len;
                    } else {
                        out.push_word(&ident);
                        i = end;
                    }
                }
                Kind::NewLine => {
                    out.push_newline(1);
                    i += len;
                }
                Kind::NewParagraph => {
                    out.push_newline(2);
                    i += len;
                }
                Kind::Bullet => {
                    let end = capture_end(&words, i + len);
                    let captured = &words[i + len..end];
                    if captured.is_empty() {
                        for w in &words[i..i + len] {
                            out.push_word(w);
                        }
                        i += len;
                    } else {
                        out.push_bullet(&captured.join(" "));
                        i = end;
                    }
                }
                Kind::OpenSym(s) => {
                    out.push_open_symbol(s);
                    i += len;
                }
                Kind::CloseSym(s) => {
                    out.push_close_symbol(s);
                    i += len;
                }
                Kind::InfixSym(s) => {
                    out.push_infix_symbol(s);
                    i += len;
                }
            },
            None => {
                out.push_word(words[i]);
                i += 1;
            }
        }
    }
    out.finish()
}

#[derive(Clone, Copy)]
enum Casing {
    Camel,
    Pascal,
    Snake,
    Constant,
    Kebab,
}

struct Trigger {
    len: usize,
    kind: Kind,
}

enum Kind {
    Case(Casing),
    NewLine,
    NewParagraph,
    Bullet,
    OpenSym(&'static str),
    CloseSym(&'static str),
    InfixSym(&'static str),
}

/// Lowercase a word and keep only alphanumerics — used for trigger matching and casing parts,
/// so a cleanup-capitalized "ID." normalizes to "id".
fn norm(w: &str) -> String {
    w.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Uppercase the first character; the rest is assumed already normalized (lowercase).
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Join captured words into a single identifier per the casing style. Empty/punctuation-only
/// words are dropped; returns "" when nothing usable remains.
fn join_case(words: &[&str], style: Casing) -> String {
    let parts: Vec<String> = words.iter().map(|w| norm(w)).filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return String::new();
    }
    match style {
        Casing::Camel => parts
            .iter()
            .enumerate()
            .map(|(idx, p)| if idx == 0 { p.clone() } else { capitalize(p) })
            .collect(),
        Casing::Pascal => parts.iter().map(|p| capitalize(p)).collect(),
        Casing::Snake => parts.join("_"),
        Casing::Constant => parts.iter().map(|p| p.to_uppercase()).collect::<Vec<_>>().join("_"),
        Casing::Kebab => parts.join("-"),
    }
}

/// Index of the first word (at/after `start`) that begins a new command trigger, else `words.len()`.
fn capture_end(words: &[&str], start: usize) -> usize {
    let mut j = start;
    while j < words.len() {
        if match_trigger(words, j).is_some() {
            break;
        }
        j += 1;
    }
    j
}

/// Match a command trigger phrase starting at word index `i` (longest phrase first).
fn match_trigger(words: &[&str], i: usize) -> Option<Trigger> {
    let w = |k: usize| words.get(i + k).map(|s| norm(s));
    let (w0, w1, w2) = (w(0), w(1), w(2));

    // 3-word triggers (must be tried before "snake case").
    if let (Some(a), Some(b), Some(c)) = (&w0, &w1, &w2) {
        match (a.as_str(), b.as_str(), c.as_str()) {
            ("screaming", "snake", "case") | ("upper", "snake", "case") => {
                return Some(Trigger { len: 3, kind: Kind::Case(Casing::Constant) });
            }
            _ => {}
        }
    }

    // 2-word triggers.
    if let (Some(a), Some(b)) = (&w0, &w1) {
        match (a.as_str(), b.as_str()) {
            ("camel", "case") => return Some(Trigger { len: 2, kind: Kind::Case(Casing::Camel) }),
            ("pascal", "case") => return Some(Trigger { len: 2, kind: Kind::Case(Casing::Pascal) }),
            ("snake", "case") => return Some(Trigger { len: 2, kind: Kind::Case(Casing::Snake) }),
            ("constant", "case") => return Some(Trigger { len: 2, kind: Kind::Case(Casing::Constant) }),
            ("kebab", "case") => return Some(Trigger { len: 2, kind: Kind::Case(Casing::Kebab) }),
            ("new", "line") => return Some(Trigger { len: 2, kind: Kind::NewLine }),
            ("new", "paragraph") => return Some(Trigger { len: 2, kind: Kind::NewParagraph }),
            ("new", "bullet") => return Some(Trigger { len: 2, kind: Kind::Bullet }),
            ("open", "paren") => return Some(Trigger { len: 2, kind: Kind::OpenSym("(") }),
            ("close", "paren") => return Some(Trigger { len: 2, kind: Kind::CloseSym(")") }),
            ("open", "brace") => return Some(Trigger { len: 2, kind: Kind::OpenSym("{") }),
            ("close", "brace") => return Some(Trigger { len: 2, kind: Kind::CloseSym("}") }),
            ("open", "bracket") => return Some(Trigger { len: 2, kind: Kind::OpenSym("[") }),
            ("close", "bracket") => return Some(Trigger { len: 2, kind: Kind::CloseSym("]") }),
            ("open", "angle") => return Some(Trigger { len: 2, kind: Kind::OpenSym("<") }),
            ("close", "angle") => return Some(Trigger { len: 2, kind: Kind::CloseSym(">") }),
            _ => {}
        }
    }

    // 1-word triggers.
    if let Some(a) = &w0 {
        match a.as_str() {
            "semicolon" => return Some(Trigger { len: 1, kind: Kind::CloseSym(";") }),
            "hyphen" => return Some(Trigger { len: 1, kind: Kind::InfixSym("-") }),
            _ => {}
        }
    }

    None
}

/// Builds the output string while controlling spacing around inserted symbols and newlines.
struct OutBuilder {
    buf: String,
    /// Whether the next textual token should be preceded by a space.
    need_space: bool,
}

impl OutBuilder {
    fn new() -> Self {
        Self { buf: String::new(), need_space: false }
    }

    fn sep(&mut self) {
        if self.need_space {
            self.buf.push(' ');
        }
    }

    /// A normal word (or an already-built identifier): space-separated from the previous token.
    fn push_word(&mut self, w: &str) {
        self.sep();
        self.buf.push_str(w);
        self.need_space = true;
    }

    /// An opening symbol like `(` `{` `[` `<`: spaced from the previous token, but the NEXT
    /// token attaches with no space (`(x`).
    fn push_open_symbol(&mut self, s: &str) {
        self.sep();
        self.buf.push_str(s);
        self.need_space = false;
    }

    /// A closing/terminating symbol like `)` `}` `]` `>` `;`: attaches to the previous token
    /// with no leading space (`x;`); the next token is spaced normally.
    fn push_close_symbol(&mut self, s: &str) {
        self.buf.push_str(s);
        self.need_space = true;
    }

    /// An infix symbol like `-` (from "hyphen"): attaches on both sides with no space
    /// (`well-known`, `--verbose`).
    fn push_infix_symbol(&mut self, s: &str) {
        self.buf.push_str(s);
        self.need_space = false;
    }

    /// Insert `n` line breaks; the next token starts the line with no leading space.
    fn push_newline(&mut self, n: usize) {
        for _ in 0..n {
            self.buf.push('\n');
        }
        self.need_space = false;
    }

    /// Start a `- ` bullet on its own line with the given (plain) text.
    fn push_bullet(&mut self, text: &str) {
        if !self.buf.is_empty() && !self.buf.ends_with('\n') {
            self.buf.push('\n');
        }
        self.buf.push_str("- ");
        self.buf.push_str(text);
        self.need_space = true;
    }

    fn finish(self) -> String {
        self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Task 1: passthrough ---

    #[test]
    fn test_empty_passthrough() {
        assert_eq!(apply_commands(""), "");
        assert_eq!(apply_commands("   "), "   ");
    }

    #[test]
    fn test_plain_text_unchanged() {
        assert_eq!(apply_commands("hello world"), "hello world");
        assert_eq!(apply_commands("The quick brown fox."), "The quick brown fox.");
    }

    // --- Task 2: casing ---

    #[test]
    fn test_camel_case() {
        assert_eq!(apply_commands("camel case get user by id"), "getUserById");
    }

    #[test]
    fn test_pascal_case() {
        assert_eq!(apply_commands("pascal case user service"), "UserService");
    }

    #[test]
    fn test_snake_case() {
        assert_eq!(apply_commands("snake case max retry count"), "max_retry_count");
    }

    #[test]
    fn test_constant_case_and_aliases() {
        assert_eq!(apply_commands("constant case max buffer size"), "MAX_BUFFER_SIZE");
        assert_eq!(apply_commands("screaming snake case max buffer size"), "MAX_BUFFER_SIZE");
        assert_eq!(apply_commands("upper snake case max buffer size"), "MAX_BUFFER_SIZE");
    }

    #[test]
    fn test_kebab_case() {
        assert_eq!(apply_commands("kebab case main nav bar"), "main-nav-bar");
    }

    #[test]
    fn test_casing_with_numbers() {
        assert_eq!(apply_commands("camel case get user 2"), "getUser2");
        assert_eq!(apply_commands("snake case max 2 count"), "max_2_count");
    }

    #[test]
    fn test_casing_tolerates_capitalized_and_punctuated_input() {
        // Simulates the cleanup pass having capitalized the trigger and appended a period.
        assert_eq!(apply_commands("Camel case get user by id."), "getUserById");
    }

    #[test]
    fn test_casing_stops_at_next_command() {
        // A second casing command bounds the first one's capture.
        assert_eq!(
            apply_commands("camel case get user snake case max count"),
            "getUser max_count"
        );
    }

    #[test]
    fn test_casing_trigger_with_no_words_is_literal() {
        // No words to transform -> emit the trigger literally rather than an empty identifier.
        assert_eq!(apply_commands("camel case"), "camel case");
    }

    #[test]
    fn test_casing_capture_is_greedy_to_end() {
        // Per spec: a casing command captures until the next command or end of utterance.
        // Trailing prose with no bounding command is absorbed into the identifier (documented).
        assert_eq!(
            apply_commands("camel case get user by id thanks"),
            "getUserByIdThanks"
        );
    }

    // --- Task 3: layout ---

    #[test]
    fn test_new_line() {
        assert_eq!(apply_commands("first new line second"), "first\nsecond");
    }

    #[test]
    fn test_new_paragraph() {
        assert_eq!(apply_commands("first new paragraph second"), "first\n\nsecond");
    }

    #[test]
    fn test_new_bullet() {
        assert_eq!(apply_commands("new bullet buy some milk"), "- buy some milk");
    }

    #[test]
    fn test_bullet_after_text_starts_new_line() {
        assert_eq!(apply_commands("todo new bullet buy milk"), "todo\n- buy milk");
    }

    #[test]
    fn test_bullet_stops_at_next_command() {
        assert_eq!(
            apply_commands("new bullet buy milk new line done"),
            "- buy milk\ndone"
        );
    }

    #[test]
    fn test_layout_combined_with_casing() {
        assert_eq!(
            apply_commands("camel case get user by id new line done"),
            "getUserById\ndone"
        );
    }

    // --- Task 4: symbols + spacing ---

    #[test]
    fn test_semicolon_attaches() {
        assert_eq!(
            apply_commands("camel case get user by id semicolon"),
            "getUserById;"
        );
    }

    #[test]
    fn test_parens_spacing() {
        // Opener spaced from previous token; next token attaches; closer attaches.
        assert_eq!(apply_commands("call open paren x close paren"), "call (x)");
    }

    #[test]
    fn test_all_bracket_families() {
        assert_eq!(apply_commands("open brace close brace"), "{}");
        assert_eq!(apply_commands("open bracket close bracket"), "[]");
        assert_eq!(apply_commands("open angle close angle"), "<>");
    }

    #[test]
    fn test_symbol_then_word_spacing() {
        assert_eq!(apply_commands("semicolon next"), "; next");
    }

    #[test]
    fn test_hyphen_infix() {
        assert_eq!(apply_commands("well hyphen known"), "well-known");
        assert_eq!(apply_commands("hyphen hyphen verbose"), "--verbose");
    }

    #[test]
    fn test_mixed_utterance() {
        assert_eq!(
            apply_commands("camel case get user by id semicolon new line"),
            "getUserById;\n"
        );
    }

    // --- Task 5: false-positive guards + regression lock ---

    #[test]
    fn test_prose_without_triggers_unchanged() {
        assert_eq!(apply_commands("the ratio was two to one"), "the ratio was two to one");
        assert_eq!(apply_commands("she opened the box"), "she opened the box");
        assert_eq!(apply_commands("a camel walked by"), "a camel walked by");
        assert_eq!(apply_commands("please close the door"), "please close the door");
    }

    #[test]
    fn test_partial_trigger_words_are_literal() {
        // "case" without a style word, "open"/"new" without their pair -> untouched.
        assert_eq!(apply_commands("in this case we proceed"), "in this case we proceed");
        assert_eq!(apply_commands("open the file"), "open the file");
        assert_eq!(apply_commands("a new idea"), "a new idea");
    }

    #[test]
    fn test_documented_layout_false_positive() {
        // Documented, accepted risk: the exact phrase "new line" fires anywhere (watch-item).
        assert_eq!(apply_commands("a new line of credit"), "a\nof credit");
    }
}
