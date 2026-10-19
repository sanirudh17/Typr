//! Ephemeral learning session and suggestion window. No network or clipboard
//! polling. The native observer is isolated in selection.rs.
use crate::{
    settings::Settings,
    vocabulary::{new_id, now_ms, Candidate, Entry},
};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

#[derive(Debug, Clone, serde::Serialize)]
pub struct Suggestion {
    pub id: String,
    pub candidate: Candidate,
    pub created_at: u64,
    pub expires_at: u64,
}
#[derive(Clone, Default)]
pub struct Learning {
    enabled: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    pending: Arc<Mutex<Option<(Suggestion, Instant)>>>,
}
impl Learning {
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }
    pub fn cancel(&self, app: &AppHandle) {
        let mut pending = self.pending.lock().unwrap();
        self.generation.fetch_add(1, Ordering::SeqCst);
        *pending = None;
        if let Some(w) = app.get_webview_window("learning") {
            let _ = w.hide();
        }
        let _ = app.emit("dictionary-suggestion", Option::<Suggestion>::None);
    }
    pub fn pending(&self) -> Option<Suggestion> {
        let mut slot = self.pending.lock().unwrap();
        if slot
            .as_ref()
            .is_some_and(|(_, deadline)| Instant::now() >= *deadline)
        {
            *slot = None;
        }
        slot.as_ref().map(|(s, _)| s.clone())
    }
    #[cfg(test)]
    fn confirm(&self, id: &str) -> Result<Candidate, String> {
        let mut slot = self.pending.lock().unwrap();
        match slot.as_ref() {
            Some((s, deadline)) if s.id == id && Instant::now() < *deadline => {
                Ok(slot.take().unwrap().0.candidate)
            }
            _ => Err("This suggestion expired. Add the term in Dictionary.".into()),
        }
    }
    /// Hold the suggestion until storage succeeds, so a disk error is retryable.
    pub fn accept_with(
        &self,
        id: &str,
        save: impl FnOnce(&Candidate) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut slot = self.pending.lock().unwrap();
        let candidate = match slot.as_ref() {
            Some((s, deadline)) if s.id == id && Instant::now() < *deadline => s.candidate.clone(),
            _ => return Err("This suggestion expired. Add the term in Dictionary.".into()),
        };
        save(&candidate)?;
        *slot = None;
        Ok(())
    }
    pub fn dismiss(&self, id: &str, app: &AppHandle) {
        if self.discard(id) {
            if let Some(w) = app.get_webview_window("learning") {
                let _ = w.hide();
            }
            let _ = app.emit("dictionary-suggestion", Option::<Suggestion>::None);
        }
    }
    fn discard(&self, id: &str) -> bool {
        let mut slot = self.pending.lock().unwrap();
        // pending() may already have removed an expired suggestion. Hide its
        // window anyway, but an old timer must never dismiss a newer proposal.
        if slot.as_ref().map_or(true, |(s, _)| s.id == id) {
            *slot = None;
            true
        } else {
            false
        }
    }
    pub fn observe(
        &self,
        app: AppHandle,
        mut marker: crate::selection::Observation,
        inserted: String,
        settings: Settings,
        known: Vec<Entry>,
    ) {
        if !self.enabled.load(Ordering::SeqCst) {
            return;
        }
        let generation = self.generation.load(Ordering::SeqCst);
        let service = self.clone();
        std::thread::spawn(move || {
            let started = Instant::now();
            let duration =
                Duration::from_secs(settings.learning_observation_seconds.clamp(10, 60) as u64);
            let original = inserted.replace("\r\n", "\n").replace('\r', "\n");
            let mut verified = false;
            let mut last = String::new();
            let mut changed_at = Instant::now();
            let mut first_poll = true;
            let mut last_read = Instant::now();
            while started.elapsed() < duration {
                if !first_poll {
                    std::thread::sleep(Duration::from_millis(500));
                }
                first_poll = false;
                if !service.enabled.load(Ordering::SeqCst)
                    || service.generation.load(Ordering::SeqCst) != generation
                {
                    return;
                }
                let passage = match crate::selection::observed_passage(&mut marker, &original) {
                    Ok(p) => { last_read = Instant::now(); p },
                    // Selection menus and browser rerendering can interrupt one
                    // poll. No foreign field is read; retry this same anchored
                    // target briefly instead of abandoning the whole session.
                    Err(_) if last_read.elapsed() < Duration::from_secs(4) => continue,
                    Err(_) => return,
                };
                // Never infer corrections unless we first verified the actual
                // insertion into this anchored field (UIPI may block paste).
                if !verified {
                    if passage == original {
                        verified = true;
                        last = passage;
                    } else if started.elapsed() > Duration::from_secs(2) {
                        return;
                    }
                    continue;
                }
                if passage != last {
                    last = passage;
                    changed_at = Instant::now();
                    continue;
                }
                if changed_at.elapsed() < Duration::from_millis(1500) || last == original {
                    continue;
                }
                let candidates = crate::vocabulary::corrections(
                    &original,
                    &last,
                    settings.learning_max_phrase_chars as usize,
                    &known,
                );
                if let Some(candidate) = candidates.into_iter().find(|c| {
                    !known.iter().any(|e| {
                        e.canonical_term.eq_ignore_ascii_case(&c.correct)
                            && e.aliases.iter().any(|a| a.eq_ignore_ascii_case(&c.wrong))
                    })
                }) {
                    let mut slot = service.pending.lock().unwrap();
                    if !service.enabled.load(Ordering::SeqCst)
                        || service.generation.load(Ordering::SeqCst) != generation
                    {
                        return;
                    }
                    let seconds = settings.learning_suggestion_seconds.clamp(5, 60);
                    let created_at = now_ms();
                    let suggestion = Suggestion {
                        id: new_id(),
                        candidate,
                        created_at,
                        expires_at: created_at + seconds as u64 * 1000,
                    };
                    *slot = Some((
                        suggestion.clone(),
                        Instant::now() + Duration::from_secs(seconds as u64),
                    ));
                    drop(slot);
                    let _ = app.emit("dictionary-suggestion", Some(suggestion.clone()));
                    let handle = app.clone();
                    let theme = settings.theme.clone();
                    let current = service.clone();
                    let expected = suggestion.id.clone();
                    let _ = app.run_on_main_thread(move || {
                        if current.generation.load(Ordering::SeqCst) == generation
                            && current.pending().is_some_and(|s| s.id == expected)
                        {
                            let _ = show_window(&handle, &theme);
                        }
                    });
                    let deadline = Instant::now() + Duration::from_secs(seconds as u64);
                    while Instant::now() < deadline {
                        std::thread::sleep(Duration::from_millis(250));
                        if service.generation.load(Ordering::SeqCst) != generation
                            || service.pending().is_none()
                        {
                            break;
                        }
                    }
                    service.dismiss(&suggestion.id, &app);
                    return;
                }
            }
        });
    }
}
fn show_window(app: &AppHandle, theme: &str) -> Result<(), String> {
    // Created only when there is a confirmed local observation to review. No
    // activation: showing the suggestion must not steal the editor's focus.
    let w = if let Some(w) = app.get_webview_window("learning") {
        w
    } else {
        WebviewWindowBuilder::new(app, "learning", WebviewUrl::App("src/learning.html".into()))
            .title("Learn a word")
            .inner_size(440.0, 70.0)
            .resizable(false)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible(false)
            .focused(false)
            .initialization_script(format!(
                "window.__LEARNING_THEME__={};",
                serde_json::to_string(theme).unwrap()
            ))
            .build()
            .map_err(|e| e.to_string())?
    };
    let _ = w.emit("learning-theme", theme);
    if let Some(m) = app.get_webview_window("overlay")
        .and_then(|overlay| overlay.current_monitor().ok().flatten())
        .or_else(|| w.current_monitor().ok().flatten())
        .or_else(|| w.primary_monitor().ok().flatten())
    {
        let scale = m.scale_factor();
        let size = m.size();
        let origin = m.position();
        let (x, y) = suggestion_position(origin.x, origin.y, size.width, size.height, scale);
        let _ = w.set_position(tauri::Position::Logical(tauri::LogicalPosition { x, y }));
    }
    w.show().map_err(|e| e.to_string())
}
fn suggestion_position(x: i32, y: i32, width: u32, height: u32, scale: f64) -> (f64, f64) {
    // Same bottom-center band as the dictation pill, including monitor origin
    // and DPI so secondary screens do not shift it onto the primary display.
    (x as f64 / scale + (width as f64 / scale - 440.0) / 2.0,
     y as f64 / scale + height as f64 / scale - 160.0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn suggestion_centers_at_pill_band_across_monitor_scales() {
        assert_eq!(suggestion_position(0, 0, 1920, 1080, 1.0), (740.0, 920.0));
        assert_eq!(suggestion_position(-1920, 0, 1920, 1080, 1.25), (-988.0, 704.0));
        assert_eq!(suggestion_position(2560, 1440, 2560, 1440, 2.0), (1700.0, 1280.0));
    }
    #[test]
    fn confirmation_is_explicit_and_single_use() {
        let l = Learning::default();
        assert!(l.confirm("absent").is_err());
        *l.pending.lock().unwrap() = Some((
            Suggestion {
                id: "one".into(),
                candidate: Candidate {
                    wrong: "super base".into(),
                    correct: "Supabase".into(),
                    confidence: 0.9,
                },
                created_at: now_ms(),
                expires_at: now_ms() + 1000,
            },
            Instant::now() + Duration::from_secs(1),
        ));
        assert!(l.confirm("wrong-id").is_err());
        assert!(l.confirm("one").is_ok());
        assert!(l.confirm("one").is_err());
    }
    #[test]
    fn expired_suggestions_cannot_be_accepted() {
        let l = Learning::default();
        *l.pending.lock().unwrap() = Some((
            Suggestion {
                id: "one".into(),
                candidate: Candidate {
                    wrong: "x".into(),
                    correct: "Xyz".into(),
                    confidence: 0.9,
                },
                created_at: 0,
                expires_at: 0,
            },
            Instant::now() - Duration::from_secs(1),
        ));
        assert!(l.confirm("one").is_err());
        assert!(l.pending().is_none());
    }
    #[test]
    fn failed_save_keeps_suggestion_pending() {
        let l = Learning::default();
        *l.pending.lock().unwrap() = Some((
            Suggestion {
                id: "one".into(),
                candidate: Candidate {
                    wrong: "super base".into(),
                    correct: "Supabase".into(),
                    confidence: 0.9,
                },
                created_at: now_ms(),
                expires_at: now_ms() + 1000,
            },
            Instant::now() + Duration::from_secs(1),
        ));
        assert!(l.accept_with("one", |_| Err("disk full".into())).is_err());
        assert!(l.pending().is_some());
        assert!(l.accept_with("one", |_| Ok(())).is_ok());
        assert!(l.pending().is_none());
    }
    #[test]
    fn old_expiry_cannot_dismiss_new_suggestion() {
        let l = Learning::default();
        assert!(l.discard("expired"));
        *l.pending.lock().unwrap() = Some((
            Suggestion {
                id: "new".into(),
                candidate: Candidate {
                    wrong: "super base".into(),
                    correct: "Supabase".into(),
                    confidence: 0.9,
                },
                created_at: now_ms(),
                expires_at: now_ms() + 1000,
            },
            Instant::now() + Duration::from_secs(1),
        ));
        assert!(!l.discard("expired"));
        assert!(l.pending().is_some());
        assert!(l.discard("new"));
        assert!(l.pending().is_none());
    }
}
