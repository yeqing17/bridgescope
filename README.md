<div align="center">
  <img src="docs/social-preview.png" width="640" alt="fadb — a featherweight ADB toolbox, in Rust"/>
  <p>
    <a href="CHANGELOG.md"><img alt="Version" src="https://img.shields.io/badge/version-0.8.8-3DDC84"></a>
    <a href="rust-toolchain.toml"><img alt="Rust" src="https://img.shields.io/badge/rust-1.90-DEA584?logo=rust&logoColor=white"></a>
    <a href="https://github.com/emilk/egui"><img alt="GUI" src="https://img.shields.io/badge/GUI-egui%20%2F%20eframe-FEBD2F"></a>
    <img alt="Platform" src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-4A90D9">
    <a href="https://github.com/yeqing17/fadb/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/yeqing17/fadb/actions/workflows/ci.yml/badge.svg"></a>
    <a href="LICENSE-MIT"><img alt="License: MIT OR Apache-2.0" src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue"></a>
  </p>
  <p><a href="README.md">中文</a> | <a href="README.en.md">English</a></p>
</div>

**fadb** 是一个独立实现的纯 Rust 桌面工具集,通过 ADB 检查与管理 Android 设备——轻量、跨平台、开箱即用。功能边界与路线图见 [`docs/feature-matrix.md`](docs/feature-matrix.md),完整变更见 [`CHANGELOG.md`](CHANGELOG.md)。

## ✨ 功能

### 📱 设备与连接

- **设备发现:** 自动发现 USB 与网络设备,显式选择后才操作。
- **设备管理器:** 无线配对、`adb tcpip 5555`、mDNS 发现、网络端点直连与一键断开,最多记忆八个历史端点。

### 🛠 开发工具

- **交互式终端:** 二进制安全的双向流,支持拖拽选中文本(松开即复制)、右键菜单复制/粘贴与括号粘贴;快捷指令栏可自定义,本地持久化,支持 JSON 导入/导出。
- **文件管理:** 浏览远端目录,上传/下载(覆盖需显式确认,可取消),新建文件夹、重命名、删除文件与目录(递归删除需确认);列表显示修改时间,可按名称/大小/修改时间排序。
- **应用管理:** 列出安装包与启动图标,查看版本、安装来源与权限;支持启动、强制停止、清除数据、冻结/解冻与 APK 安装——APK 直接拖入窗口即可,破坏性操作均需确认。

### 📊 观测与调试

- **设备概览:** 型号、序列号、Android/kernel 版本、CPU、内存、存储、电池、分辨率等一屏速览,任意字段点击即复制。
- **进程与性能:** 进程表(PID/用户/CPU/内存)自动刷新;CPU、内存、存储与电池指标持续采样,渐变面积图展示历史趋势,采样间隔按设备速度自适应。
- **实时日志:** `logcat -v threadtime` 流式输出,按级别着色、过滤、暂停、自动滚动、保存到文件。
- **布局检查:** `uiautomator dump` 捕获前台视图树,可搜索、查看节点属性、复制 dump、导出 XML。
- **网页检查:** 发现设备上的 WebView 调试 socket,转发端口并列出可调试页面,一键在 Chrome DevTools 中打开。

### 🎬 截图与投屏

- **截图:** 二进制安全的 `screencap`,UI 线程外解码,支持适应/100% 显示、复制图像与保存 PNG。
- **投屏:** 内置 scrcpy server 3.3.4 转发隧道,可调最大尺寸/码率,按键行遥控设备,一键录制 MP4。

### 🤖 AI 助手

- 停靠面板对接任意 OpenAI 兼容接口,base URL、API key 与模型名全部本机存储;系统提示词锚定在 Android 调试与标准 `adb` 命令上。

## 🖥 界面

- 简体中文 / English 一键切换,浅色 / 深色主题。
- 左侧导航可折叠为图标栏,折叠状态跨启动记忆。
- 设置窗口(顶栏齿轮)集中提供主题、语言、ADB 信息(可执行文件路径、版本、设备数)与关于信息。
- 无边框自绘窗口:标题栏拖拽移动、边缘拖拽缩放、双击标题栏最大化。

## 🚀 快速开始

| 依赖 | 说明 |
| --- | --- |
| Rust 1.90 | 版本由 [`rust-toolchain.toml`](rust-toolchain.toml) 固定,rustfmt 与 clippy 一并就位 |
| `adb` | 通过 `PATH`、`ANDROID_SDK_ROOT` 或 `ANDROID_HOME` 查找;也可设置 `FADB_ADB` 直接指定 `adb` 可执行文件路径(优先级最高,路径无效会在界面明确报错)。还没有 adb 时,设置窗口的 ADB 区域提供官方下载入口 |
| 桌面构建依赖 | `eframe` 在 Windows / macOS / Linux 上的常规构建依赖 |

```bash
cargo run -p fadb-desktop
```

不接设备、使用假后端体验界面(按所用终端任选一种):

```bash
# macOS / Linux / Git Bash
FADB_FAKE=1 cargo run -p fadb-desktop

# Windows PowerShell
$env:FADB_FAKE = "1"; cargo run -p fadb-desktop

# Windows cmd
set FADB_FAKE=1
cargo run -p fadb-desktop
```

## 🧪 质量检查

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build --workspace --release
```

## 🔒 安全

Fadb 将每个结构化操作绑定到显式的设备序列号与连接代数,破坏性能力由后端强制要求确认。交互式终端是不设限的专家功能;任意的 Android shell 命令无法做到安全化。

## 🤝 独立性与致谢

fadb 的功能设计参考了 [AYA](https://github.com/liriliri/aya),感谢它验证了这条产品路线。fadb 是独立的净室实现:不复制 AYA 的源代码、品牌、图标、翻译、截图或视觉素材,规则见 [`docs/clean-room.md`](docs/clean-room.md)。

Android 是 Google LLC 的商标,fadb 与 Google、Android 无隶属或背书关系。投屏功能内置并调用 [scrcpy](https://github.com/Genymobile/scrcpy) 的 server(Genymobile,Apache-2.0,未经修改地再分发并在此署名),fadb 客户端为独立实现;工件版本、哈希与许可见 [`docs/protocol-sources.md`](docs/protocol-sources.md)。

## 📄 许可

Fadb 源码在 MIT 许可或 Apache 许可 2.0 之下提供,由你自行选择。第三方工件保留其各自的许可与声明。
