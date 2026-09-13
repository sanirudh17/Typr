## [v0.1.9] - 2026-09-13 - Real-Time Streaming Ingestion, Acoustic Preprocessing & Engine Hardening

### 1. Real-Time Streaming Ingestion for Nemotron
* **Continuous Background Transcription:** The Nemotron engine now continuously streams and decodes microphone audio on a background worker thread while speech is in progress using an OnlineRecognizer transducer stream.
* **Instant Turnaround:** Decoding completes concurrently with speech, reducing post-recording turnaround latency from over 10 seconds to sub-second (~79 ms) for near-instant pasting regardless of dictation duration.
* **Fail-Safe Fallback:** Full audio continues to be captured in parallel. If the live stream yields an empty transcript or encounters an interruption, the engine automatically falls back to full offline batch processing with zero speech loss.

### 2. Real-Time Acoustic Preprocessing & Adaptive RMS Normalization
* **50 Hz High-Pass Filtering:** Removes sub-audible DC drift, desk vibrations, and microphone rumble before acoustic feature extraction.
* **Dynamic Speech Normalization:** An adaptive exponential moving average (EMA) gain tracker continuously normalizes active speech to the target 0.08 RMS level, preventing rapid or unstressed syllables from falling near the noise floor without amplifying background silence.
* **Soft-Knee Limiting:** Integrated soft-knee tanh limiting caps audio peaks at 0.95 to eliminate digital clipping across all volume ranges.

### 3. Concurrency Hardening & Lock Decoupling
* **Fine-Grained Mutex Scoping:** Redesigned recorder lock hierarchy to release state and audio capture mutexes immediately upon state transition, eliminating lock contention with overlay waveform polling and hotkey handling.
* **Resource Reclamation:** Strict Drop implementation and error-path cleanup ensure streaming worker threads are reliably stopped and joined without orphaned processes or memory leaks.

### 4. Download Progress Synchronization & Model Management
* **Linear Easing & IPC Throttling:** Replaced easing transitions with calibrated linear interpolation and throttled progress emissions to ensure the visual progress bar fill remains synchronized with the displayed download percentage.
* **Model Removal from Disk:** Added a dedicated option in the model selector to completely remove downloaded model files from local storage, returning the interface to the clean initial download state.

### 5. Local CPU & GPU Inference Acceleration
* **Thread Contention Elimination:** Clamped CPU thread allocations to prevent core over-subscription on high-thread processors, delivering a 15% to 17% inference speedup.
* **Zero-Overhead Hot Path:** Removed redundant per-dictation token map reloads, eliminated verbose disk logging during transcription, and pre-allocated sample buffers.
* **Latency Reduction:** Shaved 250 ms of dead sleep off every dictation by optimizing WASAPI buffer drain from 250 ms to 100 ms and paste settlement from 150 ms to 50 ms.
* **GPU Whisper Optimization:** Enabled Flash Attention, explicit device binding, and single-pass decoding without temperature fallback stalls for local GPU Whisper pipelines.

### 6. Test Suite Expansion
* **Expanded Coverage:** Test suite expanded from 267 to 315 unit and integration tests, verifying real-time streaming chunk ingestion, live session lifecycles, and golden accuracy benchmarks across all models.
