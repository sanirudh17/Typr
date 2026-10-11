# Typr 0.3.1

Smaller Windows installers, optional NVIDIA support, lower idle model memory, and more reliable follow-up edits.

## Installation and updates

- Whisper uses CPU by default. The installer contains the CPU runtime rather than the large CUDA libraries.
- Speech models remain separate downloads. Existing settings, history, vocabulary, snippets, and downloaded models are preserved when updating.
- Users on 0.3.0 can use the in-app updater. Users on 0.2.2 or earlier must download and run the installer once because the updater signing key changed in 0.3.0.
- Download the EXE for the usual Windows installation or the MSI when an MSI package is required. Exit Typr from the tray before manually installing.

## Optional NVIDIA acceleration

- Choose NVIDIA GPU under Engine > Local Whisper > Whisper processing to download support separately.
- The official whisper.cpp b5454 archive is pinned and verified with SHA-256. Required runtime files and executable loading are checked before installation completes.
- The optional download is about 685 MB and uses about 1.2 GB after extraction. It is stored in app data and reused across updates.
- Delete NVIDIA support from Engine settings to reclaim that space. Removal returns Whisper to CPU and preserves speech models. The button remains available when CPU is selected.
- Runtime removal requires an idle recorder and stops the Whisper server before deleting its files. Remaining files after interrupted deletion can be removed by retrying.
- Failed NVIDIA execution can recover through CPU Whisper. CPU inference can take longer than accelerated inference; speed depends on the model and hardware.
- The EXE upgrade removes known obsolete bundled runtime files from earlier installations without deleting user data or speech models.

## Background memory

- Inactive Parakeet and Nemotron recognizers are released after changing engines.
- The remaining recognizer is released after three idle minutes while recording and processing are inactive. The periodic check can add up to thirty seconds before release.
- A later recording reloads the model, which can add a short cold-start delay after inactivity.
- Large raw-audio buffer allocations are trimmed after transcription. Active recordings are not evicted.

## Write Mode

- Follow-up edits use the current rewritten passage and the previous instruction, with the latest instruction taking priority.
- Supported editors can select the same unchanged passage again for up to five minutes. Moving to another field or window, or changing the passage manually, requires a fresh selection.
- Spoken editing instructions bypass Snippet replacements.
- Write Mode is unavailable in terminals, including Codex TUI and embedded xterm controls. Unsupported attempts stop before microphone capture or AI processing and display a clear message instead of a processing spinner.
- Regular editor rewriting and normal dictation into terminals remain available.
- Editor clipboard capture requires a fresh copy owned by the focused app and preserves newer user clipboard content.

## Verification and limits

- TypeScript checks, the production frontend build, the Rust regression suite, and native editor checks passed.
- CPU Whisper preserved every reference word in two warm-server runs. Nemotron preserved the short reference sentence; Parakeet's single omitted function word matched the previous release exactly.
- Live AI follow-up editing preserved the first edit while applying the later instruction to the current passage.
- CPU packaging, runtime removal with disposable files, Windows junction rejection, and the GPU archive's checksum and load probes were checked.
- Live GPU inference and live GPU-process removal were not exercised. Recognition remains dependent on the speech model and audio; this release does not promise perfect transcription.
- Updater signatures authenticate the downloaded update. They are separate from Windows Authenticode signing; Windows may still display an unknown-publisher prompt.
