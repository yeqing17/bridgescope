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
        _ => "",
    }
}
