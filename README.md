<div align="center">

<img src="src-tauri/icons/128x128@2x.png" width="112" alt="Typr icon" />

# Typr

**A fast, local-first voice-to-text dictation app for Windows.**

Press a hotkey, speak, and your words are typed straight into the window you are already in — no cloud required, no accounts.

[![Latest release](https://img.shields.io/github/v/release/sanirudh17/Typr?label=download&color=5e6ad2)](https://github.com/sanirudh17/Typr/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-5e6ad2.svg)](LICENSE)
[![Platform: Windows](https://img.shields.io/badge/platform-Windows%2010%2F11-0078d6.svg)](#download--install)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%20v2-24c8db.svg)](https://v2.tauri.app/)

</div>

---

## Overview

A voice-to-text dictation app for Windows. Press a hotkey, speak, and your words are typed into whatever window you are already in — Gmail, VS Code, WhatsApp, a terminal.

Transcription runs **fully offline** — on your CPU with Whisper, Parakeet or Nemotron, or with optional NVIDIA acceleration for Whisper — or through the **Groq Cloud API** when you want the lowest latency. An optional AI pass cleans up filler words and punctuation, and can match its writing style to the app you are dictating into.

> **New here?** Follow the step-by-step [**Setup guide**](#download--install) — about five minutes, and it covers the one-time model download.

---

## Contents

- [Features](#features)
- [How It Works](#how-it-works)
- [Choosing an Engine](#choosing-an-engine)
- [Settings Reference](#settings-reference)
- [Download & Install](#download--install)
- [Updating](#updating)
- [Building from Source](#building-from-source)
- [Tech Stack](#tech-stack)
- [Project Structure](#project-structure)
- [Configuration & Data](#configuration--data)
- [Privacy](#privacy)
- [Troubleshooting](#troubleshooting)
- [License](#license)

---

## Features

**Auto-Learning Dictionary**
- Manage preferred spellings, aliases, categories and context notes.
- Local correction observation is always active and suggests names you fix after dictation. The slim bar asks **Add ‘word’ to dictionary?** Choose **Yes** to add it; **No**, Escape or expiry discard it.
- Local Whisper uses ranked recognition hints even with AI cleanup off. Confirmed terms also accompany cloud recognition and context-aware AI cleanup automatically.
- Current Parakeet/Nemotron decoders do not support hotword biasing. Edit observation depends on Windows accessibility support; add terms directly in **Dictionary** when observation is unavailable.
- [Details, privacy, limitations and a short check](docs/AUTO_LEARNING_DICTIONARY.md).
- [Focused check using saved dictionary words](docs/DICTIONARY_FOCUSED_CHECK.md).

**Dictation**
- **Toggle or Push-to-Talk**, on a global hotkey (default `Ctrl+Shift+Space`) that works from any app.
- **Types directly into the focused window** rather than leaving text on your clipboard.
- **A second optional hotkey** that applies an AI profile on demand, even when the AI pass is otherwise off.
- **Live waveform overlay** while recording, so you can see audio is actually reaching the app.

**Three transcription engines**
- **Local Whisper on CPU, with optional NVIDIA acceleration.** Runs entirely on your machine. NVIDIA support is downloaded separately and includes its matching CUDA libraries, so no system-wide CUDA install is required. Four model sizes, ~190 MB to ~1.5 GB.
- **Remove NVIDIA support** from Engine settings to reclaim its disk space. Whisper returns to CPU and your downloaded speech models are kept.
- **Local Parakeet on the CPU.** NVIDIA's Parakeet TDT 0.6B, run in-process — no GPU, no sidecar process, no CUDA. One model, ~640 MB, covering 25 European languages with automatic detection.
- **Groq Cloud.** `whisper-large-v3` for accuracy, or `whisper-large-v3-turbo` for lower latency.

**Optional AI cleanup**
- Fixes spelling, casing, punctuation, filler words, and mis-hearings that only context can resolve.
- **Profiles** — *Cleanup* tidies what you said, *Prompt Mode* rewrites a spoken ramble into a structured prompt, and *Auto* matches the style to the app in front of you.
- **Auto context detection** picks Messaging, Email, Professional, Developer, or General from the focused application, with custom rules for anything it does not recognise.
- **Model choice** — Qwen 3.8 27B by default, or either GPT-OSS model. If one fails, another takes over; if all of them do, deterministic cleanup still runs, so a dictation is never lost.
- **Tone and formatting controls**, plus custom instructions applied to every pass.

**Voice commands** — spoken mid-sentence, applied offline, and independent of whether AI cleanup is on
- Casing: *"camel case get user by id"* → `getUserById`
- Spelling: *"spell G P U I end spelling"* → `GPUI`. Use *"spell golf papa uniform india end spelling"* if the speech model confuses individual letters. Recognized standalone letters are retained; spelling commands never guess letters missing from the transcript.
- Layout: *"new line"*, *"new paragraph"*, *"new bullet"*
- Symbols: *"open paren"*, *"semicolon"*, *"hyphen"*
- Editing: *"scratch that"*, *"delete last word"*, *"all caps that"*, *"make it a list"*

**Getting your words right**
- **Dictionary** — preferred spellings bias supported recognition engines and guide contextual AI cleanup. Current Parakeet/Nemotron decoders use these terms through AI cleanup. Dictionary never applies a deterministic replacement rule; ambiguous recognition can still be wrong.
- **Snippets** — exact find-and-replace rules. Good for a word that is always misheard the same way, or for expanding a shortcut like `brb` or `my email`. A shortcut dictated on its own pastes the saved expansion directly, without AI cleanup. Leave case matching off for spoken triggers, since speech engines often capitalize standalone phrases.

**Write Mode**
- Select text in an editable field, invoke the Write hotkey, speak an edit, and invoke it again.
- Subsequent edits reuse the previous unchanged passage for up to five minutes. The last instruction accompanies the current text as context; the latest instruction takes priority. Changing the document, field or app prevents an automatic replacement. Select a new passage to start over.
- Write Mode is unavailable in terminals, including Codex TUI and embedded xterm controls. Use an editable field for rewriting. Normal dictation still works in terminals.
- Clipboard contents are preserved during capture and editor replacement. Rewrites are excluded from dictation history.

**Background resource use**
- Inactive CPU recognizers are released after engine changes. After three minutes without local dictation, the remaining recognizer is released while the recorder is idle. The next use reloads it, which can add a short cold-start delay; prewarming begins at record-start where possible.
- Active recordings are never evicted. Large raw-audio allocations are trimmed after transcription; idle microphones remain paused.
- Local resource audits: `cargo run --example profile_model_memory` and `cargo run --example write_mode_smoke` from `src-tauri`. The editor smoke check creates a disposable Windows fixture and uses no cloud requests.

**Around the app**
- **History** with search and export to JSON or Markdown.
- **Tray / background mode** and optional start on login, so the hotkey works without the window open.

---

## How It Works

Everything between releasing the hotkey and text appearing runs as one pipeline. Each stage can fail without taking the dictation with it — if the network drops or a model misbehaves, you still get your words.

```
  hotkey released
        │
        ▼
  audio captured ──────────► resampled to 16 kHz mono
   (cpal, any rate)          (what every Whisper model expects)
        │
        ▼
  transcription ───────────► Local Whisper (CPU / optional GPU) · Local Parakeet (CPU) · Groq Cloud
                             long audio is split at speech boundaries first
        │
        ▼
  dictionary hints  ───────► terms you added bias what the engine hears
        │
        ▼
  snippets  ───────────────► exact find → replace rules
        │
        ▼
  email assembly ──────────► "name at gmail dot com" → name@gmail.com
        │
        ▼
  AI cleanup (optional) ───► profile + context decide the writing style
        │                     guarded: refusals, dropped URLs, reasoning
        │                     leaks and empty output all fall back
        ▼
  voice commands ──────────► casing, layout, symbols, editing
        │
        ▼
  typed into the focused window
```

**Why the ordering matters.** Email assembly runs *before* the AI pass, because the AI can only preserve an address it can recognise as one — five separate spoken words are not something it can protect. Voice commands run *last*, so nothing downstream can undo them, and they behave identically whether AI cleanup is on or off.

**The fallback chain.** The AI pass is guarded rather than trusted. If a model refuses, returns nothing, leaks its own reasoning into the output, or drops a URL or email address, the result is discarded and another model is tried. If that fails too, deterministic cleanup runs instead. The text you dictated is never lost to a failed AI call.

---

## Choosing an Engine

**Transcription**

| | Local Whisper | Local Parakeet | Groq Cloud |
|---|---|---|---|
| Runs on | CPU; optional NVIDIA CUDA | Any modern CPU | Groq's servers |
| Network | None | None | Required |
| Audio leaves your machine | No | No | Yes |
| Model download | 190 MB – 1.5 GB | ~640 MB | None |
| Languages | 99+ | 25 (v3) or English (v2) | 99+ |
| Speed, measured on an 82s dictation | ~12s | ~15s | ~2s |
| Best for | Sensitive work on a GPU machine | Machines with no NVIDIA GPU | Speed |

Speed figures are measured end-to-end on one machine, not vendor claims. The ordering matters
more than the numbers: Parakeet exists to make local transcription work **without** a GPU, not
to be the fastest option on a machine that has one.

Groq offers two speech models. `whisper-large-v3` is the more accurate of the two and is the default; `whisper-large-v3-turbo` trades some accuracy for lower latency.

**AI cleanup models**

The model picker discovers available Groq text models using your saved API key. It checks on launch, after a key change, and hourly (including while running in the tray). **Refresh** checks manually, with a short cooldown to avoid repeated requests. AI cleanup and Write Mode share this selection.

The last successful catalog is cached locally for offline use. Failed refreshes keep your selection. If a successful refresh no longer lists it, Typr chooses an available replacement, preferring the same family, and displays the change. Discovery sends no audio or transcript; only the API key authenticates the model-list request.

Groq's catalog does not declare model capabilities. Typr excludes known speech, speech-synthesis, safety, embedding and tool-agent families from the text picker. Future models using standard Chat Completions can be selected without an app update; a new model requiring a different API or special parameters may still need compatibility work. Availability and cleanup quality also depend on Groq's account permissions and the chosen model.

These starter models were measured end-to-end in-app, on one machine and network — treat as indicative, not benchmarks:

| Model | Typical latency | Notes |
|---|---|---|
| **Qwen 3.8 27B** *(default)* | ~150–450 ms | Fastest, and strongest on email layout and structured prompts |
| GPT-OSS 20B | ~700 ms – 1.5 s | Reasons before answering |
| GPT-OSS 120B | ~700 ms – 1.3 s | Largest |

Qwen is the bootstrap default. Fallback cleanup prefers another available family from the discovered catalog; before discovery succeeds, the existing Qwen/GPT-OSS fallback remains in use. Transcript-preservation checks apply to every selected model.

---

## Settings Reference

**General** — microphone selection; whether Typr keeps running in the tray when you close the window; whether it starts on login.

**Engine** — Local Whisper, Local Parakeet, or Groq Cloud. Whisper exposes model size and a download button; Parakeet exposes its variant (v3 multilingual or v2 English-only) and a download button; Cloud exposes your API key and the speed/accuracy choice.

**AI** — the cleanup toggle, and everything it controls:
- *Profile* — Cleanup, Prompt Mode, or Auto
- *Style* (Auto only) — pin one writing style, or let it detect from the focused app
- *Prompt format* (Prompt Mode only) — Natural prose or a Context / Task / Constraints / Output structure
- *Model* — available Groq text models, plus a Refresh action; shared with Write Mode
- *Presets, Tone, Formatting* — one-click combinations, or set tone and structure directly
- *Custom instructions* — applied to every pass
- *Advanced → App context rules* — force a style for a given app, or for a site by window title in any browser

**Commands** — a reference list of every spoken command, with examples.

**Recording** — Toggle or Push-to-Talk, the main hotkey, and an optional second hotkey with its own AI profile.

**Dictionary** — preferred spellings and context notes, with recognition hints where the engine supports them and contextual spelling guidance during AI cleanup.

**Snippets** — exact replacements when the recognized trigger matches. Standalone shortcuts paste their saved text directly, with AI on or off. Leave case matching off for spoken shortcuts; enabling it requires the transcript to use the same capitalization as the trigger. Use these when a hint is not forceful enough.

---

## Download & Install

**Requirements**
- Windows 10 or 11 (64-bit)
- An NVIDIA GPU is **optional** — see Step 2 for the engine that suits your machine
- Roughly 1 GB of free disk space for a local model (not needed if you use the Cloud engine)

### Step 1 — Install the app

1. Go to the [**Releases page**](https://github.com/sanirudh17/Typr/releases/latest).
2. Under **Assets**, download **`Typr_0.3.1_x64-setup.exe`** *(the `.msi` is an alternative if your workplace requires MSI)*.
3. Run it. Windows SmartScreen may warn you because the installer is not code-signed — click **More info → Run anyway**.
4. Launch **Typr**. It opens on the **Home** tab, which will be empty until you dictate for the first time. The tabs down the left side are where everything below happens.

> Typr keeps running in the system tray after you close the window, so the hotkey keeps working. That is intentional — find it by the waveform icon near the clock.

**You only have to do this once.** From 0.1.3 onward Typr updates itself — see [Updating](#updating) below.

### Step 2 — Pick your transcription engine

Open the **Engine** tab and choose **one**. This is the most important choice, and it decides what you set up next:

| Your situation | Choose | What it needs |
|---|---|---|
| You have an **NVIDIA** graphics card | **Local Whisper** | A one-time model download (Step 3) |
| **No NVIDIA card** (most laptops, AMD/Intel graphics) | **Local Parakeet** | A one-time model download (Step 3) |
| You want the **fastest** results and don't mind audio going to a server | **Groq Cloud** | A free API key (Step 4) |

Not sure whether you have an NVIDIA card? Press `Ctrl+Shift+Esc` to open Task Manager → **Performance** tab. If you see an entry starting with **NVIDIA**, choose Local Whisper. Otherwise choose Local Parakeet.

Both local engines run entirely on your computer, and nothing you say leaves the machine.

### Step 3 — Download a model (Local Whisper or Local Parakeet only)

**Neither local engine can transcribe anything until you do this.** Until a model is downloaded, pressing the hotkey produces nothing. Typr shows a yellow warning on the Engine tab while a model is missing.

**For Local Whisper:**
1. On the **Engine** tab, make sure **Local Whisper** is selected.
2. From **Model size**, pick **Medium · English (~539 MB) · recommended**.
3. Click **Download** and wait for the progress bar to finish. It only has to be done once.

**For Local Parakeet:**
1. On the **Engine** tab, make sure **Local Parakeet** is selected.
2. Leave **Parakeet model** on **v3 · 25 languages · recommended**.
3. Click **Download** (about 640 MB) and wait for it to finish. Once only.

Models are saved to `%APPDATA%\com.typr.app\`. Paste that path into the File Explorer address bar to see them.

**Skip to Step 5 if you chose a local engine and don't want AI cleanup.** You are done setting up.

### Step 4 — Get a Groq API key (only for Cloud transcription or AI cleanup)

A key is needed for **exactly two optional things**: the **Groq Cloud** engine, and the optional **AI cleanup** pass. If you are using a local engine with AI cleanup off, you do not need a key at all and can skip this step.

> ### ⚠️ Groq, not Grok
> The site is **`console.groq.com`** — **Groq** with a **Q**, an AI inference company.
> It is **not** `grok.com`, which is Elon Musk's Grok chatbot and a completely different
> product. A key from grok.com (or an OpenAI / ChatGPT key) **will not work** in Typr.
> Groq keys always start with **`gsk_`**.

**Getting the key:**

1. Go to **[console.groq.com](https://console.groq.com)** and sign in — you can use a Google account. It is free, and no credit card is asked for.
2. In the left sidebar, click **API Keys**.
3. Click **Create API Key**, give it any name (e.g. `Typr`), and confirm.
4. The key is shown **once**. Click **Copy**. It looks like `gsk_AbCd1234...`.
   If you close the dialog without copying, the key cannot be shown again — just delete it and create another.

**Putting the key into Typr:**

- **For Cloud transcription** — **Engine** tab → select **Groq Cloud** → paste into the **Groq API key** box.
- **For AI cleanup** — **AI** tab → turn **AI Cleanup** to **On** → paste into the **Groq API key** box that appears.

It is the same key and the same setting in both places — entering it in one fills in the other. Click anywhere outside the box to save; there is no Save button. The key is stored on your machine in `%APPDATA%\com.typr.app\config.json`.

### Step 5 — Dictate

1. Click into any window where you can type — Notepad, Gmail, WhatsApp, VS Code.
2. Press **`Ctrl+Shift+Space`** and speak.
3. Press **`Ctrl+Shift+Space`** again to stop. Your words are typed into that window.

A waveform overlay appears while recording, which confirms your microphone is being heard.

Out of the box the hotkey **toggles** — press once to start, once more to stop. If you would rather hold the keys down while speaking and have it stop when you let go, go to **Recording** → **Push-to-Talk**. Both modes use the same hotkey, which you can change on that tab.

If nothing is typed, see [Troubleshooting](#troubleshooting).

---

## Updating

From **0.1.3** onward Typr tells you when it can update itself. On startup it quietly asks GitHub whether a newer release exists; if one does, a message appears in the title bar — *"Typr 0.1.4 is available"* — with **Update** and **Later** beside it. Installing runs with a progress bar and no setup wizard: Typr closes, updates, and comes back. Your settings, history, dictionary, and downloaded models are all left alone.

**Upgrading from 0.2.2 or earlier:** download and run the installer from Releases once. The updater signing key changed in 0.3.0, so those older builds cannot verify updates with their original key. Installing over the existing copy keeps your local data and downloaded models. Users on 0.3.0 can use the in-app updater.

Nothing is ever downloaded or installed without you clicking **Update**.

**Pressing Later dismisses that version for good.** It will not reappear on later launches, no matter how many times you restart. When a *newer* version is released the message returns, because it only stays quiet about the exact version you dismissed. A dismissed update is never lost — **General → Updates** still lists it and will still install it on demand.

You can check whenever you like from **General → Updates**. A failed check on startup is silent by design (no network, or GitHub rate-limiting, is not something worth interrupting you for); press the button and it will tell you exactly what went wrong.

> **Coming from 0.1.0 or 0.1.1?** Those builds have no updater in them, so this one time you need to download and run the installer from the [Releases page](https://github.com/sanirudh17/Typr/releases/latest) yourself. Install it over your existing copy — nothing is lost. Every update after that is automatic.
>
> **On 0.1.2?** You can update from inside the app, but 0.1.2 has no title-bar message — it only shows the new version under **General → Updates**, so you have to look. From 0.1.3 on, Typr comes and tells you.

Version 0.3.1 includes the CPU runtime in the small installer. For optional NVIDIA support, open **Engine**, choose **NVIDIA GPU** under **Whisper processing**, then click **Download NVIDIA support**. The pinned, checksum-verified download is about 685 MB and occupies about 1.2 GB after extraction; it is reused across app updates. Use **Delete NVIDIA support** to reclaim that space and return Whisper to CPU. Speech models are separate downloads.

---

## Building from Source

**Prerequisites**
- [Node.js](https://nodejs.org) 18 or newer
- [Rust](https://rustup.rs) 1.75+ (stable)
- Visual Studio Build Tools with the C++ workload, and the [Tauri v2 prerequisites](https://tauri.app/start/prerequisites/)

### Sidecar binaries (required)

CPU Whisper runs through two sidecars. `src-tauri/binaries/cpu/` is gitignored.
Prepare the pinned official CPU runtime before compiling:

```powershell
powershell -NoProfile -File scripts/prepare-whisper-runtime.ps1
```

The script checks the archive hash, supplies target-suffixed `whisper-cpp` and
`whisper-server` executable names, copies their matching Whisper/ggml CPU DLLs,
and checks that both executables start. It refuses to overwrite a nonempty
folder or follow a directory junction. Only this CPU directory is bundled.

NVIDIA support downloads to a versioned `runtimes/whisper-cuda-b5454` folder in
app data when requested. The download is limited in size, checked against its
pinned SHA-256, extracted through a flat filename allowlist, and validated
before becoming selectable. CPU remains available if downloading or CUDA fails.
Local Parakeet and Nemotron do not need this download.

### Run and build

```bash
git clone https://github.com/sanirudh17/Typr.git
cd Typr
npm install

npm run tauri dev     # development
npm run tauri build   # installers
```

Installers land in:
- `src-tauri/target/release/bundle/nsis/Typr_0.3.1_x64-setup.exe`
- `src-tauri/target/release/bundle/msi/Typr_0.3.1_x64_en-US.msi`

Speech models are downloaded separately through Engine settings and stored in app data.

### Cutting a release (updater signing)

The updater only trusts installers signed with the private key matching `plugins.updater.pubkey`
in `tauri.conf.json`. Build the installers first, then sign and cryptographically verify the NSIS
installer using the secure local credential store. Keep private keys and passwords outside Git.
The signing tool requires a nonempty password and rejects the revoked key.

```powershell
npm run tauri -- build --ci --no-sign --bundles nsis,msi
powershell -NoProfile -File scripts/sign-local-release.ps1 -Version 0.3.1
powershell -NoProfile -File scripts/write-latest-json.ps1 -Version 0.3.1 -NotesFile release-notes-v0.3.1.md
```

The local signing script reads the external key and a Windows-protected password, restores its
temporary environment afterward, and refuses a signature that fails verification against the
configured public key. It writes a matching `.sig` next to the installer. Upload the installer,
MSI, `.sig`, BOM-free `latest.json`, and release checksums as GitHub release assets. The updater
fetches `latest.json` from the latest release tag. Do not publish an unverified signature.

```json
{
  "version": "0.3.1",
  "notes": "What changed",
  "pub_date": "2026-10-11T00:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "<contents of the .sig file>",
      "url": "https://github.com/sanirudh17/Typr/releases/download/v0.3.1/Typr_0.3.1_x64-setup.exe"
    }
  }
}
```

Keep the external updater key and password securely backed up. If the key is lost or rotated,
existing clients using that key need a manual installation to trust its replacement. Updater
signatures are separate from Windows Authenticode signing; SmartScreen may still warn.

### Tests

```bash
cd src-tauri && cargo test   # Rust unit tests
npx tsc --noEmit             # frontend typecheck
```

---

## Tech Stack

| Layer | Technology |
|---|---|
| Shell | [Tauri v2](https://tauri.app) |
| Backend | Rust |
| Frontend | TypeScript, vanilla CSS, [Vite](https://vitejs.dev) |
| Audio capture | [`cpal`](https://github.com/RustAudio/cpal) |
| Local inference | [`whisper.cpp`](https://github.com/ggml-org/whisper.cpp) on CPU or optional CUDA, run as a sidecar |
| Local Parakeet | [sherpa-onnx](https://k2-fsa.github.io/sherpa/onnx/) (statically linked), NVIDIA Parakeet TDT 0.6B |
| Cloud transcription | Groq (`whisper-large-v3`, `whisper-large-v3-turbo`) |
| AI cleanup | Groq chat completions (Qwen 3.8, GPT-OSS) |
| System integration | Win32 APIs for foreground-window detection, global hotkeys, keystroke injection |

---

## Project Structure

```
Typr/
├── index.html                  # single-page settings UI
├── src/                        # frontend
│   ├── main.ts                 # UI wiring and settings
│   └── style.css               # design system
└── src-tauri/
    ├── src/
    │   ├── recorder.rs         # the stop-to-text pipeline
    │   ├── audio.rs            # capture, resampling, level metering
    │   ├── transcribe_local.rs
    │   ├── transcribe_groq.rs
    │   ├── transcribe_parakeet.rs # in-process CPU engine, cached recognizer
    │   ├── audio_chunker.rs    # split long audio at speech boundaries
    │   ├── whisper_server.rs   # local sidecar lifecycle
    │   ├── ai_postprocess.rs   # profiles, prompts, model fallback
    │   ├── context_detector.rs # focused app → writing style
    │   ├── commands.rs         # spoken voice commands
    │   ├── dictionary.rs       # vocabulary persistence and snippets
    │   ├── vocabulary.rs       # ranking, hints and correction candidates
    │   ├── learning.rs         # observation sessions and confirmation
    │   ├── cleanup.rs          # deterministic fallback cleanup
    │   ├── hotkey.rs  paste.rs  history.rs  settings.rs
    │   └── ...
    └── tauri.conf.json
```

---

## Configuration & Data

Settings, history, dictionary, and snippets are stored as plain JSON in your user config directory:

```
%APPDATA%\com.typr.app\
├── config.json        # settings, including your Groq API key
├── dictionary.json    # vocabulary entries and snippets
├── history.json       # transcription history
└── ggml-*.bin         # downloaded Whisper models
```

Your Groq API key is stored in plaintext in `config.json`. It never leaves your machine except as an authorization header to Groq.

---

## Privacy

Where your audio and text go depends on which engine you choose, so it is worth being precise:

**Fully offline** — with the **Local Whisper** engine and **AI cleanup off**, nothing leaves your machine. Audio is transcribed on your CPU or your optional NVIDIA runtime and typed straight into the focused window.

**Sent to Groq** — two features are network-backed, and both are opt-in:
- The **Groq Cloud** engine uploads your recorded audio to Groq for transcription.
- **AI cleanup** sends the transcribed text to Groq, whichever engine produced it.

**Local storage** — history, dictionary, snippets, and settings stay on disk. Correction observation and pending suggestions stay local. Confirmed dictionary spellings, aliases and notes automatically accompany existing Groq/AI requests. Typr has no analytics, no telemetry, and no account.

**Window titles are never recorded.** Auto context detection reads the focused application to choose a writing style, but the debug log stores only the process name, window class, and resulting category — never the title of the window you are typing into.

---

## Troubleshooting

**Nothing happens when I press the hotkey.**
Another application may already own that combination — Windows gives it to whoever registered first. Typr keeps your previous hotkey and tells you when a binding fails, so check Recording for a status message and try a different combination.

**Local transcription does nothing.**
A Whisper model has to be downloaded first, from the Engine tab. Without one there is nothing to transcribe with.

**Local transcription is slow.**
Whisper is slower on CPU, especially with larger models. For NVIDIA acceleration, download GPU support in Engine settings and keep your NVIDIA driver current. Parakeet and Nemotron are alternatives optimized for local CPU use.

**Cloud transcription fails or retries.**
Check the API key in Engine, and that you have network access. Groq's free tier is rate-limited per minute — rapid back-to-back dictations can hit that ceiling, which shows up as a retry rather than an error.

**AI cleanup seems to do nothing.**
First check it has a key: **AI** tab → **Groq API key**. Without one, AI cleanup cannot run at all — Typr falls back to standard cleanup so your dictation is never lost, and shows a yellow warning on the AI tab saying so. It is also off by default; when off, the settings beneath it are hidden and a note says so. If it is on, has a key, and output still looks unstyled, the guards may be rejecting the result and falling back to deterministic cleanup — the log records which.

**My API key is rejected, or the key page looks nothing like the instructions.**
Check you are on **`console.groq.com`** — **Groq** with a **Q**. `grok.com` is Elon Musk's Grok chatbot, an unrelated product, and its keys will not work here. Neither will an OpenAI/ChatGPT key. A valid Groq key starts with `gsk_`.

**I downloaded a model but dictation still does nothing.**
Confirm the engine you selected is the one you downloaded for — the Whisper and Parakeet models are separate downloads, and selecting the other engine will report its own model as missing. The Engine tab shows a yellow warning whenever the selected engine has no model.

**Auto mode picks the wrong style.**
Add an App context rule under **AI → Advanced**. Leave the app blank and set only a title filter to match a website in any browser.

**Parakeet transcription fails to start.**
The model has to be downloaded from the Engine tab first — it is not bundled with the installer. Parakeet also needs a CPU with AVX support (Intel 6th generation / Skylake or the AMD equivalent, or newer).

**A long Parakeet dictation repeats a phrase.**
Long recordings are split into overlapping pieces so no words are lost at the joins, and the overlap is normally removed automatically. Occasionally the model repeats a phrase within a single piece, which the AI cleanup pass strips if it is enabled.

**A name is always transcribed wrong.**
Add the preferred spelling to Dictionary first. Whisper/Groq receive recognition hints; Parakeet/Nemotron require AI cleanup for contextual spelling guidance. Optional aliases and context notes can help disambiguate. If you want an unconditional replacement, use a Snippet.

---

## License

[MIT](LICENSE) © 2026 Sanirudh (sanirudh17)

Built on [Tauri](https://tauri.app), [whisper.cpp](https://github.com/ggml-org/whisper.cpp), [OpenAI Whisper](https://github.com/openai/whisper), [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) (Apache-2.0), and NVIDIA's [Parakeet TDT 0.6B v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3) (CC-BY-4.0, NVIDIA Open Model License). Cloud transcription and AI cleanup are served by [Groq](https://groq.com).
