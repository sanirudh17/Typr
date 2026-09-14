## [v0.2.0] - 2026-09-14 - Nemotron Engine, Real-Time Streaming, Acoustic Preprocessing & End-to-End Accuracy Architecture

### 1. NVIDIA Nemotron Speech Engine & Real-Time Background Streaming
* **Nemotron 0.6B Streaming Integration:** Added native local speech-to-text powered by NVIDIA's Nemotron-3.5-ASR transducer model, delivering exceptional transcription accuracy on consumer hardware.
* **Continuous Background Transcription:** Audio streams and decodes concurrently on a dedicated background worker thread while recording is in progress, reducing post-recording turnaround latency to sub-second (~79 ms) regardless of dictation duration.
* **Fail-Safe Batch Fallback:** Full audio is captured in parallel; if the live stream ever encounters an interruption or sparse output, the engine automatically verifies and falls back to full offline batch processing with zero speech loss.

### 2. Audio DSP Stabilization & Tail Retention
* **Zero Tail Truncation:** Added 1.0s tail silence padding (16,000 samples at 16 kHz) and complete stream-draining synchronization, ensuring the final clauses of long, spontaneous utterances are fully flushed and never clipped before transducer emission.
* **Acoustic Discontinuity Elimination:** Removed IIR filter resets and jumping block-RMS recalculations during chunk transitions, preventing transient spectrogram distortion.
* **Dynamic Speech Normalization & Soft Limiting:** 50 Hz high-pass filtering eliminates DC drift and desk vibration; adaptive EMA gain continuously normalizes speech to target 0.08 RMS without amplifying background silence; and soft-knee tanh limiting prevents digital clipping.

### 3. AI Post-Processing Dynamic Vocabulary Biasing (Local & Cloud)
* **User Dictionary Injection:** Transducer models (Nemotron and Parakeet) now receive active vocabulary biasing. Stage 4 post-processing automatically injects custom dictionary words (`dictionary.json`) and core technical terms directly into the LLM system prompt.
* **Acoustic Mishearing Recovery:** System prompts actively detect and repair phonetic slips in coding and technical contexts (e.g. sound-alike substitutions, garbled software tools, and domain jargon) based on surrounding context.
* **Intelligent Spoken Shortcut Formatting:** Spoken keyboard hotkeys (e.g. "Control Shift 1 to 4" → "Ctrl+Shift+1 to 4", "Shift plus A" → "Shift+A", "Ctrl plus P" → "Ctrl+P") are automatically formatted as standard developer keybindings across all contexts.
* **Universal Multi-Engine Availability:** Phonetic recovery, vocabulary biasing, and intelligent formatting apply universally across all supported engines: Nemotron, Parakeet, Local Whisper, and Cloud Groq.

### 4. AI Divergence Guard & Refusal Shield
* **Vocabulary Grounding Guard:** Replaced rigid symmetric token multiset matching with directional vocabulary grounding (≥ 25%), ensuring natural speech filler removal, stutters, and spontaneous self-corrections are preserved without false fallback to uncleaned raw audio.
* **Refusal Pattern Interception:** Actively intercepts conversational refusal leaks ("I'm sorry, I cannot comply") and transparently falls back to deterministic cleanup so the user's spoken words are never discarded.

### 5. Developer & Agentic Tool Context Recognition
* **Agentic IDE Support:** Added modern agentic development environments and terminals (`orca.exe`, `opencode.exe`) to the Developer context category, ensuring accurate, code-aware formatting rather than generic prose.
* **Context Snapshot:** Captures the focused foreground application and window class at the exact moment recording begins, ensuring context is preserved even if window focus shifts during dictation.

### 6. Concurrency Hardening & Engine Management
* **Fine-Grained Mutex Scoping:** Redesigned recorder lock hierarchy to release state and audio capture mutexes immediately upon state transition, eliminating lock contention with overlay waveform polling and hotkey handling.
* **VRAM & Model Management:** Model files can be removed directly from disk via the settings selector, and Whisper server child processes cleanly terminate and reclaim VRAM when switching engines.

### 7. Test Suite Expansion
* **319 Passing Tests:** Expanded the automated test suite from 267 to 319 unit and integration tests, verifying real-time streaming chunk ingestion, live session lifecycles, golden accuracy benchmarks, vocabulary injection, and acoustic error correction guards.
