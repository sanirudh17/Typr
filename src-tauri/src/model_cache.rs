//! Idle CPU recognizers are evicted only while the recorder's state lock is
//! held. A nonblocking model lock also avoids interrupting a model load.
use std::{sync::{Mutex, OnceLock}, time::{Duration, Instant}};
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(180);
static ACTIVITY: OnceLock<Mutex<Instant>> = OnceLock::new();
pub fn note_activity() { *ACTIVITY.get_or_init(|| Mutex::new(Instant::now())).lock().unwrap() = Instant::now(); }
pub fn expired() -> bool { ACTIVITY.get().is_some_and(|last| last.lock().unwrap().elapsed() > IDLE_TIMEOUT) }
pub fn should_release(selected: &str, cached: &str, idle_expired: bool) -> bool {
    selected != cached || idle_expired
}
pub fn reap(selected: &str) {
    let idle = expired();
    if should_release(selected, "parakeet", idle) { crate::transcribe_parakeet::try_release_model(); }
    if should_release(selected, "nemotron", idle) { crate::transcribe_nemotron::try_release_model(); }
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn keep_only_active_engine_until_idle_timeout() {
        assert!(!should_release("nemotron","nemotron",false));
        assert!(should_release("parakeet","nemotron",false));
        assert!(should_release("cloud","parakeet",false));
        assert!(should_release("parakeet","parakeet",true));
    }
}
