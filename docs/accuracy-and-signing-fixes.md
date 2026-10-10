# Accuracy and signing fixes

Worktree: **Accuracy improvisations and general fixes**, based on `12c328e`.

**Latest review:** [Recorded-audio evaluation and completion status](accuracy-evaluation.md). Shared fixes cover all four engine families, and the latest comparisons include cloud and local Whisper. This document also retains earlier checkpoints; their measurements are historical rather than a universal accuracy claim.

**Final tail/seam pass:** capture grace and Nemotron closure remain; regression tests verify unread final samples and complete short/long endings. Conservative shared overlap merging preserves newly recognized words, and Parakeet's timestamp corroboration removes reproduced seam duplicates. Empty audible chunks now fail local Whisper/Parakeet transcription rather than becoming partial success. Parakeet v3 buffered streaming is implemented with `TYPR_PARAKEET_BUFFERED=1`, but remains opt-in because the default batch path retained more internal words on the long reference. See the latest evaluation for measured limits and final verification results.

## Changes shared by every speech engine

Cloud Whisper, local Whisper, Parakeet, and Nemotron all pass through the recorder's dictionary, cleanup, and optional AI stages. These changes apply there, including when AI is disabled:

- Preserve paragraph breaks, numbered lists, tabs, and literal snippets during dictionary replacement and the post-AI dictionary pass. Match variable horizontal spacing without joining separate lines. Previously, that pass flattened all whitespace even with no configured replacements.
- Preserve dollar signs in snippets. Avoid expanding a snippet twice when its expansion contains its own trigger.
- Normalize the confirmed `accuries` and `acuracy` errors to `accuracy` and the exact spoken product name `fluid voice` to `FluidVoice`. Explicit compound dictionary names such as `GitHub` can likewise match `git hub`. These are exact spelling corrections, not a general instruction to join ordinary phrases into identifiers. Paths and addresses remain protected; explicit dictionary spellings take priority.
- Preserve meaningful `You know the answer` and `I mean the second model` during filler cleanup. The existing filler pass already preserved newlines; the dictionary pass was the formatting regression.

## Fixed gain and quiet speech review

The removed slider is replaced by **+12 dB digital pre-gain** (about 3.98 times sample amplitude). Capture validation, batch preparation for all four engines, live Nemotron, and the visual meter use that same constant; the meter does not modify recorded samples. Automatic speech-level adjustment follows the pre-gain, raising quiet audio or reducing loud audio toward the target level before limiting peaks. The fixed boost is not the final audio level or a hardware microphone setting.

Found and corrected a quiet-speech mismatch: the live Nemotron level tracker previously required RMS 0.01 after pre-gain, although the capture gate accepts sustained RMS 0.003. A quiet 0.002-amplitude signal was therefore accepted but left around RMS 0.0056 in live recognition, while batch processing raised it toward 0.08. Lowering the live tracking floor to 0.005 restores normalization for that quiet-speech case. Regression coverage checks both paths at +12 dB, multiple streaming chunk sizes, unchanged low-level noise behavior, and attenuation/limiting of loud input. It does not establish intelligibility of every real whisper or improve the microphone's speech-to-noise ratio.

Capture validation now accounts for interleaved channels when measuring a 20 ms frame. The fixed-gain comments were corrected: float multiplication does not itself hard-clip, and increasing gain cannot repair clipping already present at the device.

The recorded comparisons are complete; no repeated microphone matrix is required. [The optional quick review](quick-review.md) is available for a final smoke check.

## AI processing, independent of the speech engine

The Cleanup and Developer prompts now request small, context-supported corrections. Removed the conflicting `picture → feature`, `team → theme`, and `shot → screenshot` mappings and the accumulating speculative mappings, including negation and preposition substitutions. Existing anti-quote, anti-camelCase, never-answer, self-correction, and automatic layout rules remain.

Vocabulary guidance now permits canonical spelling only when the transcript refers to the term. Names in the list must not displace ordinary words. `FluidVoice` is included in that guidance.

Each model attempt rejects output that changes the count of each kind of negation, drops a meaningful transcript tail, or changes/drops ordered content words. Negative contractions and their expanded forms remain equivalent. Ordinary Cleanup and every Auto context now use the same conservative wording contract. The Professional prompt selected by Notepad no longer asks for tighter phrasing or more precise synonyms; Messaging, Email, and style modifiers also preserve complete clauses. Prompt Mode retains its permission to restructure, while entity and negation guards still apply.

The October 9 follow-up fixes a silent fallback found in the supplied long Notepad test. Recovery now aligns unchanged words across new sentence, paragraph and list boundaries, keeps safe spelling/punctuation/layout edits, and restores rejected spans from the original transcript. Literal URLs, paths, identifiers and dotted numbers remain intact, including case and repeated references. Every recovered result passes the complete original fidelity guards. Alignment work and the size of restored spans are bounded; uncertain whole rewrites are not reconstructed by guessing.

Local recovery makes no additional cloud request. If it cannot preserve the transcript, one cross-family repair attempt retains the same effective profile, vocabulary, tone, formatting and custom instructions. In the Dictionary worktree's October 10 follow-up, both attempts share a 6-second ordinary deadline (length-dependent up to 10 seconds for long text), with time reserved for the alternate model; Prompt Mode retains its 10-second ceiling. Prompt Mode's reordered sections and literal terminal input are not repaired by monotonic word alignment. Qwen's alternate model remains GPT-OSS 120B. Truncated, empty, refusal and reasoning-leak replies remain failures. Already stripped filler is supplied to AI so safe filler removal does not look like a dropped clause. See [the reported Dictionary cleanup diagnosis](AUTO_LEARNING_DICTIONARY.md#october-10-reported-cleanup-failures).

Successful logs now include `recovered` and `retried`. If both attempts fail or time out, Typr keeps the full source fallback and shows a toast/system notification; AI remains enabled for the next dictation. Terminal fallback remains literal. This avoids an undisclosed AI-on result looking like AI-off, while preserving words when the service cannot safely process them. See [the follow-up evidence](accuracy-evaluation.md#october-9-ai-mode-recovery).

The guards prefer the original transcript when uncertain. A guard cannot reconstruct speech missing from the source transcript, and preserving negation kinds and counts does not prove complete semantic equivalence.

## Findings from the supplied microphone review

The review logs separate recognition errors from AI errors. `pick`/`pic`, `key called final words`, a missing second clause, and `disk` were already present in Stage 1. AI introduced other changes, including `improvisations` to `improvements`, `taking a shot` to `attempting`, and `work`/`keep` to `function`/`retain`. Regression cases from these supplied phrases now reject those edits across all normal AI profiles while admitting punctuation, contractions, layout, and small malformed spelling repairs.

For the 78.082-second Nemotron dictation, the log measured 1,444 ms preparing audio, 130 ms final live decoding, and 5,947 ms total. Two AI attempts both changed negations and were rejected. Healthy live dictations now stop/validate the raw capture and finish its unread tail without preparing an unused batch WAV. Batch recovery and comparison still prepare the same processed WAV. Input-level notices and the release grace remain. `wav_ms=0 wav_saved=false` identifies the new healthy live path; rejected AI logs now include elapsed milliseconds.

The shared batch resampler evaluates the same 31-tap filter only at source positions used for interpolation. An 80-second 48 kHz synthetic probe in the unoptimized debug build measured 1,059 ms before and 371 ms after, with identical output samples. Bit-level parity tests cover multiple input/output rates and edge lengths. This helps Parakeet, local/cloud Whisper, and Nemotron batch clips without changing their input waveform; these numbers are filter timings, not measured microphone release-to-paste improvements.

`TYPR_REVIEW_AUDIO=1` optionally retains exact raw device samples locally for same-audio comparisons. It is disabled by default, refuses overwrites, and does not upload recordings. The October 8 follow-up finally supplied raw recordings. Equal fast-speech accuracy and an aggregate recognition improvement remain unestablished; replay findings are recorded below.

## Nemotron recognition and latency

- Both live and batch finalization retain at least 1.25 seconds of silence, exceeding the model's 1.12 second chunk plus feature-window headroom. Batch finalization counts exact trailing zeros at 16 kHz, reusing the recorder's existing 400 ms comfort silence instead of adding it twice. Nonzero microphone noise is never counted as exact padding.
- Short dictations up to 8 seconds join the live worker without its unnecessary final decode, then use the processed WAV. The paused stream is finished only if batch decoding fails, preserving the recovery path. Comparison mode still finishes the live path so it can compare both transcripts. Total decoder errors now propagate to the existing user notification instead of looking like an empty successful dictation.
- Live resampling retains FIR history, lookahead, and fractional position across microphone reads. Restarting the batch resampler at each read previously introduced artificial zero-padded edges and rounding losses. The continuous implementation matches whole-recording resampling at 8, 16, 44.1, and 48 kHz in regression tests.
- Stopping or dropping the worker wakes its idle wait immediately instead of waiting up to 150 ms. An in-flight decode can finish, while remaining frames stay in the stream for finalization or batch-failure recovery. The raw audio is not discarded.
- Comparison mode reuses a batch result when the primary path already needed it; long live-only comparisons use a unique capture file so consecutive diagnostics cannot overwrite each other's audio.
- Healthy long live captures skip WAV preparation; batch and diagnostic paths retain it. Deferred saving retains the exact batch preprocessing, 400 ms comfort padding, and raw buffer until the engine is finished.
- The existing release grace and quiet-tap guard remain. Nemotron's NeMo implementation rejects `modified_beam_search`; the supported greedy decoder remains. Local Whisper retains beam five. Parakeet now uses its separately measured greedy decoder and bounded input-level adjustment.

## Parakeet latency and timing logs

Parakeet now uses `greedy_search` and a bounded speech-level adjustment to RMS 0.20, after the shared capture chain, with a maximum extra gain of 2.5 and the existing soft limiter. Native comparisons recovered the overlay and final-word clauses that beam eight dropped in the actual normal/fast recordings. Other engines retain their tested audio levels. Speech boundaries, overlap, and merge policy remain. Parakeet explicitly selects the NeMo transducer model implementation. Machines reporting at least 12 logical CPUs use six inference threads; smaller machines retain up to four (one on a single-CPU machine).

A missing decoder result now reports an error rather than silently omitting an entire chunk from an otherwise successful partial transcript. Valid empty results for silence retain their existing behavior.

The earlier beam-eight thread probe on this Ryzen 7 7445HS measured a median of 2.018 seconds at four threads and 1.782 seconds at six on a 20-second reference excerpt. Short-clip medians were 0.407 and 0.439 seconds: that historical probe did not establish a short-clip improvement. The latest decoder/input-level results and remaining long-clip word/seam errors are documented in [the recorded-audio evaluation](accuracy-evaluation.md).

Nemotron's four-thread configuration remains: two and six threads were slower on the measured probes. Five warm short-clip runs with the recorder's 400 ms zero padding had medians of 0.859 seconds with additional 1.25 second padding and 0.679 seconds with padding reuse, about 21% less decoder time and identical transcripts. These are decoder-only timings on reference audio; they exclude microphone capture grace, AI, paste, and real recording variation.

`typr.log` now records `PERF capture=...` markers and audio, recognition, processing, paste/history, and total elapsed milliseconds. Existing AI timing/model lines and transcript stage logs remain. A common capture ID also labels asynchronous Nemotron comparisons. Timing measurements should use comparison mode off.

Reproduce the CPU probes, using Python `sherpa-onnx` and `numpy`:

```powershell
python scripts/benchmark_local_latency.py
python scripts/benchmark_local_latency.py --engine nemotron --threads 4 --short-only --compare-padding --repeats 5
```

## Earlier validation and measurement checkpoint

- Rust library suite after the latest microphone feedback and prompt review: **380 passed**, with one manual resampler benchmark ignored in the normal suite and run separately successfully earlier. Includes Nemotron model tests, a long-clip streaming test at both 44.1 and 48 kHz, exact transcript comparisons with pre-padded WAVs, complete Parakeet four-versus-six-thread comparisons, ordered AI fidelity regressions, quiet live/batch normalization, deferred capture saving, raw review-WAV round-trips/refused overwrites, modifier punctuation repairs, and structured-template negation checks. The live test verifies that the final `robust decoding algorithms` phrase survives.
- TypeScript and Vite production build passed.
- Rust binary checks passed.
- **Seven signing tests passed**, covering revoked keys, missing passwords, injected credentials, malformed public keys, empty optional variables, scanner detection without secret output, and cryptographic artifact/comment verification (including tampering and a different public key with the same key ID).
- A temporary artifact was signed with the replacement key, cryptographically verified against its public key, and tested with an empty password, which failed. No real release was signed or published during this verification.
- The updated in-process key generator was exercised in a restricted external fixture directory; captured CLI output was suppressed and the test keys removed. The existing replacement credentials and public key were not rotated again.
- PowerShell scripts parse cleanly. The review-log collector passed fixture tests for timestamp filtering, multi-line transcripts, and refusing output overwrite.
- On the three existing local reference WAVs, both 1.0 and 1.25 second batch padding produced 5 word errors in 251 words: **1.99% WER**. The benchmark establishes no aggregate WER improvement on those clips. They do not reproduce the recent microphone failures. Benchmark timings overlapped Rust tests and are not evidence of a latency gain.

Reproduce the local padding comparison, with Python `sherpa-onnx` and `numpy` installed:

```powershell
python scripts/benchmark_nemotron.py
```

For an actual long dictation comparison, launch a build from this worktree with `TYPR_NEMOTRON_COMPARE=1` in its environment. Compare the live and batch results with the words actually spoken. The Markdown history export supplied for review contains final text, not PCM audio or pre-AI transcripts, so it cannot isolate acoustic errors from AI edits or establish a WER score. No private history entries were sent to a cloud service for these checks.

## Signing key recovery

The exposed updater key ID **2340F69B968E764E** is compromised. Removing its script literals does not undo public history or revoke trust in already-installed apps.

All four versioned signing scripts now delegate to `scripts/sign-release.mjs`, which requires injected credentials and a nonempty password, rejects the compromised public key, and cryptographically verifies the resulting artifact signature and trusted comment against the configured public key, as well as checking its key ID. Unverified signatures are removed. `scripts/check-secrets.mjs` and the signing-security GitHub workflow guard against embedded updater key material returning to the repository.

The replacement public key in `src-tauri/tauri.conf.json` has ID **59AB6C4E5AF7EEC7**. Its encrypted private key and Windows DPAPI-protected password are stored outside every worktree at:

```text
C:\Users\sanir\AppData\Local\Typr\signing\updater-2026-10-08\
```

The directory ACL grants access only to the current Windows user. The key's password was generated randomly, and neither it nor private key material is in source files, tests, or this document. Future rotation uses the Tauri CLI API in-process so the password is absent from the OS command line. DPAPI recovery is bound to this Windows account and machine. Back up the encrypted key and its password separately in secure storage before relying on this key for releases.

The original checkout and its handoff artifacts remain untouched. Older patches, scripts, or exported history containing the old key still represent the compromised key, not safe reusable release credentials.

For local release signing, after building the intended installer:

```powershell
powershell -NoProfile -File scripts/sign-local-release.ps1 -Version 0.2.3
```

For CI, supply `TAURI_SIGNING_PRIVATE_KEY` (key content) or `TAURI_SIGNING_PRIVATE_KEY_PATH` (external key file), plus `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, through the credential store. The repository contains only the public key. The generic signing command also accepts an explicit installer path as its second argument.

**Release migration is required:** publish a reviewed, manually installable release containing the replacement public key and migrate installations that still trust the exposed key through a trusted manual download. Do not sign a bridge release with the compromised key. Existing installations cannot validate a new-key update until their embedded public key has changed. This worktree has not changed public releases, published repository history, or already-installed applications. History cleanup is separate; copies of the compromised key must continue to be treated as public even after a rewrite.

## Reference review

FluidVoice was cloned into `C:\Users\sanir\AppData\Local\Temp\typr-fluidvoice-reference-20261008` at commit `7daffb529fb9f7c8d075f686a2d3da34695a66e4`. Its source separates transcript data from instructions using a JSON envelope and finishes/drains a live Nemotron stream before resetting it. Typr already implemented the JSON envelope; it was retained. No FluidVoice source code was copied into Typr.

- [FluidVoice transcript framing](https://github.com/altic-dev/FluidVoice/blob/7daffb529fb9f7c8d075f686a2d3da34695a66e4/Sources/Fluid/Services/DictationPromptRequest.swift)
- [FluidVoice Nemotron provider](https://github.com/altic-dev/FluidVoice/blob/7daffb529fb9f7c8d075f686a2d3da34695a66e4/Sources/Fluid/Services/NemotronProvider.swift)
- [Tauri updater signing and configuration](https://v2.tauri.app/plugin/updater/)
- [Groq reasoning model parameters](https://console.groq.com/docs/reasoning)
- [ONNX Runtime thread tuning](https://onnxruntime.ai/docs/performance/tune-performance/threading.html)
- [Minisign artifact and trusted-comment verification](https://github.com/jedisct1/rust-minisign/blob/master/src/lib.rs)

## Cross-reference to the reported issues

| Reported issue | Implemented and checked | Limits or remaining release work |
| --- | --- | --- |
| Accuracy without AI, across all engines | Shared vocabulary/dictionary corrections; continuous Nemotron resampling; measured Parakeet clause recovery; saved recordings compared across all four engine families | Every model weight/voice/microphone was not tested; Nemotron and long Parakeet clips still have recognition errors |
| Confusing AI edits, particularly the small model | All eight profiles checked with Qwen and GPT-OSS 120B; ordered content/entity/negation guards; independent valid sentence/spelling edits can survive rejected rewrites | Conservative guards may preserve an ambiguous recognition error; no complete semantic guarantee |
| `fluid voice` product spelling | Exact `FluidVoice` normalization; narrowly confirmed Nemotron aliases; paths and handles protected | Valid ordinary words such as `fluid-wise` are not globally replaced |
| Final words and long Nemotron dictations | Continuous resampling, complete final drain, at least 1.25 s closure, and live/batch recovery; healthy live captures skip unused WAV work; local AI edit recovery preserves words and layout without a second request; actual follow-up recordings show 1.416/1.954 s total | Immediate release cannot recover speech that was never captured; cloud AI adds latency; unrecoverable AI replies get one repair attempt within the original budget |
| Short Nemotron delay | Avoid redundant live finalization, wake worker immediately, reuse comfort padding, avoid duplicate diagnostic batch decoding; measured short decoder reduction | Total release-to-paste time with and without AI; batch accuracy safeguard remains through 8 s |
| Parakeet delay after release | Six threads on eligible machines; greedy TDT decoder and bounded level adjustment recover dropped clauses in the saved recordings; faster bit-identical batch resampling | v3 int8 measured; v2 not separately evaluated; long clips can still have word errors and overlap duplicates |
| Quiet tap pasting a stray `D` | Existing narrow fragment guard retained; tests protect real words | Quiet-room hotkey tap and softly spoken words |
| Lists and paragraph formatting | Layout guards retained; dictionary no longer flattens output | Lists in Notepad and your usual target app |
| Leaked updater key | New encrypted external credentials, new public key, safe signing/generation helpers, secret scan and cryptographic tests | Trusted new-key release/manual migration and secure backup; public history and old installs remain compromised |
| FluidVoice reference review | Reviewed its source and retained JSON transcript framing; no GPL source copied | No separate action needed |

Use [the dictation review checklist](review-checklist.md) for test phrases, the model matrix, launching the worktree build, capturing logs, and a feedback template. Code checks and reference clips do not establish a general microphone WER improvement for all engines.

## Earlier follow-up review from October 8

The user's normal, quiet, and fast speech results still contain material recognition errors. Matching logs place most of these errors at **STAGE 1**, before dictionary correction or AI. Those short recordings used `BatchPrimary`; the quiet live-level adjustment cannot explain improvement on those clips. The final 43.742-second dictation used `LiveOnly`, with `wav_ms=0`, a 377 ms final decode, and a 2,059 ms release-to-paste total. Its rejected AI edit fell back in 996 ms without making a second model request. Short recordings in the visible follow-up logs took roughly 1.5–2.7 seconds in total.

No raw WAVs were retained for this follow-up: the review-audio directory was absent. Text exports and logs cannot support a controlled comparison of normal/quiet/fast recognition on the same sound. In particular, `force`, `versus`, `recording`, and `recoding` are meaningful words; global substitutions into the expected test phrase would distort other dictations. The ordinary-word protection list now also explicitly includes decoding, recoding, improvisations, and curiosity so close spellings of valid terms cannot be treated as malformed words by the AI guard.

The prompt review covers Cleanup, Messaging, Email, Professional, Developer, Terminal, and both Prompt Mode formats. All effective prompts now preserve uncertainty and modifiers, and forbid invented quotes or merged identifiers. Prompt Mode retains its explicit permission to reorganize a request; ordinary cleanup permits only conservative spelling/grammar/layout edits. A malformed split spelling such as `acc uracy` can be rejoined, while ordinary phrases, sentence boundaries, and code boundaries remain protected. These are cleanup safeguards, not evidence of a general acoustic accuracy gain.

Qwen replay of the supplied text showed that instructions alone still allowed commas inside `accuracy improvisations`. Cleanup now restores source punctuation within common technical modifier/noun phrases, retaining the rest of the model's valid edits. Explicit lists and unrelated commas stay intact. The same review found that Structured Prompt Mode's required `Not specified.` placeholders falsely tripped the negation guard; whole empty template sections are now excluded from its count. Removing or adding a negation in actual dictated content still fails validation. Repeated digits in versions are also protected from being mistaken for adjacent verbal stutters.

Live Qwen text checks covered all eight variants. After the final corrections, Cleanup and Professional returned the supplied phrase with its words and grouping intact. Structured Prompt Mode also returned all tasks and the negative constraint, with `Not specified.` for the missing background and output format. Its section rules now prohibit inferring a project or image existence from task instructions and discourage duplicate requirements. These text checks do not evaluate microphone recognition or establish the same results with GPT-oss. Initial burst requests hit the provider's input-token rate limit; paced checks completed without changing application timing.

[The quick review](quick-review.md) covers one optional latency check and the Clear data dialog. `src-tauri/examples/replay_review.rs` reuses production batch preprocessing and native Nemotron/Parakeet decoders so one recording can be compared without repeated microphone tests. That local helper performs no cloud upload and does not reproduce streaming timing. The later completion review added separate local Whisper and explicit cloud comparisons, described in [the latest evaluation](accuracy-evaluation.md).

The replay tool passed its float/stereo/44.1 kHz processing check with an exact i16 sample comparison. On the existing four-second reference recording, Nemotron and Parakeet both retained the complete sentence, with warm decodes of 474 ms and 432 ms respectively in this run. This is a tool sanity check, not a measurement of the user's recent failures.

## Review recording delay: fixed October 8

The 5–17-second regression came from my opt-in review recorder. `WavWriter::new(File, ...)` did not buffer writes: each four-byte raw sample caused a filesystem write while the transcription path waited. The live recognizer itself was finishing quickly.

| Capture | Audio duration | Final engine time | Total stop-to-paste |
| --- | --- | --- | --- |
| `1791470517639404600` | 16.832 s | 345 ms | 4,737 ms |
| `1791470564381566000` | 18.212 s | 190 ms | 5,639 ms |
| `1791470671615347500` | 72.384 s | 286 ms | 17,798 ms |

About 3.7, 4.0, and 16.0 seconds respectively were spent between the capture checkpoint and engine timing, matching the synchronous raw saves. Buffered writes now save the exact float samples on a background thread, without holding the live recorder's sample mutex. The only foreground work is a memory snapshot. A 72-second stereo/48 kHz fixture (27.6 MB) saved in 215 ms in an isolated debug check, and every sample round-tripped unchanged. A heavily loaded full test run measured 716 ms; disk time no longer blocks paste in either case. Concurrent debug records are serialized to keep review output from merging into stage lines.

The user's next actual recordings already exercised the updated path:

| Capture | Audio duration | Snapshot | Background save | Final live decode | Total stop-to-paste |
| --- | --- | --- | --- | --- | --- |
| `1791473179301621200` | 9.960 s | 0 ms | 52 ms | 223 ms | 1,416 ms |
| `1791473578567585500` | 29.440 s | 2 ms | 235 ms | 305 ms | 1,954 ms |

Both used Nemotron 3.5 with Qwen Auto. Neither needed a processed WAV. The 400 ms release grace, resampler, gain/limiter, chunk context, padding, and transcript guards remain intact. AI still takes network/model time: rejected edits in these two captures took 470 and 918 ms respectively. This change removes recording export from the critical path across all engines; it does not promise zero end-to-end latency.

## Initial replay findings, before the latest Parakeet changes

Replayed the actual 16.832- and 18.212-second microphone recordings through both installed local engines with the production batch audio chain. Nemotron still misheard several words, so an unconditional batch re-decode would add seconds without reliably solving these failures. Parakeet recognized the opening product phrase better, but dropped whole clauses on these two clips. There is no demonstrated universal accuracy winner from two clips.

For the new 9.960-second tab question, live Nemotron returned `pupple actions`; a batch replay returned `pupil actions` (1,517 ms), while Parakeet returned `perform actions` (943 ms) on the same raw audio. Parakeet still included an opening false start. This specific example points to a recognition/model difference, rather than only the live tail flush or AI prompt. The AI fidelity guard rejected the proposed edits in the original capture and kept the raw wording; its close-spelling rule cannot safely treat every unknown token as an arbitrary replacement. No global `pupil`/`pupple` → `perform` substitution was added.

Checked NVIDIA's [Nemotron 3.5 model card](https://huggingface.co/nvidia/nemotron-3.5-asr-streaming-0.6b) and the [sherpa-onnx export instructions](https://github.com/k2-fsa/sherpa-onnx/blob/master/scripts/nemo/nemotron-3.5-asr-streaming-0.6b/README.md). The current 1.12-second context favors accuracy; shrinking it for latency would trade away context, and forced English is already applied on both paths. This initial checkpoint did not include Whisper or an alternate Nemotron model. The subsequent completion review compared both Whisper paths and the English Nemotron variant; three prepared test passages were explicitly sent to Groq under the request to complete the remaining checks. Raw review WAVs and history were preserved. See [the latest evaluation](accuracy-evaluation.md) for results and limits.

## Clear local data

Added **General → Data → Clear data…** and a confirmation dialog using the incumbent surface/text/error colors, IBM Plex Sans stack, button shapes, and live light/dark theme variables. Cancel is initially focused; Escape and clicking the backdrop cancel; Tab/Shift+Tab stay inside. Clearing has a disabled/loading state and visible errors. Cancel cannot interrupt deletion once it is underway.

The command clears `history.json` in memory and on disk, `typr.log`, `temp_recording.wav`, numbered `nemotron-compare-*.wav`, and numbered review WAVs inside `review-audio`. It preserves configuration, dictionary, API credentials, model downloads, unrelated files, and manually named WAVs. It does not recursively delete directories or follow symlinks/junctions. Failed file removals are reported rather than hidden. Recording/transcription must finish first, including paste/history persistence, and pending background review saves are joined before deletion. Old history fetches and Undo actions cannot restore the cleared entries.

Typr currently uses JSON history, not SQLite. New recordings/logs accumulate after clearing. Only isolated fixtures were deleted during verification; the user's live history and review recordings were preserved.

Validation: the full library suite passed **386 tests** with one manual benchmark ignored; the subsequently added concurrent-log regression passed separately, bringing the checked total to **387**. Rust binary build, TypeScript, Vite production build, signing secret scan, and browser fixture checks passed. Browser checks covered light/dark at 1160×720 and the supported 700×500 minimum, focus/cancel, busy/pending/error states, history statistics, and retired Undo timers. The design detector found existing app contrast/decorative warnings outside this feature; this work kept the existing visual system. No release or commit was created.
