中文 | [English](en/architecture.md)

# 架构

Fadb 将界面、应用编排、设备状态与传输层相互分离。

```text
egui 桌面
  -> 有界的类型化命令
后端运行时 (Tokio)
  -> 设备注册表
  -> 功能服务
  -> AdbTransport
  -> adb server / Android 设备
  <- 有界的类型化事件
```

## 规则

1. egui 线程永不等待 ADB 或文件系统 I/O。
2. 每个操作都携带显式的设备序列号;长时间运行的操作还携带设备代数 (generation)。
3. 流式会话携带会话 ID,过期事件可以直接丢弃。
4. `AdbTransport` 可替换为确定性的假实现 (fake)。
5. 校验、风险分级、超时、取消与清理都由后端负责。
6. UI 状态、持久化偏好、设备会话状态是互相独立的类型。
7. 设备不支持的能力必须显式表达,而不是编造零值。

## 0.4 运行时

当前版本使用受控的 `adb` 子进程适配器:轮询 `adb devices -l` 并在 `DeviceRegistry` 中对账快照;通过固定的只读命令获取概览字段;启动交互式 `adb shell -tt`;以二进制安全的 `exec-out screencap -p` 截图;浏览与管理远端文件。文件传输运行在可取消的后端任务中,保证命令循环保持响应;取消令牌会终结有界的 ADB 子进程,且每个操作都绑定设备代数和请求 ID。文件修改使用位置 shell 参数、由后端强制执行覆盖/删除检查,完成后刷新受影响目录。原生 ADB host、shell-v2 resize、sync、forward、reverse 协议会逐步替换依赖子进程的路径。
