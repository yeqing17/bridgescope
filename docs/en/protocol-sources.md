*[中文](../protocol-sources.md) | English*

# Protocol sources

Implementation must use public specifications or fixtures captured by BridgeScope.

- Android Debug Bridge overview: https://developer.android.com/tools/adb
- ADB source/protocol reference: https://android.googlesource.com/platform/packages/modules/adb/
- Android shell service protocol source: https://android.googlesource.com/platform/packages/modules/adb/+/refs/heads/main/SERVICES.TXT
- ADB CLI shell PTY and exec-out behavior: https://android.googlesource.com/platform/packages/modules/adb/+/refs/heads/main/docs/user/adb.1.md
- Android screencap source/tool behavior: https://android.googlesource.com/platform/frameworks/base/+/refs/heads/main/cmds/screencap/
- scrcpy development documentation: https://github.com/Genymobile/scrcpy/blob/master/doc/develop.md
- Chrome DevTools Protocol: https://chromedevtools.github.io/devtools-protocol/
- UI Automator: https://developer.android.com/training/testing/other-components/ui-automator

Feature-specific source versions and access dates must be recorded when implementation begins. Protocol bytes must never be inferred only from an AI response.

## scrcpy video mirroring (0.7, implemented 2026-08-29)

Normative sources, all accessed 2026-08-29:

- Protocol description for the pinned server version (`doc/develop.md` at tag `v3.3.4`, which documents "the current protocol in scrcpy v2.1" wire format): https://raw.githubusercontent.com/Genymobile/scrcpy/v3.3.4/doc/develop.md
- Pinned server artifact `scrcpy-server-v3.3.4` (90,980 bytes), SHA-256 `8588238c9a5a00aa542906b6ec7e6d5541d9ffb9b5d0f6e1bc0e365e2303079e`, downloaded from https://github.com/Genymobile/scrcpy/releases/download/v3.3.4/scrcpy-server-v3.3.4 (release list cross-checked via the GitHub API `digest` field for release 271207312). The artifact is vendored at `crates/bridgescope-scrcpy/assets/scrcpy-server-v3.3.4`; BridgeScope verifies the SHA-256 at build time via `include_bytes!` + a unit test. scrcpy is Apache-2.0 licensed; this artifact is redistributed unmodified with attribution here.
- Decoder: `openh264` Rust crate, pinned to 0.9.8 (crates.io, published 2026-08-08, `rust-version = 1.85`), https://crates.io/crates/openh264 (docs: https://docs.rs/openh264). Chosen over an ffmpeg sidecar because it compiles the bundled OpenH264 (BSD-2, Cisco) C sources through `cc` inside a plain `cargo build` with no external binary, and its throughput (≈1080p decode in single-digit milliseconds) is sufficient for mirroring. H.264 only (`video_codec=h264`); H.265/AV1 are not decoded in 0.7.

Protocol summary as documented by the pinned `develop.md` (informational restatement; the normative text is the link above):

1. Push the server jar to `/data/local/tmp/scrcpy-server.jar`.
2. Tunnel: BridgeScope uses the forward variant — `adb forward tcp:<port> localabstract:scrcpy_<scid>` before starting the server, and passes `tunnel_forward=true` plus the same `scid=<31-bit random>` to the server, which then listens on that abstract socket name. The docs render the name as `scrcpy_<SCID>` without fixing the string encoding; the encoding was verified on-device (allowed input: observable behavior, 2026-08-30, LDPlayer emulator / Android 14): the server parses `scid=` with `Integer.parseInt(value, 16)` (radix 16 — a decimal value containing `8`/`9` throws `NumberFormatException` with "under radix 16" in logcat) and names the socket `scrcpy_` + `%08x` (lowercase hex, zero-padded to 8; `scid=888` produced `@scrcpy_00000888` in `/proc/net/unix`). BridgeScope therefore formats both the option and the socket name as `{scid:08x}`.
3. Server launch: `adb shell CLASSPATH=/data/local/tmp/scrcpy-server.jar app_process / com.genymobile.scrcpy.Server 3.3.4 log_level=… scid=… tunnel_forward=true audio=false control=false max_size=… video_bit_rate=…`.
4. With `audio=false` and `control=false` the server opens exactly one socket (video), which is the "first socket" for metadata purposes.
5. On the first socket over a forward tunnel the device sends one dummy byte first (detects stale connections), followed by device metadata (device name).
6. Video stream after metadata: codec metadata (codec id `u32` BE — `h264` = `0x68323634` — then video width `u32`, height `u32`), then a sequence of 12-byte-header packets: 8-byte header where the top bit is the config-packet flag, the next bit the key-frame flag, and the low 62 bits the PTS; followed by a `u32` BE payload size and that many payload bytes.
7. BridgeScope options truth table for 0.7: `audio=false`, `control=false` (video only, no injection — both are Phase-2 scope exclusions); `max_size` and `video_bit_rate` user-adjustable — the bit-rate key is `video_bit_rate`, matching client `--video-bit-rate` per `doc/video.md` at tag `v3.3.4` (https://raw.githubusercontent.com/Genymobile/scrcpy/v3.3.4/doc/video.md, accessed 2026-08-30); remaining options keep server defaults.

On-device verification notes (allowed input: observable behavior on contributor-owned devices; LDPlayer emulator reporting as REDMI 24117RK2CC / Android 14, 1600×900 @ 240dpi, software encoder `c2.android.avc.encoder`, 2026-08-30):
- The default `cleanup=true` server removes `/data/local/tmp/scrcpy-server.jar` when the server process exits (observed: jar present at push, gone after the server was killed), so BridgeScope re-pushes the jar on every mirror start.
- An unknown option aborts the server immediately with `Aborted` (Java exception), and a missing jar aborts with `ClassNotFoundException: com.genymobile.scrcpy.Server`; both surface through the shell log tail in the mirror error detail.
- In forward-tunnel mode adb accepts the local TCP connection even before the server listens, closing it if the device-side connect fails; BridgeScope polls connect + stream-header reads until the server announces itself.
- The server emits a frame only when display content changes: static screens legitimately yield 0–1 fps (a 6 s static `screenrecord` contains exactly one keyframe). During sustained interaction the receive rate tracked both a raw-socket control client reading the same stream and the device's own `screenrecord` (≈3–7 fps at 720p on an idle-host LDPlayer whose primary display composites at 60 fps host-side) — the emulator's virtual-display composition feed is the limiter, not BridgeScope's decode/UI pipeline: a 0.5 s animation backlog arrived with PTS deltas ≈31 ms (32 fps cadence), so the pipeline delivers whatever frames exist as soon as they exist. `max_size` below the display width stalls this build's server entirely (downscale projection bug), so 1280 is the safe preset there.

BridgeScope-sourced verification (allowed input: observable behavior on contributor-owned devices): the end-to-end session (push, forward, launch, stream header, demux, decode, stop, restart after the device-side server dies) was verified manually against a live device on 2026-08-30; no automated real-device test exists and CI stays hermetic. Unit tests pin the byte layout (jar hash, argument list, socket-name derivation, header parsing) with synthetic fixtures built only from the documented formats above.
