# Typr

[![Release](https://img.shields.io/badge/release-v0.1.0-5e6ad2)](https://github.com/sanirudh17/Typr/releases)
[![License](https://img.shields.io/badge/license-MIT-green)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Windows-0078d4)](https://github.com/sanirudh17/Typr/releases)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%20v2-24c8db)](https://tauri.app)

A voice-to-text dictation app for Windows. Press a hotkey, speak, and your words are typed into whatever window you are already in — Gmail, VS Code, WhatsApp, a terminal.

Transcription runs either **fully offline** on your own GPU, or through the **Groq Cloud API** when you want speed on a machine without one. An optional AI pass cleans up filler words and punctuation, and can match its writing style to the app you are dictating into.

---

## Contents

- [Features](#features)
- [How It Works](#how-it-works)
- [Choosing an Engine](#choosing-an-engine)
- [Settings Reference](#settings-reference)
- [Download & Install](#download--install)
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

**Two transcription engines**
- **Local Whisper with CUDA acceleration.** Runs entirely on your machine. Ships its own CUDA libraries, so no system-wide CUDA install is required. Four model sizes, ~190 MB to ~1.5 GB.
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
  transcription ───────────► Local Whisper (GPU)  or  Groq Cloud
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

| | Local Whisper | Groq Cloud |
|---|---|---|
| Runs on | Your GPU (CUDA) | Groq's servers |
| Network | None | Required |
| Audio leaves your machine | No | Yes |
| Model download | 190 MB – 1.5 GB, once | None |
| Best for | Sensitive work, offline use | Machines without an NVIDIA GPU |

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

**Engine** — Local Whisper or Groq Cloud. Local exposes model size and a download button; Cloud exposes your API key and the speed/accuracy choice.

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

Grab the latest installer from the [Releases page](https://github.com/sanirudh17/Typr/releases):

- **`Typr_0.1.0_x64-setup.exe`** — NSIS installer (recommended, smaller)
- **`Typr_0.1.0_x64_en-US.msi`** — MSI installer

**Requirements**
- Windows 10 or 11 (64-bit)
- An NVIDIA GPU for the local engine's CUDA acceleration. Without one, use the Groq Cloud engine.
- A [Groq API key](https://console.groq.com) if you want cloud transcription or AI cleanup. Both are optional.

### First run

A Whisper model must be downloaded before local transcription will work:

1. Open the **Engine** tab.
2. Pick a **Model size** — Medium English (~539 MB) is a good default.
3. Click **Download**. The model is saved to `%APPDATA%\com.typr.app\`.

If you use the Groq Cloud engine instead, no download is needed — just enter your API key.

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
- `src-tauri/target/release/bundle/nsis/Typr_0.1.0_x64-setup.exe` (~278 MB)
- `src-tauri/target/release/bundle/msi/Typr_0.1.0_x64_en-US.msi` (~436 MB)

To skip the first-run model download for your users, drop a `ggml-small.bin` into `src-tauri/binaries/` before building.

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
It is off by default. When off, the settings beneath it are hidden and a note says so. If it is on and output still looks unstyled, the guards may be rejecting the result and falling back to deterministic cleanup — the log records which.

**Auto mode picks the wrong style.**
Add an App context rule under **AI → Advanced**. Leave the app blank and set only a title filter to match a website in any browser.

**A name is always transcribed wrong.**
Try a Dictionary hint first, since it biases recognition without touching anything else. If it is still wrong, a Snippet will replace it outright.

---

## License

[MIT](LICENSE) © 2026 Sanirudh (sanirudh17)

Built on [Tauri](https://tauri.app), [whisper.cpp](https://github.com/ggml-org/whisper.cpp), and [OpenAI Whisper](https://github.com/openai/whisper). Cloud transcription and AI cleanup are served by [Groq](https://groq.com).
