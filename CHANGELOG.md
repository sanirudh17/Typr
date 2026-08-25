# Changelog - Typr

This changelog documents the major engineering milestones and features added during the recent development sprint of **Typr**.

---

## [v0.1.4] - 2026-08-24 — Centered Launch, Unified Dropdowns & AI Formatting Hardening

### 1. Window Always Centered — Fresh Launch & Tray Restore
* **True Center on Every Launch**: Fresh launches and re-shows from the system tray now call `window.center()` (`src-tauri/tauri.conf.json: center:true` + `src-tauri/src/main.rs`) so the 1160×720 window appears in the middle of the current monitor. Previously it drifted to the top-left.
* **Second-Instance Focus**: Launching `Typr.exe` while hidden to tray (single-instance mutex `Local\TyprSingleInstanceMutex`) now finds the existing `"Typr"` HWND and restores/centers it (`FindWindowW` → `ShowWindow(SW_RESTORE)` → `SetWindowPos` at `(screen_w-1160)/2, (screen_h-720)/2` → `SetForegroundWindow`) instead of silently exiting.
* **Background-Mode Safe**: Works whether `background_mode` is on (hide to tray) or off (exit on close).

### 2. Unified Dark Dropdowns — No More Native Grey
* **Custom Select Everywhere**: Replaced every native Windows `<select>` grey list (Mic, Whisper model, Parakeet model, AI model, `auto-context-override`, `app-rule-category`) with a dark themed panel that matches the existing running-apps picker (`src/style.css:.custom-select*` + `src/main.ts:enhanceSelect()`).
* **Fixed Positioning & Keyboard**: Panel is `position:fixed` appended to `body` with flip-above logic, `MutationObserver` for dynamic mic list, `ArrowUp/Down`, `Enter`, `Home/End`, `Escape`, outside-click and `resize`/`scroll` auto-close, and `syncCustomSelectDisplay()` after every programmatic `value = ...`.

### 3. AI Prompt Hardening — No More Phantom Quotes & Smart Layout
* **Developer Mode Fixed**: `CONTEXT_DEVELOPER` no longer joins `"quick overlay button"` into `quickOverlayButton` or wraps it in `"..."`/`backticks`. Now `NEVER join`/`NEVER wrap` unless an explicit casing command (`"camel case ..."` etc.) or a path with separators (`"src slash main dot rs"`). Terse but detail-preserving (`keep every meaningful detail`).
* **Intelligent Paragraphs & Bullets — Automatic, Not Explicit**: `CLEANUP_PROMPT`, `CONTEXT_PROFESSIONAL`, `CONTEXT_EMAIL`, `PROMPT_MODE_NATURAL` now break long/multi-topic dictation into 2–4 short paragraphs and auto-use `"- "` bullets for enumerations, without needing a spoken `"make it a list"` command. Single short thoughts stay plain prose.
* **Anti-Quote Guard Everywhere**: All Auto prompts (`Messaging`, `Email`, `Professional`, `Developer`) carry `NEVER add/wrap quotation marks` and `NEVER join` invariants.
* **Regression Tests**: 4 new `ai_postprocess` tests (`267` total `cargo test --lib` passing) assert the above invariants survive future edits.

### 4. Agent Reference & Reliability
* **AGENTS.md**: New repo-root guide documenting the centered-launch invariant, custom-select invariant, and AI prompt invariants + check commands (`tsc --noEmit`, `vite build`, `cargo test --lib`).
* **Builds Still Clean**: `tsc --noEmit` and `vite build` pass; `cargo test --lib` at 267.

---

## [v0.1.0] - Recent Development & Polishing

### 1. Hardware-Accelerated Local Whisper Engine (GPU/CUDA Integration)
* **High-Performance Local Inference**: Implemented local Whisper transcription using GPU acceleration, dramatically reducing local voice processing time.
* **Dynamic CUDA/DLL Resolution**: Added automatic dynamic loading of CUDA runtime libraries (including `cublasLt64_12.dll`, `cublas64_12.dll`, `ggml-cuda.dll`, etc.) from the application `binaries/` directory. No system-wide CUDA installation is required anymore.
* **Whisper Server Runner**: Integrated a background local Whisper server runner (`src-tauri/src/whisper_server.rs` and `transcribe_local.rs`) that starts dynamically on launch, enabling extremely low-latency transcription requests via local HTTP sockets.

### 2. Instant Voice Dictation Startup
* **Zero-Lag Recording**: Redesigned the audio capture pipeline (`src-tauri/src/recorder.rs` and `audio.rs`) to pre-warm the microphone stream. Dictation starts the exact millisecond the global shortcut is pressed.
* **Global Shortcut Safety**: Solved hotkey race conditions to prevent multiple instances of recording threads. 
* **Optimized Buffer Swapping**: Re-engineered file I/O and WAV writing processes so the transition from speaking to transcribing is completely seamless.

### 3. Custom Dictionary & Vocabulary Fixes
* **Accurate Spelling & Jargon**: Resolved logic issues in the dictionary loader (`src-tauri/src/dictionary.rs`). Custom words, acronyms, names, and industry-specific jargon are now accurately loaded into the transcription context.
* **Smart Word Weighting**: Integrated vocabulary boosting in the local transcription pipeline to ensure custom words are prioritized and spelled correctly the first time.

### 4. Premium Sidebar UI & Branding Polish
* **Modern Header Refinement**: Revamped the UI sidebar header (`src/style.css` and `index.html`) with a sleeker, darker glassmorphism design.
* **Thematic Icon Branding**: Added customized modern SVG iconography representing key app features (Dictation, History, Dictionary, and Settings).
* **Live Glowing Status Indicator**: Designed an animated, glowing breathing-pill "Ready" status indicator that shifts state gracefully from *Ready* to *Listening* to *Transcribing*.
* **Micro-Animations**: Added hover scales, elegant sliding transitions, and active-state highlights for all main sidebar navigation options.

### 5. Clean Process Shutdown & Leak Prevention
* **Automated Cleanup Hook**: Created a robust, dedicated lifecycle cleanup module (`src-tauri/src/cleanup.rs`) hooked directly into Tauri's window event listeners.
* **Zombie Process Prevention**: Ensures that if the app is closed, crashed, or exited, any background-running `whisper-server` executables are instantly terminated.
* **Memory & Resource Leak Fixes**: Verified that all open file handles (audio recordings) and audio input streams are cleanly freed and closed upon exit.

### 6. Seamless Frameless Window & Custom Titlebar Controls
* **Borderless Visual Design**: Removed the native OS window frame to allow the sidebar and main panels to extend to the very top edge of the window.
* **Interactive Window Controls**: Added Windows 11-style Minimize, Maximize/Restore, and Close buttons on the frontend using custom SVGs, layout positioning, and transitions.
* **Maximize/Restore Icon Toggle**: Integrated dynamic resize listeners to detect when the window state changes and swap between the maximize (single square) and restore (double square) icons.
* **Custom Drag Region**: Designed draggable areas across the titlebar and sidebar header while maintaining responsiveness on the window control buttons.

