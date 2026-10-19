# Quiet speech, cloud latency and independent editing models

Worktree: `quiet-speech-and-cleanup`, based on v0.3.1. This update is not merged
or published. No personal settings, dictionary entries or installed models were
changed during development.

## Findings from local logs

- One cloud recognition request took 22,674 ms. This was the `engine_ms` stage,
  not AI cleanup. Independent 60-second request timeouts and three retries could
  keep a medium dictation waiting much longer than the cleanup deadline.
- A noise-only recording was accepted by the energy gate and returned
  `Thank you.`. An energy gate can detect loudness but cannot distinguish speech
  from fans or other sustained room noise.
- Quiet speech and noise need different handling. Increasing every recording
  from +12 to +18 dB would amplify both; it cannot reconstruct missing acoustic
  detail or improve the signal-to-noise ratio.

## Changes

- A bundled, MIT-licensed Silero detector checks a normalized analysis copy on
  CPU before recognition. It does not trim the capture, delete final words, or
  blacklist genuine greetings. All engines use this capture check. Failures to
  initialize the detector are logged and use the previous signal gate.
- The fixed pre-gain stays at +12 dB. Adaptive normalization gains another 6 dB
  of ceiling (40x instead of 20x), applied only when needed to reach the same
  target. Live normalization accepts a quiet modulated envelope while keeping
  the steady-noise floor and avoiding high-pass filter ringing after speech.
- Stereo downmixing falls back to the stronger channel if averaging would lose
  over 90% of energy through cancellation. Both batch and live Nemotron use it.
- Cloud recognition has one deadline across upload, chunks, response parsing and
  retries: eight seconds up to 30 seconds of mono PCM16 audio, rising for larger
  recordings to a maximum of 30 seconds. One retry shares that deadline. On
  failure, nothing partial or fabricated is pasted; the user sees an error.
- Groq segment metadata rejects a transcript only when every segment reports
  both high no-speech probability and low recognition confidence. Individual
  words are never removed by this check. Genuine `Thank you` and `Bye` remain
  allowed.
- Existing cleanup deadlines, fidelity/negation guards, vocabulary grounding,
  anti-quote/identifier rules and final-word guards remain in place. Presets
  retain the existing style system, with a new cross-profile contract test.
- A live style check found Raw mode flattening existing line breaks. Raw mode
  now restores the source's whitespace around aligned corrections, including
  blank lines and tabs. If a multiline edit cannot be aligned safely, the
  original is retained. This also applies to basic fallback cleanup.
- Cleanup and Write Mode have independent saved model IDs, custom pickers and
  retirement handling. Migration inherits the saved cleanup choice once.
- The pill pulls authoritative recording state after resume/reload and sends a
  heartbeat. The watchdog can recreate an unresponsive WebView, rather than
  assuming a visible native window means its content is alive. Placement uses
  monitor origins and DPI, including monitors at negative coordinates.

## Reference review

BetterWispr was reviewed at revision
`07c55da3e6a95739c84135ad777498d8cae52dff`, specifically its recorder, transcript
cleaner/polisher and vocabulary pipeline. Its recorder uses an amplitude gate,
which the source explicitly distinguishes from semantic speech detection.
Its context-grounded cleanup and bounded retry patterns are useful references;
Typr uses original Rust code and retains its stricter fidelity checks and shared
deadline instead of copying macOS code or accumulating per-attempt deadlines.

FluidVoice's prompt request, post-processing gate and post-processing service
were also reviewed for architecture only. No GPL source code was copied.

Sources:

- https://github.com/opennookorg/betterwispr
- https://github.com/altic-dev/FluidVoice
- https://console.groq.com/docs/speech-to-text
- https://console.groq.com/docs/reasoning
- https://github.com/snakers4/silero-vad

## FluidVoice model assessment

The organization lists three builds of Fluid Speech Mini (Core ML, ONNX, GGUF)
and a Qwen text model. Mini is an English **audio-to-text recognizer**, not a
text cleanup model. Its Windows ONNX build is 302 MB; its published benchmarks
are self-reported runs, not an independently verified improvement on this
user's recordings. It was not added under a misleading cleanup label.

The text model, `FluidVoice-Labs/Qwen3.5-9B-4bit`, has a 6,169,713,989-byte weight
file. Its card calls it a modified package for Fluid Intelligence, says it is
not fine-tuned and may not work with other tools, and includes a DFlash drafter.
It is not the proposed sub-300 MB cleanup model. Following the user's instruction
to focus on the current fixes, no multi-gigabyte local cleanup download or new
runtime was added. X denied access to the tweet/replies; the model cards and
repository files were inspected directly rather than assuming the comments
verified the advertised benchmarks.

- https://huggingface.co/FluidVoice-Labs/fluid-speech-mini-onnx
- https://huggingface.co/FluidVoice-Labs/fluid-speech-mini-gguf
- https://huggingface.co/FluidVoice-Labs/Qwen3.5-9B-4bit

## Verification and limits

- Final Rust library suite: 499 passed, zero failures, six ignored. Binary and
  example checking, TypeScript checking and the production Vite build passed.
- Sixteen browser checks passed with zero page errors. Three serialized live
  cleanup-style requests passed: bullets (467 ms), formal paragraphs (246 ms),
  and Raw layout (229 ms). No user document was used for these requests.
- Locally synthesized `Thank you` and `Bye` both passed the speech detector;
  silence after a real speech fixture was rejected. These checks do not use a
  text blacklist or send the synthesized fixtures to a cloud service.
- Reference speech was retained by the detector at original amplitude and after
  scaling by 0.1 and 0.025 (20 and 32 dB attenuation). Silence, DC, a fan-like
  tone and a single click were rejected.
- The same normal and attenuated reference was normalized and decoded through
  Nemotron 3.5, Parakeet v3 and live Groq Whisper large-v3. Normal and quiet
  transcripts matched for all three. Groq took 580 and 406 ms; warmed local
  decodes were approximately 0.5-0.8 seconds on this run. These are small-fixture
  measurements, not latency guarantees or evidence about every accent/noise.
- Browser checks cover independent model choices, all five presets, dark/light
  themes at 700 and 1160 px, and pill-state recovery when an event is missed.
- The microphone-to-paste path after an actual Windows suspend cycle was not
  simulated. User failures without retained audio cannot establish acoustic
  ground truth. Attenuated clean audio is not equivalent to every sore-throat
  voice or noisy room. Whisper CPU acoustic evaluation needs an installed model;
  no additional large model or GPU package was downloaded in this update.
- No phrase-specific correction for Vercel, FluidVoice or the user's examples
  was hard-coded. Add an unfamiliar name and its context to Dictionary to give
  cleanup a spelling reference; ambiguous speech may still be misrecognized.

## Optional focused check

With `TYPR_REVIEW_AUDIO=1`, stopped captures are retained locally under
`review-audio` before capture-error handling, including clips rejected as silent
or too short. Previously those failures returned before diagnostic saving, so
the very clips needed to investigate gate sensitivity were lost. Retention is
still off by default; this diagnostic change does not alter detection thresholds.

1. With cloud recognition and cleanup on, say at a comfortable quiet volume:
   `Deploy the preview to Vercel. Keep all final words.` Add Vercel to Dictionary
   with its product context first if you want a saved vocabulary reference.
2. Start a recording, stay silent for five seconds, then stop. Nothing should
   be pasted. Actually speaking `Thank you` or `Bye` must still be retained.
3. Choose different cleanup and Write Mode models, switch tabs and reopen the
   app. Both choices should remain independent. Verify the pill after a tray
   re-show or sleep/resume when convenient.
