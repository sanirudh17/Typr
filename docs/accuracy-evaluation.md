# Recorded-audio evaluation — October 8, 2026

Implementation and automated review are complete in **Accuracy improvisations and general fixes**. These are uncommitted worktree changes, not a published update to the installed 0.2.2 application.

Shared capture, vocabulary, dictionary, cleanup, and AI safeguards cover **cloud Whisper, local Whisper, Parakeet, and Nemotron**. The saved recordings establish specific improvements. They do not establish a universal accuracy gain, perfect fast/quiet-speech recognition, or an aggregate word error rate across all models.

## What changed in the final review

- **Capture and final words:** the 400 ms release grace, continuous resampler drain, and Nemotron's 1.25-second closure remain. The Nemotron lifecycle regression now adds the last second after pausing its worker, then verifies that finalization still includes `would like to make`. Long microphone-rate regressions retain `robust decoding algorithms` at both 44.1 and 48 kHz. Missing native final results now return errors rather than successful empty output.
- **Chunk seams:** every skipped word in the shared text merger needs corroboration. A richer overlapping hypothesis can restore extra words, while unique negations, list numbers, and differing valid words survive. Parakeet additionally matches tokens by absolute audio time when their alignment agrees with the complete transcript; uncertain alignment uses conservative text merging. No production decoder blindly trims the first three seconds of an overlapping chunk.
- **Parakeet:** native comparisons found that beam eight silently omitted the overlay and final-word clauses in the recent microphone recordings. Greedy TDT decoding with bounded normalization to speech RMS 0.20 recovered those clauses. Additional amplification is capped at 2.5, followed by the existing limiter. The shared capture target, other engines' audio levels, speech boundaries, and overlap policy remain intact.
- **AI cleanup:** safe spelling, punctuation and layout edits survive localized rejected changes. For example, `Acuracy improvisations` can become `Accuracy improvisations` without accepting `Accuracy improvements`. The October 9 recovery also handles changed sentence counts and added paragraph/list breaks, restoring unsafe edits from the source and revalidating the entire result. Literal tokens stay exact.
- **Negation:** counts are now compared by kind. A missing `not` cannot be hidden by an extra `without`, including in Prompt Mode. Negative contractions and equivalent expanded forms remain accepted. Structured prompts explicitly preserve reported failures rather than turning them into claims or requirements that the failure has been solved. This is a stronger guard, not a proof of complete semantic equivalence.
- **Names:** the reproduced `lemotron` and `limotron` spellings now map narrowly to `Nemotron` across all engines. Explicit dictionary spellings take priority. Ordinary words, paths, and handles retain their protection.
- **Local Whisper:** the checkout's sidecar runtime failed before startup because `whisper.dll` was missing. The worktree now has a complete, checksum-verified official runtime. CPU fallback explicitly passes `-ng`; the shared CPU/CUDA CLI no longer relies on its filename to choose the backend. The original checkout's binaries were preserved. A reproducible setup script is included because binary assets are gitignored.
- **Incomplete chunks:** local Whisper's server and both sidecars now reject an empty transcript for audible audio, including a brief word surrounded by silence. Server/CUDA failures take the existing fallback path; if recovery also fails, the app reports the failure instead of pasting the remaining chunks as a successful complete dictation. Parakeet likewise rejects empty audible chunks. This detects complete chunk loss, not an individual omitted word inside otherwise nonempty text.

## Same-audio comparisons

The three user-owned raw recordings were replayed through the production capture chain, including resampling, fixed +12 dB pre-gain, level adjustment, limiting, and i16 WAV preparation. No new microphone matrix was requested, and app settings/history were not changed.

| Recording | Duration | Findings |
| --- | --- | --- |
| Fast test passage, capture `1791470517639404600` | 16.832 s | Updated Parakeet retains the overlay and final-word clauses that beam eight omitted. `Accuracy` is correct; `new one` and `final buttons` remain wrong. Cloud Whisper accurate/turbo and local Whisper retain the test phrases, including `robust decoding algorithms`. |
| Normal test passage, capture `1791470564381566000` | 18.212 s | Updated Parakeet retains the complete passage, including `quick overlay button` and the final phrase. Both Whisper paths retain those clauses. Local Whisper's `Lemotron` and turbo's `limotron` are covered by the narrow shared correction. |
| Tab question, capture `1791473179301621200` | 9.960 s | Parakeet and both Whisper paths recognize `perform actions`. Nemotron 3.5's batch result still says `pupil actions`; its recorded live result said `pupple actions`. Parakeet retains an opening false start. |

Native Parakeet decode times in the final production replay were **1.351 s, 1.666 s, and 0.757 s**, respectively. Earlier beam runs on the same recordings were approximately **2.273 s, 1.786 s, and 0.999 s**. These are individual decoder measurements, not a latency distribution or release-to-paste comparison.

Cloud checks used `whisper-large-v3` and `whisper-large-v3-turbo`: six requests for the three **prepared test passages**, explicitly sent to the configured Groq service under the request to finish the remaining review. Request times ranged from **279–585 ms**. These times exclude microphone release, preparation, AI cleanup, and paste. The retained raw recordings and private history remain local and unchanged.

Local Whisper checks used one evaluation model, **medium.en-q5_0**, on the RTX 4050 CUDA path, plus an explicit CPU replay of the tab question and an isolated persistent-server replay of all three clips. All retained the main phrases. CPU decoding took about **16.8 s** for the tab clip. Individual CUDA CLI measurements included model loading and ranged from **2.0–7.4 s**; they are not a prediction of the warmed application's latency. Every downloadable Whisper model was not evaluated.

## Nemotron and long-clip limits

The additional English Nemotron variant was downloaded into an evaluation directory and checked on the same three recordings. It recognized `Accuracy` and `perform actions` better, but omitted or misheard other words, including part of the quick-overlay clause and a final `me`. The selected model was not changed because those results do not support an unconditional improvement.

Updated Parakeet also replayed the existing 4-second, 27.855-second, and 79.885-second reference clips. The short sentence and all words in the middle passage survived. The final production replay retains all ten long-clip items and its complete final sentence. Timestamp corroboration removes the duplicated `special language tags` and `Beach engines before deployment` seams. The model still misses `between` in another clause; chunk merging cannot recover a word absent from both hypotheses. Parakeet v2 was not separately evaluated.

The original microphone samples did not show hard clipping or opposing stereo channels. Gain cannot recover information the acoustic model misrecognizes. No global `pupil` → `perform`, `pic` → `quick`, or other valid-word substitution was added to make these particular tests appear perfect.

The final tail/seam pass made **12 default native replays** (Nemotron 3.5 and Parakeet v3 on six saved clips), plus **six complete/incremental buffered Parakeet comparisons**. Short and long reference endings survived. In the normal user passage, Nemotron retained the ending but still recognized `recoding` instead of `decoding`; Parakeet retained `robust decoding algorithms` in both test passages. These recognition errors remain visible rather than being replaced with guessed expected text.

## Streaming latency

The recorded 5–17-second regression was the opt-in raw review WAV writer blocking transcription with one filesystem write per float sample. It now saves exact samples through a buffer on a tracked background thread. Healthy long Nemotron dictations skip unused processed WAV preparation; short clips retain their batch accuracy safeguard.

The user's recordings after that fix measured **1.416 s** total for the 9.960-second question and **1.954 s** for a 29.440-second follow-up, both with Qwen Auto. Final live decoding took **223/305 ms**, and raw saves took **52/235 ms** in the background. These are actual stop-to-paste observations. They preserve the capture grace, stream drain, and accuracy guards; they do not promise literal instant output or eliminate network/model time.

### Parakeet v3 buffered streaming

NVIDIA documents [buffered streaming for Parakeet v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3). The installed sherpa-onnx export uses an [offline transducer recognizer](https://k2-fsa.github.io/sherpa/onnx/pretrained_models/offline-transducer/nemo-transducer-models.html), so it does not expose Nemotron's cached streaming interface.

An opt-in worker now continuously resamples arriving audio and decodes completed, overlapping chunks during capture. It retains the existing 12-second silence boundary / 25-second hard limit / 3-second overlap policy. On stopping, it reads all remaining device samples, flushes the resampler, and decodes the final partial chunk. A worker failure retains the original capture for batch recovery. Short dictations still decode after stopping because they have not completed a chunk.

**The existing batch path remains the default.** Buffered chunk-local level adjustment matched complete-buffer decoding of that same algorithm on all six reference/review clips, but omitted additional internal words on the long reference (`consecutive`, as well as the already-missed `between`). This does not meet the requirement to preserve default accuracy. Streaming can be explicitly evaluated with `$env:TYPR_PARAKEET_BUFFERED = '1'`; removing that variable restores the default path. No selected model or saved setting was changed.

In the final 79.885-second buffered replay, three chunks completed before finalization; final decoding took **1,068 ms**, compared with **6,456 ms** for complete-buffer decoding of that same algorithm. Decoder-tail timings exclude model loading, capture grace, any in-flight chunk, AI processing, and paste, and therefore are not release-to-paste guarantees. Regression tests verify the buffered worker and default production batch path retain the ending and seam phrases, and the default path's test additionally checks all ten items. The buffered replay also retained all ten. Separate tests check stereo 48 kHz unread tails and incremental/batch boundary equality with a final partial frame.

## AI and implementation checks

- **407 Rust library tests passed; zero failed; one manual benchmark remains ignored by the normal suite.** That benchmark was run separately earlier. The replay helper's WAV-preparation test passed separately.
- TypeScript and Vite production build passed. The final Rust application binary was built from this worktree.
- Live text checks covered all eight effective profiles with **Qwen and GPT-OSS 120B**: Cleanup, Messaging, Email, Professional, Developer, Terminal, and both Prompt Mode formats. Unsafe ordinary rewrites were rejected or partially restored. After the negation correction, both final Structured proposals were rejected for changed negations; the application uses its deterministic source fallback in this case, so requested section formatting may be lost. API limits during the initial burst were handled by pacing the evaluation, without adding application delays.
- Seven signing tests and the repository secret scan passed. The replacement private key remains encrypted outside the repository. Already-installed applications trusting the exposed key still require a reviewed release/manual migration; removing source literals does not revoke those clients.
- General's theme-aware **Clear data** confirmation passed the earlier isolated light/dark browser and backend fixture checks. Live history, settings, models, and raw review recordings were not deleted during this review.

The full issue-by-issue report is [Accuracy and signing fixes](accuracy-and-signing-fixes.md). [Quick review](quick-review.md) contains one optional recording and the short dialog check; neither is a prerequisite for this automated review.

## Reproducible tools

`replay_review` uses native production preprocessing and the installed local decoders without uploading audio. Its `--prepare-only` option writes a new processed WAV without overwriting an existing file. `replay_buffered_parakeet` checks complete/incremental parity of the experimental buffered algorithm; it does not claim parity with the default batch level adjustment. `inspect_parakeet_chunks`, `compare_local_decoders`, and `compare_audio_levels` are isolated local probes. `replay_cloud` explicitly sends named prepared WAVs to Groq. `replay_cleanup` sends its fixed regression text by default, or an explicitly named JSON case with `--case-json`, through the production AI pipeline. `--model` and a profile name select a single check. No new cloud requests were made in the earlier final tail/seam pass; the October 9 follow-up used text-only checks below.

## October 9 AI mode recovery

The supplied log's 74.582-second capture used **Nemotron + Qwen, Auto → Professional in Notepad**. Qwen's rejected completion was not recorded. A text-only replay reproduced the failure: the completion added useful sentences and paragraphs, but omitted source words. The old equal-sentence recovery could not handle that layout, so the complete result fell back to basic cleanup. The separate Parakeet buffered capture in that log used AI off; its 3 completed chunks and 449 ms tail decode do not indicate an AI-mode or streaming failure.

The new recovery retains localized safe AI edits across layout changes, restores rejected content, and reruns the unchanged whole-transcript word, entity and negation checks. Paths, identifiers and dotted numbers also have exact literal checks. Reordered Prompt Mode sections and literal terminal input use a bounded model repair rather than word alignment. Repair keeps the original mode/style/vocabulary prompt and both requests share the existing total budget. An ultimate failure now generates a visible notification while preserving the full fallback; it does not change the saved AI setting.

| Live text replay | Outcome | AI pipeline time |
| --- | --- | --- |
| Supplied long source, Qwen, Professional | Accepted with local recovery; paragraphs and full wording retained; no retry | 921 ms |
| Same source, GPT-OSS 120B, Professional | Accepted with local recovery; no retry | 2,230 ms |
| Fixed regression passage, Qwen, Structured Prompt | Accepted on GPT-OSS repair with requested sections and original negations | 7,434 ms, within the existing 10-second Prompt Mode budget |

These timings exclude audio capture/recognition and paste, and depend on network/model/CPU conditions. Replays used the already supplied test text and configured credentials, with no audio uploads or settings changes. Correct layout does not prove every ambiguous ASR word is correct; words such as `restrock` can remain when no safe correction is established.

Follow-up checks: **416 Rust tests passed, zero failed, one manual benchmark ignored**; TypeScript, Vite production build and the Rust app binary build passed. This includes the complete native final-word/streaming regressions and nine new AI recovery/literal/budget checks. Seven signing tests and the secret scan of 149 repository files also passed. The earlier 407-test checkpoint above is historical.

The regression fixture is `src-tauri/test-fixtures/long-cleanup-recovery.json`. A focused replay is:

```powershell
Set-Location 'C:\Users\sanir\orca\workspaces\Typr\Accuracy-improvisations-and-general-fixes\src-tauri'
$env:CARGO_TARGET_DIR = 'C:\Users\sanir\Typr\src-tauri\target'
cargo run --example replay_cleanup -- --model qwen/qwen3.8-27b --case-json test-fixtures/long-cleanup-recovery.json professional
```

This deliberately sends only the named regression text to the configured AI service. The fixture is also checked locally without a network request in the Rust suite. No new microphone recordings are required to reproduce this issue.

Raw evaluation transcripts, timings, downloaded evaluation models, and checksums are retained outside the repository at:

```text
C:\Users\sanir\AppData\Local\Temp\typr-accuracy-evaluation-20261008
```

For a fresh checkout's empty Whisper binaries directory:

```powershell
powershell -NoProfile -File scripts/prepare-whisper-runtime.ps1
```

The setup pins the official [whisper.cpp b5454 runtime](https://github.com/ggml-org/whisper.cpp/releases/tag/b5454), verifies its SHA-256, copies its matching executable/DLL set, and checks CLI startup. It refuses a nonempty or linked destination. The archive verification, matched runtime, load checks, script syntax, and refusal path were checked; the complete setup script was not rerun into a second large fixture directory.
