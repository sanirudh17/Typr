use serde::{Serialize, Deserialize};
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ContextCategory {
    Messaging,
    Email,
    Professional,
    Developer,
    General,
}

impl std::fmt::Display for ContextCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContextCategory::Messaging => write!(f, "Messaging"),
            ContextCategory::Email => write!(f, "Email"),
            ContextCategory::Professional => write!(f, "Professional"),
            ContextCategory::Developer => write!(f, "Developer"),
            ContextCategory::General => write!(f, "General"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForegroundApp {
    pub process_name: String,
    pub window_title: String,
}

impl ForegroundApp {
    pub fn detect() -> Self {
        #[cfg(windows)]
        {
            detect_foreground_windows()
        }
        #[cfg(not(windows))]
        {
            ForegroundApp {
                process_name: String::new(),
                window_title: String::new(),
            }
        }
    }
}

#[cfg(windows)]
fn detect_foreground_windows() -> ForegroundApp {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows_sys::Win32::System::ProcessStatus::GetModuleFileNameExW;
    use windows_sys::Win32::Foundation::CloseHandle;

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return ForegroundApp {
                process_name: String::new(),
                window_title: String::new(),
            };
        }

        // Get window title
        let mut title_buf = [0u16; 512];
        let title_len = GetWindowTextW(hwnd, title_buf.as_mut_ptr(), title_buf.len() as i32);
        let window_title = if title_len > 0 {
            OsString::from_wide(&title_buf[..title_len as usize])
                .to_string_lossy()
                .to_string()
        } else {
            String::new()
        };

        // Get process name
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        let process_name = if pid != 0 {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if !handle.is_null() && handle != -1isize as _ {
                let mut name_buf = [0u16; 512];
                let name_len = GetModuleFileNameExW(handle, std::ptr::null_mut(), name_buf.as_mut_ptr(), name_buf.len() as u32);
                CloseHandle(handle);
                if name_len > 0 {
                    let full_path = OsString::from_wide(&name_buf[..name_len as usize])
                        .to_string_lossy()
                        .to_string();
                    full_path.rsplit('\\').next().unwrap_or(&full_path).to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        ForegroundApp {
            process_name,
            window_title,
        }
    }
}

/// Parse a category name (case-insensitive) into a `ContextCategory`, erroring on an
/// unknown value. Used by the `add_app_rule` command to validate UI input.
pub fn parse_category(value: &str) -> Result<ContextCategory, String> {
    match value.trim().to_lowercase().as_str() {
        "messaging" => Ok(ContextCategory::Messaging),
        "email" => Ok(ContextCategory::Email),
        "professional" => Ok(ContextCategory::Professional),
        "developer" => Ok(ContextCategory::Developer),
        "general" => Ok(ContextCategory::General),
        other => Err(format!("unknown context category: {other}")),
    }
}

/// Parse a persisted "auto context override" value into a pinned category. `"auto"`,
/// empty, or any unrecognized value means "no override" → fall through to detection.
fn parse_override(value: &str) -> Option<ContextCategory> {
    parse_category(value).ok()
}

/// Maps a detected foreground app to a context category. Pure and fully unit-testable: the
/// focused child-window class and the manual override arrive as parameters (the Win32 lookup
/// lives in `focused_child_class`). Precedence: (1) manual override → (2) focused native
/// terminal → Developer → (3) custom rules → (4) default process map → (5) browser-title
/// heuristics → (6) General.
pub fn resolve_category(
    app: &ForegroundApp,
    custom_rules: &[AppRule],
    focused_class: &str,
    override_: &str,
) -> ContextCategory {
    // (1) Manual override wins over all detection.
    if let Some(category) = parse_override(override_) {
        return category;
    }

    // (2) A focused native terminal/console control is Developer anywhere it appears,
    // including inside a non-developer host app.
    if is_native_terminal_class(focused_class) {
        return ContextCategory::Developer;
    }

    let proc_lower = app.process_name.to_lowercase();
    let title_lower = app.window_title.to_lowercase();

    // (3) User-defined custom rules.
    for rule in custom_rules {
        if proc_lower == rule.process_name.to_lowercase() {
            if let Some(ref title_match) = rule.title_contains {
                if title_lower.contains(&title_match.to_lowercase()) {
                    return rule.category.clone();
                }
            } else {
                return rule.category.clone();
            }
        }
    }

    // Some modern apps launch under a wrapper/child process name (e.g. the native
    // WhatsApp desktop app reports "WhatsApp.Root.exe", not "whatsapp.exe"). Match the
    // family by prefix so process-name variants still resolve.
    if proc_lower.starts_with("whatsapp") {
        return ContextCategory::Messaging;
    }

    // (4) Default process-name mappings.
    match proc_lower.as_str() {
        // Messaging
        "whatsapp.exe" | "telegram.exe" | "discord.exe" | "slack.exe"
        | "teams.exe" | "signal.exe" | "messenger.exe" => return ContextCategory::Messaging,

        // Email
        "outlook.exe" | "thunderbird.exe" => return ContextCategory::Email,

        // Developer — editors/IDEs (Electron + native) and terminal emulators. Whole app
        // maps to Developer: terse/code-aware styling is what a developer wants in both the
        // editor and the terminal, so we deliberately do not try to distinguish panes here.
        "code.exe" | "cursor.exe" | "windowsterminal.exe" | "cmd.exe"
        | "powershell.exe" | "pwsh.exe" | "idea64.exe" | "devenv.exe" | "sublime_text.exe"
        | "alacritty.exe" | "wezterm-gui.exe" | "wt.exe"
        | "orca.exe"
        | "pycharm64.exe" | "webstorm64.exe" | "rider64.exe" | "clion64.exe"
        | "goland64.exe" | "zed.exe" | "windsurf.exe"
        | "conemu64.exe" | "hyper.exe" | "tabby.exe" | "mintty.exe"
        | "putty.exe" | "kitty.exe" | "ghostty.exe" => return ContextCategory::Developer,

        // Professional
        "winword.exe" | "excel.exe" | "powerpnt.exe" | "notion.exe"
        | "onenote.exe" | "notepad.exe" => return ContextCategory::Professional,

        _ => {}
    }

    // (5) Browser title heuristics.
    if is_browser(&proc_lower) {
        if title_lower.contains("gmail") || title_lower.contains("outlook") || title_lower.contains("mail") || title_lower.contains("protonmail") {
            return ContextCategory::Email;
        }
        if title_lower.contains("slack") || title_lower.contains("discord") || title_lower.contains("whatsapp") || title_lower.contains("messenger") || title_lower.contains("telegram") {
            return ContextCategory::Messaging;
        }
        if title_lower.contains("github") || title_lower.contains("stackoverflow") || title_lower.contains("localhost") || title_lower.contains("codepen") || title_lower.contains("codesandbox") {
            return ContextCategory::Developer;
        }
        if title_lower.contains("docs.google") || title_lower.contains("notion") || title_lower.contains("confluence") {
            return ContextCategory::Professional;
        }
    }

    // (6) General fallback.
    ContextCategory::General
}

fn is_browser(proc: &str) -> bool {
    matches!(proc, "chrome.exe" | "msedge.exe" | "firefox.exe" | "brave.exe" | "opera.exe" | "vivaldi.exe" | "arc.exe")
}

/// True when a focused child-control window class belongs to a native terminal/console
/// host (a real command-line surface), so dictation into it should use Developer styling.
/// Matches exact console classes and known terminal-emulator class families, and rejects
/// ordinary editor/text/browser control classes (`Chrome_WidgetWin_1`, `Edit`, …). Pure.
pub fn is_native_terminal_class(class: &str) -> bool {
    // Exact, well-known native console / terminal host window classes.
    const EXACT: &[&str] = &[
        "ConsoleWindowClass",            // classic conhost (cmd.exe / powershell.exe)
        "CASCADIA_HOSTING_WINDOW_CLASS", // Windows Terminal (Cascadia)
        "PuTTY",                         // PuTTY
        "mintty",                        // mintty (Git Bash / Cygwin / MSYS2)
    ];
    if EXACT.iter().any(|c| c.eq_ignore_ascii_case(class)) {
        return true;
    }
    // Prefix families: ConEmu classes its consoles "VirtualConsoleClass" / "ConEmu*".
    let lower = class.to_ascii_lowercase();
    lower.starts_with("conemu") || lower.starts_with("virtualconsoleclass")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppRule {
    pub process_name: String,
    pub title_contains: Option<String>,
    pub category: ContextCategory,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_messaging_apps() {
        let app = ForegroundApp { process_name: "WhatsApp.exe".into(), window_title: String::new() };
        assert_eq!(resolve_category(&app, &[], "", ""), ContextCategory::Messaging);
    }

    #[test]
    fn test_whatsapp_native_wrapper_process() {
        // The native WhatsApp desktop app reports "WhatsApp.Root.exe" as its process name.
        let app = ForegroundApp { process_name: "WhatsApp.Root.exe".into(), window_title: "WhatsApp".into() };
        assert_eq!(resolve_category(&app, &[], "", ""), ContextCategory::Messaging);
    }

    #[test]
    fn test_email_apps() {
        let app = ForegroundApp { process_name: "OUTLOOK.exe".into(), window_title: String::new() };
        assert_eq!(resolve_category(&app, &[], "", ""), ContextCategory::Email);
    }

    #[test]
    fn test_developer_apps() {
        let app = ForegroundApp { process_name: "Code.exe".into(), window_title: String::new() };
        assert_eq!(resolve_category(&app, &[], "", ""), ContextCategory::Developer);
    }

    #[test]
    fn test_broadened_dev_apps() {
        for proc in [
            "orca.exe", "pycharm64.exe", "webstorm64.exe", "rider64.exe",
            "clion64.exe", "goland64.exe", "zed.exe", "windsurf.exe",
            "conemu64.exe", "hyper.exe", "tabby.exe", "mintty.exe",
            "putty.exe", "kitty.exe", "ghostty.exe",
        ] {
            let app = ForegroundApp { process_name: proc.into(), window_title: String::new() };
            assert_eq!(
                resolve_category(&app, &[], "", ""),
                ContextCategory::Developer,
                "expected {proc} to map to Developer",
            );
        }
    }

    #[test]
    fn test_browser_gmail() {
        let app = ForegroundApp { process_name: "chrome.exe".into(), window_title: "Gmail - Inbox - Google Chrome".into() };
        assert_eq!(resolve_category(&app, &[], "", ""), ContextCategory::Email);
    }

    #[test]
    fn test_browser_slack() {
        let app = ForegroundApp { process_name: "msedge.exe".into(), window_title: "Slack | general | Company".into() };
        assert_eq!(resolve_category(&app, &[], "", ""), ContextCategory::Messaging);
    }

    #[test]
    fn test_browser_github() {
        let app = ForegroundApp { process_name: "chrome.exe".into(), window_title: "GitHub - Pull Requests".into() };
        assert_eq!(resolve_category(&app, &[], "", ""), ContextCategory::Developer);
    }

    #[test]
    fn test_general_fallback() {
        let app = ForegroundApp { process_name: "randomapp.exe".into(), window_title: "Untitled".into() };
        assert_eq!(resolve_category(&app, &[], "", ""), ContextCategory::General);
    }

    #[test]
    fn test_notepad_professional() {
        let app = ForegroundApp { process_name: "notepad.exe".into(), window_title: "Untitled".into() };
        assert_eq!(resolve_category(&app, &[], "", ""), ContextCategory::Professional);
    }

    #[test]
    fn test_custom_rule_overrides() {
        let rules = vec![AppRule {
            process_name: "notepad.exe".into(),
            title_contains: None,
            category: ContextCategory::Developer,
        }];
        let app = ForegroundApp { process_name: "notepad.exe".into(), window_title: "test.rs".into() };
        assert_eq!(resolve_category(&app, &rules, "", ""), ContextCategory::Developer);
    }

    #[test]
    fn test_is_native_terminal_class() {
        // Native console / terminal-host window classes → true.
        assert!(is_native_terminal_class("ConsoleWindowClass"));       // conhost (cmd/powershell)
        assert!(is_native_terminal_class("CASCADIA_HOSTING_WINDOW_CLASS")); // Windows Terminal
        assert!(is_native_terminal_class("mintty"));                   // Git Bash / Cygwin
        assert!(is_native_terminal_class("PuTTY"));                    // PuTTY
        assert!(is_native_terminal_class("VirtualConsoleClass"));      // ConEmu child
        assert!(is_native_terminal_class("ConEmuMultiConsole"));       // ConEmu family
        // Case-insensitive on the exact set.
        assert!(is_native_terminal_class("consolewindowclass"));
        // Ordinary editor / text / browser control classes → false.
        assert!(!is_native_terminal_class("Chrome_WidgetWin_1"));      // Electron/Chromium editor
        assert!(!is_native_terminal_class("Edit"));
        assert!(!is_native_terminal_class("RichEditD2DPT"));
        assert!(!is_native_terminal_class("Notepad"));
        assert!(!is_native_terminal_class(""));
    }

    #[test]
    fn test_custom_rule_with_title() {
        let rules = vec![AppRule {
            process_name: "chrome.exe".into(),
            title_contains: Some("Jira".into()),
            category: ContextCategory::Professional,
        }];
        let app = ForegroundApp { process_name: "chrome.exe".into(), window_title: "Jira - Sprint Board".into() };
        assert_eq!(resolve_category(&app, &rules, "", ""), ContextCategory::Professional);
    }

    #[test]
    fn test_manual_override_wins() {
        // A Developer app, but the user pinned Email → Email regardless of everything else.
        let app = ForegroundApp { process_name: "code.exe".into(), window_title: String::new() };
        assert_eq!(resolve_category(&app, &[], "ConsoleWindowClass", "email"), ContextCategory::Email);
        // "auto" / "" / unknown means no override → normal detection.
        assert_eq!(resolve_category(&app, &[], "", "auto"), ContextCategory::Developer);
        assert_eq!(resolve_category(&app, &[], "", ""), ContextCategory::Developer);
    }

    #[test]
    fn test_native_terminal_focus_is_developer() {
        // A non-developer host (generic app) but a focused native console → Developer.
        let app = ForegroundApp { process_name: "randomapp.exe".into(), window_title: "x".into() };
        assert_eq!(resolve_category(&app, &[], "ConsoleWindowClass", ""), ContextCategory::Developer);
    }

    #[test]
    fn test_precedence_override_beats_terminal_beats_rule_beats_default() {
        let rules = vec![AppRule {
            process_name: "randomapp.exe".into(),
            title_contains: None,
            category: ContextCategory::Professional,
        }];
        let app = ForegroundApp { process_name: "randomapp.exe".into(), window_title: "x".into() };
        // Override beats native terminal.
        assert_eq!(resolve_category(&app, &rules, "ConsoleWindowClass", "messaging"), ContextCategory::Messaging);
        // Native terminal beats the custom rule.
        assert_eq!(resolve_category(&app, &rules, "ConsoleWindowClass", ""), ContextCategory::Developer);
        // Custom rule beats the (General) default when no terminal/override.
        assert_eq!(resolve_category(&app, &rules, "Edit", ""), ContextCategory::Professional);
    }

    #[test]
    fn test_parse_category_roundtrip() {
        assert_eq!(parse_category("Developer").unwrap(), ContextCategory::Developer);
        assert_eq!(parse_category("email").unwrap(), ContextCategory::Email);
        assert!(parse_category("nonsense").is_err());
    }
}
