# Changelog - Typr

This changelog documents the major engineering milestones and features added during the recent development sprint of **Typr**.

---

## [v0.1.0] - Recent Development & Polishing

### 🚀 1. Hardware-Accelerated Local Whisper Engine (GPU/CUDA Integration)
* **High-Performance Local Inference**: Implemented local Whisper transcription using GPU acceleration, dramatically reducing local voice processing time.
* **Dynamic CUDA/DLL Resolution**: Added automatic dynamic loading of CUDA runtime libraries (including `cublasLt64_12.dll`, `cublas64_12.dll`, `ggml-cuda.dll`, etc.) from the application `binaries/` directory. No system-wide CUDA installation is required anymore.
* **Whisper Server Runner**: Integrated a background local Whisper server runner (`src-tauri/src/whisper_server.rs` and `transcribe_local.rs`) that starts dynamically on launch, enabling extremely low-latency transcription requests via local HTTP sockets.

### ⚡ 2. Instant Voice Dictation Startup
* **Zero-Lag Recording**: Redesigned the audio capture pipeline (`src-tauri/src/recorder.rs` and `audio.rs`) to pre-warm the microphone stream. Dictation starts the exact millisecond the global shortcut is pressed.
* **Global Shortcut Safety**: Solved hotkey race conditions to prevent multiple instances of recording threads. 
* **Optimized Buffer Swapping**: Re-engineered file I/O and WAV writing processes so the transition from speaking to transcribing is completely seamless.

### 📚 3. Custom Dictionary & Vocabulary Fixes
* **Accurate Spelling & Jargon**: Resolved logic issues in the dictionary loader (`src-tauri/src/dictionary.rs`). Custom words, acronyms, names, and industry-specific jargon are now accurately loaded into the transcription context.
* **Smart Word Weighting**: Integrated vocabulary boosting in the local transcription pipeline to ensure custom words are prioritized and spelled correctly the first time.

### 🎨 4. Premium Sidebar UI & Branding Polish
* **Modern Header Refinement**: Revamped the UI sidebar header (`src/style.css` and `index.html`) with a sleeker, darker glassmorphism design.
* **Thematic Icon Branding**: Added customized modern SVG iconography representing key app features (Dictation, History, Dictionary, and Settings).
* **Live Glowing Status Indicator**: Designed an animated, glowing breathing-pill "Ready" status indicator that shifts state gracefully from *Ready* to *Listening* to *Transcribing*.
* **Micro-Animations**: Added hover scales, elegant sliding transitions, and active-state highlights for all main sidebar navigation options.

### 🧹 5. Clean Process Shutdown & Leak Prevention
* **Automated Cleanup Hook**: Created a robust, dedicated lifecycle cleanup module (`src-tauri/src/cleanup.rs`) hooked directly into Tauri's window event listeners.
* **Zombie Process Prevention**: Ensures that if the app is closed, crashed, or exited, any background-running `whisper-server` executables are instantly terminated.
* **Memory & Resource Leak Fixes**: Verified that all open file handles (audio recordings) and audio input streams are cleanly freed and closed upon exit.
