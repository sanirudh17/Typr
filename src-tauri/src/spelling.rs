//! Explicit, offline letter dictation. No dictionary replacement or fuzzy guess.
fn token(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_ascii_alphabetic()).to_ascii_lowercase()
}
fn letter(words: &[&str], i: usize) -> Option<(char, usize)> {
    let w = token(words.get(i)?);
    if w == "x" && words.get(i + 1).is_some_and(|w| token(w) == "ray") { return Some(('X', 2)); }
    if w.len() == 1 { return Some((w.as_bytes()[0].to_ascii_uppercase() as char, 1)); }
    let c = match w.as_str() {
        "alpha" | "alfa" | "ay" => 'A', "bravo" | "bee" => 'B',
        "charlie" | "see" | "cee" => 'C', "delta" | "dee" => 'D',
        "echo" | "ee" => 'E', "foxtrot" | "eff" => 'F',
        "golf" | "gee" => 'G', "hotel" | "aitch" => 'H',
        "india" | "eye" => 'I', "juliet" | "juliett" | "jay" => 'J',
        "kilo" | "kay" => 'K', "lima" | "el" | "ell" => 'L',
        "mike" | "em" => 'M', "november" | "en" => 'N',
        "oscar" | "oh" => 'O', "papa" | "pee" => 'P',
        "quebec" | "cue" | "queue" => 'Q', "romeo" | "are" => 'R',
        "sierra" | "ess" => 'S', "tango" | "tee" | "tea" => 'T',
        "uniform" | "you" => 'U', "victor" | "vee" => 'V',
        "whiskey" | "whisky" => 'W', "xray" | "x-ray" | "ex" => 'X',
        "yankee" | "why" => 'Y', "zulu" | "zed" | "zee" => 'Z',
        "double" if words.get(i + 1).is_some_and(|w| matches!(token(w).as_str(), "u" | "you")) => return Some(('W', 2)),
        _ => return None,
    };
    Some((c, 1))
}
pub fn standalone_letter(text: &str) -> Option<String> {
    let word = text.trim().trim_end_matches(['.', '!', '?']);
    (word.len() == 1 && word.as_bytes()[0].is_ascii_uppercase()).then(|| word.to_string())
}
/// Match an explicit spelling span, or at least three separately recognized
/// ASCII letters. Bare English words never combine.
pub fn match_sequence(words: &[&str], start: usize) -> Option<(usize, String)> {
    let head = token(words.get(start)?);
    if head == "letter" {
        if start > 0 && !matches!(token(words[start - 1]).as_str(), "insert" | "type" | "press" | "key") { return None; }
        let raw = words.get(start + 1)?;
        let w = token(raw);
        // A pronoun in "the letter you wrote" is not the letter U.
        if w.len() != 1 && matches!(w.as_str(), "you" | "are" | "why" | "see" | "tea" | "oh") { return None; }
        let (c, n) = letter(words, start + 1)?;
        return Some((n + 1, c.to_string()));
    }
    if head != "spell" {
        let mut end = start;
        let mut result = String::new();
        while let Some(w) = words.get(end) {
            let w = w.trim_matches(|c: char| !c.is_ascii_alphabetic());
            if w.len() != 1 || !w.as_bytes()[0].is_ascii_alphabetic() || result.len() == 64 { break; }
            result.push(w.as_bytes()[0].to_ascii_uppercase() as char); end += 1;
        }
        // Repeated I/I/I or A/A/A can be a stutter, not an acronym.
        return (result.len() >= 3 && !result.chars().all(|c| Some(c) == result.chars().next()))
            .then_some((end - start, result));
    }
    let mut i = start + 1;
    if words.get(i).is_some_and(|w| token(w) == "out") { i += 1; }
    let payload = i;
    let mut result = String::new();
    while result.len() < 64 {
        let Some((c, n)) = letter(words, i) else { break; };
        result.push(c); i += n;
    }
    let ended = words.get(i).is_some_and(|w| matches!(token(w).as_str(), "end" | "stop" | "and"))
        && words.get(i + 1).is_some_and(|w| matches!(token(w).as_str(), "spell" | "spelling"));
    if result.is_empty() || (result.len() == 1 && !ended) { return None; }
    if ended { i += 2; }
    // Do not treat unbounded prose such as "spell out a new word" as a command.
    if i == payload { return None; }
    Some((i - start, result))
}
#[cfg(test)]
mod tests {
    use crate::commands::{apply_commands, contains_command};
    #[test]
    fn letters_and_nato_preserve_following_words_and_final_letter() {
        for (source, expected) in [
            ("Use spell G P U I end spelling framework", "Use GPUI framework"),
            ("spell golf papa uniform india end spelling", "GPUI"),
            ("Spell Golf Papa Uniform India and Spelling", "GPUI"),
            ("spell out gee pee you eye end spelling", "GPUI"),
            ("letter G", "G"), ("G P U I", "GPUI"), ("g p u i", "GPUI"), ("G. P. U. I.", "GPUI"),
            ("G.", "G"), ("spell double you eye en end spelling", "WIN"),
        ] { assert!(contains_command(source), "{source}"); assert_eq!(apply_commands(source), expected, "{source}"); }
    }
    #[test]
    fn prose_is_not_spelling_or_camel_case() {
        for source in ["quick overlay button", "coding agent built in Rust", "spell out a new word", "spell the name correctly", "the letter you wrote", "the letter I wrote", "I I I", "i i i", "GPU CPU"] {
            assert!(!contains_command(source), "{source}"); assert_eq!(apply_commands(source), source);
        }
    }
    #[test]
    fn explicit_spelling_keeps_layout_and_other_commands() {
        assert_eq!(apply_commands("spell G P U I end spelling new line coding agent built in Rust"), "GPUI\ncoding agent built in Rust");
        assert_eq!(apply_commands("spell G P U I end spelling\nKeep every final word."), "GPUI\nKeep every final word.");
    }
}
