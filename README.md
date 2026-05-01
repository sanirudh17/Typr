# Typr

A lightning-fast, premium desktop application for voice-to-text dictation. Typr sits quietly in the background and allows you to transcribe your speech into text using either a local, privacy-first Whisper model, or the ultra-fast Groq Cloud API. It automatically types out your transcribed text into whatever application you currently have open.

## ✨ Features

- **Push-to-Talk & Toggle Modes:** Trigger recording via a global hotkey (default `Cmd/Ctrl+Shift+D`), or set it to Push-to-Talk for precise dictation.
- **Auto-Pasting:** Once you finish speaking, Typr instantly processes the audio and automatically pastes the text into your active window.
- **Dual Transcription Engines:**
  - **Local Whisper (`whisper.cpp`):** 100% private, offline transcription. Heavily optimized for CPU inference with custom thread allocation and greedy decoding.
  - **Cloud (Groq API):** Ultra-fast transcription using `whisper-large-v3-turbo` for near-instantaneous results on low-end hardware.
- **Custom Dictionary / Vocabulary:** Feed specific names, acronyms, and jargon into the AI's context memory so it spells them right the first time.
- **Premium Audio Visualizer:** A stunning, desktop-overlay audio pill that uses unison rippling and dynamic high-frequency flaring to provide real-time visual feedback of your speech.
- **Categorized History:** A persistent, locally stored history of all your dictations, neatly grouped by "Today," "Yesterday," and specific dates. Click any previous dictation to instantly copy it back to your clipboard.
- **Smart Text Cleanup:** Automatically handles capitalization and punctuation rules to ensure your text looks professional.

## 🛠️ Architecture

Typr is built for speed and minimal resource usage:
- **Frontend:** HTML, Vanilla CSS, and TypeScript powered by Vite. The UI is designed to be sleek, dark-themed, and incredibly responsive.
- **Backend:** Rust, using the **Tauri** framework. Handles global hotkeys, audio recording (`cpal`), filesystem operations, and clipboard management (`arboard`).
- **Inference:** Uses `whisper.cpp` as a sidecar binary for high-performance, low-level CPU audio processing.

## 🚀 Getting Started

### Prerequisites
- Node.js (v18+)
- Rust (latest stable)
- Build tools (C++ compiler, Windows SDK / Xcode Command Line Tools)

### Installation
1. Clone the repository:
   ```bash
   git clone https://github.com/sanirudh17/Typr.git
   cd Typr
   ```
2. Install dependencies:
   ```bash
   npm install
   ```
3. Run in development mode:
   ```bash
   npm run tauri dev
   ```

### Building for Production
To create a standalone `.exe`, `.msi`, or `.app` installer:
```bash
npm run tauri build
```
The resulting installers will be located in `src-tauri/target/release/bundle/`.

## ⚙️ Configuration

All user settings, dictionary vocabularies, and transcription histories are stored locally and privately in your system's `AppData` or `Application Support` directory, ensuring your data never leaves your computer unless you explicitly opt into the Groq API.