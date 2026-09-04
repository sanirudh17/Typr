## [v0.1.8] - 2026-09-04 — Light Theme, Anti-Flash Bootstrapping & Dictation Guardrails

### 1. Appearance Themes: Light, Dark & System
* **Light Theme Support:** Added full light theme styling across all dialogs, panels, badges, chips, and overlays with careful contrast calibration.
* **Appearance Setting:** New selector in Settings → General allows choosing between `System` (follows OS color scheme, default), `Light`, or `Dark`.
* **Zero Cold-Start Flash:** Eliminates the startup black/white screen flash. Main window is built dynamically with native background seeded at construction (`#09090b` for dark, `#eef0f3` for light) and starts hidden; `__TYPR_BOOT__` initializes CSS variables before DOM parsing; and window reveal is deferred until the post-paint double `requestAnimationFrame` completes.

### 2. Model Catalog Cleanup & VRAM Reclamation
* **Deprecated Model Removal:** Removed `Qwen 3.6 27B` from the UI catalog and automatic config fallbacks, standardizing on `Qwen 3.8 27B` and `gpt-oss-20b/120b`.
* **Engine-Switch Cleanup:** Whisper server child processes now cleanly terminate and reclaim VRAM when switching models or transcription engines.

### 3. Prompt Hardening — Anti-Negation & False-Start Resolution
* **Anti-Negation Guard:** Strictly prohibits post-processing prompts from inverting meaning or inserting/removing negations (e.g. turning "is working" into "is not working").
* **Self-Correction & Hesitation Resolution:** Resolves mid-sentence stutters and immediate corrections (e.g. "the model, the stale model removal") into the single intended phrase instead of duplicating words as list items.

### 4. Context Detection Snapshot
* **Recording-Start Snapshot:** Captures the focused foreground application and window class at the exact moment recording begins, ensuring context is preserved even if window focus shifts during dictation.
