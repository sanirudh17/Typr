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

### First-Time Setup (Installed App)

When launching Typr for the first time after installing a release build:

1. Open the app and navigate to the **Engine** tab via the sidebar.
2. Select your preferred **Model Size** (`small` ~466 MB or `medium` ~1.5 GB).
3. Click the **Download** button to fetch the Whisper model file from HuggingFace.
   *(The model is saved to `C:\Users\<You>\AppData\Roaming\com.typr.app\ggml-<size>.bin`)*
4. Once the download completes, the engine is ready to transcribe.

> **Note:** A Whisper model must be downloaded before any local transcription can occur. If you switch to the Cloud (Groq) engine, no model download is required — just enter your API key.

### Setup & Local Development (From Source)

The app requires three **whisper.cpp sidecar binaries** in `src-tauri/binaries/` to function:
- `whisper-cpp` (CPU fallback)
- `whisper-cpp-cuda` (GPU direct)
- `whisper-server-cuda` (persistent HTTP server)

These are **not** the model files (which are downloaded later via the app UI) — they are compiled executables from the [whisper.cpp](https://github.com/ggml-org/whisper.cpp) project. The `src-tauri/binaries/` directory is gitignored, so you must build or obtain them yourself.

#### Option A: Build from whisper.cpp source

```bash
# Clone whisper.cpp alongside the Typr repo
git clone --depth 1 https://github.com/ggml-org/whisper.cpp.git
cd whisper.cpp

# Build with CUDA support (requires CUDA toolkit)
cmake -B build -DGGML_CUDA=ON
cmake --build build --config Release

# Copy the sidecar binaries to the Typr binaries directory
# (adjust extensions/paths for your platform)
cp build/bin/whisper-cli.exe ../Typr/src-tauri/binaries/whisper-cpp.exe
cp build/bin/whisper-cli.exe ../Typr/src-tauri/binaries/whisper-cpp-cuda.exe
cp build/bin/whisper-server.exe ../Typr/src-tauri/binaries/whisper-server-cuda.exe
```

> **Note:** All three sidecar entries (`whisper-cpp`, `whisper-cpp-cuda`, `whisper-server-cuda`) in `tauri.conf.json` use the same compiled whisper.cpp binary — the app selects the appropriate execution mode at runtime via command-line flags. You may use the same binary for all three.

#### Option B: Download pre-built releases

Download pre-built whisper.cpp executables for your platform from the [whisper.cpp releases page](https://github.com/ggml-org/whisper.cpp/releases) and place them in `src-tauri/binaries/` with the correct names above.

#### Running the app

1. **Clone the Repository**:
   ```bash
   git clone https://github.com/sanirudh17/Typr.git
   cd Typr
   ```

2. **Ensure sidecar binaries are in place** (see above).

3. **Install Frontend Dependencies**:
   ```bash
   npm install
   ```

4. **Install CUDA DLLs** (optional, for GPU acceleration):
   Place `cublas64_12.dll`, `cublasLt64_12.dll`, and `ggml-cuda.dll` into `src-tauri/binaries/`.

5. **Start the App in Dev Mode**:
   ```bash
   npm run tauri dev
   ```
   *This starts the frontend Vite server and compiles the Rust backend, launching the local app window.*

6. **Download a Whisper model** via the Engine settings tab in the app (see "First-Time Setup" above).

---

## 📦 Packaging & Compiling for Production

To compile Typr into a single fully-packaged production installer:

1. **Place the whisper.cpp sidecar binaries** (`whisper-cpp.exe`, `whisper-cpp-cuda.exe`, `whisper-server-cuda.exe`) into `src-tauri/binaries/` (see "Setup & Local Development" above for how to obtain these).
2. **Place CUDA DLLs** (optional) — `cublas64_12.dll`, `cublasLt64_12.dll`, `ggml-cuda.dll` — into `src-tauri/binaries/` for GPU acceleration.
3. **Bundle a Whisper model** (optional) — to skip the first-run download, place a `ggml-small.bin` into `src-tauri/binaries/`. If omitted, users download a model from within the app's Engine settings on first launch.
4. Run the production build command:
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
