# Architecture

BridgeScope separates UI, application orchestration, device state, and transports.

```text
egui desktop
  -> bounded typed commands
backend runtime (Tokio)
  -> device registry
  -> feature services
  -> AdbTransport
  -> adb server / Android device
  <- bounded typed events
```

## Rules

1. The egui thread never waits for ADB or filesystem I/O.
2. Every operation carries an explicit device serial and, for long-running work, a device generation.
3. Streaming sessions carry session IDs so stale events can be discarded.
4. `AdbTransport` is replaceable by a deterministic fake.
5. The backend owns validation, risk classification, timeout, cancellation, and cleanup.
6. UI state, persisted preferences, and device-session state are separate types.
7. Unsupported device capabilities are represented explicitly rather than as invented zero values.

## 0.1 runtime

The first version uses a controlled `adb` child process adapter. It polls `adb devices -l`, reconciles snapshots in `DeviceRegistry`, and retrieves overview fields through fixed read-only commands. Native ADB host, shell-v2, sync, forward, and reverse protocol implementations will replace subprocess-dependent paths incrementally.
