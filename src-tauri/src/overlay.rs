use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::recorder::RecordingState;

static OVERLAY_READY: AtomicBool = AtomicBool::new(false);
static WATCHDOG_RUNNING: AtomicBool = AtomicBool::new(false);
static CURRENT_STATE: Mutex<(RecordingState, bool)> = Mutex::new((RecordingState::Ready, false));
static PENDING_UPDATE: Mutex<Option<(RecordingState, bool)>> = Mutex::new(None);
static LAST_HEARTBEAT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn clock_ms() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64 + 1
}
pub fn heartbeat() { LAST_HEARTBEAT.store(clock_ms(), Ordering::SeqCst); }
pub fn snapshot() -> serde_json::Value {
    heartbeat();
    let (state, show) = get_current_state();
    serde_json::json!({"state": if !show { "ready" } else { match state {
        RecordingState::Ready => "ready", RecordingState::Recording => "recording",
        RecordingState::Transcribing => "processing",
    }}, "writeMode": WRITE_MODE.load(Ordering::SeqCst)})
}

fn heartbeat_stale(now: u64, last: u64) -> bool { now.saturating_sub(last) > 8_000 }

fn overlay_position(origin: (i32, i32), size: (u32, u32), scale: f64) -> (f64, f64) {
    (origin.0 as f64 + (size.0 as f64 - 300.0 * scale) / 2.0,
     origin.1 as f64 + size.1 as f64 - 160.0 * scale)
}

pub fn is_overlay_ready() -> bool {
    OVERLAY_READY.load(Ordering::SeqCst)
}

pub fn set_overlay_ready(ready: bool) {
    OVERLAY_READY.store(ready, Ordering::SeqCst);
}

pub fn get_pending_update() -> Option<(RecordingState, bool)> {
    PENDING_UPDATE.lock().unwrap().clone()
}

pub fn set_pending_update(update: Option<(RecordingState, bool)>) {
    *PENDING_UPDATE.lock().unwrap() = update;
}

pub fn get_current_state() -> (RecordingState, bool) {
    CURRENT_STATE.lock().unwrap().clone()
}

pub fn set_current_state(state: RecordingState, show_pill: bool) {
    *CURRENT_STATE.lock().unwrap() = (state, show_pill);
}

pub fn is_active_state(state: &RecordingState) -> bool {
    matches!(state, RecordingState::Recording | RecordingState::Transcribing)
}

pub fn reposition_overlay(window: &WebviewWindow) {
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());
    if let Some(m) = monitor {
        let size = m.size();
        let scale = m.scale_factor();
        let origin = m.position();
        let (x, y) = overlay_position((origin.x, origin.y), (size.width, size.height), scale);
        let _ = window.set_position(tauri::Position::Physical(tauri::PhysicalPosition { x: x.round() as i32, y: y.round() as i32 }));
    }
}


pub fn create_overlay_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    if let Some(old) = app.get_webview_window("overlay") {
        let _ = old.destroy();
    }
    let monitor = app.primary_monitor().ok().flatten();
    let (x, y) = if let Some(m) = monitor {
        let size = m.size();
        let scale = m.scale_factor();
        let logical_w = size.width as f64 / scale;
        let logical_h = size.height as f64 / scale;
        ((logical_w - 300.0) / 2.0, logical_h - 160.0)
    } else {
        (810.0, 950.0)
    };

    let builder = WebviewWindowBuilder::new(
        app,
        "overlay",
        WebviewUrl::App("src/overlay.html".into()),
    )
    .title("")
    .inner_size(300.0, 120.0)
    .position(x, y)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .shadow(false);

    let window = builder.build().map_err(|e| e.to_string())?;
    heartbeat(); // Give the new document time to load before the watchdog checks it.
    let _ = window.set_ignore_cursor_events(true);
    set_overlay_ready(false);
    let curr = get_current_state();
    set_pending_update(Some(curr));
    Ok(window)
}

pub fn mark_overlay_ready(app: &AppHandle) {
    println!("[Typr] Overlay readiness handshake received");
    set_overlay_ready(true);
    let pending = {
        let mut guard = PENDING_UPDATE.lock().unwrap();
        guard.take()
    };
    if let Some((state, show_pill)) = pending {
        update_overlay(app, &state, show_pill);
    }
}

pub async fn ensure_overlay_shown_and_visible(app: AppHandle) {
    let delays = [150, 400, 1000];
    let mut is_vis = false;

    for delay in delays {
        if let Some(win) = app.get_webview_window("overlay") {
            let _ = win.set_always_on_top(true);
            reposition_overlay(&win);
            let _ = win.show();
            let _ = win.unminimize();
            if win.is_visible().unwrap_or(false) {
                is_vis = true;
                break;
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
    }

    if !is_vis {
        eprintln!("[Typr] Overlay not visible after retries, recreating window");
        if let Ok(new_win) = create_overlay_window(&app) {
            let _ = new_win.set_always_on_top(true);
            reposition_overlay(&new_win);
            let _ = new_win.show();
            let _ = new_win.unminimize();
        }
    }
}

fn start_watchdog_if_needed(app: &AppHandle) {
    if !WATCHDOG_RUNNING.swap(true, Ordering::SeqCst) {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                let (state, show_pill) = get_current_state();
                if !is_active_state(&state) || !show_pill {
                    break;
                }
                let needs_recreate = match handle.get_webview_window("overlay") {
                    Some(win) => {
                        let _ = win.set_always_on_top(true);
                        reposition_overlay(&win);
                        let _ = win.show();
                        !win.is_visible().unwrap_or(false)
                            || heartbeat_stale(clock_ms(), LAST_HEARTBEAT.load(Ordering::SeqCst))
                    }
                    None => true,
                };
                if needs_recreate {
                    let (curr_state, curr_show) = get_current_state();
                    if is_active_state(&curr_state) && curr_show {
                        eprintln!("[Typr] Watchdog detected invisible/missing overlay, recreating");
                        heartbeat();
                        let main_handle = handle.clone();
                        let _ = handle.run_on_main_thread(move || {
                            let (state, show) = get_current_state();
                            if is_active_state(&state) && show {
                                if let Ok(new_win) = create_overlay_window(&main_handle) {
                                    reposition_overlay(&new_win);
                                    let _ = new_win.show();
                                }
                            }
                        });
                    }
                }
            }
            WATCHDOG_RUNNING.store(false, Ordering::SeqCst);
        });
    }
}

static WRITE_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_write_mode(app: &AppHandle, active: bool) {
    WRITE_MODE.store(active, std::sync::atomic::Ordering::SeqCst);
    use tauri::Emitter;
    let _ = app.emit("write-mode-state", active);
}

pub fn update_overlay(app: &AppHandle, state: &RecordingState, show_pill: bool) {
    let was_active = is_active_state(&get_current_state().0);
    set_current_state(state.clone(), show_pill);

    if show_pill {
        if let Some(win) = app.get_webview_window("overlay") {
            // Re-sync the cached native visibility after sleep/tray transitions.
            if !was_active { heartbeat(); let _ = win.hide(); }
            let _ = win.set_always_on_top(true);
            reposition_overlay(&win);
            let _ = win.show();
            let _ = win.unminimize();
            if !win.is_visible().unwrap_or(false) {
                tauri::async_runtime::spawn(ensure_overlay_shown_and_visible(app.clone()));
            }
        } else {
            let _ = create_overlay_window(app);
            tauri::async_runtime::spawn(ensure_overlay_shown_and_visible(app.clone()));
        }
        start_watchdog_if_needed(app);
    }

    if !is_overlay_ready() {
        set_pending_update(Some((state.clone(), show_pill)));
    }

    if let Some(overlay) = app.get_webview_window("overlay") {
        let pill_state = match state {
            RecordingState::Ready => "ready",
            RecordingState::Recording => "recording",
            RecordingState::Transcribing => "processing",
        };
        let js = format!(
            "if (window.__setPillState) window.__setPillState('{}', {}); else if (document.getElementById('pill')) document.getElementById('pill').style.display = '{}';",
            pill_state,
            WRITE_MODE.load(std::sync::atomic::Ordering::SeqCst),
            if show_pill { "flex" } else { "none" }
        );
        let _ = overlay.eval(&js);
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn placement_respects_secondary_monitor_origin_and_dpi() {
        assert_eq!(overlay_position((-1920, 0), (1920, 1080), 1.0), (-1110.0, 920.0));
        assert_eq!(overlay_position((1920, -100), (2560, 1440), 1.5), (2975.0, 1100.0));
        assert!(!heartbeat_stale(8500, 1000));
        assert!(heartbeat_stale(10000, 1000));
    }

    #[test]
    fn test_active_state_detection() {
        assert!(!is_active_state(&RecordingState::Ready));
        assert!(is_active_state(&RecordingState::Recording));
        assert!(is_active_state(&RecordingState::Transcribing));
    }

    #[test]
    fn test_pending_update_queue_when_not_ready() {
        set_overlay_ready(false);
        set_pending_update(None);
        assert!(!is_overlay_ready());
        assert_eq!(get_pending_update(), None);

        // Queue an update
        set_pending_update(Some((RecordingState::Recording, true)));
        assert_eq!(
            get_pending_update(),
            Some((RecordingState::Recording, true))
        );

        // Mark ready and retrieve
        set_overlay_ready(true);
        assert!(is_overlay_ready());
    }

    #[test]
    fn test_state_tracking() {
        set_current_state(RecordingState::Transcribing, true);
        assert_eq!(
            get_current_state(),
            (RecordingState::Transcribing, true)
        );
        set_current_state(RecordingState::Ready, false);
        assert_eq!(
            get_current_state(),
            (RecordingState::Ready, false)
        );
    }
}
