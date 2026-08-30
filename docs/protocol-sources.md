中文 | [English](en/protocol-sources.md)

# 协议来源

实现必须使用公开规范或 Fadb 自行捕获的 fixture。

- Android Debug Bridge 概览: https://developer.android.com/tools/adb
- ADB 源码/协议参考: https://android.googlesource.com/platform/packages/modules/adb/
- Android shell 服务协议源码: https://android.googlesource.com/platform/packages/modules/adb/+/refs/heads/main/SERVICES.TXT
- ADB CLI shell PTY 与 exec-out 行为: https://android.googlesource.com/platform/packages/modules/adb/+/refs/heads/main/docs/user/adb.1.md
- Android screencap 源码/工具行为: https://android.googlesource.com/platform/frameworks/base/+/refs/heads/main/cmds/screencap/
- scrcpy 开发文档: https://github.com/Genymobile/scrcpy/blob/master/doc/develop.md
- Chrome DevTools 协议: https://chromedevtools.github.io/devtools-protocol/
- UI Automator: https://developer.android.com/training/testing/other-components/ui-automator

特性相关的来源版本与访问日期必须在实现开始时记录。绝不允许仅凭 AI 回答推断协议字节。

## scrcpy 视频投屏 (0.7,2026-08-29 实现)

规范性来源,均访问于 2026-08-29:

- 固定 server 版本的协议描述(tag `v3.3.4` 的 `doc/develop.md`,其记载的是"scrcpy v2.1 的当前协议"线上格式): https://raw.githubusercontent.com/Genymobile/scrcpy/v3.3.4/doc/develop.md
- 固定 server 工件 `scrcpy-server-v3.3.4`(90,980 字节),SHA-256 为 `8588238c9a5a00aa542906b6ec7e6d5541d9ffb9b5d0f6e1bc0e365e2303079e`,下载自 https://github.com/Genymobile/scrcpy/releases/download/v3.3.4/scrcpy-server-v3.3.4(通过 GitHub API 中 release 271207312 的 `digest` 字段交叉核对)。该工件 vendor 在 `crates/fadb-scrcpy/assets/scrcpy-server-v3.3.4`;Fadb 在构建期通过 `include_bytes!` + 单元测试校验该 SHA-256。scrcpy 采用 Apache-2.0 许可;此工件未经修改地再分发并在此署名。
- 解码器:`openh264` Rust crate,固定 0.9.8(crates.io,2026-08-08 发布,`rust-version = 1.85`),https://crates.io/crates/openh264(文档: https://docs.rs/openh264)。相比 ffmpeg sidecar 的优势:在普通 `cargo build` 中经 `cc` 编译内置的 OpenH264(BSD-2,Cisco)C 源码,无外部二进制;其吞吐(≈1080p 解码只需个位数毫秒)足以支撑投屏。仅 H.264(`video_codec=h264`);0.7 不解码 H.265/AV1。

按固定的 `develop.md` 记载的协议摘要(信息性转述;规范性文本以上方链接为准):

1. 把 server jar 推送到 `/data/local/tmp/scrcpy-server.jar`。
2. 隧道:Fadb 采用 forward 变体 — 启动 server 前 `adb forward tcp:<port> localabstract:scrcpy_<scid>`,并向 server 传入 `tunnel_forward=true` 与相同的 `scid=<31 位随机数>`,server 在该 abstract socket 名上监听。文档把名字写作 `scrcpy_<SCID>` 但未固定字符串编码;编码已在设备上验证(允许的输入:可观察行为,2026-08-30,LDPlayer 模拟器 / Android 14):server 用 `Integer.parseInt(value, 16)` 解析 `scid=`(radix 16 — 含 `8`/`9` 的十进制值会抛 `NumberFormatException`,logcat 报 "under radix 16"),并把 socket 命名为 `scrcpy_` + `%08x`(小写十六进制,补零到 8 位;`scid=888` 在 `/proc/net/unix` 中产生 `@scrcpy_00000888`)。因此 Fadb 把选项与 socket 名都格式化为 `{scid:08x}`。
3. server 启动:`adb shell CLASSPATH=/data/local/tmp/scrcpy-server.jar app_process / com.genymobile.scrcpy.Server 3.3.4 log_level=… scid=… tunnel_forward=true audio=false control=false max_size=… video_bit_rate=…`。
4. `audio=false` 且 `control=false` 时,server 恰好打开一个 socket(视频),即元数据意义上的"第一个 socket"。
5. forward 隧道上,设备在第一个 socket 先发一个哑字节(用于检测陈旧连接),随后是设备元数据(设备名)。
6. 元数据之后的视频流:codec 元数据(codec id `u32` BE — `h264` = `0x68323634` — 再视频宽 `u32`、高 `u32`),然后是一串 12 字节头的包:8 字节头最高位是配置包标志,次高位是关键帧标志,低 62 位是 PTS;随后是 `u32` BE 载荷长度与相应字节数的载荷。
7. Fadb 0.7 选项真值表:`audio=false`、`control=false`(仅视频,无注入 — 两者均属第二阶段范围外);`max_size` 与 `video_bit_rate` 用户可调 — 码率键为 `video_bit_rate`,与客户端 `--video-bit-rate` 一致,依据 tag `v3.3.4` 的 `doc/video.md`(https://raw.githubusercontent.com/Genymobile/scrcpy/v3.3.4/doc/video.md,访问于 2026-08-30);其余选项保持 server 默认。

真机验证备注(允许的输入:贡献者自有设备上的可观察行为;LDPlayer 模拟器,上报为 REDMI 24117RK2CC / Android 14,1600×900 @ 240dpi,软编码器 `c2.android.avc.encoder`,2026-08-30):
- 默认 `cleanup=true` 的 server 在进程退出时会删除 `/data/local/tmp/scrcpy-server.jar`(观察到:推送时 jar 存在,server 被杀后消失),因此 Fadb 每次投屏启动都重新推送 jar。
- 未知选项会让 server 立即中止(`Aborted`,Java 异常);缺 jar 则中止于 `ClassNotFoundException: com.genymobile.scrcpy.Server`;两者都通过投屏错误详情中的 shell 日志尾部呈现。
- forward 隧道模式下,即使 server 尚未监听,adb 也会接受本地 TCP 连接,设备侧连接失败时才将其关闭;Fadb 轮询连接 + 流头读取,直到 server 自报家门。
- server 仅在显示内容变化时出帧:静态画面合法地只有 0–1 fps(6 秒静态 `screenrecord` 恰好含一个关键帧)。持续交互时,接收速率与裸 socket 控制客户端(读同一路流)及设备自身 `screenrecord` 一致(空闲主机的 LDPlayer 720p 约 3–7 fps,其主显示在主机侧以 60 fps 合成)— 瓶颈是模拟器虚拟显示的合成馈送,而非 Fadb 的解码/UI 管线:0.5 秒的动画积压以 ≈31 ms 的 PTS 间隔(32 fps 节奏)一次性到达,管线是"有帧就立刻发"。`max_size` 低于显示宽度会让此构建的 server 完全卡住(缩放投影 bug),因此 1280 是该环境的安全预设。

Fadb 自证验证(允许的输入:贡献者自有设备上的可观察行为):端到端会话(推送、forward、启动、流头、解复用、解码、停止、设备端 server 死亡后重启)已于 2026-08-30 在真机上手动验证;没有自动化真机测试,CI 保持封闭。单元测试以仅依据上述文档格式构造的合成 fixture 固定字节布局(jar 哈希、参数列表、socket 名推导、头解析)。
