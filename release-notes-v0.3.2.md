# Typr 0.3.2

Quiet-speech handling, final-word capture, bounded cloud requests, independent Write Mode models, and recording-overlay recovery.

## Quiet speech and recognition

- Shared input handling benefits cloud recognition and the local speech engines. Stereo downmixing detects severe channel cancellation and uses the stronger channel when averaging would erase the signal.
- Adaptive normalization has more headroom for quiet input while retaining the same ordinary speech target. Fixed input gain remains 12 dB; the update does not indiscriminately amplify every recording.
- Brief words inside long pauses no longer use only the recording's 95th-percentile frame level as their gain reference. A sustained speech reference prevents the voice from being amplified excessively when most of the recording is quiet. Samples and pauses are preserved.
- A bundled local CPU speech detector checks a normalized analysis copy before recognition. It uses a balanced speech-probability threshold, preserves the recognition audio, and does not blacklist words such as "you", "thank you", or "bye".
- Cloud recognition uses segment confidence as a second check for probable non-speech. It rejects a result only when every segment has both high no-speech probability and low recognition confidence; individual uncertain words are not removed.
- Cloud and Parakeet skip a decode chunk only when both low energy and local neural analysis agree it contains no speech. This prevents quiet gaps inside a longer dictation from creating extra text or aborting Parakeet. Detection of quiet speech or an analysis error keeps the chunk.
- Nemotron batch finalization now appends a complete fresh decoder-flush interval. Previously, subtracting silence already present in a prepared WAV could lose a final word. The live path retains its full flush and drains unread microphone samples before finishing.
- Dictionary hints remain contextual spelling guidance. Whisper recognition prompts remain disabled because of previously observed omissions; Parakeet and Nemotron do not support recognition hotwords in the current decoders. This release does not add unconditional dictionary replacements.

## Cloud latency and cleanup

- Cloud transcription has one deadline covering upload, chunks, response parsing and retries: eight seconds for audio up to approximately thirty seconds, increasing for larger recordings to a maximum of thirty seconds. A single retry shares that budget.
- Failed cloud recognition reports an error instead of pasting a partial result. AI cleanup retains its existing fidelity, negation and final-word protections and falls back to deterministic cleanup if an unsafe rewrite is rejected.
- Raw formatting preserves aligned line breaks, blank lines and tabs. If a multiline correction cannot be aligned safely, the source layout and content are retained.
- Cleanup prompts retain intelligent paragraphs and lists, ordinary word boundaries, and protection against unsolicited quotation marks and identifier casing.

## Independent Write Mode models

- AI cleanup and Write Mode have separate model selections and saved settings.
- Existing installations inherit the current cleanup model for Write Mode once during migration. The choices can then be changed independently.
- Both selectors use dynamically discovered Groq text models and the existing custom dropdown styling.

## Correction learning

- Windows accessibility observation can resolve an editable browser container when focus lands on a child paragraph or text node.
- Value-only inputs can be observed when a before/after comparison proves one unique insertion in the same field.
- Temporary accessibility interruptions are retried for up to four seconds. Passwords, terminal surfaces, unrelated fields and unsupported editors remain excluded.
- Suggestions still require explicit confirmation. Direct additions through Dictionary remain available when a third-party editor does not expose a usable accessibility interface. Support is not universal across browser editors.

## Recording overlay and diagnostics

- The recording pill periodically recovers authoritative state after reload or resume. A heartbeat lets the watchdog recover an unresponsive overlay.
- Pill placement accounts for monitor origins and scaling, including displays positioned to the left of the primary monitor.
- Optional raw-audio diagnostics now retain stopped recordings before capture-error handling, including recordings rejected as silent or too short. Enable with TYPR_REVIEW_AUDIO=1; the WAVs remain on this computer under the app's review-audio folder. Retention is off by default.

## Installation and limits

- The Windows installer continues to include CPU Whisper support. Speech models and optional NVIDIA support remain separate downloads; no GPU package or S1-mini model is bundled.
- Updating preserves settings, history, dictionary entries, snippets and downloaded speech models. Users on 0.3.0 or 0.3.1 can use the in-app updater. Users on 0.2.2 or earlier need a manual installer because the signing key changed in 0.3.0.
- Recognition quality still depends on the model, microphone, pronunciation and room noise. These fixes do not guarantee perfect transcription or complete removal of every noise hallucination.
- GPU inference and live GPU-process removal were not retested. Updater signatures authenticate the update artifact; they are separate from Windows Authenticode signing.
