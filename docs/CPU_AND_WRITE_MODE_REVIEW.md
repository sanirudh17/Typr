# CPU packaging, background memory, and Write Mode review

These changes were developed on `optional-cuda-and-write-mode-fixes`, based on published v0.3.0, and merged into local `main` after validation. Review installers are unsigned and still carry version 0.3.0. They are not published release or updater artifacts. The GitHub v0.3.0 release retains its original packages.

## Packaging

- CPU is the default Whisper backend, including migration from settings without a hardware preference.
- The bundled runtime contains two executables and twelve matching CPU DLLs: 12,329,984 bytes total. Neither CUDA nor unrelated native inference DLLs are included.
- The MSI file inventory contains fifteen files totaling 55,474,688 bytes before installer compression, with no CUDA files. WebView2 is a separate shared system runtime.
- The final optimized build from merged main produces a 12,169,721-byte EXE installer (12.2 MB) and a 19,873,792-byte MSI (19.9 MB). The previous published EXE was 370,584,087 bytes; the MSI was 686,821,376 bytes. The EXE is approximately 96.7% smaller.
- NVIDIA acceleration requires a separate explicit download in Engine settings: 684,913,404 bytes, with about 1.2 GB extracted on disk. It is stored outside the installation folder and reused across app updates.
- **Delete NVIDIA support** remains available whenever runtime files are present, including when CPU is selected. Removal requires an idle recorder, persists the CPU choice, stops the Whisper server, and removes only the pinned runtime's flat files. Speech models, the bundled CPU runtime, and sibling runtime versions are preserved. Interrupted deletion can be retried. Windows junctions and unexpected folder contents are rejected before deletion.
- Runtime removal was checked with disposable placeholder files and a real Windows junction, without downloading the large CUDA archive again or installing it in the user's app data. The UI's download/removal/error paths were checked with mocked IPC; GPU inference and live GPU-process teardown were not exercised in this pass.
- Both runtime archives are pinned to official whisper.cpp b5454 and checked against their published SHA-256 digests. CPU executable load probes passed. The actual CUDA archive passed checksum verification, allowlisted extraction, completeness checks, and load probes for both executables.
- The CUDA probe verifies runtime loading, not GPU inference speed or compatibility with every NVIDIA driver. CPU execution remains available as recovery.
- The NSIS upgrade hook deletes only known obsolete runtime filenames. It does not delete speech models or user data. An installed MSI upgrade was not exercised during this review.

## Background measurements

The original hidden cloud-engine app and its WebView processes consumed approximately 232 MiB of private memory after startup. A ten-second idle sample showed no measurable CPU use. The UI/WebView background behavior was therefore retained.

During the preceding review, the rebuilt app was launched hidden with the existing cloud-engine configuration and sampled after startup. Its eight processes totaled 274.9 MiB of private memory and no measurable CPU over ten seconds, with no startup errors. This run did not reduce the WebView baseline; WebView allocations varied between launches. The material reduction verified below is the release of retained recognizer weights. The disposable audit instance was stopped afterward without modifying user documents. A repeat sample after the final merge was skipped because an existing Typr process was running; that process was left untouched.

A separate local-only audit loaded the installed Nemotron 3.5 and Parakeet v3 models and exercised the production cache eviction functions:

| Stage | Private memory | Working set |
| --- | ---: | ---: |
| Empty audit process | 2.2 MiB | 8.6 MiB |
| Nemotron loaded | 712.8 MiB | 725.6 MiB |
| Both models loaded | 1,383.0 MiB | 1,393.5 MiB |
| Parakeet selected; Nemotron released | 701.1 MiB | 712.4 MiB |
| Both recognizers released | 18.4 MiB | 31.5 MiB |

These recognizer measurements exclude the app's WebViews and are not total installed-app RAM measurements. Exact use varies with the model, operating system, and capture length.

Inactive recognizers are now released after an engine change. After three minutes without a recording or processing operation, the remaining recognizer can be released while the recorder is ready. The periodic reaper can add up to thirty seconds before eviction. Nonblocking model locks avoid interrupting a model load. A later recording reloads the model; this trades a short cold start for lower idle RAM. Existing Whisper server idle shutdown remains in place.

Long recordings also release excess raw-audio buffer capacity after processing. Capture, preprocessing, decoder beam settings, and tail-preservation behavior are unchanged.

Reproduce the local recognizer audit without recording or cloud requests:

```powershell
cargo run --manifest-path src-tauri/Cargo.toml --example profile_model_memory --no-default-features
```

It requires the installed Nemotron 3.5 and Parakeet v3 models.

## Write Mode

- Follow-up edits use the current passage plus the previous editing instruction. The latest instruction takes priority. The same unchanged passage can be selected again for up to five minutes in supported editors.
- Changed documents, changed fields, changed windows, and cancellation prevent later replacement. Snippet replacements no longer alter the spoken editing instruction.
- Terminal Write Mode support and its preview were removed after the user reported inconsistent behavior in their host. Native terminal processes/classes and embedded xterm controls are rejected before microphone capture or AI processing. Normal editable fields in terminal-hosting apps remain eligible for Write Mode.
- Selection capture no longer shows a processing spinner. Recording UI appears only after an editor selection is accepted. Terminal attempts display a clear message and reset the recorder.
- Editor clipboard fallback requires fresh copied text owned by the focused app. Newer user clipboard copies are preserved. Normal dictation in terminals is retained.

Native checks passed in a disposable RichEdit editor: capture, multiline replacement, follow-up without manually reselecting, changed-document rejection, and cancellation. Separate checks used real xterm.js 6.0.0 in a disposable Chromium window: attempts with and without a terminal highlight were rejected, the clipboard sequence stayed unchanged, and no input reached the terminal.

During the preceding review, two small, serial live requests with the saved AI model also passed. The first request converted disposable text to two bullets while preserving every word. The follow-up changed only the requested phrase in the second bullet, preserving the first bullet and layout. This exercised the production rewrite function with the preceding instruction as context and a conflicting latest instruction taking priority. No user document or audio was sent. The final terminal/removal pass made no additional cloud requests.

Write Mode remains dependent on the editor's accessibility support. A fresh microphone-to-AI follow-up was not run in this pass; native capture/replacement and live AI instruction behavior were verified separately.

## Validation

- TypeScript type checking and Vite production build passed.
- Rust binary checking passed.
- Rust library suite: 488 passed, 0 failed, 6 ignored. Native recognizer and tail-preservation tests passed, as did runtime deletion, partial installation removal, sibling/model preservation, and Windows junction rejection checks.
- The optional CUDA archive integration test was run separately during the preceding review and passed.
- Thirty frontend checks passed with no browser errors, covering CPU defaults, explicit downloading, progress, retry behavior, deletion, CPU fallback, re-download, and hardware controls in both themes at 700 and 1160 pixels.
- The production AI rewrite function passed a live two-request follow-up check using disposable text and the existing key/model. Requests were serialized rather than sent as a batch.
- Optimized NSIS and MSI builds succeeded with signing explicitly disabled for local review.

Native editor check, run serially on an interactive Windows desktop:

```powershell
cargo run --manifest-path src-tauri/Cargo.toml --example write_mode_smoke --no-default-features
```

The test opens its own disposable editor and restores the previous foreground window. It does not call an AI service or modify a user document.

Optional live follow-up check (uses two requests with the configured Groq key):

```powershell
$env:TYPR_LIVE_WRITE_TEST = '1'
cargo run --manifest-path src-tauri/Cargo.toml --example verify_write_follow_up --no-default-features
Remove-Item Env:TYPR_LIVE_WRITE_TEST
```

Implementation references: [Microsoft clipboard owner semantics](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getclipboardowner), [xterm keyboard handling](https://github.com/xtermjs/xterm.js/blob/master/src/common/input/Keyboard.ts), [xterm clipboard handling](https://github.com/xtermjs/xterm.js/blob/master/src/browser/Clipboard.ts).
