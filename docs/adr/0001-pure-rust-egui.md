# ADR 0001: Use a pure Rust egui desktop stack

- Status: Accepted
- Date: 2026-07-25

## Decision

Use `eframe`/`egui` for the desktop interface and Rust for all host-side components. Do not use Electron, React, Tauri, or a Kotlin Android helper.

## Consequences

The project has one primary language and direct control over lifecycle and memory. In exchange, terminal emulation, rich data grids, video decoding, accessibility, and Android Framework integration require more original engineering and broader platform testing. Unsupported device-side capabilities must be reported honestly rather than approximated with fabricated data.
