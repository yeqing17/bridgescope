*[中文](README.md) | English*

<p align="center"><img src="apps/fadb-desktop/assets/icon-256.png" width="128" alt="fadb icon"></p>

# fadb

> a featherweight ADB toolbox, in Rust

fadb is an independently implemented, pure-Rust desktop toolkit for inspecting and managing Android devices through ADB. Current release: **0.8.7**; see [`CHANGELOG.md`](CHANGELOG.md) for the full history and [`docs/feature-matrix.md`](docs/en/feature-matrix.md) for scope and roadmap.

## Features

- **Devices:** automatic discovery of USB and network devices with explicit selection before any action. The Device Manager covers wireless pairing, `adb tcpip 5555`, mDNS discovery, direct network connects with one-click disconnect, and remembers up to eight endpoints.
- **Overview:** model, serial, Android/kernel version, CPU, memory, storage, battery, and resolution at a glance.
- **Interactive shell:** binary-safe two-way streaming with drag-to-select text (release to copy), a right-click menu for copy/paste, bracketed paste, and a customizable quick-command bar (persisted locally, JSON import/export).
- **Files:** browse remote directories; upload/download with explicit overwrite confirmation and cancellation; create folders, rename, delete files and directories (recursive deletion is confirmed). The listing shows modification times and sorts by name/size/time.
- **Applications:** installed packages with launcher icons, version/installer/permission details, launch, force stop, clear data, freeze/unfreeze, and APK install — destructive actions always require confirmation.
- **Processes & performance:** a process table (PID/user/CPU/memory) refreshed every three seconds; CPU, memory, storage, and battery sampled every second with the last 60 samples charted.
- **Logcat:** live `logcat -v threadtime` streaming with level colors, filters, pause, autoscroll, and save-to-file.
- **Layout inspector:** foreground view hierarchy via `uiautomator dump`, rendered as a searchable tree with per-node attributes, copyable dumps, and XML export.
- **WebView inspection:** discovers WebView debug sockets on the device, forwards a port, lists debuggable pages, and opens them in Chrome DevTools.
- **Screenshots:** binary-safe `screencap` with off-thread PNG decoding, fit/100% modes, image copy, and PNG export.
- **Mirroring:** forward-tunnel mirroring with the pinned scrcpy server 3.3.4, adjustable max size/bitrate, key-based remote control, and one-tap MP4 recording.
- **AI assistant:** a docked panel for any OpenAI-compatible endpoint; base URL, API key, and model stay on your machine, and the system prompt anchors answers to Android debugging and standard `adb` commands.

## Desktop UI

- Simplified Chinese / English switch and light / dark themes.
- The left navigation collapses to an icon rail; the choice is remembered across launches.
- The settings window (gear in the top bar) gathers theme, language, ADB info (executable path, version, device count), and about information.
- Frameless custom window: drag the title bar to move, drag the edges to resize, double-click to maximize.

## Getting started

- Rust 1.90
- Android SDK Platform Tools (`adb`) found through `PATH`, `ANDROID_SDK_ROOT`, or `ANDROID_HOME`. Alternatively set `FADB_ADB` to the direct path of the `adb` executable (highest priority; a missing path is reported prominently). If you do not have adb yet, the ADB section of the settings window links the official download page.
- Windows, macOS, or Linux desktop build prerequisites for `eframe`.

```bash
cargo run -p fadb-desktop
```

Try the UI without a device using the fake backend:

```bash
FADB_FAKE=1 cargo run -p fadb-desktop
```

## Quality checks

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build --workspace --release
```

## Safety

Fadb binds every structured operation to an explicit device serial and connection generation. Destructive capabilities require backend-enforced confirmation. The interactive shell is an unrestricted expert feature; arbitrary Android shell commands cannot be made safe.

## Independence and Credits

fadb takes feature cues from [AYA](https://github.com/liriliri/aya) — thanks for proving the product path. fadb is an independent clean-room implementation: it does not copy AYA source code, branding, icons, translations, screenshots, or visual assets; see [`docs/clean-room.md`](docs/en/clean-room.md).

Android is a trademark of Google LLC; fadb is not affiliated with, or endorsed by, Google. Mirroring bundles and launches the [scrcpy](https://github.com/Genymobile/scrcpy) server (Genymobile, Apache-2.0, redistributed unmodified with credit), while the fadb client is an independent implementation; artifact versions, hashes, and licenses are recorded in [`docs/protocol-sources.md`](docs/en/protocol-sources.md).

## License

Fadb source is available under either the MIT License or Apache License 2.0, at your option. Third-party artifacts retain their own licenses and notices.
