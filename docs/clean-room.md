中文 | [English](en/clean-room.md)

# 净室政策

Fadb 独立重新实现 Android 调试工作流。

## 允许的输入

- 公开的 Android 与 ADB 文档。
- 公开的 scrcpy 与 Chrome DevTools 协议文档。
- 贡献者拥有或授权的设备上,工具的可观察输入/输出行为。
- Fadb 自身测试捕获的 fixture。

## 禁止的输入

- 复制、翻译、移植或机械变换 AYA 源码。
- 复制 AYA 的图标、截图、CSS、文本、翻译、测试 fixture、包标识、字节码或设备端 helper。
- 将 Fadb 宣称为官方 AYA 或 Android 产品。

每个协议特性必须在 `protocol-sources.md` 中引用规范性来源。如果受保护的 AYA 代码被有意引入,净室工作立即停止,直到审查完许可义务(包括可能适用的 AGPL-3.0 要求)。
