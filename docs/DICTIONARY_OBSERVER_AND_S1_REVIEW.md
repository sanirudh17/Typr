# Dictionary observation and local cleanup review

## Correction observation

The observer now resolves an editable UI Automation owner when Chromium focuses
a descendant inside a contenteditable field. It also supports bounded ValuePattern
inputs without a TextPattern selection: the before/after snapshots must prove a
unique insertion before any correction can be learned. Identical pre-existing
text, ambiguous insertions and another field are rejected.

Transient accessibility failures are retried for up to four seconds, instead of
ending observation on the first failed poll. Provider warm-up runs while recording
only when learning is enabled, and does not read editor text. Actual observation
still starts around insertion and requires verification of the pasted text.

This does not guarantee support in every ChatGPT, Claude or other web editor.
The browser must expose a stable editable accessibility target. Replacing the
editor element, navigating away, privilege isolation, large documents and terminal
surfaces can prevent observation. Passwords and read-only content remain excluded.
There is no clipboard polling or global keystroke capture. Suggestions still need
explicit confirmation; common grammar, casing-only edits and uncertain candidates
remain filtered. Add a term on the Dictionary page when observation is unavailable.

The disposable Chrome end-to-end fixture could not reliably acquire foreground
focus in this automation session, so native popup behavior in ChatGPT/Claude is
not claimed as verified. The fixture rejects a non-fixture foreground window
before preparing observation. No personal document was used for this check.
The available desktop automation provider also reports no window-focus support.
Unit tests cover unique insertion binding, ambiguity rejection and Unicode;
existing confirmation and expiry tests remain in the Rust regression suite.
Final validation: 500 Rust library tests passed, zero failed, six ignored;
TypeScript checking and the Vite production build also passed.

One focused end-to-end check, when running the worktree app: dictate `Open the
orvexer app.` in a browser editor, manually change `orvexer` to a new spelling
`Orvexa`, then wait about two seconds without leaving that editor. A suggestion
should appear if that field exposes the required accessibility interfaces.
Use a term not already saved with that alias; existing learned pairs are not
suggested repeatedly. Accepting should be the only step that stores the term.

## S1-mini by Superwhisper: CPU evaluation

The existing cached Q4_K_M model was tested without a new download or GPU use.
Its SHA-256 was checked against the pinned model artifact. The trial uses the
model card's normalization system instruction, styling/structure/context control
line, disabled thinking, four CPU threads and greedy generation.

- Twelve synthetic fixtures covered punctuation, negations, technical terms,
  names, literal paths/URLs, questions, self-correction, email, currency, fillers,
  a list and a longer final sentence.
- Ten passed the fixture assertions. Numbered lists count as valid lists.
- Two failed meaning preservation: `schedule the meeting` became `The meeting
  keep...`, and `never replaces ordinary words automatically` was split so
  `Automatically` modified the following instruction instead.
- Most short requests in the first run took approximately 0.2–0.5 seconds. Its
  first request took 10.7 seconds and the longer request took 3.7 seconds. A
  second run reproduced both quality failures, with the longer request at 1.9
  seconds. These timings are machine/cache dependent, not guarantees.

The [second-run inputs, outputs and timings](s1-mini-cpu-results.json) are kept
with this review; its median was 451 ms. A passing assertion checks selected
properties and does not imply that every detail of the output is correct.

Conclusion: promising for basic normalization, but this trial does not establish
parity with cloud cleanup. It is not enabled or integrated into Typr's production
cleanup choices. The model also cannot recover words that the ASR never recognized.

Reproduce with an existing installation (no automatic downloads):

```powershell
node scripts/verify-s1-mini.cjs
```

By default it reads `%APPDATA%\com.typr.app\local-cleanup`. Set `TYPR_S1_DIR`
to a directory containing `s1-mini-q4_k_m.gguf` and `runtime\llama-server.exe`.
Set `TYPR_S1_OUTPUT` to choose the report directory. Synthetic inputs/results and
the local server log otherwise go under `.impeccable/review`. No personal text
or cloud key is used. This small fixture set is not a general quality benchmark.

References: [model card](https://huggingface.co/superwhisper/s1-mini),
[GGUF instructions](https://huggingface.co/superwhisper/s1-mini-GGUF).
