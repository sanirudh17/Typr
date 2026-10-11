# Typr 0.3.0

Write Mode, confirmed dictionary learning, dynamic Groq models, and dictation reliability improvements for Windows.

## Installation and updater migration

- Download `Typr_0.3.0_x64-setup.exe` for the usual Windows installation, or `Typr_0.3.0_x64_en-US.msi` for an MSI installation.
- If you are upgrading from 0.2.2 or earlier, download and run this installer once. The updater signing key has been rotated; those versions cannot verify updates signed with the replacement key. Do not rely on their in-app updater for this upgrade.
- Installing over your existing copy preserves settings, history, dictionary, snippets, and downloaded models. Exit Typr from the tray before installing.
- This release uses the replacement updater key for future updates. Updater signatures authenticate the update bundle; they are separate from Windows Authenticode signing. Windows SmartScreen may still display a warning.

## Write Mode

- Select text in a supported editable Windows field, press `Ctrl+Shift+W`, speak an editing instruction, and press the shortcut again to replace only the selection.
- Enable and configure Write Mode under Recording. Its shortcut can be changed independently of ordinary dictation.
- Rewrite tone, fix typos, shorten text, translate, or request bullet points without copying the passage into a separate AI chat.
- Uses the selected speech engine for the instruction and the shared Groq key and text model for rewriting. Write Mode works even when ordinary AI cleanup is off.
- Press Escape during recording or processing to cancel. A cancelled worker cannot paste a delayed response afterward.
- Uses Windows UI Automation, with a guarded copy fallback where supported. Changed focus, changed selections, password fields, read-only fields, and unsupported editors are rejected rather than overwritten.
- Preserves the clipboard when possible, without replacing a newer copy made by the user. Replacement never selects the entire document.
- Use the target application's normal `Ctrl+Z` to undo a completed edit. No separate revert shortcut or retained edit cache is required.
- Selected text and the rewrite are excluded from dictation history and logs. They are sent to Groq for the requested edit. Plain text replacement does not reconstruct rich text styling.

## Auto-learning dictionary

- Typr can observe corrections to recently inserted dictation in supported editable Windows fields and suggest a preferred spelling.
- A compact, centered confirmation bar asks whether to add the term. Yes saves it; No, Escape, or expiry discards it. The countdown is represented by a thin progress line.
- Every learned entry requires explicit confirmation. Learned spellings and aliases never become automatic global replacement rules.
- Saved entries support preferred spellings, optional heard forms, categories, context notes, enabled status, and usage counts. Existing spelling hints migrate to the updated storage format.
- Frequently used terms receive recognition-hint priority, with reserved space for newly added entries and bounded prompt sizes.
- Local Whisper and Groq Whisper receive recognition hints. Current Parakeet and Nemotron decoders do not support these hotwords; their dictionary guidance is applied during AI cleanup instead.
- AI cleanup receives contextual vocabulary guidance for every speech engine. It must preserve ordinary meanings and use a preferred spelling only when the sentence supports it.
- Correction observation and pending suggestions stay local. Confirmed vocabulary, aliases, and notes accompany existing cloud transcription and AI cleanup requests.
- Learning and cloud vocabulary hints are active by default without separate global switches. Individual entries can still be disabled.
- Dictionary management now uses compact rows, search, hover and keyboard actions, and click-to-expand details. Advanced learning controls have improved spacing and disclosure behavior.
- Observation duration, suggestion duration, and maximum phrase length remain configurable. Import/export controls and history word-learning actions were removed from the interface.
- When an editor cannot expose corrections safely, add the term directly in Dictionary. Accessibility support and privilege boundaries vary by application.

## Dynamic Groq text models

- The AI model list is discovered from Groq instead of being limited to a fixed set of model names. AI cleanup and Write Mode share the selected model.
- Discovery refreshes automatically and includes a manual Refresh action. Cached model choices remain usable when discovery is offline.
- Standard text models are separated from speech, safety, embedding, and tool-agent models. Newly listed compatible text models can appear without an app update.
- If a verified live catalog no longer lists the selected model, Typr selects an available replacement and reports the change.
- Refreshes are deduplicated and rate limited. Responses from a previous API key cannot overwrite the current account's choices.
- Refresh and availability messages clear when switching tabs, including late responses. Cooldown text counts down and disappears when it finishes.
- Model-specific reasoning options are sent only for known compatible models. A new model requiring a different API or special parameters may still need an app update.

## Dictation accuracy and AI cleanup

- Strengthened preservation checks for negations, numbers, email addresses, URLs, file paths, code identifiers, complete clauses, and final words.
- Cleanup can retain supported spelling corrections and layout while restoring unrelated unsafe AI edits instead of discarding every valid change.
- Repair and fallback attempts retain the effective mode, vocabulary, tone, formatting, and custom instructions. A visible notification explains when AI cleanup cannot be applied.
- Short cleanup requests share a bounded deadline across primary and alternate model attempts. Longer passages receive a length-dependent allowance, capped at ten seconds. Successful requests return immediately.
- Smaller output allowances for short Qwen requests reduce unnecessary generation overhead. Truncated responses are rejected rather than pasted as complete text.
- Intelligent paragraphs and conditional bullet lists are covered across Cleanup and every Auto profile. Single thoughts are not forced into lists, and complete list items are retained.
- Ordinary phrases stay separate: cleanup must not add invented quotes or join words into code-style identifiers unless explicitly requested.
- Prompt Mode retains its distinct rewriting contract. Literal terminal commands remain protected from prose restyling.
- Recognition and cleanup can still mishear ambiguous words. These safeguards reduce application-caused omissions and unsafe rewrites; they do not guarantee perfect recognition for every speaker or model.

## Audio, local engines, and spelling

- Hardened capture-tail draining, resampling, stream finalization, and chunk-seam handling to protect the end of recordings.
- Healthy long Nemotron recordings finish from the live stream. Short clips use one batch decode rather than a redundant live-plus-batch pass; sparse or failed output retains recovery safeguards.
- Healthy live Nemotron sessions avoid unnecessary WAV preparation. Optional review recordings are saved in the background instead of blocking paste.
- Parakeet retains the measured greedy decoder and bounded CPU thread policy. Its v3 buffered chunk worker remains opt-in because reference recordings showed omissions; it is not described as a native streaming model.
- Whisper keeps its server/batch transcription path. Its complete executable and DLL runtime can be prepared and checked with the repository helper.
- Shared fixed digital gain and bounded decoder normalization are retained. These do not replace hardware microphone controls.
- Recognized individual letters are no longer rejected solely because recording lasted more than a second.
- Explicit spelling commands support ordinary and NATO letter names, such as `spell golf papa uniform india end spelling` for `GPUI`. Commands cannot reconstruct letters the speech engine never recognized.

## Snippets

- A snippet dictated on its own now pastes its saved expansion directly, with AI on or off, skipping an unnecessary cleanup request.
- Standalone matching tolerates surrounding whitespace and speech-engine sentence punctuation while respecting the entry's case setting.
- Literal expansions are protected from filler removal, repeated-word cleanup, voice-command interpretation, and chained snippet expansion.
- Sentence-level snippet matching remains available. Leave case matching off for spoken triggers because engines often capitalize standalone phrases.
- Email and URL expansions retain their saved casing. No user-specific email addresses are bundled with the app.

## Data management and interface fixes

- General now includes Clear data with a confirmation dialog that follows the selected light or dark theme.
- Clears generated history, logs, and eligible recordings while preserving settings, API credentials, dictionary, snippets, and downloaded models.
- Clearing is blocked during active recording or processing and coordinated with pending diagnostic saves. The interface refreshes after completion and reports failures.
- Improved narrow-window and full-screen engine settings layout, reducing clipped controls and horizontal overflow.
- Preserved centered startup and tray restore, hidden WebView startup, theme-aware controls, custom model dropdowns, and failure notifications when the app is hidden.

## Release security and verification

- Removed embedded updater signing credentials, rejected the compromised key, and moved replacement signing credentials outside the repository with Windows-protected password storage.
- Release tooling verifies installer signatures against the configured public key and rejects malformed or tampered signatures. The updater manifest is written as UTF-8 without a byte-order mark.
- Added repository credential checks and signing regression coverage, including UTF-16 exports.
- The Rust library suite passed 473 tests with zero failures and five intentionally ignored external review tests. All eight signing regression tests passed.
- Release checks also cover TypeScript, the production frontend, the Windows application build, local Nemotron/Parakeet reference fixtures, model-picker interactions, and light/dark interface fixtures.
- No live Groq completion requests are needed for the release checks.

## Known limits

- AI cleanup and Write Mode require a Groq connection and are subject to service availability and quotas. Dynamic model availability does not establish model quality.
- Correction observation and in-place rewriting depend on Windows accessibility support; elevated, unsupported, remote, or unusual editor controls may not work.
- Parakeet/Nemotron dictionary guidance requires AI cleanup with the current decoders. Adding a term does not train the speech model or force every occurrence of a similar word to change.
- Streaming recognition does not remove the subsequent cloud cleanup latency. Accuracy and timing vary with the model, hardware, microphone, recording, and network.
