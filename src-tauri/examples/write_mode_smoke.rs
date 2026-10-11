//! Disposable Windows RichEdit fixture: no real document or AI service is used.
//! Run serially on an interactive Windows desktop. Restores the previous window.
#[cfg(windows)]
mod native {
    use std::{io::{BufRead, Write}, time::Duration};
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    use typr_lib::{selection, write_mode::Cancellation};
    #[link(name="kernel32")] extern "system" { fn LoadLibraryW(name: *const u16) -> *mut std::ffi::c_void; }
    fn wide(text: &str) -> Vec<u16> { text.encode_utf16().chain(Some(0)).collect() }
    const ORIGINAL: &str = "Keep the first sentence. Keep the second sentence.";
    const REVISED: &str = "- Keep the first sentence.\r\n- Keep the second sentence.";
    fn fixture() {
        unsafe {
            assert!(!LoadLibraryW(wide("Msftedit.dll").as_ptr()).is_null());
            let parent = CreateWindowExW(0, wide("STATIC").as_ptr(),wide("Typr isolated Write Mode check").as_ptr(),WS_OVERLAPPEDWINDOW|WS_VISIBLE,100,100,620,320,std::ptr::null_mut(),std::ptr::null_mut(),std::ptr::null_mut(),std::ptr::null());
            assert!(!parent.is_null());
            let edit = CreateWindowExW(0,wide("RICHEDIT50W").as_ptr(),wide(ORIGINAL).as_ptr(),WS_CHILD|WS_VISIBLE|WS_TABSTOP|ES_MULTILINE as u32,16,16,560,240,parent,std::ptr::null_mut(),std::ptr::null_mut(),std::ptr::null());
            assert!(!edit.is_null());
            SetForegroundWindow(parent);
            // SetFocus is in KeyboardAndMouse, and SendMessage lets this fixture
            // route focus through the parent's normal WM_NEXTDLGCTL handler.
            windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(edit);
            SendMessageW(edit,0x00b1,0,-1); // EM_SETSEL
            let (send,receive)=std::sync::mpsc::channel();
            std::thread::spawn(move || { for line in std::io::stdin().lock().lines().map_while(Result::ok) { if send.send(line).is_err() { break; } } });
            println!("ready"); std::io::stdout().flush().unwrap();
            loop {
                let mut message: MSG=std::mem::zeroed();
                while PeekMessageW(&mut message,std::ptr::null_mut(),0,0,PM_REMOVE)!=0 { TranslateMessage(&message); DispatchMessageW(&message); }
                if let Ok(command)=receive.try_recv() {
                    match command.as_str() {
                        "select" => { SendMessageW(edit,0x00b1,0,-1); }
                        "caret" => { SendMessageW(edit,0x00b1,usize::MAX,-1); }
                        "change" => { SetWindowTextW(edit,wide("Manually changed. Do not overwrite this.").as_ptr()); SendMessageW(edit,0x00b1,usize::MAX,-1); }
                        "quit" => { DestroyWindow(parent); break; }
                        _ => {}
                    }
                    println!("done"); std::io::stdout().flush().unwrap();
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    pub fn run() {
        if std::env::args().any(|arg| arg=="--fixture") { fixture(); return; }
        if std::env::args().any(|arg| arg=="--terminal-fixture") {
            unsafe {
                // Focus only this harness's disposable window. Browser page
                // focus alone does not make it the native foreground window.
                let mut fixture_window = GetTopWindow(std::ptr::null_mut());
                while !fixture_window.is_null() {
                    let mut candidate = [0u16; 512];
                    let count = GetWindowTextW(fixture_window, candidate.as_mut_ptr(), candidate.len() as i32);
                    if String::from_utf16_lossy(&candidate[..count.max(0) as usize]).contains("Typr terminal fixture") {
                        SetForegroundWindow(fixture_window);
                        std::thread::sleep(Duration::from_millis(250));
                        break;
                    }
                    fixture_window = GetWindow(fixture_window, GW_HWNDNEXT);
                }
                let mut title=[0u16;512]; let count=GetWindowTextW(GetForegroundWindow(),title.as_mut_ptr(),title.len() as i32);
                assert!(String::from_utf16_lossy(&title[..count.max(0) as usize]).contains("Typr terminal fixture"),"Focus the disposable terminal fixture first");
            }
            let selected=selection::capture().unwrap_or_else(|error| {
                unsafe {
                    use windows::{core::Interface, Win32::{System::Com::*,UI::Accessibility::*}};
                    CoInitializeEx(None,COINIT_MULTITHREADED).ok().unwrap();
                    let automation:IUIAutomation=CoCreateInstance(&CUIAutomation8,None,CLSCTX_INPROC_SERVER).unwrap();
                    let element=automation.GetFocusedElement().unwrap();
                    eprintln!("Fixture focus: class={} control={:?} text_pattern={}",element.CurrentClassName().unwrap(),element.CurrentControlType().unwrap(),element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId).is_ok());
                    let _=automation.cast::<IUIAutomation2>(); CoUninitialize();
                    use windows::Win32::System::DataExchange::{GetClipboardOwner, IsClipboardFormatAvailable};
                    eprintln!("Fixture clipboard: owner={:?} unicode={:?}",GetClipboardOwner(),IsClipboardFormatAvailable(13));
                }
                panic!("{error}");
            }); assert!(selected.terminal);
            assert_eq!(selected.text,"A disposable terminal prompt.");
            assert!(selection::replace(&selected,"Must never be pasted",&Cancellation::default()).is_err());
            println!("PASS: xterm field captured via terminal copy; direct replacement blocked");
            return;
        }
        unsafe {
            let previous=GetForegroundWindow();
            let mut child=std::process::Command::new(std::env::current_exe().unwrap()).arg("--fixture")
                .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
            let outcome=std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut input=child.stdin.take().unwrap();
                let mut output=std::io::BufReader::new(child.stdout.take().unwrap());
                let mut line=String::new(); output.read_line(&mut line).unwrap(); assert_eq!(line.trim(),"ready");
                std::thread::sleep(Duration::from_millis(250));
                let cancel=Cancellation::default();
                let selected=selection::capture().unwrap();
                assert_eq!(selected.text.trim(),ORIGINAL);
                assert!(!selected.terminal);
                assert!(selection::replace(&selected,REVISED,&cancel).unwrap().is_none());
                let marker=selected.with_edited_text(REVISED.into());
                writeln!(input,"caret").unwrap(); line.clear(); output.read_line(&mut line).unwrap();
                let repeated=selection::capture_with_previous(Some(&marker),&cancel).unwrap();
                assert_eq!(repeated.text.replace("\r\n","\n").replace('\r',"\n").trim(),REVISED.replace("\r\n","\n"));
                assert!(repeated.same_passage(&marker));
                assert!(selection::replace(&repeated,"Two preserved sentences.",&cancel).unwrap().is_none());
                let latest=repeated.with_edited_text("Two preserved sentences.".into());
                writeln!(input,"change").unwrap(); line.clear(); output.read_line(&mut line).unwrap();
                assert!(selection::capture_with_previous(Some(&latest),&cancel).is_err());
                assert!(selection::replace(&latest,"Must not appear",&cancel).is_err());
                let cancelled=Cancellation::default(); cancelled.cancel();
                assert!(selection::capture_with_previous(Some(&latest),&cancelled).is_err());
                println!("PASS: capture, replacement, follow-up without reselecting, changed-document guard, cancellation");
                writeln!(input,"quit").unwrap();
            }));
            let _=child.kill(); let _=child.wait();
            if !previous.is_null() { SetForegroundWindow(previous); }
            if let Err(payload)=outcome { std::panic::resume_unwind(payload); }
        }
    }
}
#[cfg(windows)] fn main() { native::run(); }
#[cfg(not(windows))] fn main() { eprintln!("This smoke check requires a Windows desktop."); }
