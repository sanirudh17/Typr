# Accuracy update: final merge review

Reviewed on October 9, 2026. Approved for a local fast-forward into `main` from
`sanirudh17/Accuracy-improvisations-and-general-fixes`. Both branches and the
freshly fetched `origin/main` started at `12c328e87107f1faaa7e878724ca617b90417312`.
This review does not publish an installer or update the remote branch.

## Checks rerun

| Check | Result |
| --- | --- |
| Rust library suite, including native Nemotron/Parakeet replay tests | 416 passed, zero failed; one manual timing benchmark ignored |
| Raw stereo WAV preparation fixture | One passed |
| Rust application and every example/test target | `cargo build --all-targets` passed |
| TypeScript and production frontend | `tsc --noEmit` and `npm run build` passed |
| Clear data browser fixtures | Light/dark, 1160x720 and 700x500, keyboard focus, Escape/cancel, busy/error states, history refresh and retired Undo/delete handlers passed |
| Dialog contrast | Body text, buttons and error text pass 4.5:1, including the destructive button's hover state in both themes |
| Signing suite | Eight passed, including UTF-16 LE/BE scanner fixtures and cryptographic tamper rejection |
| Replacement signing credentials | Signed a disposable fixture and verified its artifact and trusted comment against the configured public key |
| Repository signing-secret scan | Passed; credentials remain outside the repository under the current user's restricted ACL |
| Windows runtime setup | Complete cached, checksum-verified archive installation passed; all three executables loaded |
| Helper syntax and whitespace | PowerShell/Python helper parsing and `git diff --check` passed |

The scoped UI audit used the existing app theme, typography and dialog behavior
as authority. The detector also flagged pre-existing muted labels and decorative
styles elsewhere in the app; this update does not redesign those surfaces.
The new dialog's contrast was checked on the rendered output rather than inferred
from token names. Browser tests use mocked Tauri data and never clear live history.

## Issues cross-checked

- **Dropped local final words:** microphone release grace, unread stereo frames,
  continuous resampling, final filter drain and Nemotron's 1.25-second flush are
  retained. Missing/empty audible local decoder chunks fail or recover rather
  than producing a successful partial transcript. Seam merging keeps unmatched
  words and protects numbers and negations.
- **All transcription engines:** shared capture gain, batch resampling,
  conservative vocabulary normalization, dictionary handling and AI preservation
  guards are used across the applicable engine paths. Local Whisper keeps its
  explicit CPU recovery; Parakeet's measured decoder/level adjustment remains
  specific to that model family.
- **Nemotron delay:** healthy long streaming dictations skip an unused batch WAV;
  review audio writes occur in the background. Short clips retain their batch
  safeguard without a redundant live final decode.
- **Parakeet speed:** the measured greedy decoder and CPU thread policy remain.
  V3 buffered streaming stays opt-in because its internal word omissions did not
  meet the default accuracy requirement. V2 retains the batch path.
- **AI enabled but apparently off:** safe recovery retains mode-specific AI
  layout while restoring rejected spans from the transcript. A bounded repair
  attempt keeps the same effective mode and style instructions. Final failure
  preserves the source and displays a notification; it does not disable AI.
- **Prompt regressions:** ordinary wording, complete clauses, dictionary spelling,
  anti-quote/anti-camelCase rules and intelligent paragraph/list layout remain
  guarded across Cleanup and every Auto profile. Prompt Mode and terminal input
  keep their distinct contracts.
- **Microphone gain:** the shared fixed +12 dB digital gain is retained; it is
  neither a hardware volume control nor a guarantee of hearing every word.
- **Clear data:** only generated history, logs and eligible recordings are
  cleared. Settings, dictionary, credentials, models and unrelated files remain.
  The operation is blocked during dictation and waits for review audio saves.
- **Window/dropdown/notification invariants:** centered show paths and custom
  selects remain. Native notification plugin calls still live only in the binary.
- **Signing key:** embedded private-key literals are removed, the exposed key is
  rejected, and the replacement is encrypted outside Git. Existing installed
  clients still need a reviewed release/manual migration to the replacement key.

## Additional fixes from this review

1. The runtime helper used `--version`, which the pinned Whisper build does not
   support. It now checks the supported help entry point for all three binaries,
   captures both streams without PowerShell 5 stderr failures, and bounds startup.
2. The Clear data hover tint is reduced so small red text remains readable in
   both themes. The browser fixture now asserts rendered contrast.
3. The signing scanner now checks BOM-marked UTF-16 exports rather than skipping
   them as binary files. Tests cover both byte orders without printing credentials.
4. The original `main` checkout's ignored Whisper runtime was incomplete and
   exited with `STATUS_DLL_NOT_FOUND`. Its ten original files were copied and
   checksum-verified in
   `%LOCALAPPDATA%/Typr/runtime-backups/20261009-before-accuracy-merge` before
   installing the worktree's 25 matching executable/DLL files. All three now load.
   Binary assets remain ignored; a fresh clone uses the documented setup script.

## Merge reasoning and limits

The passing native tail/seam checks, shared preservation tests, frontend checks,
dialog fixtures and signing verification support merging this update. The three
existing untracked exports in the main checkout (`CHANGES.md`, `TESTING_SUMMARY.md`
and `changes.patch`) are unrelated and are excluded from the commit.

These checks prevent the known application-caused losses and unsafe AI rewrites;
they cannot establish perfect recognition for every voice, microphone or model
variant. Known acoustic errors remain documented in [the evaluation](accuracy-evaluation.md).
No new cloud requests or live history deletion were needed for this final review.
The public history still contains the compromised old key; this local merge does
not rewrite that history or migrate already-installed clients.

Fresh review logs and screenshots are stored outside Git under
`%LOCALAPPDATA%/Temp/typr-accuracy-evaluation-20261008/merge-review-20261009`.
