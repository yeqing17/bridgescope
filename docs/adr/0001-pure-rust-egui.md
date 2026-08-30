中文 | [English](../en/adr/0001-pure-rust-egui.md)

# ADR 0001:采用纯 Rust 的 egui 桌面技术栈

- 状态:已接受
- 日期:2026-07-25

## 决策

桌面界面使用 `eframe`/`egui`,主机侧组件全部使用 Rust。不使用 Electron、React、Tauri 或 Kotlin Android 辅助程序。

## 后果

项目只有一种主语言,并直接掌控生命周期与内存。代价是:终端模拟、复杂数据表格、视频解码、无障碍、Android 框架集成都需要更多原创工程和更广的平台测试。设备端不支持的能力必须如实呈现,不得用编造的数据近似。
