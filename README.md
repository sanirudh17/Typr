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

Transcription runs **fully offline** — on your GPU with Whisper, or on any CPU with Parakeet — or through the **Groq Cloud API** when you want the lowest latency. An optional AI pass cleans up filler words and punctuation, and can match its writing style to the app you are dictating into.

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

**Dictation**
- **Toggle or Push-to-Talk**, on a global hotkey (default `Ctrl+Shift+Space`) that works from any app.
- **Types directly into the focused window** rather than leaving text on your clipboard.
- **A second optional hotkey** that applies an AI profile on demand, even when the AI pass is otherwise off.
- **Live waveform overlay** while recording, so you can see audio is actually reaching the app.

**Three transcription engines**
- **Local Whisper with CUDA acceleration.** Runs entirely on your machine. Ships its own CUDA libraries, so no system-wide CUDA install is required. Four model sizes, ~190 MB to ~1.5 GB.
- **Local Parakeet on the CPU.** NVIDIA's Parakeet TDT 0.6B, run in-process — no GPU, no sidecar process, no CUDA. One model, ~640 MB, covering 25 European languages with automatic detection.
- **Groq Cloud.** `whisper-large-v3` for accuracy, or `whisper-large-v3-turbo` for lower latency.

**Optional AI cleanup**
- Fixes spelling, casing, punctuation, filler words, and mis-hearings that only context can resolve.
- **Profiles** — *Cleanup* tidies what you said, *Prompt Mode* rewrites a spoken ramble into a structured prompt, and *Auto* matches the style to the app in front of you.
- **Auto context detection** picks Messaging, Email, Professional, Developer, or General from the focused application, with custom rules for anything it does not recognise.
- **Model choice** — Qwen 3.6 27B by default, or either GPT-OSS model. If one fails, another takes over; if all of them do, deterministic cleanup still runs, so a dictation is never lost.
- **Tone and formatting controls**, plus custom instructions applied to every pass.

**Voice commands** — spoken mid-sentence, applied offline, and independent of whether AI cleanup is on
- Casing: *"camel case get user by id"* → `getUserById`
- Layout: *"new line"*, *"new paragraph"*, *"new bullet"*
- Symbols: *"open paren"*, *"semicolon"*, *"hyphen"*
- Editing: *"scratch that"*, *"delete last word"*, *"all caps that"*, *"make it a list"*

**Getting your words right**
- **Dictionary** — names and rare terms are sent to the engine so it is likelier to hear them correctly. These bias recognition; they never rewrite your text, so ordinary words are never mangled.
- **Snippets** — exact find-and-replace rules that always fire. Good for a word that is always misheard the same way, or for expanding a shortcut like `brb` or `my email`.

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
  transcription ───────────► Local Whisper (GPU) · Local Parakeet (CPU) · Groq Cloud
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
| Runs on | Your GPU (CUDA) | Any modern CPU | Groq's servers |
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

All three run on Groq. Measured end-to-end in-app, on one machine and network — treat as indicative, not benchmarks:

| Model | Typical latency | Notes |
|---|---|---|
| **Qwen 3.6 27B** *(default)* | ~150–450 ms | Fastest, and strongest on email layout and structured prompts |
| GPT-OSS 20B | ~700 ms – 1.5 s | Reasons before answering |
| GPT-OSS 120B | ~700 ms – 1.3 s | Largest |

Qwen is the default because it was both quickest and produced the best-structured output in testing. The GPT-OSS models remain selectable and serve as the automatic fallback.

---

## Settings Reference

**General** — microphone selection; whether Typr keeps running in the tray when you close the window; whether it starts on login.

**Engine** — Local Whisper, Local Parakeet, or Groq Cloud. Whisper exposes model size and a download button; Parakeet exposes its variant (v3 multilingual or v2 English-only) and a download button; Cloud exposes your API key and the speed/accuracy choice.

**AI** — the cleanup toggle, and everything it controls:
- *Profile* — Cleanup, Prompt Mode, or Auto
- *Style* (Auto only) — pin one writing style, or let it detect from the focused app
- *Prompt format* (Prompt Mode only) — Natural prose or a Context / Task / Constraints / Output structure
- *Model* — the three above
- *Presets, Tone, Formatting* — one-click combinations, or set tone and structure directly
- *Custom instructions* — applied to every pass
- *Advanced → App context rules* — force a style for a given app, or for a site by window title in any browser

**Commands** — a reference list of every spoken command, with examples.

**Recording** — Toggle or Push-to-Talk, the main hotkey, and an optional second hotkey with its own AI profile.

**Dictionary** — recognition hints. These bias the engine and never rewrite text.

**Snippets** — exact replacements that always fire. Use these when a hint is not forceful enough.

---

## Download & Install

**Requirements**
- Windows 10 or 11 (64-bit)
- An NVIDIA GPU is **optional** — see Step 2 for the engine that suits your machine
- Roughly 1 GB of free disk space for a local model (not needed if you use the Cloud engine)

### Step 1 — Install the app

1. Go to the [**Releases page**](https://github.com/sanirudh17/Typr/releases/latest).
2. Under **Assets**, download **`Typr_0.1.2_x64-setup.exe`** *(the `.msi` is an alternative if your workplace requires MSI)*.
3. Run it. Windows SmartScreen may warn you because the installer is not code-signed — click **More info → Run anyway**.
4. Launch **Typr**. It opens on the **Home** tab, which will be empty until you dictate for the first time. The tabs down the left side are where everything below happens.

> Typr keeps running in the system tray after you close the window, so the hotkey keeps working. That is intentional — find it by the waveform icon near the clock.

**You only have to do this once.** From 0.1.2 onward Typr updates itself — see [Updating](#updating) below.

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

From **0.1.2** onward Typr updates itself. On startup it quietly asks GitHub whether a newer release exists; if one does, **General → Updates** shows the new version and a **Download & install** button. Installing runs with a progress bar and no setup wizard — Typr closes, updates, and comes back. Your settings, history, dictionary, and downloaded models are all left alone.

You can check whenever you like from **General → Updates**. A failed check on startup is silent by design (no network, or GitHub rate-limiting, is not something worth interrupting you for); press the button and it will tell you exactly what went wrong.

> **Coming from 0.1.0 or 0.1.1?** Those builds have no updater in them, so this one time you need to download and run the installer from the [Releases page](https://github.com/sanirudh17/Typr/releases/latest) yourself. Install it over your existing copy — nothing is lost. Every update after that is automatic.

Each update downloads the full ~300 MB installer rather than a small patch, because the bundled CUDA runtime dominates the package. It replaces your install in place; it does not accumulate copies.

---

## Building from Source

**Prerequisites**
- [Node.js](https://nodejs.org) 18 or newer
- [Rust](https://rustup.rs) 1.75+ (stable)
- Visual Studio Build Tools with the C++ workload, and the [Tauri v2 prerequisites](https://tauri.app/start/prerequisites/)

### Sidecar binaries (required)

The app runs `whisper.cpp` as a sidecar. `src-tauri/binaries/` is gitignored, so you must supply three executables yourself:

| File | Role |
|---|---|
| `whisper-cpp.exe` | CPU fallback |
| `whisper-cpp-cuda.exe` | GPU direct |
| `whisper-server-cuda.exe` | persistent HTTP server |

All three entries may be the **same compiled binary** — the app selects the execution mode at runtime with command-line flags.

*Build them:*

```bash
git clone --depth 1 https://github.com/ggml-org/whisper.cpp.git
cd whisper.cpp
cmake -B build -DGGML_CUDA=ON
cmake --build build --config Release

cp build/bin/whisper-cli.exe    ../Typr/src-tauri/binaries/whisper-cpp.exe
cp build/bin/whisper-cli.exe    ../Typr/src-tauri/binaries/whisper-cpp-cuda.exe
cp build/bin/whisper-server.exe ../Typr/src-tauri/binaries/whisper-server-cuda.exe
```

*Or download them* from the [whisper.cpp releases page](https://github.com/ggml-org/whisper.cpp/releases) and rename them as above.

For GPU acceleration, also place `cublas64_12.dll`, `cublasLt64_12.dll`, and `ggml-cuda.dll` into `src-tauri/binaries/`.

### Run and build

```bash
git clone https://github.com/sanirudh17/Typr.git
cd Typr
npm install

npm run tauri dev     # development
npm run tauri build   # installers
```

Installers land in:
- `src-tauri/target/release/bundle/nsis/Typr_0.1.2_x64-setup.exe` (~278 MB)
- `src-tauri/target/release/bundle/msi/Typr_0.1.2_x64_en-US.msi` (~436 MB)

To skip the first-run model download for your users, drop a `ggml-small.bin` into `src-tauri/binaries/` before building.

### Cutting a release (updater signing)

The updater only trusts bundles signed with the private key matching `plugins.updater.pubkey`
in `tauri.conf.json`. **A release built without the key still installs fine, but no existing
install will ever accept it as an update** — the signature check fails silently on the client.

```bash
export TAURI_SIGNING_PRIVATE_KEY="$HOME/.tauri/typr.key"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
npm run tauri build
```

This writes `Typr_<version>_x64-setup.exe` plus a matching `.sig` next to it. Upload **both**,
together with a `latest.json` describing the release, as assets on the GitHub release. The
updater fetches `latest.json` from the *latest* release tag, so it must be attached to
whichever release should be handed out.

```json
{
  "version": "0.1.3",
  "notes": "What changed",
  "pub_date": "2026-07-24T00:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "<contents of the .sig file>",
      "url": "https://github.com/sanirudh17/Typr/releases/download/v0.1.3/Typr_0.1.3_x64-setup.exe"
    }
  }
}
```

Keep `~/.tauri/typr.key` backed up somewhere safe and out of the repository. If it is lost,
existing installs can never be updated again — every user would have to reinstall by hand.

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
| Local inference | [`whisper.cpp`](https://github.com/ggml-org/whisper.cpp) with CUDA, run as a sidecar |
| Local Parakeet | [sherpa-onnx](https://k2-fsa.github.io/sherpa/onnx/) (statically linked), NVIDIA Parakeet TDT 0.6B |
| Cloud transcription | Groq (`whisper-large-v3`, `whisper-large-v3-turbo`) |
| AI cleanup | Groq chat completions (Qwen 3.6, GPT-OSS) |
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
    │   ├── dictionary.rs       # hints and snippets
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
├── dictionary.json    # hints and snippets
├── history.json       # transcription history
└── ggml-*.bin         # downloaded Whisper models
```

Your Groq API key is stored in plaintext in `config.json`. It never leaves your machine except as an authorization header to Groq.

---

## Privacy

Where your audio and text go depends on which engine you choose, so it is worth being precise:

**Fully offline** — with the **Local Whisper** engine and **AI cleanup off**, nothing leaves your machine. Audio is transcribed on your GPU and typed straight into the focused window.

**Sent to Groq** — two features are network-backed, and both are opt-in:
- The **Groq Cloud** engine uploads your recorded audio to Groq for transcription.
- **AI cleanup** sends the transcribed text to Groq, whichever engine produced it.

**Always local** — history, dictionary, snippets, and settings stay on disk. Typr has no analytics, no telemetry, and no account.

**Window titles are never recorded.** Auto context detection reads the focused application to choose a writing style, but the debug log stores only the process name, window class, and resulting category — never the title of the window you are typing into.

---

## Troubleshooting

**Nothing happens when I press the hotkey.**
Another application may already own that combination — Windows gives it to whoever registered first. Typr keeps your previous hotkey and tells you when a binding fails, so check Recording for a status message and try a different combination.

**Local transcription does nothing.**
A Whisper model has to be downloaded first, from the Engine tab. Without one there is nothing to transcribe with.

**Local transcription is slow.**
The local engine is only fast with CUDA. Confirm the three CUDA DLLs are present next to the sidecar binaries, and that your NVIDIA drivers are current. On a machine without an NVIDIA GPU, the Groq Cloud engine will be far quicker.

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
Try a Dictionary hint first, since it biases recognition without touching anything else. If it is still wrong, a Snippet will replace it outright.

---

## License

[MIT](LICENSE) © 2026 Sanirudh (sanirudh17)

Built on [Tauri](https://tauri.app), [whisper.cpp](https://github.com/ggml-org/whisper.cpp), [OpenAI Whisper](https://github.com/openai/whisper), [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) (Apache-2.0), and NVIDIA's [Parakeet TDT 0.6B v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3) (CC-BY-4.0, NVIDIA Open Model License). Cloud transcription and AI cleanup are served by [Groq](https://groq.com).
