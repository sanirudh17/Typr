//! Driver for disposable editor fixtures. No clipboard, input injection or cloud.
use std::io::{BufRead, Write};
#[cfg(windows)]
fn focus_fixture(title: &str) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    unsafe extern "system" fn find(window: windows_sys::Win32::Foundation::HWND, arg: isize) -> i32 {
        let title = &*(arg as *const String);
        let mut text = [0u16; 1024];
        let size = GetWindowTextW(window, text.as_mut_ptr(), text.len() as i32);
        if size > 0 && String::from_utf16_lossy(&text[..size as usize]).contains(title.as_str()) {
            ShowWindow(window, SW_RESTORE); SetForegroundWindow(window);
            return 0;
        }
        1
    }
    unsafe {
        let title = title.to_string(); EnumWindows(Some(find), (&title as *const String) as isize);
        std::thread::sleep(std::time::Duration::from_millis(250));
        let mut text = [0u16;1024];
        let size = GetWindowTextW(GetForegroundWindow(), text.as_mut_ptr(), text.len() as i32);
        size > 0 && String::from_utf16_lossy(&text[..size as usize]).contains(title.as_str())
    }
}
fn main() {
    if std::env::var("TYPR_OBSERVER_FIXTURE").as_deref() != Ok("1") { panic!("Set TYPR_OBSERVER_FIXTURE=1 for disposable fixture checks"); }
    let title = std::env::args().nth(1).expect("Fixture title required");
    assert!(title.starts_with("Typr learning fixture "), "Only disposable learning fixtures are supported");
    let mut marker = None;
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        let response = match request["command"].as_str().unwrap_or("") {
            "prepare" => {
                #[cfg(windows)] if !focus_fixture(&title) {
                    println!("{}",serde_json::json!({"ok":false,"error":"Disposable fixture could not acquire foreground focus"}));
                    continue;
                }
                std::thread::sleep(std::time::Duration::from_millis(150));
                match typr_lib::selection::prepare_observation() {
                    Ok(value) => { marker = Some(value); serde_json::json!({"ok":true}) },
                    Err(error) => { marker = None; serde_json::json!({"ok":false,"error":error}) },
                }
            },
            "read" => {
                let inserted = request["inserted"].as_str().unwrap();
                match marker.as_mut().ok_or("No marker".to_string()).and_then(|m|typr_lib::selection::observed_passage(m,inserted)) {
                    Ok(passage) => serde_json::json!({"ok":true,"candidates":typr_lib::vocabulary::corrections(inserted,&passage,12,&[]),"passage":passage}),
                    Err(error) => serde_json::json!({"ok":false,"error":error}),
                }
            },
            "quit" => break,
            _ => serde_json::json!({"ok":false}),
        };
        println!("{response}"); std::io::stdout().flush().unwrap();
    }
}
