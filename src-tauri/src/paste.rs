pub fn paste_text(text: &str) -> Result<(), String> {
    // Set clipboard with retry loop in case another application temporarily locked the clipboard
    let mut last_err = None;
    for _ in 0..5 {
        match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text)) {
            Ok(_) => {
                last_err = None;
                break;
            }
            Err(e) => {
                last_err = Some(e.to_string());
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
        }
    }
    if let Some(err) = last_err {
        return Err(format!("Failed to set clipboard: {}", err));
    }

    // Increased delay to ensure clipboard is fully propagated on Windows/macOS before pasting (150ms)
    std::thread::sleep(std::time::Duration::from_millis(150));

    // Simulate Cmd+V via osascript (works from any thread, unlike enigo which
    // calls TSMGetInputSourceProperty requiring the main thread)
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("osascript")
            .args(["-e", r#"tell application "System Events" to keystroke "v" using command down"#])
            .output()
            .map_err(|e| format!("Failed to simulate paste: {}", e))?;
    }

    #[cfg(target_os = "windows")]
    {
        use enigo::{Enigo, Keyboard, Settings, Key, Direction};
        let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
        
        // Press Control
        enigo.key(Key::Control, Direction::Press).map_err(|e| e.to_string())?;
        std::thread::sleep(std::time::Duration::from_millis(15));
        
        // Press V
        enigo.key(Key::V, Direction::Press).map_err(|e| e.to_string())?;
        std::thread::sleep(std::time::Duration::from_millis(15));
        
        // Release V
        enigo.key(Key::V, Direction::Release).map_err(|e| e.to_string())?;
        std::thread::sleep(std::time::Duration::from_millis(15));
        
        // Release Control
        enigo.key(Key::Control, Direction::Release).map_err(|e| e.to_string())?;
    }

    Ok(())
}
