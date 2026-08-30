中文 | [English](README.en.md)

<p align="center"><img src="apps/fadb-desktop/assets/icon-256.png" width="128" alt="fadb 图标"></p>

# fadb

> a featherweight ADB toolbox, in Rust

fadb 是一个独立实现的纯 Rust 桌面工具集,通过 ADB 检查与管理 Android 设备。

> 状态:**0.8.0。** 已发布面板:ADB 发现与诊断、显式设备选择(USB 与网络)、设备概览、交互式终端(含可编辑的快捷指令栏)、二进制安全截图、提供方中立的 AI 助手、远端文件管理、应用管理与 APK 安装、进程与性能监控、实时 Logcat、布局检查器、WebView 检查、设备管理器中的无线调试(配对、TCP 模式、mDNS 发现),以及 scrcpy 投屏(可调最大尺寸/码率,带按键遥控与一键 MP4 录像)。其余路线图(AI 流式响应、投屏触摸/文本注入与音频)记录在 [`docs/feature-matrix.md`](docs/feature-matrix.md)。文件删除仅限普通文件;目录删除是有意不提供的。

## 目标

- 一个用 Rust 编写的跨平台 `egui` 桌面应用。
- 安全、显式的 Android 设备定位;Fadb 绝不悄悄选中第一台设备。
- 设备概览、文件、应用、进程、性能、终端、布局、截图、Logcat、WebView 检查、投屏与无线调试逐步交付。
- 对 ADB 子进程与流做确定性的取消与清理。
- 基于公开协议与可观察行为的独立净室实现。

## 前置要求

- Rust 1.90
- Android SDK Platform Tools(`adb`),通过 `PATH`、`ANDROID_SDK_ROOT` 或 `ANDROID_HOME` 可用
- Windows、macOS 或 Linux 上 `eframe` 的桌面构建依赖

## 运行

```bash
cargo run -p fadb-desktop
```

不接设备、使用假后端运行:

```bash
FADB_FAKE=1 cargo run -p fadb-desktop
```

Windows 命令提示符下:

```bat
set FADB_FAKE=1
cargo run -p fadb-desktop
```

## 质量检查

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build --workspace --release
```

## 已实现的工作流

- **交互式终端:** 启动 `adb -s SERIAL shell -tt`,流式传输键盘输入与 ANSI 输出,支持显式关闭/重连。初始适配器使用固定 80×24 的远端 PTY;远端 stderr 通常已合并,真正的远端 resize 要等原生 ADB shell-v2。
- **截图:** 以二进制安全的 `adb -s SERIAL exec-out screencap -p` 捕获,在 UI 线程之外校验/解码 PNG,提供适应/100% 两种显示模式,可复制解码后的图像,也可保存原始 PNG。
- **文件管理器:** 浏览远端目录、上传与下载文件(覆盖需显式确认)、支持取消、创建目录、重命名条目、仅删除普通文件。操作始终绑定所选设备的代数,完成后刷新当前列表。
- **进程:** 通过 ADB 读取在线设备的进程表,显示 PID、进程名、用户、状态、CPU、内存与常驻内存。面板可见时快照每三秒自动刷新,也可手动刷新。
- **性能:** 面板可见时每秒采样一次 CPU 使用率、负载均衡、内存、存储与电池指标。面板保留最近 60 个采样,并以轻量图表渲染 CPU、内存与电池历史。
- **网络设备:** 设备管理器直接接受 Android 设备的主机/IP 与端口进行 adb connect,保留最多八个成功端点作为本地历史,可重连或忘记已存端点。设备发现在启动时运行一次,之后只在显式刷新或网络连接后再次运行。
- **应用:** 列出已安装包及其启动图标,显示包详情(版本、安装来源、权限),支持启动、强制停止、清除数据、冻结/解冻与卸载 — 每个破坏性操作都要求显式确认。
- **Logcat:** 实时流式传输 `logcat -v threadtime`,按级别着色、严重度与文本过滤、暂停、自动滚动、保存到文件。面板打开时自动启动流,设备切换后仍保持。
- **布局检查器:** 通过 `uiautomator dump` 捕获前台窗口层级,渲染为可搜索的视图树,带逐节点属性、可复制的节点 dump 与 XML 导出。
- **WebView 检查:** 发现设备上的 WebView DevTools socket,转发本地端口,列出可调试页面,并可在浏览器或 Chrome DevTools 前端中打开。

## 安全

Fadb 将每个结构化操作绑定到显式的设备序列号与连接代数。破坏性能力将由后端强制要求确认。交互式终端是不设限的专家功能;任意的 Android shell 命令无法做到安全化。

## 独立性

Fadb 与 AYA、Android、Google 或 scrcpy 均无关联。它不复制 AYA 的源代码、品牌、图标、翻译、截图或视觉素材。见 [`docs/clean-room.md`](docs/clean-room.md)。

## 许可

Fadb 源码在 MIT 许可或 Apache 许可 2.0 之下提供,由你自行选择。第三方工件保留其各自的许可与声明。
