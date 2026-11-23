# Kokoro TTS - Lightweight Windows System Tray Text-to-Speech

A high-performance, low-latency Windows 10/11 system tray application for local text-to-speech, built in pure Rust. Optimized specifically for CPU-only execution (e.g. Intel Core i5 10th gen, 16 GB RAM), it provides instant text-to-speech for any selected text across Windows via global hotkeys, paired with an embedded real-time Web UI.

---

## Key Features

### 1. System Tray & Background Operation
- **Single Instance Guarantee**: Protected by a Windows Named Mutex (`Global\KokoroTTS_SingleInstance_Mutex`).
- **No Main Window / Pure Background**: Resides cleanly in the notification area with minimal resource consumption (~0% CPU when idle).
- **Interactive Tray Menu**: Quick access to *Open Web UI*, *Pause / Resume*, *Stop Playback*, and *Quit*.
- **Native Windows Toast Notifications**: Informs you of playback events, hotkey conflicts, and model loading/download progress.

### 2. Global Hotkey & Smart Clipboard Capture
- **Global Shortcut (Default `Ctrl+Alt+R`)**: Fully configurable and rebindable via an interactive key recorder in the Web UI.
- **Strict Activation**: Reacts *only* to the configured hotkey; normal `Ctrl+C` or clipboard changes never trigger audio.
- **Transparent Selection Capture**:
  - Automatically simulates `Ctrl+C` via `SendInput`.
  - Polls clipboard briefly (~300 ms).
  - Restores your previous clipboard text/history seamlessly (optional toggle to disable restore).
- **Direct Clipboard Mode**: Option to read existing clipboard text without simulating keyboard copy.
- **Toggle-to-Stop**: Pressing the hotkey while audio is playing immediately halts playback.

### 3. Kokoro-82M INT8 ONNX Speech Engine
- **Local Inference via ONNX Runtime (`ort`)**: No PyTorch or libtorch runtime overhead; executes on CPU with INT8 quantization (~92 MB model).
- **Smooth System Performance**: Inference is strictly limited to 4 intra-threads, and worker threads run at `THREAD_PRIORITY_BELOW_NORMAL` to ensure other applications and games never experience stutter.
- **Pure Rust Phonemization (`misaki-rs`)**: Converts English text directly to IPA phonemes with zero native C phonemizer dependencies.
- **Automatic Model Provisioning**: On first launch, required models and voice embeddings (`af_bella`, `af_sarah`, `am_adam`, `bf_emma`, `bm_george`) are automatically downloaded to `%APPDATA%\KokoroTTS\models` with real-time progress indicators.
- **Extensible `TtsEngine` Trait**: Clean abstraction layer (`synthesize(text, voice, speed) -> PCM samples`) allowing online TTS engines (e.g., OpenAI / ElevenLabs) to be added effortlessly.

### 4. Low-Latency Lookahead Streaming Playback
- **Sub-100ms Initial Audio Start**: Splits text into sentence chunks, keeping the first chunk under 100 characters for instantaneous speech onset.
- **Lookahead Worker**: Synthesizes 2–3 chunks ahead in the background. Finished chunks are kept in memory for gapless playback.
- **Instant Seeking & Cancellation**: Stopping or jumping to a line increments an atomic epoch, instantly cancelling pending synthesis threads.
- **Full Transport Controls**: *Play*, *Pause / Resume*, *Previous Line*, *Next Line*, *Rewind (-10s)*, *Fast-Forward (+10s)*, and *Stop*.

### 5. Embedded Modern Web UI
- **Zero-Dependency Architecture**: Bound exclusively to `127.0.0.1:8787`. All HTML, CSS, and JavaScript are bundled directly into the executable binary—zero CDN or external requests.
- **Real-Time WebSocket Sync**: Broadcasts the current active sentence index. The UI highlights the spoken line with smooth auto-scrolling.
- **Click-to-Seek**: Click on any sentence in the list to immediately jump speech playback to that position.
- **Settings Modal**:
  - Interactive Hotkey Recorder.
  - Clipboard Mode selector (`Auto-copy` vs `Direct clipboard`).
  - "Don't restore clipboard" toggle.
  - "Start with Windows" autostart toggle (writes to `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`).
  - Online TTS API key field (for future online provider integration).

---

## Project Structure

```
koko_tts/
├── Cargo.toml               # Cargo package configuration and release profile
├── src/
│   ├── main.rs              # Application entry point, single-instance mutex, tray loop
│   ├── audio.rs             # Rodio audio handle & dedicated audio thread
│   ├── autostart.rs         # Windows Registry autostart integration
│   ├── clipboard.rs         # Windows clipboard capture, Ctrl+C simulation & restore
│   ├── hotkey.rs            # Global hotkey registration & key string parser
│   ├── playback.rs          # Playback coordinator, lookahead synthesis & chunking
│   ├── settings.rs          # Settings management (%APPDATA%\KokoroTTS\settings.json)
│   ├── tray.rs              # Windows Shell_NotifyIconW native system tray implementation
│   ├── tts/
│   │   ├── mod.rs           # TtsEngine trait definition
│   │   └── kokoro.rs        # Kokoro-82M INT8 ONNX inference & HuggingFace downloader
│   └── web/
│       ├── mod.rs           # Axum HTTP server & WebSocket event broadcaster
│       └── index.html       # Single-file glassmorphic dark UI
└── target/
    └── release/
        └── koko-tts.exe     # Optimized single-file binary (~6-10 MB)
```

---

## Build Instructions (Windows 10/11)

### Prerequisites
1. **Rust Toolchain**: Install Rust via [rustup.rs](https://rustup.rs) (`stable-x86_64-pc-windows-msvc`).
2. **Linker**: Rust's built-in `rust-lld` is configured in `~/.cargo/config.toml`:
   ```toml
   [target.x86_64-pc-windows-msvc]
   linker = "rust-lld"
   ```
3. **Windows SDK & CRT**: Set `INCLUDE` and `LIB` environment variables pointing to the MSVC and Windows SDK header/library paths (or standard Visual Studio C++ Build Tools).

### Compiling Release Binary
In PowerShell or Command Prompt, run:
```powershell
cargo build --release
```

The resulting binary will be at:
```
target\release\koko-tts.exe
```

---

## Configuration & Data Paths

- **Configuration File**: `%APPDATA%\KokoroTTS\settings.json`
- **Models & Voices**: `%APPDATA%\KokoroTTS\models/`
  - `model_quantized.onnx` (~92 MB)
  - `tokenizer.json` (~17 KB)
  - `voices/` (`af_bella.bin`, `af_sarah.bin`, `am_adam.bin`, etc.)

---

## Usage Guide

1. **Launch**: Run `koko-tts.exe`. On first startup, it verifies models and loads the ONNX runtime into memory.
2. **Read Selected Text**: In any application (browser, PDF, code editor), highlight text and press **Ctrl+Alt+R**.
3. **Stop Speech**: Press **Ctrl+Alt+R** again while speaking, or use the tray icon menu.
4. **Web UI**: Double-click the tray icon or right-click $\rightarrow$ *Open Web UI* to navigate to `http://127.0.0.1:8787`.
5. **Paste & Listen**: Paste long text into the web interface and click **Play** to follow along with live sentence highlighting.
