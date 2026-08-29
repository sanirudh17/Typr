# Changelog - Typr

This changelog documents the major engineering milestones and features added during the recent development sprint of **Typr**.

---

## [v0.1.6] - 2026-08-29 — Unified Developer, Punctuation-Aware Terminal & Filler Control

### 1. Developer Unification — Accurate, Not Terse
* **One Common Developer:** `CONTEXT_DEVELOPER` is now accurate, not terse — keeps every meaningful detail, fixes spelling/caps/punct, but never condenses or deletes content to be shorter. The same accurate style is used for IDEs and for prose dictated inside a terminal. The old terse `Concise and direct...` that mis-interpreted `quick overlay button` and short-ened prose is gone. `Never join`/`Never wrap`/`Never prepend git` guards remain.
* **Terminal Is Now Punctuation-Aware:** `CONTEXT_TERMINAL` (Windows Terminal/cmd/PowerShell) previously said `NEVER add a trailing period, NEVER capitalize` for everything, so prose like `ship the unification...` pasted as `ship the unification...` lowercased with no stops. Now: `if it looks like a command (git/npm/slash/flags) → preserved EXACTLY, dash→-, slash→/, NEVER caps/period; else if prose/sentences → normal caps + punctuation`. Fixes `lacks punctuation and full stops in everywhere and also capitalizations` in Windows Terminal and fixes the hallucination `I said the things in quotations only` → `git I said...` (added `NEVER prepend git/npm/cargo if not said` + quoted-text verbatim guard).
* **No More Phantom `git` Prefix:** Both prompts now carry `NEVER prepend a command like "git"/"npm"/"cargo" if the user did not say it` with the `I said the things in quotations only must not become git...` example. Covers the `randomly added git commit in front` regress.

### 2. Filler Always Stripped When AI Is On — Now Configurable
* **Filler Never Leaks:** When `AI cleanup is on`, filler (`um/uh/er/ah/hmm`, `you know`/`I mean`, repeated words) is now stripped deterministically even if the LLM is bypassed (voice-command `dash`/`slash` paths) or fails/times out. Previously terminal fallback kept filler raw, so `um git status` pasted the `um`.
* **Developer Toggle:** `Settings → AI → Advanced → Developer → Strip filler words` (`Keep`/`Strip`, default `Strip`) backed by `developerStripFiller` in `config.json` (`src-tauri/src/settings.rs`, `src-tauri/src/cleanup.rs:strip_filler_words`, `src-tauri/src/recorder.rs`). When off, the old raw fallback is kept.
* **Terminal Detection Hardened:** `is_terminal_focus` now checks `is_terminal_process || is_native_terminal_class` (`src-tauri/src/recorder.rs`, `src-tauri/src/context_detector.rs:ConsoleWindowClass` etc.), so a `Herdr` multiplexer inside `WindowsTerminal.exe` (which shows only `Comet`/`Windows Terminal` in the running-apps picker) is still correctly `Developer + terminal focus`.

### 3. UI Polish
* **Model Labels:** `Qwen 3.6 27B` now reads `Legacy · Qwen 3.6 27B` like the other three options (`Recommended · Qwen 3.8 27B`, `Fast · gpt-oss-20b`, `Quality · gpt-oss-120b`) in `index.html` (`src/main.ts:AI_MODELS`).
* **Dropdown At Top Of Page:** `positionCustomPanel` measured `panel.scrollHeight` while `hidden` as `0 → maxH 240`, so a 4-item panel flipped and clamped to `top: 8px` (the `dropdown at the top of the page` bug). Now it temporarily reveals the panel off-screen to measure the real `neededH` and flips only when `spaceBelow < neededH` (`src/main.ts`).
* **Engine Warning Note:** `src/style.css:.settings-warning-note` was `color: var(--text-secondary)` on a `0.06` yellow wash with a `2px` left accent — barely visible and uneven. Now `color: var(--text)` on `0.10` wash with uniform `1px` border.

### 4. Models
* **Qwen 3.8 Default:** `qwen/qwen3.8-27b` is now the default AI model (`src-tauri/src/ai_postprocess.rs:resolve_model`, `src/main.ts:AI_MODEL_DEFAULT`), with `qwen/qwen3.6-27b` kept selectable; Groq `llama-3.x` decommissioned keys fall back to Qwen.
* **Cloud Models:** `Fast (Turbo)` / `Accurate` still available via `openai/gpt-oss-20b` / `120b`.

---

## [v0.1.5] - 2026-08-27 — Updater Etiquette: Once-Only Banner & Panel-Only Checks

### 🔔 1. Update Banner Shown Exactly Once Per Release
* **Single Launch Prompt**: The titlebar update banner ("Typr X is available." with **Update**/**Later**) now appears only on the check that discovers a new version at app launch (`src/main.ts:runUpdateCheck`). Once dismissed with **Later**, it never re-appears for that version — on restarts either, via the persisted `dismissedUpdateVersion` setting.
* **Per-Version, Not Forever**: Dismissal is an equality check, so saying **Later** to 0.1.4 still allows the 0.1.5 banner through — silence only ever applies to the exact version refused.

### ⚙️ 2. Settings Checks Answered In-Panel Only
* **No Pop-Up From The Button**: Triggering **Check for updates** in General → Updates now clears any visible banner and reports its result solely as the panel's status line + **Download & install** button (`src/main.ts:runUpdateCheck`). A launch check that lands while the user is already viewing the Updates panel is likewise suppressed there instead of popping the titlebar banner.
* **Race Eliminated**: A shared in-flight check (`checkForUpdateOnce`) means a button click during the still-running startup check joins the original request instead of firing a second concurrent GitHub query — the panel and banner can no longer disagree about what was found.

### 🧹 3. Release Infrastructure
* **Manifest Hygiene**: `latest.json` is written without a UTF-8 BOM after a byte-order mark slipped into the v0.1.4 manifest and broke the updater's JSON decode ("error decoding response body") until it was replaced; the release pipeline now keeps the manifest BOM-free.

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

