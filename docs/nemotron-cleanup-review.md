# Nemotron cleanup review — October 10, 2026

## Finding

The latest local log shows ordinary dictation using Nemotron 3.5 with GPT OSS 120B and Auto mode. Notepad resolved to Professional; Orca resolved to Developer. AI completed successfully with `recovered=true`, rather than being disabled or skipped. However, recovery restored malformed words from the raw transcript when its preservation rules rejected the AI's spelling or grammar repairs.

A live replay of the user's review passage reproduced this with both GPT OSS and Qwen: the service corrected `vulabilities` and `issu`, but Typr restored those errors. The flat two-letter spelling limit also missed inflected dictionary words, the ordered word comparison rejected a missing infinitive `to`, and a fused word such as `reviewon` could not be split.

## Changes

| Raw transcript | Repair now retained | Limit |
| --- | --- | --- |
| `vulabilities` | `vulnerabilities` | Bounded longer spelling repair; known English destination and matching beginning/end |
| `issu` | `issue` or `issues` | Known word completing an exact malformed prefix |
| `reviewon this app` | `review on this app` | Exact fused spelling, ordinary words, one function word; no arbitrary name or identifier splitting |
| `I want you do a review` | `I want you to do a review` | Request/action context with no sentence or paragraph boundary |

Safe grammar repairs survive local recovery when another part of the completion needs restoration. Recovery also keeps source spacing when a rejected compound would otherwise produce malformed hyphenation such as `repo-in depth`.

`flock` was missing from the compact ordinary-word lexicon. It is now protected: a model's uncertain change to `knock`, `lock`, or `find` must not replace that valid source word. Existing protections for complete clauses, final qualifiers, negations, tense, ownership, URLs, paths, numbers, and shortcuts remain. Tests cover the new repairs across Cleanup, Messaging, Email, Professional, and Developer. The shared checks apply to all speech engines; Write Mode keeps its separate editing contract.

## Validation

- Full library suite: **423 passed, 0 failed, 2 ignored**. This includes existing local-model finalization and golden-clip checks.
- Four new offline regression tests cover valid repairs, recovery amid unsafe substitutions, unchanged valid words/literals, and sentence boundaries.
- An explicitly enabled Groq replay passed three cases: the review passage with GPT OSS 120B, the same passage with Qwen, and the fused-word passage with GPT OSS. Safe spelling, grammar, and spacing survived while final clauses and uncertain source words were retained. Final request-plus-validation times were approximately **1.82 s**, **0.65 s**, and **2.00 s**, respectively; these are individual requests, not latency guarantees or microphone-to-paste measurements.
- TypeScript, Vite production build, Rust check, and Rust binary build passed.
- The user separately confirmed Write Mode rewriting, Undo, cancellation, ordinary dictation, and the layout fix. Earlier native Undo checks passed in Notepad, Chrome, and Edge; Word remains unverified.

## What this establishes

This fixes Typr discarding supported cleanup repairs. It does not change Nemotron's weights, decoder, streaming context, audio processing, or AI-off recognition. The latest microphone audio was not retained, so there is no new acoustic comparison on that recording. The service received fixed text cases already captured in the log; no new microphone recording was made by the agent.

Valid but misheard words and ambiguous false starts can remain. Without audio, replacing them with a fluent guess could change the user's meaning. The user's before/after Write example is evidence that explicit editing works, not a requirement for ordinary cleanup to make the same rewrite. This review does not establish perfect transcription, universal app compatibility, or repository-wide vulnerability coverage.
