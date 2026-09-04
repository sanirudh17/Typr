## [v0.1.7] - 2026-08-29 — Prose-vs-Command Guard & Always-Strip Filler

### 1. Terminal Prose No Longer Becomes a Command Chain
* **Prose Stays Prose:** `CONTEXT_TERMINAL` now explicitly keeps natural-language descriptions of GitHub actions as prose — e.g. `go ahead and merge it into the main branch and bump up the version and rebuild the app and push it to GitHub and write a changelog` stays `Go ahead and merge it into the main branch and bump up the version...` with caps/periods, not `git merge main && npm version patch && npm run build && git push ...`. `and` → `&&` and `git`/`npm`/`gh` insertion only happens when the user literally dictates the command syntax (`git merge main`, `npm version patch`). Fixes `instead of just typing it as text it converted into a command`.
* **Quoted-Thought Guard Hardened:** The `NEVER prepend git` rule now covers quoted thoughts as well — `I said the things in quotations only` never becomes `git I said...`.

### 2. Filler Toggle Removed — Always Strip When AI Is On
* **No Choice Needed:** Per request, removed `Settings → AI → Advanced → Developer → Strip filler words` toggle (`index.html`, `src/main.ts`, `src-tauri/src/settings.rs` field kept for compat but ignored). `src-tauri/src/recorder.rs` now `let strip_filler = settings.ai_enabled` — when `AI cleanup is on`, filler (`um/uh/er/you know/I mean`) is always stripped, even on bypass/fallback. The previous `Keep` option is gone.
* **Deduplication Always On:** `cleanup::deduplicate_text` (1-3 word consecutive repeat removal) remains always-on for Whisper/Parakeet stutters and chunk-join artefacts, as shipped in 0.1.6.

---
