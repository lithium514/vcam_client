# vcam_client

A Tauri 2 desktop app for Android device remote control and camera capture.

## Features

- **Camera** — Capture photos using your device camera and upload them to a server via multipart/form-data (uses the Rust backend to bypass CORS).
- **scrcpy Integration** — SSH tunnel into an Android device (via ADB over TCP), launch [scrcpy](https://github.com/Genymobile/scrcpy) for full screen mirroring, or stream live screencap frames directly inside the app.

## Architecture

```
Frontend (React + TypeScript + Vite)
    ↕  Tauri IPC (invoke / events)
Backend (Rust)
    ├── SSH tunnel management (start/stop)
    ├── ADB screencap streaming (adb exec-out screencap -p)
    ├── scrcpy launcher
    └── Image upload (bypasses CORS)
```

## Prerequisites

- [Node.js](https://nodejs.org/) 20+
- [Rust](https://rustup.rs/) toolchain
- [Tauri 2 system dependencies](https://v2.tauri.app/start/prerequisites/)
- [ADB](https://developer.android.com/tools/adb) (on `PATH`)
- [scrcpy](https://github.com/Genymobile/scrcpy) (optional, for screen mirroring)

## Development

```bash
# Install JS dependencies
npm install

# Run in dev mode (hot-reload)
npm run tauri dev
```

## Build

```bash
npm run tauri build
```

The distributable will be placed in `src-tauri/target/release/bundle/`.

## Usage

### Camera tab

1. Allow camera access when prompted.
2. Tap the shutter button to capture a photo.
3. Click **Upload** to send it to the configured server URL (configurable via the ⚙ button).

### scrcpy tab

1. Enter the SSH host, port, user, and local ADB port (default `5555`).
2. Click **Connect SSH** to establish a reverse tunnel (`-L localhost:5555:localhost:5555`).
3. Once connected:
   - **Start Preview** — streams live `screencap` frames from the device.
   - **Launch scrcpy** — opens an external scrcpy window (optional audio codec/encoder settings under "Audio options").

## Configuration

| Setting      | Description                              | Default                        |
|--------------|------------------------------------------|--------------------------------|
| Upload URL   | Server endpoint for image uploads        | `http://127.0.0.1:5000/upload` |

## Project Structure

```
src/                  React frontend
  App.tsx             Main UI (Camera + scrcpy tabs)
  App.css             Styles
  main.tsx            Entry point
src-tauri/            Rust backend
  src/lib.rs          Tauri commands (SSH, ADB, upload, scrcpy)
  src/main.rs         Binary entry point
  tauri.conf.json     Tauri configuration
```

## License

MIT
