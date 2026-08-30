*[中文](README.md) | English*

<p align="center"><img src="apps/fadb-desktop/assets/icon-256.png" width="128" alt="fadb icon"></p>

# fadb

> a featherweight ADB toolbox, in Rust

fadb is an independently implemented, pure-Rust desktop toolkit for inspecting and managing Android devices through ADB.

> Status: **0.8.1.** Panels ship: ADB discovery and diagnostics, explicit device selection (USB and network), device overview, interactive shell (with an editable quick-command bar), binary-safe screenshots, a provider-neutral AI assistant, remote file management, application management with APK install, process and performance monitors, live Logcat, a layout inspector, WebView inspection, wireless debugging (pairing, TCP mode, mDNS discovery) in the device manager, and scrcpy mirroring (adjustable max size/bitrate, key remote control, one-tap MP4 recording). The remaining roadmap (AI streaming responses, touch/text mirror input and audio) is tracked in [`docs/feature-matrix.md`](docs/en/feature-matrix.md). File deletion is restricted to regular files; directory deletion is intentionally unavailable.

## Goals

- A cross-platform `egui` desktop application written in Rust.
- Safe, explicit Android device targeting; Fadb never silently selects the first device.
- Device overview, files, applications, processes, performance, shell, layout, screenshots, Logcat, WebView inspection, screencasting, and wireless debugging delivered incrementally.
- Deterministic cancellation and cleanup of ADB subprocesses and streams.
- Independent clean-room implementation based on public protocols and observable behavior.

## Prerequisites

- Rust 1.90
- Android SDK Platform Tools (`adb`) available through `PATH`, `ANDROID_SDK_ROOT`, or `ANDROID_HOME`
- Windows, macOS, or Linux desktop build prerequisites for `eframe`

## Run

```bash
cargo run -p fadb-desktop
```

Run without a device using the fake backend:

```bash
FADB_FAKE=1 cargo run -p fadb-desktop
```

On Windows Command Prompt:

```bat
set FADB_FAKE=1
cargo run -p fadb-desktop
```

## Quality checks

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build --workspace --release
```

## Implemented workflows

- **Interactive Shell:** starts `adb -s SERIAL shell -tt`, streams keyboard input and ANSI output, and supports explicit close/reconnect. This initial adapter uses a fixed 80×24 remote PTY; remote stderr is usually merged and true remote resize awaits native ADB shell-v2.
- **Screenshot:** captures with binary-safe `adb -s SERIAL exec-out screencap -p`, validates/decodes PNG off the UI thread, displays Fit/100% modes, copies the decoded image, and saves the original PNG.
- **File manager:** browses remote directories, uploads and downloads files with explicit overwrite confirmation, supports cancellation, creates directories, renames entries, and deletes regular files only. Operations remain bound to the selected device generation and refresh the current listing after completion.
- **Processes:** reads an online device's process table through ADB, showing PID, process name, user, state, CPU, memory, and resident memory. The snapshot refreshes every three seconds while the panel is visible and can also be refreshed manually.
- **Performance:** samples CPU usage, load average, memory, storage, and battery metrics once per second while the panel is visible. The panel keeps the latest 60 samples and renders CPU, memory, and battery history as a lightweight chart.
- **Network devices:** the Device Manager accepts an Android device host/IP and port directly through adb connect, keeps up to eight successful endpoints as local history, and allows reconnecting or forgetting saved endpoints. Device discovery runs once at startup and again only after an explicit refresh or network connection.
- **Applications:** lists installed packages with launcher icons, shows package details (version, installer, permissions), and supports launch, force stop, clear data, freeze/unfreeze, and uninstall — every destructive action requires explicit confirmation.
- **Logcat:** streams `logcat -v threadtime` live with per-level colors, severity and text filters, pause, autoscroll, and save-to-file. The stream starts automatically when the panel is opened and survives device switches.
- **Layout inspector:** captures the foreground window hierarchy via `uiautomator dump`, renders it as a searchable view tree with per-node attributes, copyable node dumps, and XML export.
- **WebView inspection:** discovers WebView DevTools sockets on the device, forwards a local port, lists debuggable pages, and opens them in the browser or the Chrome DevTools frontend.
- **AI assistant:** a provider-neutral dock that talks to any OpenAI-compatible endpoint — base URL, API key, and model name are all configured and stored locally, and requests go only to the provider you configured. The system prompt anchors answers to Android debugging and standard `adb` commands.
- **Mirroring:** forward-tunnel mirroring with the pinned scrcpy server 3.3.4 and adjustable max size/bitrate; key rows remote-control the device via `input keyevent`, and one tap records the current stream as MP4 (writing from the first keyframe).
- **Quick commands:** custom command buttons on the shell toolbar with optional auto-Enter; the list persists locally and supports JSON import/export.
- **Wireless debugging:** pairing-code pairing, one-click `adb tcpip 5555` for the selected device, and mDNS discovery with connect, all inside the Device Manager.

## Safety

Fadb binds every structured operation to an explicit device serial and connection generation. Destructive capabilities will require backend-enforced confirmation. The interactive shell is an unrestricted expert feature; arbitrary Android shell commands cannot be made safe.

## Independence and Credits

fadb takes feature cues from [AYA](https://github.com/liriliri/aya) — thanks for proving the product path. fadb is an independent clean-room implementation: it does not copy AYA source code, branding, icons, translations, screenshots, or visual assets; see [`docs/clean-room.md`](docs/en/clean-room.md).

Android is a trademark of Google LLC; fadb is not affiliated with, or endorsed by, Google. Mirroring bundles and launches the [scrcpy](https://github.com/Genymobile/scrcpy) server (Genymobile, Apache-2.0, redistributed unmodified with credit), while the fadb client is an independent implementation; artifact versions, hashes, and licenses are recorded in [`docs/protocol-sources.md`](docs/en/protocol-sources.md).

## License

Fadb source is available under either the MIT License or Apache License 2.0, at your option. Third-party artifacts retain their own licenses and notices.
