# Auto-Learning Dictionary

Dictionary stores preferred spellings and optional aliases, categories and context notes in local `dictionary.json`. Existing spelling hints migrate without changing Snippets. Entries can be edited, disabled, deleted and searched. Dictionary import/export controls and history word-learning actions were removed at the user's request. Versioned JSON migration/round-trip utilities remain in the storage layer; they are not shown as app features.

## Where hints work

| Engine / pass | Dictionary support |
| --- | --- |
| Local Whisper, AI off or on | Budgeted recognition prompt (500 characters, up to 64 terms) |
| Groq Whisper | Budgeted recognition prompt automatically |
| Current Parakeet TDT variants | No hotword support in Typr's current Sherpa decoder |
| Current Nemotron streaming variants | No hotword support in Typr's current Sherpa decoder |
| AI cleanup, any engine | Context-aware spellings and aliases automatically |

Frequently used terms are ranked by successful dictation mentions, with three seats reserved for the newest enabled entries. Oversized entries are skipped at an entry boundary. Cleanup uses a separate bounded JSON vocabulary section (48 entries / approximately 4,000 characters). Enabled terms with exact, spacing or close phonetic evidence in the current transcript receive priority over unrelated terms. Possible heard forms are calculated locally and passed as suggestions for the LLM to assess; they never become stored aliases or deterministic replacements. Canonical-only entries therefore participate without manually entering aliases. The same prompt and retention checks apply to every engine. A listed alias is a spelling reference: the model may substitute it only when sentence context supports the preferred term. Ordinary meanings such as cloud computing and eating an apple must remain intact. A conservative validation check requires an adjacent product classifier, consistent with context notes when supplied, before accepting a single common-word alias (for example, cloud assistant with notes identifying an AI assistant). Ambiguous repeated common-word aliases receive no validation exception. This can miss a valid name; it avoids authorizing global replacements. Accuracy depends on the speech/cleanup model; hints are not guarantees.

Cleanup also receives bounded references for Typr and the app's speech model names. These references do not create dictionary entries or deterministic replacements. A disabled matching user entry stays disabled. Partial cleanup recovery retains an LLM-proposed dictionary spelling while restoring unrelated unsafe edits; the same checks still protect negations, numbers, literal tokens and final words. Logs report vocabulary length and the actual cleanup model/recovery status.

Dictionary never applies a deterministic find/replace rule. Only **Snippets** does that. Learned terms do not train model weights. Recognition hints do not require AI cleanup for local Whisper. Parakeet/Nemotron cannot gain ASR biasing from this feature with their current decoder support.

User spellings are read dynamically from enabled entries. The cleanup instructions contain no examples naming the saved test terms; a previous CAT paper illustration was removed to avoid biasing that check. The built-in Typr/Nemotron/Parakeet/Whisper references above remain explicit app vocabulary. Fixed passages and expected spellings in Rust regression tests are test data, not a production output lookup.

## Learn from corrections

1. Dictate into an editable Windows app and stay in that same field. Learning is always active.
2. Correct a misheard name or term. After the edit settles, Typr may show **Add ‘word’ to dictionary?** in a slim Yes/No bar, centered near the dictation pill.
3. Choose **Yes** to save the preferred spelling and heard alias. **No**, Escape, or expiry discard it. No suggestion silently adds a word.

The Dictionary page shows compact saved-term rows without an alias subtitle. **Add term** opens a small editor. Entry actions appear on hover or keyboard focus; clicking its preferred spelling reveals metadata. Category/context and learning options use the AI Advanced disclosure chevron. Optional aliases remain in the editor. The suggestion bar is 440 x 70 logical pixels, with a thin bottom line filling left to right until expiry instead of visible seconds. The two global enable switches have been removed as requested; older saved OFF values migrate to ON at startup. Individual entries can still be disabled.

Defaults: observation 60 seconds (10–60), suggestion 10 seconds (5–60), maximum corrected phrase 12 characters (2–32). Filtering excludes case/spacing/punctuation-only edits, common-word grammar fixes, pure additions/deletions, and recognizable accidental truncations. It favors unfamiliar capitalized names, product spellings, acronyms and technical tokens. Filters are conservative heuristics: they may miss a valid name or suggest an unfamiliar typo. Review each suggestion.

Windows UI Automation is used read-only. Typr verifies the inserted passage before observing edits, bounds the document and diff sizes, and stops on a focus/field change, another dictation, settings change, or Clear data. Surrounding document anchors and observed passages are transient; they are not written to history, logs, or a learning service. Password/read-only/unavailable fields and elevated apps that block observation require adding the term directly in Dictionary. Preparing the observer has a 600 ms limit before paste; unsupported providers do not prevent ordinary dictation.

Third-party editor accessibility varies. Observation is not guaranteed in browsers, terminals, rich document editors, remote desktops, or administrator windows. Edits outside the anchored passage cancel observation. Large passages/documents and ambiguous changes are ignored. Do not assume silence means an entry was learned.

## Manual entries

Use **Dictionary > Add term** when an app cannot expose corrections. History editing only saves history text; it does not propose or add vocabulary entries.

## Privacy

Detection, diffing and pending suggestions are local. Confirmed terms, aliases and notes automatically accompany existing Groq/AI requests. Unconfirmed suggestions and observed editor text are never sent. Normal cloud transcription/AI cleanup still follow their existing data flow. Keep sensitive names/notes out of vocabulary sent to cloud models.

## Quick check (about one minute)

1. In Notepad, dictate a short sentence containing an uncommon product name. Correct its misheard spelling within 60 seconds and leave the field untouched for about two seconds. Check the slim, centered bar; choose **Yes** and verify the entry and alias in Dictionary. If observation is unsupported, use **Add term** in Dictionary.
2. With AI cleanup on, say: **Open the Typr repository. The picture shows our team taking a shot. Keep all final words, including robust decoding algorithms.** Check the name, literal team/shot wording and complete ending. Use your usual model; no full model matrix is needed.
3. Check the saved-term list and **Add term** editor. Hover a row for actions or click its spelling for details. Expand learning options only if you need timing controls.

For recognition bias, local Whisper or Groq can use hints with AI off. Parakeet/Nemotron require AI cleanup for contextual dictionary corrections with their current decoders. Dictionary does not force every occurrence of an alias to change. No extra recording is needed for storage controls.

## Spelling letters

Voice commands run locally for every speech engine with AI on or off. Say **spell G P U I end spelling** to insert `GPUI`. For more reliable recognition of individual letters, use **spell golf papa uniform india end spelling**. A recognized standalone letter is no longer discarded just because recording lasted over one second. Three or more separately recognized letters (upper or lower case) can also form an uppercase acronym; ordinary words such as **coding agent built in Rust** remain separate. Commands cannot reconstruct letters the ASR never recognized. Silence rejection still uses audio signal levels.

Short ordinary cleanup requests share a six-second deadline across both model attempts. The primary GPT-OSS attempt gets four seconds, reserving two seconds for Qwen; the primary Qwen attempt gets three seconds, reserving three seconds for GPT-OSS. Longer passages receive a length-dependent shared deadline capped at ten seconds. Successful requests return immediately. This reduces timeout fallback without adding a fixed delay; cloud cleanup can still fail or time out. Short Qwen requests use a bounded output allowance instead of reserving the maximum long-transcript quota. Truncated replies remain failures and are never pasted as complete text.

## Streaming and cleanup latency

Nemotron is a native online recognizer. Healthy longer sessions finish from the live stream; clips of eight seconds or less use one batch decode to recover difficult final words. Sparse output can also trigger verification. Parakeet TDT v2/v3 is an offline model; the v3 buffered worker processes chunks during capture only when TYPR_PARAKEET_BUFFERED=1. It remains opt-in because earlier reference recordings showed more omissions with chunk-local normalization. Whisper uses its existing server/batch path, not a native streaming decoder. Streaming recognition does not remove the subsequent cloud cleanup request.

Measured user logs: short Nemotron engine work was roughly 0.5 to 1.2 seconds, after a 400 ms capture tail; cleanup added roughly 0.6 to 1.4 seconds. A 202-second live recording finished ASR in 495 ms but cleanup needed 5,789 ms with a retry. GPT-OSS is kept at medium reasoning to preserve the existing quality choice; the faster Qwen model remains selectable. Capture-tail draining, final padding and no-word-loss checks were retained. These observations do not establish a fixed end-to-end latency promise.

### October 10: reported cleanup failures

The supplied Nemotron capture `1791651482217119500` finished live ASR in 323 ms, then exhausted the previous 4,000 ms cleanup deadline. Its final text therefore received basic cleanup rather than AI spelling corrections. Dictionary hints were present. The Parakeet capture `1791651535752806300` completed ASR in 1,298 ms and AI cleanup in 2,563 ms; it used partial recovery, which could split a supported multiword spelling at an unchanged internal word and restore its misheard form.

Cleanup now aligns against supported dictionary spellings actually proposed by the LLM before recovering unrelated edits. Original-input spans are resolved longest-first so a short name does not block a complete compound term. Bounded word-boundary suggestions retain the adjacent app/tab word; they cannot absorb it or invent a classifier. These rules are generic and read saved terms dynamically. They do not apply dictionary find/replace to the output. Negation, literal-token and final-word guards remain active.

The exact two supplied ASR transcripts were replayed through the production cleanup chain with GPT-OSS and Qwen. All four requests completed with AI text after the final changes. Supported corrections such as Tauri, Synara, Pi agent and CAT paper survived; neighbouring app/tab words were retained. GPT-OSS cleanup took 3,845 ms for the Nemotron source and 2,170 ms for the Parakeet source; Qwen took 492 ms and 392 ms. A separate slow-primary replay exercised a successful Qwen fallback after 4,623 ms, and a deterministic regression test verifies that the cancelled primary leaves time for that retry.

These replays do not show every intended word recovered. Nemotron's `revieward`, `keephis` and `hyperfix`, and Parakeet's `review her`, `broad assistant` and `keep this first`, can remain because the ASR text is ambiguous. The dictionary cannot establish which missing or different words were spoken. Earlier rapid review requests hit Groq rate limits; paced final replays passed, but network/service quotas remain possible failures. Existing fallback notification remains active when AI cannot be applied.

## Implementation and validation

See the [final review of the latest microphone logs, local speed check and Auto layout](FINAL_DICTIONARY_REVIEW.md). Email now carries the explicit conditional bullet-list rule used by the other Auto prose profiles. No Groq requests were made during that final pass.

`vocabulary.rs` owns portable entry validation, ranking, budgeting and bounded word-LCS correction extraction. `dictionary.rs` persists entries with a temporary-file rename and migrates legacy data. `selection.rs` owns Windows access; `learning.rs` owns transient sessions and confirmation. Unit tests cover candidates, exclusions, budgets/ranking, whole-term hit accounting, legacy migration, JSON round trips, import atomicity, disabled status, explicit/single-use acceptance, expiry and failed saves. UI captures use synthetic vocabulary; they do not prove every third-party app's accessibility behavior.

Checked on 2026-10-10:

- TypeScript `tsc --noEmit` and `npm run build`: passed.
- Rust `cargo check --bins -j2`: passed.
- Rust `cargo test --lib -j2 -- --test-threads=2`: 463 passed, 0 failed, 5 intentionally ignored external/performance review tests; final run 105.13 seconds. Includes explicit/NATO spelling, retained standalone letters, acronym/code-word retention, phonetic phrase boundaries, common-word context guards, length-dependent cleanup budgets, reserved retry time, multiword dictionary recovery, neighbouring-word retention and adaptive layout in every Auto profile. Real local-model fixtures cover Nemotron live tails, Parakeet batch seams/ending and its optional buffered worker.
- Explicit live cleanup review: GPT-OSS 120B and Qwen both corrected Typr, Parakeet and Supabase in a fixed synthetic passage, retaining literal team/shot wording, the final decoding phrase and ordinary cloud-computing meaning. Cleanup requests took 2,065 ms and 636 ms respectively; these are not end-to-end dictation benchmarks.
- Canonical-only entries were also tested without aliases or notes. Both models corrected CAT paper, Glint, InsForge, Synara, herdr-agent-handoff and Pi agent, then handled a passage covering all ten saved terms, including Tauri, Herdr, Claude, terse and hyper. Both retained ordinary glint, pet cat, cloud computing, negations and final words in the context check. The final eight-request review passed without any retry or partial recovery. GPT-OSS took 1,602 to 2,839 ms; Qwen took 315 to 757 ms. An earlier unpaced review hit provider rate limits; only the explicit review requests are paced, with no added production delay. These are synthetic text cleanup checks, not microphone accuracy guarantees.
- Browser preview: add/edit/disable/enable/delete/search, hover/focus actions, focus restoration after save/cancel/toggle/delete, removed history-learning/import/export controls, editable timing controls, moving progress and retryable save errors passed. Dark/light views at 1160 × 720 and 700 × 500, plus the 440 × 70 suggestion bar, had no horizontal overflow or JavaScript errors. Eleven synthetic entries use 53-pixel closed rows at both sizes. Expanded details/options, editor dropdowns, popup/error/focus/long-text and corrected dark hover states were reviewed. Preview commands used mocked Tauri responses and synthetic data.
- After removing named dictionary examples from the cleanup instructions, an explicit live review passed on GPT-OSS and Qwen for all ten saved terms plus two previously unused synthetic entries (VelaForge and Mirovia), with no aliases, notes, retry or recovery. Both retained ordinary meanings and the complete ending. A further replay with the final boundary rules passed: positive cleanup took 2,762 ms / 980 ms and ordinary-meaning checks took 1,725 ms / 386 ms (GPT-OSS / Qwen). These are model-request measurements, not microphone or paste timings.
- The final four-request replay of the reported Nemotron/Parakeet source text passed through the production model chain. It verifies cleanup completion and specific supported spellings, not recovery of all ten names from heavily misheard speech. Local fixtures and deterministic regressions run without a cloud key; live reviews require explicit execution and credentials.

The user confirmed the native suggestion bar works and saves a spelling. Current row density and popup wording are covered by staged previews; spoken spelling commands and actual Groq/Whisper recognition bias still depend on the active ASR. Passing unit tests and a synthetic cloud cleanup review does not establish recognition accuracy or observation support in every external editor.

Reference repositories were read for architecture and UX patterns only: [OpenWhispr engine-specific dictionary budgets](https://github.com/OpenWhispr/openwhispr/blob/main/src/utils/dictionaryPromptCap.js), [FluidVoice acoustic vocabulary boosting](https://github.com/altic-dev/FluidVoice/blob/main/Sources/Fluid/Services/ParakeetVocabularyStore.swift), [OpenLess vocabulary/learning core](https://github.com/Open-Less/openless/tree/beta/openless-all/app/crates/openless-core/src) and [FluidVoice correction observation](https://github.com/altic-dev/FluidVoice/blob/main/Sources/Fluid/Services/AutomaticDictionaryCorrectionTracker.swift). No source from their AGPL/GPL implementations was copied into Typr.
