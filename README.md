# BridgeScope

BridgeScope is an independently implemented, pure-Rust desktop toolkit for inspecting and managing Android devices through ADB.

> Status: **0.4 early development.** The current milestone provides ADB discovery, explicit device selection, a device overview, an interactive Android shell, binary-safe screenshots, a provider-neutral AI assistant surface, and a remote file manager with explicit transfer confirmation and cancellation. File deletion is restricted to regular files; directory deletion is intentionally unavailable. Other panels remain roadmap placeholders.

## Goals

- A cross-platform `egui` desktop application written in Rust.
- Safe, explicit Android device targeting; BridgeScope never silently selects the first device.
- Device overview, files, applications, processes, performance, shell, layout, screenshots, Logcat, WebView inspection, screencasting, and AVD management delivered incrementally.
- Deterministic cancellation and cleanup of ADB subprocesses and streams.
- Independent clean-room implementation based on public protocols and observable behavior.

## Prerequisites

- Rust 1.90
- Android SDK Platform Tools (`adb`) available through `PATH`, `ANDROID_SDK_ROOT`, or `ANDROID_HOME`
- Windows, macOS, or Linux desktop build prerequisites for `eframe`

## Run

```bash
cargo run -p bridgescope-desktop
```

Run without a device using the fake backend:

```bash
BRIDGESCOPE_FAKE=1 cargo run -p bridgescope-desktop
```

On Windows Command Prompt:

```bat
set BRIDGESCOPE_FAKE=1
cargo run -p bridgescope-desktop
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
- **Network devices:** the Device Manager accepts an Android device host/IP and port directly through adb connect, keeps up to eight successful endpoints as local history, and allows reconnecting or forgetting saved endpoints. Device discovery runs once at startup and again only after an explicit refresh or network connection.

## Safety

BridgeScope binds every structured operation to an explicit device serial and connection generation. Destructive capabilities will require backend-enforced confirmation. The interactive shell is an unrestricted expert feature; arbitrary Android shell commands cannot be made safe.

## Independence

BridgeScope is not affiliated with AYA, Android, Google, or scrcpy. It does not copy AYA source code, branding, icons, translations, screenshots, or visual assets. See [`docs/clean-room.md`](docs/clean-room.md).

## License

BridgeScope source is available under either the MIT License or Apache License 2.0, at your option. Third-party artifacts retain their own licenses and notices.
