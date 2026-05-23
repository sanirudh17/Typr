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

    // Capitalize first letter of each sentence
    let mut chars = normalized.chars().peekable();
    let mut result = String::new();
    let mut capitalize_next = true;

    while let Some(ch) = chars.next() {
        if capitalize_next && ch.is_alphabetic() {
            result.extend(ch.to_uppercase());
            capitalize_next = false;
        } else {
            result.push(ch);
            if ch == '.' || ch == '!' || ch == '?' {
                // Peek at the next character. We only trigger sentence capitalization
                // if the punctuation is followed by a space/whitespace or is at the end of the string.
                // This prevents capitalized letters inside emails (e.g. gmail.com), numbers (e.g. 3.5), or file extensions (e.g. .exe).
                if let Some(&next_ch) = chars.peek() {
                    if next_ch.is_whitespace() {
                        capitalize_next = true;
                    }
                } else {
                    capitalize_next = true;
                }
            }
        }
    }

    // Ensure ending punctuation
    if let Some(last) = result.chars().last() {
        if !matches!(last, '.' | '!' | '?') {
            result.push('.');
        }
    }

    result
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
