# Typr ✨

**Typr** is a lightning-fast, premium desktop voice-to-text dictation application built for Windows. Siting quietly in your system tray or background, Typr allows you to transcribe your speech into text using either an ultra-fast, local, privacy-first **Whisper engine with GPU acceleration**, or the **Groq Cloud API**. Once transcription completes, Typr instantly and automatically types your speech directly into whichever text input window you currently have open.

---

## ✨ Features

- **Push-to-Talk & Toggle Modes**: Start recording instantly using a customizable global hotkey (default `Cmd/Ctrl+Shift+D`), or hold it down for Push-to-Talk precision.
- **Auto-Typing / Pasting**: Replaces the slow clipboard loop by automatically writing your transcribed voice directly into your active window (Notepad, browser, IDE, Slack, etc.) using native keyboard event injection.
- **Hardware-Accelerated Local Whisper Engine**:
  - **100% Offline & Private**: Transcribe sensitive notes locally without sending any data over the internet.
  - **Nvidia GPU/CUDA Support**: Native hardware acceleration using bundled `cublas64_12.dll`, `cublasLt64_12.dll`, and `ggml-cuda.dll` dynamic libraries, delivering lightning-fast transcrips in seconds.
  - **Dynamic DLL Resolution**: No system-wide CUDA SDK installs required—everything runs out-of-the-box from the local installer package.
- **Groq Cloud API Support**: Optionally switch to the ultra-fast Groq API (`whisper-large-v3-turbo`) for near-instant transcription on lower-end devices or laptops without dedicated GPUs.
- **Sleek, Premium Dark Theme UI**:
  - **Elegant Glassmorphic Sidebar**: Modern sidebar navigation with clean iconography and smooth animations.
  - **Live Glowing Status Indicator**: A beautiful breathing status pill showing at a glance whether the app is *Ready*, *Listening*, or *Transcribing*.
- **Custom Dictionary & Vocabulary Memory**: Feed personal names, specialized abbreviations, acronyms, or development terms (e.g., specific framework names) into Typr's context so it spells them correctly every single time.
- **Categorized History Log**: Persistent, private history database categorized by date. Click any previous card to instantly copy it back to your clipboard.
- **Self-Cleaning Core**: Safe process hook automatically shuts down background Whisper threads upon app close to prevent memory leaks or zombie processes.

---

## 🛠️ Tech Stack & Architecture

Typr is architected for maximum speed and minimal background memory overhead:

* **Frontend**: HTML5, Vanilla CSS3 (custom dark/glassmorphic design system), and TypeScript powered by **Vite** and **TypeScript Compiler**.
* **Backend**: **Rust** via the **Tauri v2** framework. Handles OS window hooks, system audio capture (`cpal`), clipboard manipulation (`arboard`), and global shortcut listeners.
* **Local Inference**: Powered by custom `whisper-server-cuda` running as a backend sidecar, communicating locally over HTTP sockets for high throughput and zero-lag audio submission.

---

## 🚀 Getting Started

### Prerequisites

To compile or build Typr from source, your system needs:
- **Node.js** (v18 or newer)
- **Rust** & Cargo (Stable channel, version 1.75+)
- **Nvidia GPU** (Recommended for GPU-accelerated local transcription) along with up-to-date graphics drivers.

### Setup & Local Development

1. **Clone the Repository**:
   ```bash
   git clone https://github.com/sanirudh17/Typr.git
   cd Typr
   ```

2. **Install Frontend Dependencies**:
   ```bash
   npm install
   ```

3. **Start the App in Dev Mode**:
   ```bash
   npm run tauri dev
   ```
   *This starts the frontend Vite server and compiles the Rust backend, launching the local app window.*

---

## 📦 Packaging & Compiling for Production

To compile Typr into a single fully-packaged production installer containing the CUDA libraries:

1. Place your target Whisper model files and CUDA binaries into `src-tauri/binaries/`.
2. Run the production build command:
   ```bash
   npm run tauri build
   ```

Upon completion, Tauri will package two high-quality installer formats:
* **EXE Installer (NSIS Setup)**:
  `src-tauri/target/release/bundle/nsis/Typr_0.1.0_x64-setup.exe` *(Highly compressed, ~278 MB)*
* **MSI Installer (Windows Installer)**:
  `src-tauri/target/release/bundle/msi/Typr_0.1.0_x64_en-US.msi` *(Cabinet bundle, ~436 MB)*

---

## ⚙️ Configuration & Data Storage

All persistent settings, history data, and custom vocabulary dictionaries are securely stored locally inside your OS's user-specific configuration directory:
* **Windows**: `C:\Users\<Your-Username>\AppData\Roaming\com.typr.app`
* **macOS**: `~/Library/Application Support/com.typr.app`

Data is stored in plaintext JSON and local SQLite databases, guaranteeing absolute privacy.