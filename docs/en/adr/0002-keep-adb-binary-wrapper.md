*[中文](../../adr/0002-keep-adb-binary-wrapper.md) | English*

# ADR 0002: Keep the adb binary wrapper instead of a native Rust ADB client

- Status: Accepted
- Date: 2026-08-30

## Decision

Keep `bridgescope-adb` as a wrapper that spawns the platform-tools `adb` binary. Do not adopt a native Rust ADB-protocol client library.

Candidates evaluated (2026-08):

- [radb](https://crates.io/crates/radb) ([oslo254804746/radb](https://github.com/oslo254804746/radb)) — a Rust port of Python's openatx/adbutils; speaks the ADB protocol directly, no adb binary needed. Rejected: sync-only API (our stack is tokio), no pairing or mDNS support in its feature list, blocking iterators for streams like logcat, and early maturity (v0.1.8, ~8k total / ~60 recent downloads, single author).
- [adb_client](https://github.com/cocool97/adb_client) — async (tokio), actively maintained, adb-server proxy plus direct USB/TCP transports, mDNS support. The stronger candidate if this decision is ever revisited.

## Context

The only real gain from a protocol-native client is dropping the `adb` binary dependency, and BridgeScope cannot drop external tooling anyway: screen mirroring shells out to scrcpy, screen recording runs `screenrecord` on the device through a shell, and AVD management drives the emulator CLI. A migration would also have to relearn the real-device ROM quirks our parsers already encode (doubled-quoted Wi-Fi SSIDs, `null` font_scale, the `ip` command prefix), and would have to rebuild the streaming surfaces — the interactive terminal and live logcat — on top of a sync API.

## Consequences

The app keeps requiring Android platform-tools, either on PATH or at a located SDK path. Frequent polling such as performance metrics pays a process-spawn cost per sample. If a portable build that ships without the SDK ever becomes a requirement, revisit this decision starting from `adb_client`.
