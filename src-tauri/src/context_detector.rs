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

/// Resolve a PID to its executable basename (e.g. "Code.exe"). Returns "" on any failure
/// (process gone, access denied, empty path). Windows-only.
#[cfg(windows)]
fn process_name_from_pid(pid: u32) -> String {
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows_sys::Win32::System::ProcessStatus::GetModuleFileNameExW;
    use windows_sys::Win32::Foundation::CloseHandle;

    if pid == 0 {
        return String::new();
    }

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() || handle == -1isize as _ {
            return String::new();
        }
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
    }
}

#[cfg(windows)]
fn detect_foreground_windows() -> ForegroundApp {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId};

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
        let process_name = process_name_from_pid(pid);

        ForegroundApp {
            process_name,
            window_title,
        }
    }
}

/// Read the window class of the control that currently has keyboard focus within the
/// foreground window's thread. Lets us spot a focused native terminal even when the host
/// process/window looks generic (e.g. a console inside a non-dev app). Returns "" on any
/// failure so the caller simply skips the native-terminal detection tier.
#[cfg(windows)]
pub fn focused_child_class() -> String {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId,
        GUITHREADINFO,
    };

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return String::new();
        }

        // GetGUIThreadInfo needs the thread id owning the foreground window.
        let tid = GetWindowThreadProcessId(hwnd, std::ptr::null_mut());
        if tid == 0 {
            return String::new();
        }

        let mut info: GUITHREADINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        if GetGUIThreadInfo(tid, &mut info) == 0 {
            return String::new();
        }

        let focus = info.hwndFocus;
        if focus.is_null() {
            return String::new();
        }

        let mut buf = [0u16; 256];
        let len = GetClassNameW(focus, buf.as_mut_ptr(), buf.len() as i32);
        if len <= 0 {
            return String::new();
        }
        OsString::from_wide(&buf[..len as usize])
            .to_string_lossy()
            .to_string()
    }
}

#[cfg(not(windows))]
pub fn focused_child_class() -> String {
    String::new()
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

    // (3) User-defined custom rules. A rule's process filter must match the foreground
    // process, OR be blank — a blank process is a title-only rule that applies to any app,
    // so a browser site (e.g. "youtube") matches regardless of which browser shows it.
    // When a title filter is present it must be a substring of the window title too.
    for rule in custom_rules {
        let rule_proc = rule.process_name.trim().to_lowercase();
        if !rule_proc.is_empty() && proc_lower != rule_proc {
            continue;
        }
        match rule.title_contains.as_deref().map(str::trim) {
            Some(title_match) if !title_match.is_empty() => {
                if title_lower.contains(&title_match.to_lowercase()) {
                    return rule.category.clone();
                }
            }
            // No title filter: a process-only rule. A rule with neither process nor
            // title is rejected at creation, so rule_proc is non-empty here.
            _ => {
                if !rule_proc.is_empty() {
                    return rule.category.clone();
                }
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
        | "pycharm64.exe" | "webstorm64.exe" | "rider64.exe" | "clion64.exe"
        | "goland64.exe" | "zed.exe" | "windsurf.exe" | "orca.exe"
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
    matches!(
        proc,
        "chrome.exe" | "msedge.exe" | "firefox.exe" | "brave.exe" | "opera.exe" | "vivaldi.exe"
            | "arc.exe"
            // Newer entrants. An unrecognized browser is not a harmless miss: title heuristics
            // are gated on this, so every site inside it (Gmail, Slack, GitHub) silently
            // resolves to General.
            | "comet.exe" | "zen.exe" | "floorp.exe" | "librewolf.exe"
    )
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

/// True when the foreground process itself is a terminal emulator or console host. Used to
/// distinguish a terminal surface from an IDE inside the Developer context: a terminal gets
/// the literal-transcription AI prompt, while an IDE keeps the general Developer restyling.
/// Needed alongside `is_native_terminal_class` because UWP-hosted terminals (Windows
/// Terminal) report a generic `Windows.UI.Input.InputSite.WindowClass` focus child, so the
/// class signal misses them. Pure.
pub fn is_terminal_process(process_name: &str) -> bool {
    const TERMINALS: &[&str] = &[
        "windowsterminal.exe", "cmd.exe", "powershell.exe", "pwsh.exe", "wt.exe",
        "conhost.exe", "alacritty.exe", "wezterm-gui.exe", "conemu64.exe", "hyper.exe",
        "tabby.exe", "mintty.exe", "putty.exe", "kitty.exe", "ghostty.exe", "wsl.exe",
        "bash.exe",
    ];
    let lower = process_name.to_ascii_lowercase();
    TERMINALS.contains(&lower.as_str())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppRule {
    pub process_name: String,
    pub title_contains: Option<String>,
    pub category: ContextCategory,
}

/// One currently-running app surfaced to the app-rule picker. Carries only the process
/// name and a friendly display label — never a window title (titles are privacy-sensitive
/// and must never leave the backend).
#[derive(Debug, Clone, Serialize)]
pub struct RunningApp {
    pub process_name: String, // e.g. "Orca.exe" (as reported by the OS)
    pub display_name: String, // e.g. "Orca"
}

/// "orca.exe" -> "Orca", "wezterm-gui.exe" -> "Wezterm-gui", "Code.exe" -> "Code".
/// Pure: strip a trailing ".exe" (case-insensitive), uppercase the first ASCII letter.
pub fn friendly_display_name(process_name: &str) -> String {
    // Strip a trailing ".exe" case-insensitively. Comparing on a lowercased copy keeps the
    // check byte-safe (avoids slicing `process_name` at a non-char boundary); when it ends
    // with ".exe" those 4 bytes are ASCII, so `len - 4` is always a valid char boundary.
    let base = if process_name.to_ascii_lowercase().ends_with(".exe") {
        &process_name[..process_name.len() - 4]
    } else {
        process_name
    };

    let mut chars = base.chars();
    match chars.next() {
        Some(first) => {
            let mut out = String::with_capacity(base.len());
            out.push(first.to_ascii_uppercase());
            out.push_str(chars.as_str());
            out
        }
        None => String::new(),
    }
}

/// Windows shell/host processes that surface visible windows but are never sensible
/// app-rule targets (they're OS chrome, not user apps). Filtered case-insensitively so
/// noise like `ApplicationFrameHost.exe` stays out of the picker.
const SHELL_HOST_BLOCKLIST: &[&str] = &[
    "applicationframehost.exe",
    "textinputhost.exe",
    "shellexperiencehost.exe",
    "startmenuexperiencehost.exe",
    "searchhost.exe",
    "systemsettings.exe",
    "lockapp.exe",
];

/// Dedup by lowercase process name (keep first occurrence), drop empty names, the given
/// `own_process` (case-insensitive, e.g. "typr.exe"), and Windows shell-host noise, sort
/// by display_name (case-insensitive), and build the RunningApp list.
pub fn build_running_apps(process_names: Vec<String>, own_process: &str) -> Vec<RunningApp> {
    let own_lower = own_process.to_lowercase();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut apps: Vec<RunningApp> = Vec::new();

    for name in process_names {
        if name.is_empty() {
            continue;
        }
        let lower = name.to_lowercase();
        if lower == own_lower {
            continue;
        }
        if SHELL_HOST_BLOCKLIST.contains(&lower.as_str()) {
            continue; // Windows shell host → never a rule target
        }
        if !seen.insert(lower) {
            continue; // duplicate (case-insensitive), keep first occurrence
        }
        let display_name = friendly_display_name(&name);
        apps.push(RunningApp { process_name: name, display_name });
    }

    apps.sort_by(|a, b| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()));
    apps
}

/// Callback for `EnumWindows`: keep visible, titled, non-tool-window top-level windows and
/// collect their process names into the `Vec<String>` passed via `lparam`. Only the window
/// title LENGTH is read (via `GetWindowTextLengthW`) to filter empty-titled windows; the
/// title text is never read, stored, or logged.
#[cfg(windows)]
unsafe extern "system" fn enum_running_apps_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> i32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongW, GetWindowTextLengthW, GetWindowThreadProcessId, IsWindowVisible,
        GWL_EXSTYLE, WS_EX_TOOLWINDOW,
    };

    let names = &mut *(lparam as *mut Vec<String>);

    if IsWindowVisible(hwnd) == 0 {
        return 1; // continue
    }
    if GetWindowTextLengthW(hwnd) == 0 {
        return 1; // no title → skip (title text itself is never read)
    }
    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
    if ex_style & WS_EX_TOOLWINDOW != 0 {
        return 1; // palette/overlay → skip
    }

    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    let name = process_name_from_pid(pid);
    if !name.is_empty() {
        names.push(name);
    }
    1 // continue enumeration
}

/// Enumerate apps that currently have a visible, titled, non-tool top-level window and
/// return them as a deduped, sorted picker list. Excludes Typr itself. Windows-only; the
/// non-Windows build returns an empty list.
#[cfg(windows)]
pub fn list_running_apps() -> Vec<RunningApp> {
    use windows_sys::Win32::UI::WindowsAndMessaging::EnumWindows;
    use windows_sys::Win32::Foundation::LPARAM;

    let mut names: Vec<String> = Vec::new();
    unsafe {
        EnumWindows(Some(enum_running_apps_proc), &mut names as *mut Vec<String> as LPARAM);
    }
    build_running_apps(names, "typr.exe")
}

#[cfg(not(windows))]
pub fn list_running_apps() -> Vec<RunningApp> {
    Vec::new()
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
            "pycharm64.exe", "webstorm64.exe", "rider64.exe",
            "clion64.exe", "goland64.exe", "zed.exe", "windsurf.exe", "orca.exe",
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
    fn test_newer_browsers_get_title_heuristics() {
        // Regression: Comet resolved to General for every site, because title heuristics only
        // run for processes recognized as browsers.
        for proc in ["comet.exe", "zen.exe", "floorp.exe", "librewolf.exe"] {
            let app = ForegroundApp {
                process_name: proc.into(),
                window_title: "Gmail - Inbox".into(),
            };
            assert_eq!(
                resolve_category(&app, &[], "", ""),
                ContextCategory::Email,
                "{} should be treated as a browser",
                proc
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
    fn test_title_only_rule_is_browser_agnostic() {
        // Blank process + title filter: matches the site in ANY browser (or any app).
        let rules = vec![AppRule {
            process_name: String::new(),
            title_contains: Some("youtube".into()),
            category: ContextCategory::Messaging,
        }];
        let chrome = ForegroundApp { process_name: "chrome.exe".into(), window_title: "Home - YouTube".into() };
        let edge = ForegroundApp { process_name: "msedge.exe".into(), window_title: "YouTube".into() };
        let firefox = ForegroundApp { process_name: "firefox.exe".into(), window_title: "cats — YouTube".into() };
        assert_eq!(resolve_category(&chrome, &rules, "", ""), ContextCategory::Messaging);
        assert_eq!(resolve_category(&edge, &rules, "", ""), ContextCategory::Messaging);
        assert_eq!(resolve_category(&firefox, &rules, "", ""), ContextCategory::Messaging);
    }

    #[test]
    fn test_title_only_rule_does_not_match_other_titles() {
        // A title-only rule must not fire when the phrase is absent → falls through to default.
        let rules = vec![AppRule {
            process_name: String::new(),
            title_contains: Some("youtube".into()),
            category: ContextCategory::Messaging,
        }];
        let app = ForegroundApp { process_name: "chrome.exe".into(), window_title: "GitHub - my repo".into() };
        // chrome + "github" title → browser heuristic Developer, not the YouTube rule.
        assert_eq!(resolve_category(&app, &rules, "", ""), ContextCategory::Developer);
    }

    #[test]
    fn test_title_only_rule_beats_browser_heuristic() {
        // A user's title rule lives in tier 3, above the built-in browser heuristics (tier 5),
        // so it overrides the default github→Developer mapping.
        let rules = vec![AppRule {
            process_name: String::new(),
            title_contains: Some("github".into()),
            category: ContextCategory::Professional,
        }];
        let app = ForegroundApp { process_name: "chrome.exe".into(), window_title: "GitHub · dashboard".into() };
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

    #[test]
    fn test_friendly_display_name() {
        assert_eq!(friendly_display_name("orca.exe"), "Orca");
        assert_eq!(friendly_display_name("Code.exe"), "Code");
        assert_eq!(friendly_display_name("wezterm-gui.EXE"), "Wezterm-gui");
        assert_eq!(friendly_display_name("noext"), "Noext");
        assert_eq!(friendly_display_name(""), "");
    }

    #[test]
    fn test_build_running_apps_drops_shell_hosts() {
        // Windows shell/host processes are never sensible rule targets and pollute the
        // picker; they must be filtered case-insensitively while real apps survive.
        let names = vec![
            "ApplicationFrameHost.exe".to_string(), // mixed case → dropped
            "TextInputHost.exe".to_string(),
            "shellexperiencehost.exe".to_string(),
            "StartMenuExperienceHost.exe".to_string(),
            "SearchHost.exe".to_string(),
            "SystemSettings.exe".to_string(),
            "LockApp.exe".to_string(),
            "Code.exe".to_string(),   // normal app → kept
            "obsidian.exe".to_string(),
        ];
        let apps = build_running_apps(names, "typr.exe");
        let procs: Vec<String> = apps.iter().map(|a| a.process_name.to_lowercase()).collect();
        assert!(!procs.iter().any(|p| p == "applicationframehost.exe"));
        assert!(!procs.iter().any(|p| p == "textinputhost.exe"));
        assert!(!procs.iter().any(|p| p == "shellexperiencehost.exe"));
        assert!(!procs.iter().any(|p| p == "startmenuexperiencehost.exe"));
        assert!(!procs.iter().any(|p| p == "searchhost.exe"));
        assert!(!procs.iter().any(|p| p == "systemsettings.exe"));
        assert!(!procs.iter().any(|p| p == "lockapp.exe"));
        // Real apps survive.
        assert_eq!(apps.len(), 2);
        assert!(procs.iter().any(|p| p == "code.exe"));
        assert!(procs.iter().any(|p| p == "obsidian.exe"));
    }

    #[test]
    fn test_build_running_apps_dedup_drop_sort() {
        // Case-insensitive dedup keeping first occurrence, drop empties and own process,
        // sort by display_name case-insensitively, preserve process_name as reported.
        let names = vec![
            "Zed.exe".to_string(),
            "orca.exe".to_string(),
            "Orca.exe".to_string(),   // dup of orca.exe (case-insensitive) → dropped
            "".to_string(),           // empty → dropped
            "Typr.exe".to_string(),   // own process → dropped
            "code.exe".to_string(),
        ];
        let apps = build_running_apps(names, "typr.exe");

        // Expect three: orca, code, zed → sorted by display_name: Code, Orca, Zed.
        assert_eq!(apps.len(), 3);
        assert_eq!(apps[0].process_name, "code.exe");
        assert_eq!(apps[0].display_name, "Code");
        assert_eq!(apps[1].process_name, "orca.exe"); // first occurrence preserved as reported
        assert_eq!(apps[1].display_name, "Orca");
        assert_eq!(apps[2].process_name, "Zed.exe"); // original casing preserved
        assert_eq!(apps[2].display_name, "Zed");
    }
}
