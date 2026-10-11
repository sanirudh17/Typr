//! Terminal selections are screen content, not replaceable editor ranges. A
//! preview lets the user copy the edit without deleting or submitting a prompt.
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
pub fn show(app: &AppHandle, text: &str, theme: &str) -> Result<(), String> {
    if let Some(previous) = app.get_webview_window("write-preview") { previous.close().map_err(|e| e.to_string())?; }
    let light = match theme { "light" => true, "dark" => false, _ => !matches!(dark_light::detect(), Ok(dark_light::Mode::Dark)) };
    let payload = serde_json::json!({"text":text,"light":light});
    WebviewWindowBuilder::new(app,"write-preview",WebviewUrl::App("src/write-preview.html".into()))
        .title("Typr — Rewrite preview").inner_size(620.0,440.0).min_inner_size(380.0,320.0)
        .decorations(false).always_on_top(true).skip_taskbar(true).center().visible(false)
        .background_color(if light { tauri::window::Color(238,240,243,255) } else { tauri::window::Color(9,9,11,255) })
        .initialization_script(format!("window.__WRITE_PREVIEW__={payload};"))
        .build().map_err(|e| e.to_string())?;
    Ok(())
}
