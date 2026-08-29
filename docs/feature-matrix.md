# Feature matrix & roadmap

Status legend: ✅ shipped, 🚧 current milestone, ⏳ planned, 🧪 exploratory, ❌ descoped.

The status table tracks what ships today. The roadmap sequences what is next; it
was re-baselined after 0.5.0 against the shipping architecture (the `adb.exe`
subprocess transport) instead of the original protocol-level ambitions.

A visible panel is not considered implemented until its backend, UI states,
tests, and real-device acceptance flow pass.

## Status (0.6.0)

| Area | Status | Notes |
|---|---|---|
| ADB discovery and diagnostics | ✅ | locator (explicit/SDK/PATH), version, device list |
| Device tracking and explicit selection | ✅ | generation-bound targeting, explicit select |
| Network devices | ✅ | `adb connect` + up to 8 remembered endpoints |
| Fake device backend | ✅ | `BRIDGESCOPE_FAKE=1` deterministic data |
| Device overview | ✅ | model, battery, memory, storage |
| Interactive Shell | ✅ | subprocess PTY (`adb shell -tt`), ANSI, resize |
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
| AVD manager | ✅ | list via SDK emulator, launch (± wipe-data), running state, `emu kill` stop with confirm |
| Wireless debugging | ✅ | `adb pair` with code, `tcpip 5555` for the selected device, mDNS discovery with connect; note: running-AVD detection needs a genuine SDK emulator console |
| scrcpy screen mirror | ⏳ | phase 2 of the roadmap below |
| AI streaming responses | ⏳ | SSE on the reserved provider surface |
| Screen recording | 🧪 | evaluate after mirroring ships |
| Pure Rust Android helper | ❌ | descoped, see roadmap |
| Native ADB shell-v2 | ❌ | descoped, see roadmap |

## Roadmap

### Phase 1 — 0.6: device workflow completion (adb.exe transport) — DONE

All three items shipped in 0.6.0 on the existing subprocess transport.

1. **APK install** — `adb install -r` behind a file picker in the applications
   panel; explicit success/failure surfacing with adb's own failure text.
2. **AVD manager** — list AVDs through the SDK emulator binary, launch (with
   optional wipe-data), show running state, stop via `adb emu kill`. The AVD
   panel is host-scoped (no device selection) and polls the device list after
   a launch. An env-gated roundtrip test
   (`launch_and_kill_roundtrip_boots_a_real_avd`) boots and stops a real AVD.
3. **Wireless debugging** — pairing-code flow (`adb pair host:port code`), a
   one-click `adb tcpip 5555` action for the selected device, and mDNS
   discovery (`adb mdns services`) with connect buttons, all inside the
   device-manager window.

### Phase 2 — 0.7: scrcpy phase 1 (video mirror)

- Spike first: H.264 decoder choice (static openh264 build vs. ffmpeg sidecar),
  plus the scrcpy protocol version and doc links, recorded in
  `docs/protocol-sources.md` per the clean-room policy before any protocol byte
  is implemented.
- Ship: device screen mirroring with adjustable max size / bitrate rendered to
  an egui texture, one mirror at a time per device.
- Explicitly out of phase 1: control injection and audio.

### Phase 3 — later

- scrcpy phase 2: touch/key/text injection (control protocol).
- scrcpy phase 3: audio capture (Android 11+).
- AI streaming responses (SSE) on the existing provider surface.
- Screen recording (`screenrecord`) if mirroring leaves a gap.

### Descoped (decisions recorded 2026-08)

- **Pure Rust Android helper** (in-process ADB server/protocol in Rust): the
  `adb.exe` transport shipped every panel below the 0.6 milestone; reimplementing
  ADB auth, server protocol, and mDNS is months of work with no user-visible
  gain. Revisit only if bundling/licensing constraints change.
- **Native shell-v2**: demoted. The subprocess PTY shell works on all three
  platforms; shell-v2 would only add remote exit codes and split stderr.
  Revisit alongside any future protocol-level work.
