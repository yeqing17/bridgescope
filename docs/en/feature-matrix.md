*[中文](../feature-matrix.md) | English*

# Feature matrix & roadmap

Status legend: ✅ shipped, 🚧 current milestone, ⏳ planned, 🧪 exploratory, ❌ descoped.

The status table tracks what ships today. The roadmap sequences what is next; it
was re-baselined after 0.5.0 against the shipping architecture (the `adb.exe`
subprocess transport) instead of the original protocol-level ambitions.

A visible panel is not considered implemented until its backend, UI states,
tests, and real-device acceptance flow pass.

## Status (0.7.1)

| Area | Status | Notes |
|---|---|---|
| ADB discovery and diagnostics | ✅ | locator (explicit/SDK/PATH), version, device list |
| Device tracking and explicit selection | ✅ | generation-bound targeting, explicit select |
| Network devices | ✅ | `adb connect` + up to 8 remembered endpoints |
| Fake device backend | ✅ | `FADB_FAKE=1` deterministic data |
| Device overview | ✅ | model, battery, memory, storage |
| Interactive Shell | ✅ | subprocess PTY (`adb shell -tt`), ANSI, resize, editable quick-command bar (JSON import/export) |
| Screenshot | ✅ | binary-safe `exec-out screencap`, Fit/100%, save/copy |
| AI assistant | ✅ | provider-neutral; OpenAI-compatible single-shot chat |
| Logcat | ✅ | live stream, level colors/filter, search, save |
| File management | ✅ | upload/download, mkdir/rename/delete, context menu |
| Application management | ✅ | list, icons, launch/stop/clear/freeze/uninstall |
| APK install | ✅ | file picker + `adb install -r`, adb's own failure text surfaced |
| Process monitor | ✅ | 3s auto-refresh table |
| Performance metrics | ✅ | 1s sampling, CPU/memory/battery history charts |
| Layout inspector | ✅ | uiautomator dump, searchable tree, attributes, XML export |
| WebView inspection | ✅ | DevTools sockets, page list, inspector URL |
| AVD manager | ❌ | removed in 0.7.1: only managed SDK-emulator AVDs, which does not help users on third-party emulators (LDPlayer, MuMu, ...); launch/stop those from their own consoles |
| Wireless debugging | ✅ | `adb pair` with code, `tcpip 5555` for the selected device, mDNS discovery with connect |
| scrcpy screen mirror | ✅ | forward-tunnel mirror of the selected device: adjustable max size / bitrate, pinned scrcpy server 3.3.4, openh264 decode to an egui texture, one mirror at a time, stop on device-side exit; key rows remote-control via `input keyevent`; one-tap MP4 recording writes from the first keyframe |
| AI streaming responses | ⏳ | SSE on the reserved provider surface |
| Screen recording | ✅ | delivered as one-tap MP4 recording inside the mirror panel (SPS/PPS passed through, microsecond timestamps), no `screenrecord` dependency; on a static screen the start waits for the next keyframe |
| Pure Rust Android helper | ❌ | descoped, see roadmap |
| Native ADB shell-v2 | ❌ | descoped, see roadmap |

## Roadmap

### Phase 1 — 0.6: device workflow completion (adb.exe transport) — DONE

Shipped in 0.6.0 on the existing subprocess transport.

1. **APK install** — `adb install -r` behind a file picker in the applications
   panel; explicit success/failure surfacing with adb's own failure text.
2. ~~**AVD manager**~~ — shipped in 0.6.0, **removed in 0.7.1**: it only managed
   AVDs of the official SDK emulator, which is irrelevant to users of
   third-party emulators; those emulators are launched and stopped from their
   own consoles.
3. **Wireless debugging** — pairing-code flow (`adb pair host:port code`), a
   one-click `adb tcpip 5555` action for the selected device, and mDNS
   discovery (`adb mdns services`) with connect buttons, all inside the
   device-manager window.

### Phase 2 — 0.7: scrcpy phase 1 (video mirror) — DONE

Shipped in 0.7.0:

- Decoder spike resolved to the `openh264` crate (bundled OpenH264 C sources, no
  external binary); protocol version, artifact hash, and doc links recorded in
  `docs/protocol-sources.md` per the clean-room policy before any protocol byte
  was implemented.
- Device screen mirroring with adjustable max size / bitrate rendered to an
  egui texture, one mirror at a time per device; the stop path covers both the
  button and the device-side server dying (forward removed, UI reset).
- Explicitly out of phase 1: control injection and audio.

### Phase 3 — later

- scrcpy phase 2: touch/text injection (control protocol); key events already
  ship via the `input keyevent` remote rows.
- scrcpy phase 3: audio capture (Android 11+).
- AI streaming responses (SSE) on the existing provider surface.

### Descoped (decisions recorded 2026-08)

- **Pure Rust Android helper** (in-process ADB server/protocol in Rust): the
  `adb.exe` transport shipped every panel below the 0.6 milestone; reimplementing
  ADB auth, server protocol, and mDNS is months of work with no user-visible
  gain. Revisit only if bundling/licensing constraints change.
- **Native shell-v2**: demoted. The subprocess PTY shell works on all three
  platforms; shell-v2 would only add remote exit codes and split stderr.
  Revisit alongside any future protocol-level work.
