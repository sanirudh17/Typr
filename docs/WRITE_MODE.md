# Write Mode

Write Mode rewrites selected text in another Windows app from a spoken editing instruction. It is off by default. Enable it under **Recording > Write Mode**, add your Groq API key there, and choose the editing model. The model and key are shared with AI cleanup; editing works even when normal cleanup is off.

## Use

1. Select one continuous passage in a supported editable field.
2. Press **Ctrl+Shift+W**, then say an instruction such as “Make this more polite” or “Shorten this to two sentences.”
3. Press **Ctrl+Shift+W** again to transcribe the instruction and replace the selection. Keep the field focused and the selection unchanged until processing finishes.

The Write shortcut always toggles recording, including when ordinary dictation uses push to talk. It uses the selected speech engine and microphone. The Write shortcut can be changed in Recording settings.

After a successful edit, keep the same editor focused and give a follow-up instruction, such as "Make the second bullet shorter." In editors that expose their document and selection through accessibility, Typr can select the unchanged rewritten passage again for up to five minutes. It uses the current text and previous instruction as context, with the latest instruction taking priority. Moving to another document or changing the passage manually requires a fresh selection.

Write Mode is unavailable in Windows terminals and embedded xterm controls, including Codex TUI. Terminal highlights are screen selections and cannot be replaced reliably across hosts. Typr displays a message before microphone recording starts; it does not copy terminal text or send terminal keystrokes. Use a normal editable field for rewriting. Normal dictation in terminals remains available.

Press **Escape** during capture or processing to cancel. The selected text is kept. To undo a completed edit, press **Ctrl+Z** in the edited app; no highlighting is needed. Undo follows that app’s own undo behavior. Typr has no separate revert shortcut and does not reserve **Ctrl+Alt+Backspace**.

## Safety and limits

- Windows UI Automation reads editor selections first. A **Ctrl+C** fallback is available for otherwise supported editable fields without a text selection pattern. Terminal controls, password fields, read-only fields, unsupported controls, and changed focus or selection are rejected. Compatibility depends on the editor’s accessibility support.
- Selection is limited to **6,000 characters**, the spoken instruction to **2,000 characters**, and the AI response to **16,000 characters**. The AI request times out after **30 seconds**. Documents too large to verify safely are rejected.
- Replacement uses a plain text paste into the existing selection. Typr never sends **Ctrl+A**. Rich text styling is not reconstructed.
- Existing clipboard formats are saved and restored after copy or paste if Typr still owns the clipboard change. A newer user copy is preserved. A busy clipboard or formats that cannot be saved safely can prevent an edit.
- AI failures and invalid or incomplete responses keep the original text. The editing instruction is never pasted as a dictation fallback.

The selected passage and transcribed instruction are sent to Groq. The selected text and rewrite are excluded from dictation history and logs. The most recent successful in-place edit and its instruction are retained only in memory for up to five minutes for follow-up edits, not for undo. The existing recording and diagnostic policy still applies to the spoken instruction and its audio.

## Run this checkout

Exit any running Typr instance before launching the worktree, because Typr uses a single instance window.

```powershell
Set-Location 'C:\Users\sanir\orca\workspaces\Typr\write-mode'
$env:CARGO_TARGET_DIR = 'C:\Users\sanir\Typr\src-tauri\target'
npm run tauri dev
```

## Small manual check

1. Enable Write Mode and configure the key. In Notepad, enter three sentences, copy a separate clipboard marker, and select only the middle sentence. Record “Make this more polite” with the two Write presses. Confirm that only the middle sentence changes, the surrounding text stays intact, and the clipboard marker is restored.
2. Keep the edited field focused and press **Ctrl+Z**, without selecting the rewritten sentence; confirm the app restores the original sentence. Start another edit, press Escape, and confirm no replacement occurs.
3. Use the usual dictation shortcut in an editable field and confirm ordinary dictation still works with the configured recording mode.

## Implementation and validation

| Source | Responsibility |
| --- | --- |
| `src-tauri/src/write_mode.rs` | Editing prompt, Groq request, response limits, cancellation |
| `src-tauri/src/selection.rs` | Windows selection capture, focus and document checks, clipboard preservation, checked replacement |
| `src-tauri/src/recorder.rs` | Instruction recording and transcription, editing session, completion and cancellation |
| `src-tauri/src/main.rs`, `settings.rs` | Persisted defaults, shortcut registration and routing |
| `index.html` (`#write-mode-settings`), `src/main.ts` | Recording settings, shortcut capture, shared model controls, status wiring |

Recorded checks for the feature worktree: TypeScript `tsc --noEmit`, Vite production build, `cargo check`, and `cargo build --bin typr` succeeded. The built frontend was previewed with mocked IPC at **1160×720** and **700×500** in light and dark themes, with no page errors or horizontal overflow. The latest visual review covered Write Mode at **700**, **1366**, and **1920** pixel widths in both themes. All four engine panels and the other settings pages were also checked at **700**, **1160**, **1366**, and **1920** pixel widths without horizontal overflow. The settings reuse the incumbent controls and theme tokens; engine labels and toggle groups accommodate narrow windows.

The initial Write Mode `cargo test --lib -j2 --quiet` run completed with **423 passed, 0 failed, and 2 ignored**. Three focused Write Mode tests passed: cancellation blocks keyboard input, a late worker cannot send input after cancellation, and a legacy revert setting is ignored and is not saved again. The ignored checks are opt-in evaluations; the new Groq cleanup replay was also run explicitly and passed. For the subsequent CPU packaging, memory, and follow-up editing changes, see [the resource and Write Mode review](CPU_AND_WRITE_MODE_REVIEW.md).

Native checks confirmed that **Ctrl+Z** restored the entire original test document after a two-line bullet paste in Chrome textarea and contenteditable fields and Edge textarea and contenteditable fields. These four cases were verified against the document contents. Notepad also restored the entire original probe document, with line endings normalized for comparison. Word was not verified because its automation startup stalled. Compatibility still depends on the target app’s editable control and undo behavior.

A real microphone-to-Groq Write edit has not been run by the agent during this validation. The user confirmed that layout, real typo rewriting, native Undo, Escape cancellation, and ordinary dictation all worked. Those reports are separate from the automated and native checks above. Three fixed ordinary-cleanup cases from the latest Nemotron log were replayed against Groq; see [the cleanup review](nemotron-cleanup-review.md) for results and limits.
