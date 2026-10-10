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
    #[cfg(windows)]
    {
        native::capture()
    }
    #[cfg(not(windows))]
    {
        Err("Write Mode currently supports Windows.".into())
    }
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
    use super::Selection;
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
        text.replace("\r\n", "\n").replace('\r', "\n")
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
            validate_length(&text)?;
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
    fn validate_length(text: &str) -> Result<(), String> {
        if text.trim().is_empty() {
            Err("Select some text first, then press the Write Mode hotkey.".into())
        } else if text.chars().count() > 6000 {
            Err("Select a shorter passage (up to 6,000 characters) for Write Mode.".into())
        } else {
            Ok(())
        }
    }
    pub(super) fn capture() -> Result<Selection, String> {
        let _com = Apartment::new()?;
        let (_automation, element, target) = focused()?;
        let (text, anchor) = unsafe {
            if element
                .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
                .is_ok()
            {
                let (text, anchor) = text_selection(&element)?;
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
        if sequence != saved.sequence.unwrap() {
            let owner = unsafe { GetClipboardOwner() }.map_err(err)?;
            let mut process = 0;
            unsafe {
                GetWindowThreadProcessId(owner, Some(&mut process));
            }
            if process != target.process || !native_focus_matches(target) {
                // A different application copied something while Ctrl+C was in
                // flight. Keep its newer clipboard rather than claiming ownership.
                return Err(CHANGED.into());
            }
        }
        saved.sequence = Some(sequence);
        let text = {
            let _open = Clipboard::open()?;
            unsafe {
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
