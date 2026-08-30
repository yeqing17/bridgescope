中文 | [English](../en/adr/0002-keep-adb-binary-wrapper.md)

# ADR 0002:保留 adb 二进制包装层,不引入原生 Rust ADB 客户端

- 状态:已接受
- 日期:2026-08-30

## 决策

`fadb-adb` 继续以子进程方式调用平台工具中的 `adb` 二进制。不引入 Rust 原生 ADB 协议客户端库。

候选评估(2026-08):

- [radb](https://crates.io/crates/radb)([oslo254804746/radb](https://github.com/oslo254804746/radb))— Python openatx/adbutils 的 Rust 移植;自行实现 ADB 协议,无需 adb 二进制。否决:纯同步 API(我们的技术栈是 tokio)、功能列表中没有配对与 mDNS、logcat 等流是阻塞迭代器、成熟度早期(v0.1.8,总下载约 8k / 近期约 60,单人作者)。
- [adb_client](https://github.com/cocool97/adb_client)— 异步(tokio)、活跃维护、adb-server 代理 + 直连 USB/TCP 传输、支持 mDNS。若重新审视本决策,它是更强的候选。

## 背景

协议原生客户端唯一的实质收益是去掉 `adb` 二进制依赖,而 Fadb 反正去不掉外部工具:投屏要调 scrcpy,录屏通过 shell 在设备上运行 `screenrecord`,AVD 管理驱动 emulator CLI。迁移还必须重新踩一遍我们解析器已经编码的真机 ROM 怪癖(双层引号的 Wi-Fi SSID、`null` font_scale、`ip` 命令前缀),并在同步 API 之上重建流式界面 — 交互终端与实时 logcat。

## 后果

应用继续要求 Android 平台工具,或在 PATH 上,或定位到 SDK 路径。性能指标等高频轮询每次采样都要付出进程启动成本。如果将来出现免 SDK 便携版的需求,从 `adb_client` 开始重新评估本决策。
