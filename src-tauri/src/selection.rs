//! Selected-text editing on a worker thread. No Ctrl+A, focus stealing, or
//! persistent storage of the original document. Clipboard ownership is checked
//! before restoration so a newer user copy is never overwritten.
#[derive(Clone)]
pub struct Selection {
    pub text: String,
    #[cfg(windows)]
    target: native::Target,
    #[cfg(windows)]
    anchor: Option<native::Anchor>,
}

pub fn capture() -> Result<Selection, String> {
    capture_with_previous(None, &crate::write_mode::Cancellation::default())
}
pub fn capture_with_previous(previous: Option<&Selection>, cancel: &crate::write_mode::Cancellation) -> Result<Selection, String> {
    #[cfg(windows)]
    {
        native::capture(previous, cancel)
    }
    #[cfg(not(windows))]
    {
        let _ = (previous, cancel);
        Err("Write Mode currently supports Windows.".into())
    }
}
impl Selection {
    pub fn with_edited_text(&self, text: String) -> Self {
        let mut next = self.clone(); next.text = text; next
    }
    pub fn same_passage(&self, other: &Self) -> bool {
        #[cfg(windows)] { self.target == other.target && self.anchor == other.anchor && normalize_lines(&self.text) == normalize_lines(&other.text) }
        #[cfg(not(windows))] { self.text == other.text }
    }
}
fn normalize_lines(text: &str) -> String { text.replace("\r\n", "\n").replace('\r', "\n") }
fn copy_origin_allowed(owner_process: Option<u32>, target_process: u32) -> bool {
    owner_process == Some(target_process)
}
const TERMINAL_UNSUPPORTED: &str = "Write Mode isn't available in terminals. Select text in an editor to rewrite it.";
fn require_editor_surface(process_name: &str, window_class: &str, focused_class: &str) -> Result<(), String> {
    let class = focused_class.to_ascii_lowercase();
    if crate::context_detector::is_terminal_process(process_name)
        || crate::context_detector::is_native_terminal_class(window_class)
        || crate::context_detector::is_native_terminal_class(focused_class)
        || class == "xterm" || class.starts_with("xterm-") {
        return Err(TERMINAL_UNSUPPORTED.into());
    }
    Ok(())
}
#[cfg(test)]
mod copy_tests {
    use super::*;
    #[test]
    fn clipboard_capture_requires_same_owner_process() {
        assert!(!copy_origin_allowed(None, 42));
        assert!(copy_origin_allowed(Some(42), 42));
        assert!(!copy_origin_allowed(Some(43), 42));
    }
    #[test]
    fn terminal_surfaces_are_rejected_before_selection_capture() {
        for (process, window, focused) in [
            ("Orca.exe", "Chrome_WidgetWin_1", "xterm-helper-textarea"),
            ("Code.exe", "Chrome_WidgetWin_1", "xterm-screen"),
            ("WindowsTerminal.exe", "", ""),
            ("randomhost.exe", "ConsoleWindowClass", ""),
            ("randomhost.exe", "", "ConEmuChild"),
            ("PuTTY.exe", "PuTTY", ""),
        ] {
            assert_eq!(require_editor_surface(process, window, focused).unwrap_err(), TERMINAL_UNSUPPORTED);
        }
    }
    #[test]
    fn editable_fields_in_terminal_hosting_apps_remain_available() {
        for process in ["Orca.exe", "Code.exe", "Chrome.exe", "Notepad.exe"] {
            assert!(require_editor_surface(process, "Chrome_WidgetWin_1", "textarea").is_ok());
        }
    }
    #[test]
    fn equivalent_editor_line_endings_match() {
        assert_eq!(normalize_lines("one\r\ntwo\rthree"), "one\ntwo\nthree");
    }
}
/// Read-only, bounded anchor for a normal dictation insertion. Unlike Write
/// Mode it permits an empty caret selection and never simulates copy.
pub fn prepare_observation() -> Result<Selection, String> {
    #[cfg(windows)] { native::prepare_observation() }
    #[cfg(not(windows))] { Err("Edit observation currently supports Windows.".into()) }
}
pub fn observed_passage(marker: &Selection) -> Result<String, String> {
    #[cfg(windows)] { native::observed_passage(marker) }
    #[cfg(not(windows))] { let _=marker; Err("Edit observation currently supports Windows.".into()) }
}
pub fn replace(
    selection: &Selection,
    text: &str,
    cancel: &crate::write_mode::Cancellation,
) -> Result<Option<String>, String> {
    #[cfg(windows)]
    {
        native::replace(selection, text, cancel)
    }
    #[cfg(not(windows))]
    {
        let _ = (selection, text, cancel);
        Err("Write Mode currently supports Windows.".into())
    }
}
#[cfg(windows)]
mod native {
    use super::{require_editor_surface, Selection};
    use std::{
        thread::sleep,
        time::{Duration, Instant},
    };
    use windows::{
        core::Interface,
        Win32::{
            Foundation::{GlobalFree, HANDLE, HGLOBAL},
            Graphics::Gdi::{
                CopyEnhMetaFileW, DeleteEnhMetaFile, DeleteObject, HENHMETAFILE, HGDIOBJ,
            },
            System::{
                Com::{
                    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
                    COINIT_MULTITHREADED,
                },
                DataExchange::*,
                Memory::*,
                Ole::*,
            },
            UI::{
                Accessibility::*,
                Input::KeyboardAndMouse::{
                    GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
                },
                WindowsAndMessaging::*,
            },
        },
    };
    const MAX_DOCUMENT: i32 = 131072;
    const CHANGED: &str =
        "Your window or selection changed. Select the text again and retry Write Mode.";
    fn err(_: windows::core::Error) -> String {
        "This app could not expose its selected text. Select text in an editable field and try again.".into()
    }
    fn normalized(text: &str) -> String {
        super::normalize_lines(text)
    }
    struct Apartment;
    impl Apartment {
        fn new() -> Result<Self, String> {
            unsafe {
                CoInitializeEx(None, COINIT_MULTITHREADED)
                    .ok()
                    .map_err(err)?;
            }
            Ok(Self)
        }
    }
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }

    #[derive(Clone, PartialEq)]
    pub(super) struct Target {
        window: usize,
        process: u32,
        focus: usize,
        runtime_id: Vec<i32>,
    }
    #[derive(Clone, PartialEq)]
    pub(super) struct Anchor {
        prefix: String,
        suffix: String,
    }
    fn focused() -> Result<(IUIAutomation, IUIAutomationElement, Target), String> {
        unsafe {
            let window = GetForegroundWindow();
            if window.0.is_null() {
                return Err(CHANGED.into());
            }
            let mut process = 0;
            let thread = GetWindowThreadProcessId(window, Some(&mut process));
            // Never act on Typr itself (its settings and overlay are not editing targets).
            if process == std::process::id() {
                return Err("Select text in another app before starting Write Mode.".into());
            }
            let process_name = crate::context_detector::process_name_from_pid(process);
            let mut window_class = [0u16; 128];
            let count = GetClassNameW(window, &mut window_class);
            let window_class = String::from_utf16_lossy(&window_class[..count.max(0) as usize]);
            require_editor_surface(&process_name, &window_class, "")?;
            let mut gui = GUITHREADINFO {
                cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
                ..Default::default()
            };
            GetGUIThreadInfo(thread, &mut gui).map_err(err)?;
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER).map_err(err)?;
            if let Ok(v2) = automation.cast::<IUIAutomation2>() {
                v2.SetConnectionTimeout(1500).map_err(err)?;
                v2.SetTransactionTimeout(1500).map_err(err)?;
            }
            let element = automation.GetFocusedElement().map_err(err)?;
            let class = element.CurrentClassName().map_err(err)?.to_string();
            require_editor_surface(&process_name, &window_class, &class)?;
            if element.CurrentIsPassword().map_err(err)?.as_bool() {
                return Err("Write Mode cannot read password fields.".into());
            }
            if !element.CurrentIsEnabled().map_err(err)?.as_bool() {
                return Err("Select text in an editable field.".into());
            }
            if let Ok(value) =
                element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            {
                if value.CurrentIsReadOnly().map_err(err)?.as_bool() {
                    return Err("This text is read-only. Select text in an editor.".into());
                }
            } else if element.CurrentControlType().map_err(err)? != UIA_EditControlTypeId
                && element
                    .GetCurrentPatternAs::<IUIAutomationTextEditPattern>(UIA_TextEditPatternId)
                    .is_err()
            {
                return Err(
                    "This app does not expose an editable text field for Write Mode.".into(),
                );
            }
            let array = element.GetRuntimeId().map_err(err)?;
            if array.is_null() {
                return Err(CHANGED.into());
            }
            let ids: Result<Vec<i32>, String> = (|| {
                let lo = SafeArrayGetLBound(array, 1).map_err(err)?;
                let hi = SafeArrayGetUBound(array, 1).map_err(err)?;
                if hi < lo || hi - lo > 128 {
                    return Err(CHANGED.into());
                }
                let mut ids = Vec::new();
                for i in lo..=hi {
                    let mut id = 0i32;
                    SafeArrayGetElement(array, &i, (&mut id as *mut i32).cast()).map_err(err)?;
                    ids.push(id);
                }
                Ok(ids)
            })();
            let _ = SafeArrayDestroy(array);
            let runtime_id = ids?;
            Ok((
                automation,
                element,
                Target {
                    window: window.0 as usize,
                    process,
                    focus: gui.hwndFocus.0 as usize,
                    runtime_id,
                },
            ))
        }
    }
    fn text_selection(element: &IUIAutomationElement) -> Result<(String, Anchor), String> {
        anchored_selection(element, true)
    }
    fn anchored_selection(element: &IUIAutomationElement, require_text: bool) -> Result<(String, Anchor), String> {
        unsafe {
            let pattern: IUIAutomationTextPattern = element
                .GetCurrentPatternAs(UIA_TextPatternId)
                .map_err(err)?;
            let ranges = pattern.GetSelection().map_err(err)?;
            if ranges.Length().map_err(err)? != 1 {
                return Err("Select one continuous passage to rewrite.".into());
            }
            let range = ranges.GetElement(0).map_err(err)?;
            let text = range.GetText(12001).map_err(err)?.to_string();
            if require_text { validate_length(&text)?; }
            let document = pattern.DocumentRange().map_err(err)?;
            if document.GetText(MAX_DOCUMENT + 1).map_err(err)?.len() > MAX_DOCUMENT as usize {
                return Err("This document is too large for safe in-place editing. Use a smaller editor document.".into());
            }
            let before = document.Clone().map_err(err)?;
            before
                .MoveEndpointByRange(
                    TextPatternRangeEndpoint_End,
                    &range,
                    TextPatternRangeEndpoint_Start,
                )
                .map_err(err)?;
            let after = document.Clone().map_err(err)?;
            after
                .MoveEndpointByRange(
                    TextPatternRangeEndpoint_Start,
                    &range,
                    TextPatternRangeEndpoint_End,
                )
                .map_err(err)?;
            Ok((
                text,
                Anchor {
                    prefix: before.GetText(MAX_DOCUMENT).map_err(err)?.to_string(),
                    suffix: after.GetText(MAX_DOCUMENT).map_err(err)?.to_string(),
                },
            ))
        }
    }
    pub(super) fn prepare_observation() -> Result<Selection, String> {
        let _com=Apartment::new()?;
        let (_,element,target)=focused()?;
        let (text,anchor)=anchored_selection(&element,false)?;
        Ok(Selection { text, target, anchor:Some(anchor) })
    }
    pub(super) fn observed_passage(marker: &Selection) -> Result<String,String> {
        let _com=Apartment::new()?;
        let (_,element,target)=focused()?;
        if target != marker.target { return Err(CHANGED.into()); }
        let anchor=marker.anchor.as_ref().ok_or(CHANGED)?;
        let document=unsafe {
            let pattern: IUIAutomationTextPattern=element.GetCurrentPatternAs(UIA_TextPatternId).map_err(err)?;
            pattern.DocumentRange().map_err(err)?.GetText(MAX_DOCUMENT+1).map_err(err)?.to_string()
        };
        if document.len()>MAX_DOCUMENT as usize { return Err(CHANGED.into()); }
        let doc=normalized(&document);
        let prefix=normalized(&anchor.prefix); let suffix=normalized(&anchor.suffix);
        if !doc.starts_with(&prefix) || !doc.ends_with(&suffix) || doc.len()<prefix.len()+suffix.len() { return Err(CHANGED.into()); }
        let passage=&doc[prefix.len()..doc.len()-suffix.len()];
        if passage.chars().count()>6000 { return Err(CHANGED.into()); }
        Ok(passage.to_string())
    }
    fn validate_length(text: &str) -> Result<(), String> {
        if text.trim().is_empty() {
            Err("Select some text first, then press the Write Mode hotkey.".into())
        } else if text.chars().count() > 6000 {
            Err("Select a shorter passage (up to 6,000 characters) for Write Mode.".into())
        } else {
            Ok(())
        }
    }
    pub(super) fn capture(previous: Option<&Selection>, cancel: &crate::write_mode::Cancellation) -> Result<Selection, String> {
        let _com = Apartment::new()?;
        let (_automation, element, target) = focused()?;
        cancel.check()?;
        let (text, anchor) = unsafe {
            if element
                .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
                .is_ok()
            {
                let (text, anchor) = anchored_selection(&element, false)?;
                if text.trim().is_empty() {
                    if let Some(previous) = previous.filter(|last| last.target == target && last.anchor.is_some()) {
                        return select_previous(&element, previous, cancel);
                    }
                }
                (text, Some(anchor))
            } else {
                (copy_selection(&target)?, None)
            }
        };
        validate_length(&text)?;
        if focused()?.2 != target {
            return Err(CHANGED.into());
        }
        Ok(Selection {
            text,
            target,
            anchor,
        })
    }
    pub(super) fn replace(
        selection: &Selection,
        text: &str,
        cancel: &crate::write_mode::Cancellation,
    ) -> Result<Option<String>, String> {
        let _com = Apartment::new()?;
        cancel.check()?;
        wait_modifiers()?;
        let (_automation, element, target) = focused()?;
        if target != selection.target {
            return Err(CHANGED.into());
        }
        if let Some(anchor) = &selection.anchor {
            let (current, current_anchor) = text_selection(&element)?;
            if current != selection.text || &current_anchor != anchor {
                return Err(CHANGED.into());
            }
        } else if copy_selection(&target)? != selection.text {
            return Err(CHANGED.into());
        }
        paste_checked(text, selection, cancel)
    }
    fn select_previous(element: &IUIAutomationElement, previous: &Selection, cancel: &crate::write_mode::Cancellation) -> Result<Selection, String> {
        let anchor = previous.anchor.as_ref().ok_or(CHANGED)?;
        unsafe {
            let pattern: IUIAutomationTextPattern = element.GetCurrentPatternAs(UIA_TextPatternId).map_err(err)?;
            let document = pattern.DocumentRange().map_err(err)?;
            let expected = normalized(&format!("{}{}{}",anchor.prefix,previous.text,anchor.suffix));
            let document_text = document.GetText(MAX_DOCUMENT + 1).map_err(err)?.to_string();
            if normalized(&document_text) != expected { return Err(CHANGED.into()); }
            let passage = document_text.strip_prefix(&anchor.prefix).and_then(|s| s.strip_suffix(&anchor.suffix)).ok_or(CHANGED)?;
            let search = document.Clone().map_err(err)?;
            for _ in 0..32 {
                let range = search.FindText(&windows::core::BSTR::from(passage),false,false).map_err(|_| CHANGED)?;
                let before = document.Clone().map_err(err)?;
                before.MoveEndpointByRange(TextPatternRangeEndpoint_End,&range,TextPatternRangeEndpoint_Start).map_err(err)?;
                if normalized(&before.GetText(MAX_DOCUMENT).map_err(err)?.to_string()) == normalized(&anchor.prefix) {
                    cancel.with_active_input(|| {
                        if !native_focus_matches(&previous.target) { return Err(CHANGED.into()); }
                        range.Select().map_err(err)
                    })?;
                    let (text, actual) = text_selection(element)?;
                    if normalized(&text) != normalized(&previous.text) || normalized(&actual.prefix) != normalized(&anchor.prefix) || normalized(&actual.suffix) != normalized(&anchor.suffix) { return Err(CHANGED.into()); }
                    return Ok(Selection { text, target:previous.target.clone(), anchor:Some(actual) });
                }
                search.MoveEndpointByRange(TextPatternRangeEndpoint_Start,&range,TextPatternRangeEndpoint_End).map_err(err)?;
            }
        }
        Err(CHANGED.into())
    }
    // Verify the active selection, including expanded editing responses.
    fn text_selection_unbounded(
        element: &IUIAutomationElement,
    ) -> Result<(String, Anchor), String> {
        unsafe {
            let pattern: IUIAutomationTextPattern = element
                .GetCurrentPatternAs(UIA_TextPatternId)
                .map_err(err)?;
            let ranges = pattern.GetSelection().map_err(err)?;
            if ranges.Length().map_err(err)? != 1 {
                return Err(CHANGED.into());
            }
            let range = ranges.GetElement(0).map_err(err)?;
            let document = pattern.DocumentRange().map_err(err)?;
            let before = document.Clone().map_err(err)?;
            before
                .MoveEndpointByRange(
                    TextPatternRangeEndpoint_End,
                    &range,
                    TextPatternRangeEndpoint_Start,
                )
                .map_err(err)?;
            let after = document.Clone().map_err(err)?;
            after
                .MoveEndpointByRange(
                    TextPatternRangeEndpoint_Start,
                    &range,
                    TextPatternRangeEndpoint_End,
                )
                .map_err(err)?;
            Ok((
                range.GetText(32001).map_err(err)?.to_string(),
                Anchor {
                    prefix: before.GetText(MAX_DOCUMENT).map_err(err)?.to_string(),
                    suffix: after.GetText(MAX_DOCUMENT).map_err(err)?.to_string(),
                },
            ))
        }
    }
    fn wait_modifiers() -> Result<(), String> {
        let start = Instant::now();
        while unsafe {
            [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN]
                .iter()
                .any(|key| GetAsyncKeyState(key.0 as i32) < 0)
        } {
            if start.elapsed() > Duration::from_secs(2) {
                return Err("Release the shortcut keys and try again.".into());
            }
            sleep(Duration::from_millis(10));
        }
        Ok(())
    }
    fn native_focus_matches(target: &Target) -> bool {
        unsafe {
            let window = GetForegroundWindow();
            if window.0 as usize != target.window {
                return false;
            }
            let mut process = 0;
            let thread = GetWindowThreadProcessId(window, Some(&mut process));
            let mut gui = GUITHREADINFO {
                cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
                ..Default::default()
            };
            GetGUIThreadInfo(thread, &mut gui).is_ok()
                && process == target.process
                && gui.hwndFocus.0 as usize == target.focus
        }
    }
    struct Clipboard;
    impl Clipboard {
        fn open() -> Result<Self, String> {
            for _ in 0..20 {
                if unsafe { OpenClipboard(Some(GetForegroundWindow())) }.is_ok() {
                    return Ok(Self);
                }
                sleep(Duration::from_millis(10));
            }
            Err("The clipboard is busy. Try Write Mode again.".into())
        }
    }
    impl Drop for Clipboard {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseClipboard();
            }
        }
    }
    struct SavedClipboard {
        formats: Vec<(u32, HANDLE)>,
        sequence: Option<u32>,
    }
    impl SavedClipboard {
        fn capture() -> Result<Self, String> {
            let _open = Clipboard::open()?;
            let mut saved = Self {
                formats: Vec::new(),
                sequence: None,
            };
            let mut format = 0;
            let mut bytes = 0;
            unsafe {
                loop {
                    format = EnumClipboardFormats(format);
                    if format == 0 {
                        break;
                    }
                    let source = GetClipboardData(format).map_err(|_| {
                        "Could not preserve the clipboard. Write Mode stopped before changing it."
                    })?;
                    let duplicate = match format {
                        3 | 0x80 | 0x83 => return Err("The clipboard contains an unsupported format. Copy plain text before using Write Mode.".into()),
                        14 | 0x8e => HANDLE(CopyEnhMetaFileW(HENHMETAFILE(source.0), None).0),
                        2 | 9 | 0x82 => OleDuplicateData(source, CLIPBOARD_FORMAT(format as u16), GMEM_MOVEABLE),
                        _ => {
                            let size = GlobalSize(HGLOBAL(source.0)); bytes += size;
                            if size == 0 || bytes > 64 * 1024 * 1024 { return Err("The clipboard could not be preserved safely. Copy plain text before using Write Mode.".into()); }
                            OleDuplicateData(source, CLIPBOARD_FORMAT(format as u16), GMEM_MOVEABLE)
                        }
                    };
                    if duplicate.0.is_null() {
                        return Err(
                            "Could not preserve the clipboard. Your selected text was kept.".into(),
                        );
                    }
                    saved.formats.push((format, duplicate));
                }
            }
            Ok(saved)
        }
        fn restore(&mut self) -> Result<(), String> {
            let Some(sequence) = self.sequence else {
                return Ok(());
            };
            let _open = Clipboard::open()?;
            unsafe {
                if GetClipboardSequenceNumber() != sequence {
                    self.sequence = None;
                    return Ok(());
                }
                EmptyClipboard().map_err(err)?;
                self.sequence = None;
                let mut failed = false;
                for (format, handle) in &mut self.formats {
                    if SetClipboardData(*format, Some(*handle)).is_ok() {
                        *handle = HANDLE::default(); // Ownership transferred to Windows.
                    } else {
                        failed = true;
                    }
                }
                if failed {
                    return Err("The clipboard could not be fully restored.".into());
                }
            }
            Ok(())
        }
    }
    impl Drop for SavedClipboard {
        fn drop(&mut self) {
            let _ = self.restore();
            unsafe {
                for (format, handle) in &self.formats {
                    if handle.0.is_null() {
                        continue;
                    }
                    match *format {
                        2 | 9 | 0x82 => {
                            let _ = DeleteObject(HGDIOBJ(handle.0));
                        }
                        14 | 0x8e => {
                            let _ = DeleteEnhMetaFile(Some(HENHMETAFILE(handle.0)));
                        }
                        _ => {
                            let _ = GlobalFree(Some(HGLOBAL(handle.0)));
                        }
                    }
                }
            }
        }
    }
    fn key_chord(key: enigo::Key) -> Result<(), String> {
        use enigo::{Direction, Enigo, Key, Keyboard, Settings};
        let mut input =
            Enigo::new(&Settings::default()).map_err(|_| "Could not send the editing shortcut.")?;
        input
            .key(Key::Control, Direction::Press)
            .map_err(|_| "Could not send the editing shortcut.")?;
        let result = input.key(key, Direction::Click);
        let released = input.key(Key::Control, Direction::Release);
        result
            .and(released)
            .map_err(|_| "Could not send the editing shortcut.".into())
    }
    fn copy_selection(target: &Target) -> Result<String, String> {
        wait_modifiers()?;
        let mut saved = SavedClipboard::capture()?;
        {
            let _open = Clipboard::open()?;
            unsafe {
                EmptyClipboard().map_err(err)?;
                saved.sequence = Some(GetClipboardSequenceNumber());
            }
        }
        if focused()?.2 != *target {
            return Err(CHANGED.into());
        }
        if !native_focus_matches(target) {
            return Err(CHANGED.into());
        }
        key_chord(enigo::Key::C)?;
        let start = Instant::now();
        while unsafe { GetClipboardSequenceNumber() } == saved.sequence.unwrap()
            && start.elapsed() < Duration::from_millis(700)
        {
            sleep(Duration::from_millis(10));
        }
        let sequence = unsafe { GetClipboardSequenceNumber() };
        if sequence == saved.sequence.unwrap() {
            return Err("No selected text was copied. Highlight text and retry Write Mode.".into());
        }
        {
            let owner_process = match unsafe { GetClipboardOwner() } {
                Ok(owner) => {
                    let mut process = 0;
                    unsafe { GetWindowThreadProcessId(owner, Some(&mut process)); }
                    Some(process)
                }
                Err(error) if error.code().0 == 0 => None,
                Err(error) => return Err(err(error)),
            };
            if !super::copy_origin_allowed(owner_process, target.process)
                || !native_focus_matches(target) {
                // A different application copied something while Ctrl+C was in
                // flight. Keep its newer clipboard rather than claiming ownership.
                return Err(CHANGED.into());
            }
        }
        saved.sequence = Some(sequence);
        let text = {
            let _open = Clipboard::open()?;
            unsafe {
                if GetClipboardSequenceNumber() != sequence { return Err(CHANGED.into()); }
                let handle = GetClipboardData(13).map_err(|_| {
                    "No selected text was copied. Select text in an editor and retry."
                })?;
                let global = HGLOBAL(handle.0);
                let length = GlobalSize(global) / 2;
                if length > 12002 {
                    return Err("Select a shorter passage for Write Mode.".into());
                }
                let pointer = GlobalLock(global) as *const u16;
                if pointer.is_null() {
                    return Err("Could not read the selected text.".into());
                }
                let data = std::slice::from_raw_parts(pointer, length);
                let end = data.iter().position(|v| *v == 0).unwrap_or(length);
                let text = String::from_utf16_lossy(&data[..end]);
                let _ = GlobalUnlock(global);
                text
            }
        };
        saved.restore()?;
        if focused()?.2 != *target { return Err(CHANGED.into()); }
        Ok(text)
    }
    fn paste_checked(
        text: &str,
        selection: &Selection,
        cancel: &crate::write_mode::Cancellation,
    ) -> Result<Option<String>, String> {
        cancel.check()?;
        if text.contains('\0') {
            return Err("The AI edit contained invalid text. Your selection was kept.".into());
        }
        let mut saved = SavedClipboard::capture()?;
        {
            let _open = Clipboard::open()?;
            unsafe {
                let data: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
                let global = GlobalAlloc(GMEM_MOVEABLE, data.len() * 2).map_err(err)?;
                let pointer = GlobalLock(global) as *mut u16;
                if pointer.is_null() {
                    let _ = GlobalFree(Some(global));
                    return Err("Could not prepare the edit.".into());
                }
                std::ptr::copy_nonoverlapping(data.as_ptr(), pointer, data.len());
                let _ = GlobalUnlock(global);
                if let Err(e) = EmptyClipboard() {
                    let _ = GlobalFree(Some(global));
                    return Err(err(e));
                }
                saved.sequence = Some(GetClipboardSequenceNumber());
                if SetClipboardData(13, Some(HANDLE(global.0))).is_err() {
                    let _ = GlobalFree(Some(global));
                    return Err("Could not prepare the edit.".into());
                }
                saved.sequence = Some(GetClipboardSequenceNumber());
            }
        }
        cancel.check()?;
        wait_modifiers()?;
        let (_, element, target) = focused()?;
        if target != selection.target {
            return Err(CHANGED.into());
        }
        if let Some(anchor) = &selection.anchor {
            let (current, position) = text_selection_unbounded(&element)?;
            if current != selection.text || &position != anchor {
                return Err(CHANGED.into());
            }
        }
        // For clipboard-only fields, the selection was copied and verified just
        // before preparing this paste. Never recopy now and replace the edit clipboard.
        cancel.with_active_input(|| {
            if !native_focus_matches(&selection.target) {
                return Err(CHANGED.into());
            }
            key_chord(enigo::Key::V)
        })?;
        sleep(Duration::from_millis(250)); // Let the target consume the clipboard.
        let clipboard_warning = saved.restore().err().map(|_| "The edit was sent, but the clipboard could not be fully restored. Use Ctrl+Z in your editor to undo the edit.".to_string());
        if clipboard_warning.is_some() {
            return Ok(clipboard_warning);
        }
        if let Some(anchor) = &selection.anchor {
            let expected = normalized(&format!("{}{}{}", anchor.prefix, text, anchor.suffix));
            let accepted: Result<bool, String> = (|| {
                let (_, element, current_target) = focused()?;
                if current_target != selection.target {
                    return Err(CHANGED.into());
                }
                let pattern: IUIAutomationTextPattern =
                    unsafe { element.GetCurrentPatternAs(UIA_TextPatternId) }.map_err(err)?;
                let document = unsafe {
                    pattern
                        .DocumentRange()
                        .map_err(err)?
                        .GetText(MAX_DOCUMENT + 1)
                        .map_err(err)?
                }
                .to_string();
                Ok(normalized(&document) == expected)
            })();
            if !matches!(accepted, Ok(true)) {
                return Ok(Some("The edit was sent, but this app did not confirm it. Check the text before retrying. Use Ctrl+Z in your editor to undo the edit.".into()));
            }
        }
        Ok(None)
    }
}
