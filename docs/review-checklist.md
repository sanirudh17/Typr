# Dictation review checklist

The remaining automated checks are complete; see [the recorded-audio results](accuracy-evaluation.md). For an optional small follow-up, use [Quick review](quick-review.md). The earlier expanded checks below are historical reference and do not need repeating now.

## Follow-up to your October 8 review

Start with these focused checks; the full matrix below can follow if they pass.

- [ ] In **Notepad, Auto, Qwen**, dictate “Accuracy improvisations and general fixes. Compare Fluid Voice with Nemotron.” Keep `improvisations`; AI must not replace it with `improvements`.
- [ ] Repeat “The picture shows our team taking a shot. Add a quick overlay button.” Keep both clauses and their wording. Compare `STAGE 1` with `STAGE 4`: an incorrect `pick`/`pic` already in Stage 1 is a recognition error.
- [ ] Dictate 30–60 seconds with **Nemotron, AI off**, then with Qwen. Healthy streaming captures should show `nemotron_path=LiveOnly` and `wav_ms=0 wav_saved=false`. Check the final sentence and total time. Comparison mode must be off.
- [ ] If AI rejects an edit for changed negation/content, verify that the log reports `AI skipped ...ms` and uses the complete fallback without retrying GPT-OSS for that fidelity failure.
- [ ] With **Parakeet v3, AI off**, repeat a warm 20–30 second dictation. Compare its `audio_ms`, `engine_ms`, and `total_ms` with your earlier logs, checking seams and the final sentence.
- [ ] Enable the optional audio capture below for one normal-speed and one fast repetition of the same sentence, initially with AI off. Provide the two capture IDs, WAV paths, intended words, and stage/timing logs. This is the next step for measuring the remaining acoustic errors.

Your supplied logs explain the six-second Nemotron case: the live tail took 130 ms, preparing the unused processed WAV took 1,444 ms, and both AI attempts failed fidelity checks. The updated live path avoids that WAV and the second fidelity-failing request. Cloud AI still has a network/model delay, and the 400 ms microphone release grace remains.

## 1. Run the worktree build

These changes are uncommitted in **Accuracy improvisations and general fixes**. The installed 0.2.2 application does not contain them. Quit the installed Typr from its tray menu before running this build, so its single-instance handling does not redirect you to the old app.

In PowerShell:

```powershell
Set-Location 'C:\Users\sanir\orca\workspaces\Typr\Accuracy-improvisations-and-general-fixes'
$env:CARGO_TARGET_DIR = 'C:\Users\sanir\Typr\src-tauri\target'
Remove-Item Env:TYPR_NEMOTRON_COMPARE -ErrorAction SilentlyContinue
Remove-Item Env:TYPR_NEMOTRON_BATCH_VERIFY -ErrorAction SilentlyContinue
Remove-Item Env:TYPR_NEMOTRON_LIVE_PREPROCESS -ErrorAction SilentlyContinue
Remove-Item Env:TYPR_REVIEW_AUDIO -ErrorAction SilentlyContinue
$reviewStart = [DateTimeOffset]::Now
$reviewStart.ToString('o') # Copy this start time for later log collection.
npm run tauri dev
```

Use Notepad as the target for the first pass. Keep the same microphone, distance, and speaking volume. Wait for the selected local model to warm up. Compare repeated warm dictations separately from the first dictation after a model change. Cloud and AI tests use your configured Groq service as usual; AI-off local tests do not require it.

After the first healthy Nemotron dictation longer than 8 seconds, confirm that `typr.log` contains `PERF capture=` and `wav_ms=0 wav_saved=false`. Their absence means you may still be reviewing an older build, or a batch/comparison path was necessary.

### Optional local recordings for the fast-speech review

Quit the test app. In the same worktree PowerShell session, enable capture and restart:

```powershell
$env:TYPR_REVIEW_AUDIO = '1'
npm run tauri dev
```

The switch is **off by default**. While enabled, validated recordings retain an exact 32-bit float WAV at the microphone's original rate and channel count, before gain, filtering, or resampling, in:

```text
%APPDATA%\com.typr.app\review-audio\<capture-id>.wav
```

The `REVIEW AUDIO capture=... raw_wav=...` line identifies the matching file. Existing files are never overwritten. This feature stores audio locally and adds no upload; normal cloud dictation still uses your configured service. Recording files remain until you remove them. Do not use enabled recording capture for baseline timing, since writing the file adds work.

After the two speech-speed samples, quit, disable capture, and restart for timing:

```powershell
Remove-Item Env:TYPR_REVIEW_AUDIO -ErrorAction SilentlyContinue
npm run tauri dev
```

## 2. Accuracy across engines

For each row below, dictate the three phrases **three times with AI off**, then repeat with AI on. Use Cleanup or Auto in Notepad. For AI-on tests, try your small model and GPT-OSS 120B separately; record the selected model. The log records any actual fallback model.

| Engine/model | AI off | AI on, small model | AI on, GPT-OSS 120B |
| --- | --- | --- | --- |
| Cloud Whisper, selected cloud model | [ ] | [ ] | [ ] |
| Local Whisper, selected local model | [ ] | [ ] | [ ] |
| Nemotron 3.5 | [ ] | [ ] | [ ] |
| Nemotron v3, if downloaded | [ ] | [ ] | [ ] |
| Parakeet v3 | [ ] | [ ] | [ ] |
| Parakeet v2, if downloaded | [ ] | [ ] | [ ] |

Phrases:

1. **“Accuracy improvisations and general fixes. Compare Fluid Voice with Nemotron.”** Expected: `accuracy`, `FluidVoice`, and every spoken word retained. Spelling and punctuation can differ where they do not change meaning.
2. **“The picture shows our team taking a shot. Add a quick overlay button.”** Expected: ordinary `picture`, `team`, and `shot` remain unchanged; `quick overlay button` remains three words without added quotation marks.
3. **“The model does not work without post-processing. Keep all final words, including robust decoding algorithms.”** Expected: both negatives and the entire ending retained.

- [ ] Quiet speech remains intelligible; an empty hotkey tap pastes nothing.
- [ ] “You know the answer. I mean the second model.” keeps its meaningful phrases.
- [ ] A dictated question such as “Can you fix this for me?” is transcribed, not answered by AI.
- [ ] Your vocabulary hint `GitHub` turns spoken “git hub” into `GitHub`; ordinary words are not replaced by unrelated hints.
- [ ] Existing snippet replacements, including one with `$` in its expansion, remain literal and do not expand twice.
- [ ] With AI on, “First, check the microphone. Second, check the model. Third, keep the final sentence.” retains all items as a sensible list. Paragraphs, bullets, and numbered lists survive the dictionary pass.
- [ ] Test one natural self-correction; report whether the intended corrected phrase is kept. Fidelity guards can conservatively fall back when the source itself contains contradictory clauses.

## 3. Nemotron delay and endings

Keep comparison mode **off** for timing. The short-clip batch safeguard still applies through **8 seconds**, including the release capture grace; recordings around that boundary may choose different paths. It is not a 10-second threshold.

- [ ] Repeat a 2–4 second sentence three times, AI off. Note hotkey-release-to-paste delay and verify the final word.
- [ ] Repeat sentences around 7, 9, and 12 seconds. Note any noticeable latency jump.
- [ ] Dictate 30–60 seconds, ending “Please retain every final word, including robust decoding algorithms.” Verify the full ending and words at pauses.
- [ ] Release the hotkey immediately after finishing the last word; repeat while finishing the word at release. Verify the capture grace protects it.
- [ ] Repeat the short tests with AI on. Compare `engine_ms` with `AI ok ...ms` and `total_ms` in the log.

Short clips retain processed batch decoding because live and batch normalization differ. The update removes worker sleep and excess padding work and computes fewer FIR samples with bit-identical output. Long healthy streams skip batch WAV preparation. It does not promise zero delay. The 400 ms capture grace remains.

For a separate live-versus-batch diagnostic, quit the test app, then restart it from PowerShell with:

```powershell
$env:TYPR_NEMOTRON_COMPARE = '1'
npm run tauri dev
```

- [ ] Test one short sentence and one 30-second sentence with AI off. Save the corresponding `NEMOTRON COMPARE` lines and the exact words spoken.
- [ ] Wait for the comparison line before starting the next dictation. Diagnostic decoding shares the recognizer and can add latency; do not use these runs for speed comparisons.
- [ ] Quit and remove `Env:TYPR_NEMOTRON_COMPARE` before returning to normal timing tests.

## 4. Parakeet delay

- [ ] With AI off and the model warm, repeat a 2–4 second sentence three times.
- [ ] Repeat a roughly 20-second sentence three times, preserving its final phrase.
- [ ] Dictate 60–90 seconds to check chunk seams, repeated words, and the final sentence.
- [ ] Repeat with AI on and distinguish engine delay from AI delay.
- [ ] If v2 is already downloaded and you dictate English, compare v2 and v3; do not mix their measurements.

The six-thread change is for machines with at least 12 logical CPUs; smaller machines retain up to four threads. Beam width remains eight. The local benchmark showed a gain on the 20-second probe, not a guaranteed improvement for every short clip or machine.

## 5. Regression and signing checks

- [ ] Fresh launch, tray re-show, and launching Typr while hidden still center the window.
- [ ] Model/microphone dropdowns retain their custom dark panels.
- [ ] Failed or silent dictation clears the spinner and reports a useful message, including while in the tray.
- [ ] `node scripts/check-secrets.mjs` reports no embedded signing secrets.
- [ ] `node --test scripts/sign-release.test.mjs` passes.
- [ ] The replacement private credentials remain outside Git; do not attach them or `settings.json` to your feedback.
- [ ] Account for release migration: existing installations trust the leaked old key until a trusted manual installation changes their embedded public key. Do not sign a bridge with the leaked key. No release has been published from this worktree.

## 6. Collect the review session

After quitting the test application, run this from the worktree in a second PowerShell window. Replace the example start time with the one printed in section 1, and choose a new output filename:

```powershell
powershell -NoProfile -File scripts/collect-review-logs.ps1 `
  -Since '2026-10-08T18:00:00+05:30' `
  -OutputFile "$env:USERPROFILE\Downloads\typr-review-logs-1.txt"
```

The default source is `%APPDATA%\com.typr.app\typr.log`. The collector preserves multi-line transcripts, filters out earlier entries, and refuses to overwrite an existing output. Logs contain dictated text: review this session before sharing it.

Paste the relevant `PERF capture=...` lines, `AI ok` / `AI skipped` lines, and `STAGE 1`, `STAGE 4`, and `STAGE 6` text for each failure. Include `NEMOTRON COMPARE` for diagnostic runs. Those stages distinguish recognition mistakes, AI edits, and the final command pass. Only metadata is added by the new performance logging.

For retained recordings, include the `REVIEW AUDIO` line and the exact words intended. A Markdown history export and terminal logs contain text and timing, rather than audio; they cannot measure recognition of the same recording across models.

Feedback template:

```text
Build: Accuracy improvisations and general fixes
Target app / microphone:
Engine and exact model:
AI off/on, selected AI model and profile:
Approximate recording length:
First run after model change, or warm repeat:
Exact words I said:
Text actually pasted:
Capture ID and relevant stage/timing logs:
Release-to-paste delay I observed:
Expected result / thoughts:
Reproduced: __ of __ attempts
```
