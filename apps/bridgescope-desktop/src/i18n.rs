use bridgescope_domain::BridgeError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Language {
    English,
    Chinese,
}

impl Language {
    pub const fn toggle(self) -> Self {
        match self {
            Self::English => Self::Chinese,
            Self::Chinese => Self::English,
        }
    }

    pub const fn short_name(self) -> &'static str {
        match self {
            Self::English => "EN",
            Self::Chinese => "中文",
        }
    }
}

#[allow(clippy::match_same_arms, clippy::too_many_lines)]
pub fn text(language: Language, key: &str) -> &'static str {
    match (language, key) {
        (Language::Chinese, "overview") => "概览",
        (Language::Chinese, "files") => "文件",
        (Language::Chinese, "applications") => "应用",
        (Language::Chinese, "processes") => "进程",
        (Language::Chinese, "performance") => "性能",
        (Language::Chinese, "shell") => "终端",
        (Language::Chinese, "layout") => "布局",
        (Language::Chinese, "screenshot") => "截图",
        (Language::Chinese, "logcat") => "日志",
        (Language::Chinese, "select_device") => "选择设备",
        (Language::Chinese, "no_device") => "未连接设备",
        (Language::Chinese, "refresh") => "刷新",
        (Language::Chinese, "device_manager") => "设备管理",
        (Language::Chinese, "diagnostics") => "ADB 诊断",
        (Language::Chinese, "coming_soon") => "后续里程碑开发",
        (Language::Chinese, "explicit_selection") => "为避免误操作，BridgeScope 不会自动选择设备。",
        (Language::Chinese, "loading") => "正在读取设备信息…",
        (Language::Chinese, "light") => "浅色",
        (Language::Chinese, "dark") => "深色",
        (Language::Chinese, "close") => "关闭",
        (Language::Chinese, "fake") => "模拟设备",
        (Language::Chinese, "connect_android") => "连接 Android 设备",
        (Language::Chinese, "ip_host") => "IP / 主机",
        (Language::Chinese, "port") => "端口",
        (Language::Chinese, "connect") => "连接",
        (Language::Chinese, "connecting") => "正在连接",
        (Language::Chinese, "recent_network_devices") => "最近连接的网络设备",
        (Language::Chinese, "forget") => "移除",
        (Language::Chinese, "connected_devices") => "已连接设备",
        (Language::Chinese, "model") => "型号",
        (Language::Chinese, "serial") => "序列号",
        (Language::Chinese, "state") => "状态",
        (Language::Chinese, "product") => "产品",
        (Language::Chinese, "action") => "操作",
        (Language::Chinese, "select") => "选择",
        (Language::Chinese, "process_snapshot_hint") => "每 3 秒更新一次进程快照",
        (Language::Chinese, "applications_hint") => {
            "列出设备上已安装的应用；默认仅显示第三方应用。"
        }
        (Language::Chinese, "app_filter_third") => "第三方",
        (Language::Chinese, "app_filter_system") => "系统应用",
        (Language::Chinese, "app_filter_all") => "全部",
        (Language::Chinese, "apps_search_hint") => "搜索包名…",
        (Language::Chinese, "no_applications") => "没有读取到应用",
        (Language::Chinese, "app_badge_system") => "系统",
        (Language::Chinese, "app_badge_frozen") => "已冻结",
        (Language::Chinese, "app_details_hint") => "在左侧选择一个应用查看详情",
        (Language::Chinese, "app_info") => "应用信息",
        (Language::Chinese, "app_version") => "版本",
        (Language::Chinese, "app_target_sdk") => "目标 SDK",
        (Language::Chinese, "app_min_sdk") => "最低 SDK",
        (Language::Chinese, "app_installer") => "安装来源",
        (Language::Chinese, "app_first_install") => "首次安装",
        (Language::Chinese, "app_last_update") => "最近更新",
        (Language::Chinese, "app_apk_path") => "APK 路径",
        (Language::Chinese, "app_permissions") => "声明的权限",
        (Language::Chinese, "app_permissions_none") => "未声明权限",
        (Language::Chinese, "app_details_unavailable") => "未能读取应用信息",
        (Language::Chinese, "app_open") => "打开",
        (Language::Chinese, "app_force_stop") => "强制停止",
        (Language::Chinese, "app_clear_data") => "清除数据",
        (Language::Chinese, "app_freeze") => "冻结",
        (Language::Chinese, "app_unfreeze") => "解冻",
        (Language::Chinese, "app_uninstall") => "卸载",
        (Language::Chinese, "app_action_running") => "正在执行",
        (Language::Chinese, "app_confirm_clear_title") => "确认清除应用数据？",
        (Language::Chinese, "app_confirm_clear_body") => {
            "将清除该应用的所有数据（设置、账号、数据库），不可恢复。"
        }
        (Language::Chinese, "app_confirm_uninstall_title") => "确认卸载？",
        (Language::Chinese, "app_confirm_uninstall_body") => {
            "将从当前用户卸载该应用。系统应用仅移除当前用户的安装。"
        }
        (Language::Chinese, "confirm") => "确认",
        (Language::Chinese, "cancel") => "取消",
        (Language::Chinese, "performance_live_hint") => "每秒采样，保留最近 60 秒",
        (Language::Chinese, "performance_waiting") => "正在采集性能数据…",
        (Language::Chinese, "no_processes") => "没有读取到进程",
        (Language::Chinese, "pid") => "PID",
        (Language::Chinese, "process_name") => "进程",
        (Language::Chinese, "user") => "用户",
        (Language::Chinese, "cpu") => "CPU",
        (Language::Chinese, "memory") => "内存",
        (Language::Chinese, "resident") => "常驻内存",
        (Language::Chinese, "load_1m") => "1 分钟负载",
        (Language::Chinese, "battery") => "电量",
        (Language::Chinese, "performance_history") => "性能趋势",
        (Language::Chinese, "assistant_ready") => "已连接",
        (Language::Chinese, "assistant_not_configured") => "未配置",
        (Language::Chinese, "assistant_configure") => "配置…",
        (Language::Chinese, "assistant_privacy_note") => {
            "未配置提供商或未授权前，不会有任何数据离开本机。流式输出属于后续里程碑。"
        }
        (Language::Chinese, "assistant_empty_hint") => {
            "可以询问当前设备、ADB 报错或某行日志；未经授权，助手无法访问设备数据。"
        }
        (Language::Chinese, "assistant_you") => "你",
        (Language::Chinese, "assistant_label") => "助手",
        (Language::Chinese, "assistant_waiting") => "等待助手回复…",
        (Language::Chinese, "assistant_input_hint") => "向助手提问…",
        (Language::Chinese, "assistant_send") => "发送",
        (Language::Chinese, "assistant_send_hint") => "Enter 发送 · Shift+Enter 换行",
        (Language::Chinese, "ai_settings") => "AI 设置",
        (Language::Chinese, "ai_endpoint") => "接口地址（Base URL）",
        (Language::Chinese, "ai_model_name") => "模型名称",
        (Language::Chinese, "ai_api_key") => "API 密钥",
        (Language::Chinese, "ai_timeout_seconds") => "超时（秒）",
        (Language::Chinese, "ai_save") => "保存并连接",
        (Language::Chinese, "ai_disable") => "停用 AI",
        (Language::Chinese, "ai_settings_hint") => {
            "兼容 OpenAI 接口即可：OpenAI、DeepSeek、智谱 GLM、Ollama 等。密钥仅保存在本机。"
        }
        (Language::Chinese, "win_minimize") => "最小化",
        (Language::Chinese, "win_maximize") => "最大化",
        (Language::Chinese, "win_restore") => "还原",
        (Language::Chinese, "win_close") => "关闭",
        (Language::Chinese, "files_select_device") => "请先选择在线设备，再浏览设备文件。",
        (Language::Chinese, "files_back") => "后退",
        (Language::Chinese, "files_up") => "上一级",
        (Language::Chinese, "files_go") => "前往",
        (Language::Chinese, "files_new_folder") => "新建文件夹",
        (Language::Chinese, "files_upload") => "上传",
        (Language::Chinese, "files_upload_dialog") => "选择要上传的文件",
        (Language::Chinese, "files_upload_invalid_name") => "上传失败：本地文件名包含无效字符。",
        (Language::Chinese, "files_download") => "下载",
        (Language::Chinese, "files_download_dialog") => "选择保存位置",
        (Language::Chinese, "files_cancel_transfer") => "取消传输",
        (Language::Chinese, "files_loading") => "正在加载目录…",
        (Language::Chinese, "files_name") => "名称",
        (Language::Chinese, "files_type") => "类型",
        (Language::Chinese, "files_size") => "大小",
        (Language::Chinese, "files_modified") => "修改时间",
        (Language::Chinese, "files_kind_file") => "文件",
        (Language::Chinese, "files_kind_directory") => "文件夹",
        (Language::Chinese, "files_kind_symlink") => "符号链接",
        (Language::Chinese, "files_kind_other") => "其他",
        (Language::Chinese, "files_rename") => "重命名",
        (Language::Chinese, "files_delete") => "删除",
        (Language::Chinese, "files_transferring") => "正在传输…",
        (Language::Chinese, "files_overwrite_remote_title") => "覆盖远端文件？",
        (Language::Chinese, "files_overwrite_local_title") => "覆盖本地文件？",
        (Language::Chinese, "files_overwrite_body") => "「{}」已存在，是否替换？",
        (Language::Chinese, "files_replace") => "替换",
        (Language::Chinese, "files_delete_title") => "删除条目？",
        (Language::Chinese, "files_delete_body") => "将删除选中的条目，此操作不可恢复。",
        (Language::Chinese, "files_invalid_name") => "远端名称或所选条目无效。",
        (Language::Chinese, "files_enter") => "进入",
        (Language::Chinese, "files_copy_path") => "复制路径",
        (Language::Chinese, "screenshot_capture") => "截图",
        (Language::Chinese, "screenshot_retake") => "重新截图",
        (Language::Chinese, "screenshot_save_png") => "保存 PNG",
        (Language::Chinese, "screenshot_fit") => "适应窗口",
        (Language::Chinese, "screenshot_actual") => "100%",
        (Language::Chinese, "screenshot_copy_image") => "复制图片",
        (Language::Chinese, "screenshot_capturing") => "正在截图并解码 PNG…",
        (Language::Chinese, "screenshot_hint") => "选择在线设备后，点击「截图」抓取当前屏幕。",
        (Language::Chinese, "screenshot_saved") => "已保存到 ",
        (Language::Chinese, "screenshot_pixels") => "像素",
        (Language::Chinese, "shell_title") => "交互式终端",
        (Language::Chinese, "shell_status_disconnected") => "未连接",
        (Language::Chinese, "shell_status_connecting") => "连接中",
        (Language::Chinese, "shell_status_connected") => "已连接",
        (Language::Chinese, "shell_status_closing") => "断开中",
        (Language::Chinese, "shell_status_exited") => "已退出",
        (Language::Chinese, "shell_status_failed") => "连接失败",
        (Language::Chinese, "shell_hint") => {
            "专家接口 · Android PTY · 远端 stderr 通常与 stdout 合并 · 连接时按面板大小设定终端尺寸"
        }
        (Language::Chinese, "shell_disconnect") => "断开",
        (Language::Chinese, "shell_clear_display") => "清屏",
        (Language::Chinese, "shell_copy_visible") => "复制可见内容",
        (Language::Chinese, "shell_focus_terminal") => "聚焦终端",
        (Language::Chinese, "shell_select_online") => "请先选择在线设备再连接。",
        (Language::Chinese, "shell_empty_hint") => "点击「连接」并聚焦此终端后即可输入命令。",
        (Language::Chinese, "logcat_start") => "开始捕获",
        (Language::Chinese, "logcat_stop") => "停止",
        (Language::Chinese, "logcat_pause") => "暂停",
        (Language::Chinese, "logcat_resume") => "继续",
        (Language::Chinese, "logcat_clear") => "清空",
        (Language::Chinese, "logcat_save") => "保存日志",
        (Language::Chinese, "logcat_autoscroll") => "自动滚动",
        (Language::Chinese, "logcat_level") => "级别",
        (Language::Chinese, "logcat_level_all") => "全部",
        (Language::Chinese, "logcat_streaming") => "正在捕获日志",
        (Language::Chinese, "logcat_starting") => "正在连接 logcat…",
        (Language::Chinese, "logcat_idle") => "未在捕获",
        (Language::Chinese, "logcat_paused") => "已暂停（新日志被丢弃）",
        (Language::Chinese, "logcat_search_hint") => "搜索 TAG 或内容…",
        (Language::Chinese, "logcat_line_count") => "显示 {visible} / {total} 行",
        (Language::Chinese, "layout_loading") => "正在捕获视图层次…（忙碌界面可能需要数秒）",
        (Language::Chinese, "layout_empty") => "暂无快照，点击「刷新」捕获当前界面。",
        (Language::Chinese, "layout_export") => "导出 XML",
        (Language::Chinese, "layout_search_hint") => "搜索 class / id / 文本…",
        (Language::Chinese, "layout_node_count") => "节点数",
        (Language::Chinese, "layout_attributes") => "属性",
        (Language::Chinese, "layout_no_selection") => "在左侧选择一个节点查看属性。",
        (Language::Chinese, "layout_copy_hint") => "复制该子树的文本摘要",
        (Language::Chinese, "attr_class") => "类名",
        (Language::Chinese, "attr_resource_id") => "资源 ID",
        (Language::Chinese, "attr_text") => "文本",
        (Language::Chinese, "attr_content_desc") => "内容描述",
        (Language::Chinese, "attr_package") => "包名",
        (Language::Chinese, "attr_bounds") => "边界 (x, y, 宽, 高)",
        (Language::Chinese, "attr_children") => "子节点",
        (Language::Chinese, "attr_clickable") => "可点击",
        (Language::Chinese, "attr_scrollable") => "可滚动",
        (Language::Chinese, "attr_enabled") => "可用",
        (Language::Chinese, "attr_selected") => "选中",
        (Language::Chinese, "attr_focused") => "聚焦",
        (Language::Chinese, "attr_yes") => "是",
        (Language::Chinese, "attr_no") => "否",
        (Language::Chinese, "attr_disabled") => "已禁用",
        (Language::Chinese, "copy") => "复制",
        (Language::Chinese, "webview_refresh_sockets") => "刷新调试服务",
        (Language::Chinese, "webview_sockets") => "调试服务（DevTools socket）",
        (Language::Chinese, "webview_pages") => "可调试页面",
        (Language::Chinese, "webview_none_found") => {
            "未发现 WebView 调试服务。应用需开启 setWebContentsDebuggingEnabled(true)，且存在运行中的 WebView/浏览器进程。"
        }
        (Language::Chinese, "webview_select_socket") => "选择一个调试服务查看页面。",
        (Language::Chinese, "webview_no_pages") => "该调试服务没有可调试页面。",
        (Language::Chinese, "webview_col_title") => "标题",
        (Language::Chinese, "webview_col_url") => "URL",
        (Language::Chinese, "webview_col_type") => "类型",
        (Language::Chinese, "webview_col_action") => "操作",
        (Language::Chinese, "webview_open_page") => "打开页面",
        (Language::Chinese, "webview_open_devtools") => "打开调试器",
        (Language::Chinese, "webview_copy_url") => "复制调试地址",
        (Language::Chinese, "webview_debug_hint") => {
            "调试器通过本地端口转发（DevTools 协议）在浏览器中打开；转发会在刷新服务列表时自动清理。"
        }
        (Language::Chinese, "applications_install") => "安装 APK…",
        (Language::Chinese, "applications_install_running") => "正在安装 APK…",
        (Language::Chinese, "applications_install_ok") => "APK 安装成功",
        (Language::Chinese, "avd") => "模拟器",
        (Language::Chinese, "avd_name") => "名称",
        (Language::Chinese, "avd_status") => "状态",
        (Language::Chinese, "avd_running") => "运行中",
        (Language::Chinese, "avd_stopped") => "已停止",
        (Language::Chinese, "avd_launch") => "启动",
        (Language::Chinese, "avd_launch_hint") => "启动该模拟器",
        (Language::Chinese, "avd_launch_wipe") => "清空数据启动",
        (Language::Chinese, "avd_launch_wipe_hint") => "清除应用数据并冷启动",
        (Language::Chinese, "avd_stop") => "停止",
        (Language::Chinese, "avd_none") => "未发现模拟器。请确认已安装 Android SDK 模拟器。",
        (Language::Chinese, "avd_starting") => "正在启动模拟器…",
        (Language::Chinese, "avd_started_hint") => "模拟器已启动，等待设备上线…",
        (Language::Chinese, "avd_kill_done") => "模拟器已停止",
        (Language::Chinese, "avd_hint") => {
            "列表来自 emulator -list-avds；启动后的设备会自动出现在设备列表中。"
        }
        (Language::Chinese, "wireless") => "无线调试",
        (Language::Chinese, "wireless_hint") => "支持配对、切换无线模式与发现局域网调试服务。",
        (Language::Chinese, "wireless_pair") => "配对地址",
        (Language::Chinese, "wireless_pair_go") => "配对",
        (Language::Chinese, "wireless_pairing") => "正在配对…",
        (Language::Chinese, "wireless_pair_ok") => "配对成功，请用设备 IP 连接。",
        (Language::Chinese, "wireless_code") => "配对码",
        (Language::Chinese, "wireless_tcpip") => "无线模式",
        (Language::Chinese, "wireless_tcpip_running") => "正在切换到无线模式…",
        (Language::Chinese, "wireless_tcpip_ok") => "已切换到无线模式，请用设备 IP 连接。",
        (Language::Chinese, "wireless_tcpip_hint") => {
            "将当前设备切换到网络调试（端口 5555），随后用设备 IP 连接。"
        }
        (Language::Chinese, "wireless_tcpip_need_device") => "请先选择一个设备",
        (Language::Chinese, "wireless_mdns") => "发现设备",
        (Language::Chinese, "wireless_mdns_hint") => "列出局域网内正在广播的调试服务。",
        (Language::Chinese, "wireless_connect") => "连接",
        (Language::Chinese, "wireless_none") => "未发现调试服务。",
        (Language::Chinese, "mirror") => "投屏",
        (Language::Chinese, "mirror_hint") => {
            "将所选设备的实时画面镜像到本窗口（仅视频，不注入触控）。"
        }
        (Language::Chinese, "mirror_start") => "开始投屏",
        (Language::Chinese, "mirror_stop") => "停止投屏",
        (Language::Chinese, "mirror_starting") => "正在启动设备端服务…",
        (Language::Chinese, "mirror_running") => "投屏中",
        (Language::Chinese, "mirror_native") => "原生分辨率",
        (Language::Chinese, "mirror_need_device") => "请先选择一个在线设备",
        (Language::Chinese, "mirror_waiting") => "等待设备视频流…",
        (Language::Chinese, "mirror.jar_write_failed") => "写入投屏服务临时文件失败",
        (Language::Chinese, "mirror.push_failed") => "推送投屏服务到设备失败",
        (Language::Chinese, "mirror.listen_failed") => "本机监听端口失败",
        (Language::Chinese, "mirror.forward_failed") => "建立 ADB 端口转发失败",
        (Language::Chinese, "mirror.shell_failed") => "启动设备端进程失败",
        (Language::Chinese, "mirror.server_start_timeout") => "设备端服务启动超时",
        (Language::Chinese, "mirror.stream_failed") => "视频流异常",
        (Language::Chinese, "mirror.codec_mismatch") => "视频流编码不是 H.264",
        (Language::Chinese, "adb.cancelled") => "操作已取消",
        (Language::Chinese, "adb.command_failed") => "ADB 命令执行失败",
        (Language::Chinese, "adb.devices.invalid_line") => "ADB 设备列表格式异常",
        (Language::Chinese, "adb.not_found") => "未找到 adb 可执行文件",
        (Language::Chinese, "adb.output_limit") => "输出超过大小限制",
        (Language::Chinese, "adb.read_failed") => "读取 adb 输出失败",
        (Language::Chinese, "adb.spawn_failed") => "无法启动 adb 进程",
        (Language::Chinese, "adb.stderr_missing") => "无法获取 adb 标准错误流",
        (Language::Chinese, "adb.stdout_missing") => "无法获取 adb 标准输出流",
        (Language::Chinese, "adb.timed_out") => "ADB 命令执行超时",
        (Language::Chinese, "adb.wait_failed") => "等待 adb 进程退出失败",
        (Language::Chinese, "device.generation_changed") => "设备已重新连接，请刷新",
        (Language::Chinese, "device.not_found") => "设备不存在或已断开",
        (Language::Chinese, "device.unavailable") => "设备当前不可用",
        (Language::Chinese, "file.create_directory_failed") => "创建文件夹失败",
        (Language::Chinese, "file.delete_failed") => "删除失败",
        (Language::Chinese, "file.delete_invalid") => "删除目标无效",
        (Language::Chinese, "file.delete_not_regular_file") => "只能删除常规文件或文件夹",
        (Language::Chinese, "file.download_failed") => "下载失败",
        (Language::Chinese, "file.list_failed") => "读取目录失败",
        (Language::Chinese, "file.list_invalid_record") => "目录列表包含异常条目",
        (Language::Chinese, "file.local_exists") => "本地文件已存在",
        (Language::Chinese, "file.local_source_missing") => "本地文件不存在",
        (Language::Chinese, "file.local_source_not_file") => "本地路径不是常规文件",
        (Language::Chinese, "file.local_symlink_refused") => "出于安全考虑已拒绝符号链接",
        (Language::Chinese, "file.path.component_invalid") => "路径包含非法字符",
        (Language::Chinese, "file.path.escapes_root") => "路径越出了允许的根目录",
        (Language::Chinese, "file.path.invalid") => "路径无效",
        (Language::Chinese, "file.remote_exists") => "远端文件已存在",
        (Language::Chinese, "file.rename_failed") => "重命名失败",
        (Language::Chinese, "file.rename_invalid") => "重命名目标无效",
        (Language::Chinese, "file.upload_failed") => "上传失败",
        (Language::Chinese, "screenshot.capture_failed") => "截图失败",
        (Language::Chinese, "screenshot.dimensions.invalid") => "截图尺寸无效",
        (Language::Chinese, "screenshot.invalid_png") => "截图不是有效的 PNG",
        (Language::Chinese, "screenshot.save_failed") => "保存截图失败",
        (Language::Chinese, "shell.already_closed") => "终端会话已关闭",
        (Language::Chinese, "shell.close_timed_out") => "关闭终端会话超时",
        (Language::Chinese, "shell.input.empty") => "输入为空",
        (Language::Chinese, "shell.input.too_large") => "输入超过大小限制",
        (Language::Chinese, "shell.kill_failed") => "终止终端进程失败",
        (Language::Chinese, "shell.pipe_missing") => "终端管道缺失",
        (Language::Chinese, "shell.read_failed") => "读取终端输出失败",
        (Language::Chinese, "shell.size.invalid") => "终端尺寸无效",
        (Language::Chinese, "shell.spawn_failed") => "无法启动终端会话",
        (Language::Chinese, "shell.wait_failed") => "等待终端进程退出失败",
        (Language::Chinese, "shell.write_failed") => "向终端写入失败",
        (Language::Chinese, "layout.dump_failed") => "捕获视图层次失败",
        (Language::Chinese, "layout.parse_failed") => "解析视图层次失败",
        (Language::Chinese, "logcat.spawn_failed") => "无法启动日志会话",
        (Language::Chinese, "logcat.wait_failed") => "等待日志进程退出失败",
        (Language::Chinese, "webview.forward_failed") => "建立端口转发失败",
        (Language::Chinese, "webview.pages_unreachable") => "无法访问 DevTools 调试接口",
        (Language::Chinese, "applications.install_failed") => "APK 安装失败",
        (Language::Chinese, "avd.emulator_missing") => {
            "未找到 emulator 程序，请确认 Android SDK 路径"
        }
        (Language::Chinese, "avd.list_failed") => "获取模拟器列表失败",
        (Language::Chinese, "avd.launch_failed") => "启动模拟器失败",
        (Language::Chinese, "avd.kill_failed") => "停止模拟器失败",
        (Language::Chinese, "adb.pair_failed") => "无线配对失败",
        (Language::Chinese, "adb.tcpip_failed") => "切换无线模式失败",
        (Language::Chinese, "adb.mdns_failed") => "发现调试服务失败",
        (Language::Chinese, "wireless.pair_invalid") => {
            "配对信息无效：请检查地址、端口与配对码（6–8 位数字）"
        }
        (Language::Chinese, "device.target_stale") => "设备已不在线，请刷新后重试",
        (_, "overview") => "Overview",
        (_, "files") => "Files",
        (_, "applications") => "Applications",
        (_, "processes") => "Processes",
        (_, "performance") => "Performance",
        (_, "shell") => "Shell",
        (_, "layout") => "Layout",
        (_, "screenshot") => "Screenshot",
        (_, "logcat") => "Logcat",
        (_, "webview") => "WebView",
        (Language::Chinese, "assistant") => "AI 助手",
        (_, "assistant") => "AI Assistant",
        (_, "select_device") => "Select a device",
        (_, "no_device") => "No connected devices",
        (_, "refresh") => "Refresh",
        (_, "device_manager") => "Device Manager",
        (_, "diagnostics") => "ADB Diagnostics",
        (_, "coming_soon") => "Planned for a later milestone",
        (_, "explicit_selection") => {
            "BridgeScope never selects a device automatically, preventing accidental operations."
        }
        (_, "loading") => "Loading device information…",
        (_, "light") => "Light",
        (_, "dark") => "Dark",
        (_, "close") => "Close",
        (_, "fake") => "Fake device",
        (_, "connect_android") => "Connect an Android device",
        (_, "ip_host") => "IP / host",
        (_, "port") => "Port",
        (_, "connect") => "Connect",
        (_, "connecting") => "Connecting",
        (_, "recent_network_devices") => "Recent network devices",
        (_, "forget") => "Forget",
        (_, "connected_devices") => "Connected devices",
        (_, "model") => "Model",
        (_, "serial") => "Serial",
        (_, "state") => "State",
        (_, "product") => "Product",
        (_, "action") => "Action",
        (_, "select") => "Select",
        (_, "process_snapshot_hint") => "Refreshes every 3 seconds",
        (_, "applications_hint") => {
            "Lists the apps installed on the device; third-party only by default."
        }
        (_, "app_filter_third") => "Third-party",
        (_, "app_filter_system") => "System apps",
        (_, "app_filter_all") => "All",
        (_, "apps_search_hint") => "Search packages…",
        (_, "no_applications") => "No applications returned",
        (_, "app_badge_system") => "System",
        (_, "app_badge_frozen") => "Frozen",
        (_, "app_details_hint") => "Select an app on the left to see its details",
        (_, "app_info") => "App info",
        (_, "app_version") => "Version",
        (_, "app_target_sdk") => "Target SDK",
        (_, "app_min_sdk") => "Min SDK",
        (_, "app_installer") => "Installer",
        (_, "app_first_install") => "First installed",
        (_, "app_last_update") => "Last updated",
        (_, "app_apk_path") => "APK path",
        (_, "app_permissions") => "Requested permissions",
        (_, "app_permissions_none") => "No requested permissions",
        (_, "app_details_unavailable") => "Could not read the app details",
        (_, "app_open") => "Open",
        (_, "app_force_stop") => "Force stop",
        (_, "app_clear_data") => "Clear data",
        (_, "app_freeze") => "Freeze",
        (_, "app_unfreeze") => "Unfreeze",
        (_, "app_uninstall") => "Uninstall",
        (_, "app_action_running") => "Running",
        (_, "app_confirm_clear_title") => "Clear app data?",
        (_, "app_confirm_clear_body") => {
            "All data of this app (settings, accounts, databases) will be erased. This cannot \
be undone."
        }
        (_, "app_confirm_uninstall_title") => "Uninstall?",
        (_, "app_confirm_uninstall_body") => {
            "The app will be uninstalled for the current user. For system apps this removes it \
only for the current user."
        }
        (_, "confirm") => "Confirm",
        (_, "cancel") => "Cancel",
        (_, "files_select_device") => "Select an online device to browse its files.",
        (_, "files_back") => "Back",
        (_, "files_up") => "Up",
        (_, "files_go") => "Go",
        (_, "files_new_folder") => "New folder",
        (_, "files_upload") => "Upload",
        (_, "files_upload_dialog") => "Choose a file to upload",
        (_, "files_upload_invalid_name") => {
            "Upload failed: the local file name contains invalid characters."
        }
        (_, "files_download") => "Download",
        (_, "files_download_dialog") => "Save the downloaded file",
        (_, "files_cancel_transfer") => "Cancel transfer",
        (_, "files_loading") => "Loading directory…",
        (_, "files_name") => "Name",
        (_, "files_type") => "Type",
        (_, "files_size") => "Size",
        (_, "files_modified") => "Modified",
        (_, "files_kind_file") => "File",
        (_, "files_kind_directory") => "Folder",
        (_, "files_kind_symlink") => "Symlink",
        (_, "files_kind_other") => "Other",
        (_, "files_rename") => "Rename",
        (_, "files_delete") => "Delete",
        (_, "files_transferring") => "Transferring…",
        (_, "files_overwrite_remote_title") => "Overwrite remote file?",
        (_, "files_overwrite_local_title") => "Overwrite local file?",
        (_, "files_overwrite_body") => "\"{}\" already exists. Replace it?",
        (_, "files_replace") => "Replace",
        (_, "files_delete_title") => "Delete entry?",
        (_, "files_delete_body") => "The selected entry will be deleted. This cannot be undone.",
        (_, "files_invalid_name") => "The remote name or selection is invalid.",
        (_, "files_enter") => "Enter",
        (_, "files_copy_path") => "Copy path",
        (_, "screenshot_capture") => "Capture",
        (_, "screenshot_retake") => "Retake",
        (_, "screenshot_save_png") => "Save PNG",
        (_, "screenshot_fit") => "Fit",
        (_, "screenshot_actual") => "100%",
        (_, "screenshot_copy_image") => "Copy image",
        (_, "screenshot_capturing") => "Capturing and decoding PNG…",
        (_, "screenshot_hint") => "Select an online device, then capture its current screen.",
        (_, "screenshot_saved") => "Saved to ",
        (_, "screenshot_pixels") => "pixels",
        (_, "shell_title") => "Interactive Shell",
        (_, "shell_status_disconnected") => "Disconnected",
        (_, "shell_status_connecting") => "Connecting",
        (_, "shell_status_connected") => "Connected",
        (_, "shell_status_closing") => "Closing",
        (_, "shell_status_exited") => "Exited",
        (_, "shell_status_failed") => "Failed",
        (_, "shell_hint") => {
            "Expert interface - Android PTY - remote stderr usually merged - panel size set on \
             connect"
        }
        (_, "shell_disconnect") => "Disconnect",
        (_, "shell_clear_display") => "Clear display",
        (_, "shell_copy_visible") => "Copy visible",
        (_, "shell_focus_terminal") => "Focus terminal",
        (_, "shell_select_online") => "Select an online device before connecting.",
        (_, "shell_empty_hint") => "Click Connect, then focus this terminal to type.",
        (_, "logcat_start") => "Start capture",
        (_, "logcat_stop") => "Stop",
        (_, "logcat_pause") => "Pause",
        (_, "logcat_resume") => "Resume",
        (_, "logcat_clear") => "Clear",
        (_, "logcat_save") => "Save log",
        (_, "logcat_autoscroll") => "Auto-scroll",
        (_, "logcat_level") => "Level",
        (_, "logcat_level_all") => "All",
        (_, "logcat_streaming") => "Capturing",
        (_, "logcat_starting") => "Connecting to logcat…",
        (_, "logcat_idle") => "Not capturing",
        (_, "logcat_paused") => "Paused (new lines are dropped)",
        (_, "logcat_search_hint") => "Search tag or content…",
        (_, "logcat_line_count") => "Showing {visible} / {total} lines",
        (_, "layout_loading") => "Capturing view hierarchy… (busy screens may take seconds)",
        (_, "layout_empty") => "No snapshot yet. Click Refresh to capture the current screen.",
        (_, "layout_export") => "Export XML",
        (_, "layout_search_hint") => "Search class / id / text…",
        (_, "layout_node_count") => "Nodes",
        (_, "layout_attributes") => "Attributes",
        (_, "layout_no_selection") => "Select a node on the left to inspect it.",
        (_, "layout_copy_hint") => "Copy a text summary of this subtree",
        (_, "attr_class") => "Class",
        (_, "attr_resource_id") => "Resource ID",
        (_, "attr_text") => "Text",
        (_, "attr_content_desc") => "Content description",
        (_, "attr_package") => "Package",
        (_, "attr_bounds") => "Bounds (x, y, w, h)",
        (_, "attr_children") => "Children",
        (_, "attr_clickable") => "Clickable",
        (_, "attr_scrollable") => "Scrollable",
        (_, "attr_enabled") => "Enabled",
        (_, "attr_selected") => "Selected",
        (_, "attr_focused") => "Focused",
        (_, "attr_yes") => "Yes",
        (_, "attr_no") => "No",
        (_, "attr_disabled") => "Disabled",
        (_, "copy") => "Copy",
        (_, "webview_refresh_sockets") => "Refresh debug services",
        (_, "webview_sockets") => "Debug services (DevTools sockets)",
        (_, "webview_pages") => "Debuggable pages",
        (_, "webview_none_found") => {
            "No WebView debug service found. The app must enable \
setWebContentsDebuggingEnabled(true) and run an active WebView/browser."
        }
        (_, "webview_select_socket") => "Select a debug service to list its pages.",
        (_, "webview_no_pages") => "No debuggable pages on this debug service.",
        (_, "webview_col_title") => "Title",
        (_, "webview_col_url") => "URL",
        (_, "webview_col_type") => "Type",
        (_, "webview_col_action") => "Actions",
        (_, "webview_open_page") => "Open page",
        (_, "webview_open_devtools") => "Open DevTools",
        (_, "webview_copy_url") => "Copy debug URL",
        (_, "webview_debug_hint") => {
            "DevTools opens in your browser through a local port forward (DevTools protocol); \
forwards are cleaned up when the service list refreshes."
        }
        (_, "applications_install") => "Install APK…",
        (_, "applications_install_running") => "Installing APK…",
        (_, "applications_install_ok") => "APK installed",
        (_, "avd") => "Emulators",
        (_, "avd_name") => "Name",
        (_, "avd_status") => "Status",
        (_, "avd_running") => "Running",
        (_, "avd_stopped") => "Stopped",
        (_, "avd_launch") => "Launch",
        (_, "avd_launch_hint") => "Start this emulator",
        (_, "avd_launch_wipe") => "Launch wiped",
        (_, "avd_launch_wipe_hint") => "Cold boot with user data cleared",
        (_, "avd_stop") => "Stop",
        (_, "avd_none") => "No emulators found. Make sure the Android SDK emulator is installed.",
        (_, "avd_starting") => "Starting emulator…",
        (_, "avd_started_hint") => "Emulator launched; waiting for the device to come online…",
        (_, "avd_kill_done") => "Emulator stopped",
        (_, "avd_hint") => {
            "Listed by emulator -list-avds; launched devices appear in the device list \
automatically."
        }
        (_, "wireless") => "Wireless debugging",
        (_, "wireless_hint") => "Pair, switch to wireless mode, and discover LAN debug services.",
        (_, "wireless_pair") => "Pair address",
        (_, "wireless_pair_go") => "Pair",
        (_, "wireless_pairing") => "Pairing…",
        (_, "wireless_pair_ok") => "Paired; now connect with the device IP.",
        (_, "wireless_code") => "Pairing code",
        (_, "wireless_tcpip") => "Wireless mode",
        (_, "wireless_tcpip_running") => "Switching to wireless mode…",
        (_, "wireless_tcpip_ok") => "Switched to wireless mode; connect with the device IP.",
        (_, "wireless_tcpip_hint") => {
            "Switch the current device to network adb (port 5555), then connect by device IP."
        }
        (_, "wireless_tcpip_need_device") => "Select a device first",
        (_, "wireless_mdns") => "Discover",
        (_, "wireless_mdns_hint") => "Lists the debug services currently advertised on the LAN.",
        (_, "wireless_connect") => "Connect",
        (_, "wireless_none") => "No debug services discovered.",
        (_, "mirror") => "Mirror",
        (_, "mirror_hint") => {
            "Mirrors the selected device's live screen into this window (video only, no touch injection)."
        }
        (_, "mirror_start") => "Start mirroring",
        (_, "mirror_stop") => "Stop mirroring",
        (_, "mirror_starting") => "Starting the on-device server…",
        (_, "mirror_running") => "Mirroring",
        (_, "mirror_native") => "Native resolution",
        (_, "mirror_need_device") => "Select an online device first",
        (_, "mirror_waiting") => "Waiting for the device video stream…",
        (_, "mirror.jar_write_failed") => "Failed to write the mirror server temp file",
        (_, "mirror.push_failed") => "Failed to push the mirror server to the device",
        (_, "mirror.listen_failed") => "Failed to listen on a local port",
        (_, "mirror.forward_failed") => "Failed to set up the ADB port forward",
        (_, "mirror.shell_failed") => "Failed to start the on-device process",
        (_, "mirror.server_start_timeout") => "The on-device server did not start in time",
        (_, "mirror.stream_failed") => "Video stream error",
        (_, "mirror.codec_mismatch") => "The video stream is not H.264",
        (_, "performance_live_hint") => "Samples every second, keeps the last 60 seconds",
        (_, "performance_waiting") => "Collecting performance data…",
        (_, "no_processes") => "No processes returned",
        (_, "pid") => "PID",
        (_, "process_name") => "Process",
        (_, "user") => "User",
        (_, "cpu") => "CPU",
        (_, "memory") => "Memory",
        (_, "resident") => "Resident",
        (_, "load_1m") => "Load (1m)",
        (_, "battery") => "Battery",
        (_, "performance_history") => "Performance history",
        (_, "assistant_ready") => "Ready",
        (_, "assistant_not_configured") => "Not configured",
        (_, "assistant_configure") => "Configure…",
        (_, "assistant_privacy_note") => {
            "No data leaves this machine until a provider is configured and a context grant is \
given. Streaming output is a planned milestone."
        }
        (_, "assistant_empty_hint") => {
            "Ask about the selected device, an ADB error, or a log line. The assistant has no \
access to device data until you grant it."
        }
        (_, "assistant_you") => "You",
        (_, "assistant_label") => "Assistant",
        (_, "assistant_waiting") => "Waiting for the assistant…",
        (_, "assistant_input_hint") => "Ask the assistant…",
        (_, "assistant_send") => "Send",
        (_, "assistant_send_hint") => "Enter to send · Shift+Enter for a new line",
        (_, "ai_settings") => "AI Settings",
        (_, "ai_endpoint") => "Endpoint (base URL)",
        (_, "ai_model_name") => "Model name",
        (_, "ai_api_key") => "API key",
        (_, "ai_timeout_seconds") => "Timeout (seconds)",
        (_, "ai_save") => "Save & connect",
        (_, "ai_disable") => "Disable AI",
        (_, "ai_settings_hint") => {
            "Any OpenAI-compatible API works: OpenAI, DeepSeek, Zhipu GLM, Ollama, … The key is \
stored locally only."
        }
        (_, "win_minimize") => "Minimize",
        (_, "win_maximize") => "Maximize",
        (_, "win_restore") => "Restore",
        (_, "win_close") => "Close",
        _ => "",
    }
}

/// Human-readable text for a domain error: localizes the message key when a
/// translation exists and keeps the raw key (plus detail) otherwise.
#[must_use]
pub fn error_text(language: Language, error: &BridgeError) -> String {
    let message = match text(language, &error.message_key) {
        "" => error.message_key.clone(),
        translated => translated.to_owned(),
    };
    if error.detail.is_empty() {
        message
    } else {
        format!("{message}: {}", error.detail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASSISTANT_KEYS: &[&str] = &[
        "assistant",
        "assistant_ready",
        "assistant_not_configured",
        "assistant_configure",
        "assistant_privacy_note",
        "assistant_empty_hint",
        "assistant_you",
        "assistant_label",
        "assistant_waiting",
        "assistant_input_hint",
        "assistant_send",
        "assistant_send_hint",
        "ai_settings",
        "ai_endpoint",
        "ai_model_name",
        "ai_api_key",
        "ai_timeout_seconds",
        "ai_save",
        "ai_disable",
        "ai_settings_hint",
    ];

    #[test]
    fn assistant_keys_translate_for_both_languages() {
        for language in [Language::English, Language::Chinese] {
            for key in ASSISTANT_KEYS {
                assert!(
                    !text(language, key).is_empty(),
                    "missing {key:?} translation for {language:?}"
                );
            }
        }
    }

    #[test]
    fn window_control_keys_translate_for_both_languages() {
        for language in [Language::English, Language::Chinese] {
            for key in ["win_minimize", "win_maximize", "win_restore", "win_close"] {
                assert!(
                    !text(language, key).is_empty(),
                    "missing {key:?} translation for {language:?}"
                );
            }
        }
    }

    const APPLICATION_KEYS: &[&str] = &[
        "applications",
        "applications_hint",
        "app_filter_third",
        "app_filter_system",
        "app_filter_all",
        "apps_search_hint",
        "no_applications",
        "app_badge_system",
        "app_badge_frozen",
        "app_details_hint",
        "app_info",
        "app_version",
        "app_target_sdk",
        "app_min_sdk",
        "app_installer",
        "app_first_install",
        "app_last_update",
        "app_apk_path",
        "app_permissions",
        "app_permissions_none",
        "app_details_unavailable",
        "app_open",
        "app_force_stop",
        "app_clear_data",
        "app_freeze",
        "app_unfreeze",
        "app_uninstall",
        "app_action_running",
        "app_confirm_clear_title",
        "app_confirm_clear_body",
        "app_confirm_uninstall_title",
        "app_confirm_uninstall_body",
        "confirm",
        "cancel",
    ];

    #[test]
    fn application_keys_translate_for_both_languages() {
        for language in [Language::English, Language::Chinese] {
            for key in APPLICATION_KEYS {
                assert!(
                    !text(language, key).is_empty(),
                    "missing {key:?} translation for {language:?}"
                );
            }
        }
    }

    const FILES_KEYS: &[&str] = &[
        "files_select_device",
        "files_back",
        "files_up",
        "files_go",
        "files_new_folder",
        "files_upload",
        "files_upload_dialog",
        "files_upload_invalid_name",
        "files_download",
        "files_download_dialog",
        "files_cancel_transfer",
        "files_loading",
        "files_name",
        "files_type",
        "files_size",
        "files_modified",
        "files_kind_file",
        "files_kind_directory",
        "files_kind_symlink",
        "files_kind_other",
        "files_rename",
        "files_delete",
        "files_transferring",
        "files_overwrite_remote_title",
        "files_overwrite_local_title",
        "files_overwrite_body",
        "files_replace",
        "files_delete_title",
        "files_delete_body",
        "files_invalid_name",
        "files_enter",
        "files_copy_path",
    ];

    const SCREENSHOT_KEYS: &[&str] = &[
        "screenshot_capture",
        "screenshot_retake",
        "screenshot_save_png",
        "screenshot_fit",
        "screenshot_actual",
        "screenshot_copy_image",
        "screenshot_capturing",
        "screenshot_hint",
        "screenshot_saved",
        "screenshot_pixels",
    ];

    const SHELL_KEYS: &[&str] = &[
        "shell_title",
        "shell_status_disconnected",
        "shell_status_connecting",
        "shell_status_connected",
        "shell_status_closing",
        "shell_status_exited",
        "shell_status_failed",
        "shell_hint",
        "shell_disconnect",
        "shell_clear_display",
        "shell_copy_visible",
        "shell_focus_terminal",
        "shell_select_online",
        "shell_empty_hint",
    ];

    const LOGCAT_KEYS: &[&str] = &[
        "logcat_start",
        "logcat_stop",
        "logcat_pause",
        "logcat_resume",
        "logcat_clear",
        "logcat_save",
        "logcat_autoscroll",
        "logcat_level",
        "logcat_level_all",
        "logcat_streaming",
        "logcat_starting",
        "logcat_idle",
        "logcat_paused",
        "logcat_search_hint",
        "logcat_line_count",
    ];

    const LAYOUT_KEYS: &[&str] = &[
        "layout_loading",
        "layout_empty",
        "layout_export",
        "layout_search_hint",
        "layout_node_count",
        "layout_attributes",
        "layout_no_selection",
        "layout_copy_hint",
        "attr_class",
        "attr_resource_id",
        "attr_text",
        "attr_content_desc",
        "attr_package",
        "attr_bounds",
        "attr_children",
        "attr_clickable",
        "attr_scrollable",
        "attr_enabled",
        "attr_selected",
        "attr_focused",
        "attr_yes",
        "attr_no",
        "attr_disabled",
        "copy",
    ];

    const WEBVIEW_KEYS: &[&str] = &[
        "webview_refresh_sockets",
        "webview_sockets",
        "webview_pages",
        "webview_none_found",
        "webview_select_socket",
        "webview_no_pages",
        "webview_col_title",
        "webview_col_url",
        "webview_col_type",
        "webview_col_action",
        "webview_open_page",
        "webview_open_devtools",
        "webview_copy_url",
        "webview_debug_hint",
    ];

    const APPLICATIONS_KEYS: &[&str] = &[
        "applications_install",
        "applications_install_running",
        "applications_install_ok",
    ];

    const AVD_KEYS: &[&str] = &[
        "avd",
        "avd_name",
        "avd_status",
        "avd_running",
        "avd_stopped",
        "avd_launch",
        "avd_launch_hint",
        "avd_launch_wipe",
        "avd_launch_wipe_hint",
        "avd_stop",
        "avd_none",
        "avd_starting",
        "avd_started_hint",
        "avd_kill_done",
        "avd_hint",
    ];

    const WIRELESS_KEYS: &[&str] = &[
        "wireless",
        "wireless_hint",
        "wireless_pair",
        "wireless_pair_go",
        "wireless_pairing",
        "wireless_pair_ok",
        "wireless_code",
        "wireless_tcpip",
        "wireless_tcpip_running",
        "wireless_tcpip_ok",
        "wireless_tcpip_hint",
        "wireless_tcpip_need_device",
        "wireless_mdns",
        "wireless_mdns_hint",
        "wireless_connect",
        "wireless_none",
    ];

    const MIRROR_KEYS: &[&str] = &[
        "mirror",
        "mirror_hint",
        "mirror_start",
        "mirror_stop",
        "mirror_starting",
        "mirror_running",
        "mirror_native",
        "mirror_need_device",
        "mirror_waiting",
    ];

    #[test]
    fn files_screenshot_shell_keys_translate_for_both_languages() {
        for language in [Language::English, Language::Chinese] {
            for key in FILES_KEYS
                .iter()
                .chain(SCREENSHOT_KEYS)
                .chain(SHELL_KEYS)
                .chain(LOGCAT_KEYS)
                .chain(LAYOUT_KEYS)
                .chain(WEBVIEW_KEYS)
                .chain(APPLICATIONS_KEYS)
                .chain(AVD_KEYS)
                .chain(WIRELESS_KEYS)
                .chain(MIRROR_KEYS)
                .chain(["refresh", "confirm", "cancel", "connect", "copy"].iter())
            {
                assert!(
                    !text(language, key).is_empty(),
                    "missing {key:?} translation for {language:?}"
                );
            }
        }
    }

    const MESSAGE_KEYS: &[&str] = &[
        "adb.cancelled",
        "adb.command_failed",
        "adb.devices.invalid_line",
        "adb.not_found",
        "adb.output_limit",
        "adb.read_failed",
        "adb.spawn_failed",
        "adb.stderr_missing",
        "adb.stdout_missing",
        "adb.timed_out",
        "adb.wait_failed",
        "adb.mdns_failed",
        "adb.pair_failed",
        "adb.tcpip_failed",
        "avd.emulator_missing",
        "avd.kill_failed",
        "avd.launch_failed",
        "avd.list_failed",
        "device.generation_changed",
        "device.not_found",
        "device.unavailable",
        "file.create_directory_failed",
        "file.delete_failed",
        "file.delete_invalid",
        "file.delete_not_regular_file",
        "file.download_failed",
        "file.list_failed",
        "file.list_invalid_record",
        "file.local_exists",
        "file.local_source_missing",
        "file.local_source_not_file",
        "file.local_symlink_refused",
        "file.path.component_invalid",
        "file.path.escapes_root",
        "file.path.invalid",
        "file.remote_exists",
        "file.rename_failed",
        "file.rename_invalid",
        "file.upload_failed",
        "screenshot.capture_failed",
        "screenshot.dimensions.invalid",
        "screenshot.invalid_png",
        "screenshot.save_failed",
        "shell.already_closed",
        "shell.close_timed_out",
        "shell.input.empty",
        "shell.input.too_large",
        "shell.kill_failed",
        "shell.pipe_missing",
        "shell.read_failed",
        "shell.size.invalid",
        "shell.spawn_failed",
        "shell.wait_failed",
        "shell.write_failed",
        "layout.dump_failed",
        "layout.parse_failed",
        "logcat.spawn_failed",
        "logcat.wait_failed",
        "webview.forward_failed",
        "webview.pages_unreachable",
        "applications.install_failed",
        "device.target_stale",
        "wireless.pair_invalid",
        "mirror.codec_mismatch",
        "mirror.forward_failed",
        "mirror.jar_write_failed",
        "mirror.listen_failed",
        "mirror.push_failed",
        "mirror.server_start_timeout",
        "mirror.shell_failed",
        "mirror.stream_failed",
    ];

    #[test]
    fn message_keys_have_chinese_translations() {
        for key in MESSAGE_KEYS {
            assert!(
                !text(Language::Chinese, key).is_empty(),
                "missing Chinese translation for {key:?}"
            );
        }
    }

    #[test]
    fn error_text_localizes_known_keys_and_falls_back_to_raw_key() {
        let known = BridgeError::new(
            bridgescope_domain::ErrorCode::AdbFailed,
            "adb.timed_out",
            "5s",
        );
        assert_eq!(
            error_text(Language::Chinese, &known),
            "ADB 命令执行超时: 5s"
        );
        assert_eq!(error_text(Language::English, &known), "adb.timed_out: 5s");

        let unknown = BridgeError::new(
            bridgescope_domain::ErrorCode::Internal,
            "future.key",
            "boom",
        );
        assert_eq!(error_text(Language::Chinese, &unknown), "future.key: boom");

        let silent = BridgeError::new(bridgescope_domain::ErrorCode::Internal, "adb.cancelled", "");
        assert_eq!(error_text(Language::Chinese, &silent), "操作已取消");
    }
}
