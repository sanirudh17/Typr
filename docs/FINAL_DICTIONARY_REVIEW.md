# Final Dictionary review — October 10, 2026

## Latest microphone results

The newest logs show successful GPT-OSS cleanup for both local engines. The earlier four-second timeout is absent from these captures. Raw ASR already contains the unresolved names and ordinary-word errors.

| Capture | Engine | Recording | ASR | AI cleanup | Stop to completion |
| --- | --- | ---: | ---: | ---: | ---: |
| `1791653538609859600` | Nemotron 3.5, LiveOnly | 17.67 s | 187 ms | 2,451 ms | 3,225 ms |
| `1791653578657052600` | Parakeet v3 | 17.24 s | 1,325 ms | 1,923 ms | 3,991 ms |
| `1791653638271099100` | Parakeet v3 | 18.02 s | 1,395 ms | 1,953 ms | 4,123 ms |

For the 41.45-second Parakeet follow-up, ASR took 3,621 ms, AI cleanup 2,784 ms, and total completion 7,373 ms. Parakeet's batch work grows with recording length. Nemotron processes the long recording during capture and generally needs only a final live drain. Cloud cleanup still follows recognition for both engines.

Examples remaining in the source include Nemotron's `review order`, `order agent hand up`, `steady notes`, `keep this person`, and `hyperfix`; Parakeet's `Hory`, `Sarah`, `PyAnd`, and `clocant`. Some supported preferred spellings are recovered, but the system cannot establish every intended word from these ambiguous sources. A dictionary entry must not force a different proper name or ordinary meaning merely to satisfy a known test passage. This review does not certify perfect speech recognition.

## Speed check without cloud requests

Ran `python scripts/benchmark_local_latency.py --threads 4 6 --repeats 2 --engine both`. The script uses existing local WAV fixtures, performs a warm-up, and checks that the text is identical at each thread count. It makes no Groq calls. Its 20-second excerpt is a CPU throughput probe, not the production capture/normalization/streaming pipeline or an end-to-end benchmark.

| Engine | Excerpt | Four threads, median | Six threads, median |
| --- | --- | ---: | ---: |
| Parakeet v3 | Short fixture | 363 ms | 357 ms |
| Parakeet v3 | 20 seconds | 1,762 ms | 1,494 ms |
| Nemotron 3.5 | Short fixture | 960 ms | 712 ms |
| Nemotron 3.5 | 20 seconds | 3,723 ms | 4,531 ms |

Parakeet's current six-thread policy already captures the approximately 15% gain on this machine's longer probe. Nemotron's result varies by clip, so increasing threads is not a general speed improvement. Decoder settings, tail capture and final padding were retained. Parakeet chunk buffering stays opt-in because previous reference clips showed omissions; enabling it globally would conflict with the word-retention requirement. No new default engine tuning is justified by these results.

## Final implementation review

- Dictionary: bounded entry validation; disabled status, IDs and hit ranking; legacy migration; copy-before-save mutations; whole-import validation; atomic file replacement. Learned aliases remain spelling evidence for the model and never become Snippet replacement rules.
- Learning: read-only UI Automation, exact field identity and surrounding-document anchors, insertion verification, password/read-only exclusions, bounded observations/diffs, and cancellation on focus/session changes. Confirmation remains explicit, single-use and expiring; failed storage is retryable. Observed editor content is transient and has no new network path.
- Cleanup: same supported-spelling/fidelity checks for every speech engine and Auto profile; bounded model-chain deadlines; reserved alternate-model time; no partial output on truncation; failure notification; original entity/negation/final-word guards. Prompt instructions do not contain the saved test passage or its preferred terms as named correction rules.
- Auto layout: General, Messaging, Professional and Developer already request conditional bullet lists and short paragraphs. Email now explicitly requests lists for distinct items while preserving greetings, sign-offs and complete wording. Terminal prose can use lists; literal terminal commands remain exempt. Regression coverage checks every Auto category, retained final items and unchanged single-thought text.
- UI and integration: compact term rows, hover/keyboard actions, metadata disclosures, theme-aware learning bar and elapsed progress were covered by the previous interaction previews. Settings migration implements the user's always-on learning/cloud-hint choice. Main-window centering/bootstrap and the binary-only notification plugin boundary remain intact.

## Validation and merge scope

The TypeScript check, production Vite build and Rust binary check passed. The final `cargo test --lib -j2 -- --test-threads=2` run passed 463 tests, with zero failures and five intentionally ignored external/performance reviews, in 105.13 seconds. It includes local Nemotron reference accuracy/padding/live-tail checks and Parakeet reference thread, chunk and ending tests, plus dictionary storage/learning/cleanup/Auto-layout regressions. No cloud review was rerun in this final pass.

The completed checks support merging this software feature. The latest difficult-name passage is not perfectly recognized. Merge readiness concerns the reviewed implementation and its documented limits; it does not mean maximum or guaranteed accuracy for all speakers, names, variants or future recordings.
