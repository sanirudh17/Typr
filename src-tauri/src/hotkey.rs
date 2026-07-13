/// Pure helpers for validating global-shortcut accelerator strings
/// (Tauri format, e.g. `CmdOrCtrl+Shift+Space`).

const MODIFIER_TOKENS: &[&str] = &[
    "ctrl", "control", "cmdorctrl", "cmd", "command", "super", "win", "meta", "alt", "option",
    "shift",
];

/// Returns `Ok(())` when `accel` has at least one modifier and exactly one
/// non-modifier key token. Returns a human-readable `Err` otherwise.
pub fn validate_accelerator(accel: &str) -> Result<(), String> {
    let trimmed = accel.trim();
    if trimmed.is_empty() {
        return Err("Press a key combination — nothing was captured.".to_string());
    }

    let mut modifier_count = 0usize;
    let mut key_count = 0usize;
    for token in trimmed.split('+') {
        let token = token.trim();
        if token.is_empty() {
            return Err(format!("`{}` is not a valid combination.", accel));
        }
        if MODIFIER_TOKENS.contains(&token.to_ascii_lowercase().as_str()) {
            modifier_count += 1;
        } else {
            key_count += 1;
        }
    }

    if modifier_count == 0 {
        return Err("Add a modifier — Ctrl, Alt, Shift, or Win — then a key.".to_string());
    }
    if key_count == 0 {
        return Err("Add a key after the modifier (e.g. Space or D).".to_string());
    }
    if key_count > 1 {
        return Err("Use one key plus modifiers, not multiple keys.".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ctrl_shift_space() {
        assert!(validate_accelerator("CmdOrCtrl+Shift+Space").is_ok());
    }

    #[test]
    fn accepts_alt_f8() {
        assert!(validate_accelerator("Alt+F8").is_ok());
    }

    #[test]
    fn accepts_single_modifier_plus_key() {
        assert!(validate_accelerator("Ctrl+D").is_ok());
    }

    #[test]
    fn rejects_empty() {
        assert!(validate_accelerator("").is_err());
    }

    #[test]
    fn rejects_modifier_only() {
        assert!(validate_accelerator("Shift").is_err());
        assert!(validate_accelerator("Ctrl+Shift").is_err());
    }

    #[test]
    fn rejects_no_modifier() {
        assert!(validate_accelerator("A").is_err());
        assert!(validate_accelerator("F8").is_err());
    }

    #[test]
    fn rejects_two_non_modifier_keys() {
        assert!(validate_accelerator("Ctrl+A+B").is_err());
    }

    #[test]
    fn is_case_insensitive_on_modifiers() {
        assert!(validate_accelerator("control+shift+space").is_ok());
    }
}
