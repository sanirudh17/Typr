# Optional quick review: accuracy, latency, and Clear data

The remaining automated review is complete: saved recordings were compared across all four engine families, and Qwen/GPT-OSS cleanup profiles were checked. See [the results and remaining recognition limits](accuracy-evaluation.md). You do not need to record another model matrix. The smoke check below is optional and takes about **one minute after the app starts**.

**October 9 AI follow-up:** the supplied long test was replayed as text; safe AI punctuation and paragraphs now survive localized rejected edits, with every source word protected. No new recording is required for that fix. Restart the worktree build to use it. Logs distinguish `recovered=true` and `retried=true`; an ultimate AI failure produces a notice and keeps your dictation. See [the focused results](accuracy-evaluation.md#october-9-ai-mode-recovery).

Quit Typr from its tray first. Start the updated worktree build in PowerShell:

```powershell
Set-Location 'C:\Users\sanir\orca\workspaces\Typr\Accuracy-improvisations-and-general-fixes'
$env:CARGO_TARGET_DIR = 'C:\Users\sanir\Typr\src-tauri\target'
Remove-Item Env:TYPR_NEMOTRON_COMPARE -ErrorAction SilentlyContinue
Remove-Item Env:TYPR_NEMOTRON_BATCH_VERIFY -ErrorAction SilentlyContinue
Remove-Item Env:TYPR_NEMOTRON_LIVE_PREPROCESS -ErrorAction SilentlyContinue
Remove-Item Env:TYPR_PARAKEET_BUFFERED -ErrorAction SilentlyContinue
$env:TYPR_REVIEW_AUDIO = '1'
npm run tauri dev
```

Keep `$env:TYPR_REVIEW_AUDIO = '1'` for this latency check. It now saves in the background, so the terminal's `REVIEW AUDIO` path may appear before or after pasted text. Clips stay locally in `%APPDATA%\com.typr.app\review-audio`; the switch adds no upload. Normal AI processing still uses your configured service. Wait for the saved-path line before quitting.

## One optional recording

Use Notepad, **Nemotron 3.5, AI off**, and your normal pace. Keep recording across the entire passage and stop once at the end. Aim for 10–15 seconds so the live path is exercised.

> I am testing the streaming model. The picture shows our team taking a shot. Add a quick overlay button. Keep all final words, including robust decoding algorithms.

- [ ] Text appears promptly after stopping. Note the delay and any missing or incorrect words.
- [ ] Log shows `nemotron_path=LiveOnly`, `wav_saved=false`, and `review_save_ms=... background=true`.
- [ ] The ending and `quick overlay button` survive. Recognition mistakes can still occur; this checks the delay fix without claiming perfect accuracy.

The already-recorded 9.96-second tab question took **1.416 seconds total** with review recording enabled after the fix. That includes AI cleanup and paste. Its final live decode took 223 ms; saving the raw review clip took 52 ms in the background. Your 29.44-second follow-up had a 305 ms final live decode and a 235 ms background save, with only a 2 ms snapshot on the dictation path. These observations replace the earlier 5–17-second file-saving delay; timings depend on CPU load and AI/network latency.

## Clear data: a short check

- [ ] Open **General → Data → Clear data…**. Check the confirmation in Light and Dark (Cancel between themes).
- [ ] Cancel or press Escape: your data remains. Keyboard Tab/Shift+Tab stays in the dialog; Cancel has initial focus.
- [ ] If you want to delete your history and review recordings, choose **Clear data**. Home should show an empty history and zero stats; settings, dictionary, API key, and downloaded models should remain.

Deletion is permanent. I have only exercised deletion against isolated test fixtures, so your current history and recordings have not been cleared. Typr currently stores history in `history.json`, not SQLite. New dictations and logs accumulate again normally afterward. Clearing waits for an active review save and is unavailable during recording/transcription.

Send just the optional transcript/delay and any dialog issue. I can read the matching logs locally; there is no need to export them again.

After testing, quit the app and disable retained recordings before your next normal launch:

```powershell
Remove-Item Env:TYPR_REVIEW_AUDIO -ErrorAction SilentlyContinue
```

The shared fixes apply to Cloud Whisper, local Whisper, Parakeet, and Nemotron. This short check concentrates on your main failures; it does not establish accuracy on every model or microphone.

## Optional Parakeet streaming check

Parakeet v3 buffered streaming is implemented **as an experiment**, with batch decoding kept as the default because it retained more internal words in the long reference. There is no need to repeat the full accuracy matrix. To try streaming, restart the worktree app with:

```powershell
$env:TYPR_PARAKEET_BUFFERED = '1'
npm run tauri dev
```

Select **Parakeet v3, AI off** and dictate normally for at least 30 seconds, ending with `Keep all final words, including robust decoding algorithms`. Stop once. Check the ending and any missing words; the log should show `parakeet_path=Buffered` with `completed_chunks` greater than zero. Short clips still need their first decode after stopping. The final automated replays already checked unread stereo tails, all ten long-clip items, and the final sentence.

After trying it, quit and run `Remove-Item Env:TYPR_PARAKEET_BUFFERED -ErrorAction SilentlyContinue` before restarting to restore the default. This is optional, separate from the one-minute Nemotron smoke check.

## Replay without another recording

The reviewer can compare a retained raw WAV through the installed Nemotron 3.5 and Parakeet v3 models with:

```powershell
Set-Location 'C:\Users\sanir\orca\workspaces\Typr\Accuracy-improvisations-and-general-fixes\src-tauri'
$env:CARGO_TARGET_DIR = 'C:\Users\sanir\Typr\src-tauri\target'
cargo run --example replay_review -- 'C:\Users\sanir\AppData\Roaming\com.typr.app\review-audio\CAPTURE_ID.wav'
```

This uses production batch preprocessing and decoders and prints raw/AI-off transcripts. It reads the dictionary without changing it, makes no cloud calls, and removes its own temporary processed WAV. It does not simulate live capture or measure release-to-paste latency. Cloud and local Whisper are outside this replay tool.
