//! 编译时配置：环境变量注入的常量与宏。
//!
//! 这些常量在编译期读取环境变量（`option_env!`），只需在 `cargo build` 前设置对应环境变量、
//! 或 CI 的 `env:` 中注入，即可生成不同配置的应用，无需修改源码。
//! 未设置 / 为空时回落到默认值，保证任何环境都能直接编译。
//!
//! 可注入的变量：
//!   APP_URL          要嵌入的网址（支持 https://user:pass@host 形式）
//!   APP_NAME         运行期显示名（窗口标题 / 托盘提示 / 设置页标题）
//!   WINDOW_TITLE     窗口标题（缺省等于 APP_NAME）
//!   DATA_DIR_DEFAULT true=WebView2 系统默认目录 / false=%LOCALAPPDATA%
//!   PROXY_ENABLED    true/false 是否启用本地反向代理
//!   PROXY_HTTP       外部 HTTP 代理地址（留空=走自身代理）；仅 Windows
//!   PROXY_HTTPS      外部 HTTPS 代理地址（留空=同 HTTP）
//!   PROXY_BYPASS     不走代理的地址（分号分隔）
//!   TABS_ENABLED     true/false 是否启用多页签（tab）模式（仅 Windows）
//! 例：APP_URL="https://u:p@example.com" APP_NAME="MyApp" cargo build --release

// const 上下文可用的字符串相等判断：`str` 的 `==` 与 `match` 在常量求值中不可用
// （`PartialEq` 尚未稳定为 const trait，见 issue #143874），这里用字节逐个比较实现。
const fn str_eq(a: &str, b: &str) -> bool {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    if ab.len() != bb.len() {
        return false;
    }
    let mut i = 0;
    while i < ab.len() {
        if ab[i] != bb[i] {
            return false;
        }
        i += 1;
    }
    true
}

// 编译期环境变量注入宏：读环境变量 $name，非空则用，否则用 $default。
macro_rules! env_str {
    ($name:literal, $default:expr) => {{
        match option_env!($name) {
            Some(s) => {
                if s.is_empty() {
                    $default
                } else {
                    s
                }
            }
            None => $default,
        }
    }};
}

// 布尔型注入宏：true/1/yes => true；false/0/no => false；其余用默认值。
// 注意：在 `const XXX: bool = env_bool!(...)` 的常量上下文中，既不能用 `==` / `match` 比较
// `str`（`PartialEq` 非 const trait），也不能用 `&&` / `||`（conditionally-const），都会触发
// E0658。故这里借助上面的 const fn `str_eq` + if-else 链实现（字节级比较，const 安全）。
macro_rules! env_bool {
    ($name:literal, $default:expr) => {{
        match option_env!($name) {
            Some(s) => {
                if str_eq(s, "true") {
                    true
                } else if str_eq(s, "1") {
                    true
                } else if str_eq(s, "yes") {
                    true
                } else if str_eq(s, "false") {
                    false
                } else if str_eq(s, "0") {
                    false
                } else if str_eq(s, "no") {
                    false
                } else {
                    $default
                }
            }
            None => $default,
        }
    }};
}

// 把应用名清洗为合法的目录名（Windows/Linux/macOS 通用）：
//   - 替换路径非法字符 < > : " / \ | ? * 与控制字符为下划线
//   - 压缩连续空白为单个空格、去首尾空白与首尾句点
//   - 清洗后为空则回落 "Web2App"
// 用于让 Windows 的 WebView2 数据目录 / 设置目录以自定义 app_name 命名，
// 而非固定的 tauri.conf.json identifier（com.example.web2app）。
pub(crate) fn sanitize_dir_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_ws = false;
    for c in name.chars() {
        let mapped = match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 32 => '_',
            c => c,
        };
        if mapped.is_whitespace() {
            // 压缩连续空白；首个字符前的空白不输出
            if !prev_ws && !out.is_empty() {
                out.push(' ');
            }
            prev_ws = true;
        } else {
            out.push(mapped);
            prev_ws = false;
        }
    }
    let trimmed = out.trim().trim_matches('.').to_string();
    if trimmed.is_empty() {
        "Web2App".to_string()
    } else {
        trimmed
    }
}

// 默认打开的网址；支持带账号密码的形式：
//   "https://用户名:密码@www.example.com"
// 注意：现代 Chromium / WebView2 已废弃“URL 内嵌 userinfo 自动发 Basic 鉴权”的行为，
// 不会自动发送。程序会把账号密码解析出来，通过本地反向代理给所有 HTTP/WebSocket 请求统一
// 注入 `Authorization: Basic ...` 头，再转发到真实服务器。
pub(crate) const APP_URL: &str = env_str!("APP_URL", "https://用户名:密码@www.example.com");

// Ed25519 公钥（PEM/SPKI 或裸 base64，32 字节原始公钥）。当 APP_URL 是【域名数组】时
// （如 `["a.com","b.com"]`），运行时依次对这些域名做 DNS TXT 查询，用本公钥验签 +
// AES-GCM 解密 TXT 值得到真实 URL。普通字符串 URL 模式下忽略本字段。
pub(crate) const PUBLIC_KEY: &str = env_str!("PUBLIC_KEY", "");

// 应用名称（用于窗口标题、托盘提示、单实例聚焦等）
pub(crate) const APP_NAME: &str = env_str!("APP_NAME", "Web2App");
// 窗口标题（取自主机名，留空则用下面这个默认标题）
pub(crate) const WINDOW_TITLE: &str = env_str!("WINDOW_TITLE", APP_NAME);

// 额外参数（编译期注入的 JSON 文本，默认 "{}"）。由手动打包 JSON 的 `extra` 字段提供，
// 会以 `window.web2appExtra = {...}` 注入到被包裹网页，前端 JS 可直接读取。
// 必须是合法 JSON（对象/数组/标量皆可），注入时原样内联为 JS 表达式。
pub(crate) const EXTRA: &str = env_str!("EXTRA", "{}");

// ---------- 开发者工具（F12 调试，仅 Windows WebView2 生效）----------
// true  = 启用 WebView2 DevTools，运行后按 F12 即可打开开发者工具，查看 Network / Console 排查问题。
// false = 关闭（发布时建议设为 false）。
// 开发者工具（F12 调试）。发布版（release）自动关闭、调试版（debug）自动开启，无需手动改这个值。
// 之前是固定 `true`，发布包里也开着 DevTools 既不安全也不必要，故改为按构建模式自动判定。
#[cfg(debug_assertions)]
pub(crate) const DEVTOOLS_ENABLED: bool = true;
#[cfg(not(debug_assertions))]
pub(crate) const DEVTOOLS_ENABLED: bool = false;

// ---------- 数据目录（WebView2 用户数据文件夹，存 cookie / localStorage / 缓存等，仅 Windows）----------
// true  = 使用 WebView2 系统默认位置（{exe 名}.WebView2，紧挨 exe 文件）。
//         注意：打包安装（msi/nsis）后 exe 通常在 Program Files，标准用户对该目录无写权限，
//         此时 WebView2 初始化会失败、窗口直接打不开。故【安装版强烈建议设为 false】。
// false = 固定到“用户本地数据目录 / 程序名”（%LOCALAPPDATA%\com.example.web2app），
//         始终可写，适合安装版与多用户场景（推荐）。
pub(crate) const DATA_DIR_DEFAULT: bool = env_bool!("DATA_DIR_DEFAULT", false);

// ---------- 代理配置 ----------
// 不需要代理时把 PROXY_ENABLED 设为 false 即可。
pub(crate) const PROXY_ENABLED: bool = env_bool!("PROXY_ENABLED", false);
// 【重要】自实现本地反向代理（HTTP+WS 统一注入 Basic 鉴权，覆盖 WebSocket）的启用条件是：
//   PROXY_ENABLED = true 且 PROXY_HTTP / PROXY_HTTPS 均为空 ""。
// 因此下面两个必须保持空串作为默认；只有你【确实要用外部 HTTP 代理】时才填。
pub(crate) const PROXY_HTTP: &str = env_str!("PROXY_HTTP", "");
// HTTPS 代理地址；留空 "" 表示和 HTTP 代理用同一个（同样：留空即走自实现代理）。
pub(crate) const PROXY_HTTPS: &str = env_str!("PROXY_HTTPS", "");
// 不走代理的地址（分号分隔），可留空 ""
pub(crate) const PROXY_BYPASS: &str = env_str!("PROXY_BYPASS", "localhost;127.0.0.1;<-loopback>");

// ---------- 多页签（tab）模式（仅 Windows）----------
// true  = 启用多页签：一个宿主窗口 + 顶部标签栏，每个 tab 是独立 WebView（同账号共享数据目录），
//         可随时新建/切换/关闭；target="_blank" / window.open() 直接新开一个 tab。
// false = 单窗口（默认），行为与历史版本一致。
// 该能力仅在 Windows 实现（依赖多 WebviewWindow 叠放 + WebView2 共享数据目录），
// 移动端按 target_os 剔除，本常量在移动端也一并保留但由 cfg 门控引用，避免 dead_code 警告。
pub(crate) const TABS_ENABLED: bool = env_bool!("TABS_ENABLED", false);

// ---------- 全局快捷键（按下切换 显示/隐藏 窗口，仅桌面端）----------
// 用下面三个开关组合修饰键，再指定一个主键。
#[cfg(not(target_os = "android"))]
pub(crate) const HOTKEY_CTRL: bool = false;
#[cfg(not(target_os = "android"))]
pub(crate) const HOTKEY_ALT: bool = true;
#[cfg(not(target_os = "android"))]
pub(crate) const HOTKEY_SHIFT: bool = false;
#[cfg(not(target_os = "android"))]
pub(crate) const HOTKEY_KEY: tauri_plugin_global_shortcut::Code =
    tauri_plugin_global_shortcut::Code::Digit4; // 数字键 4，配合 Alt → Alt+4

/// 根据编译时写死的常量生成 Chromium 代理启动参数；未启用则返回 None（仅 Windows WebView2 使用）。
#[cfg(windows)]
pub(crate) fn build_proxy_args() -> Option<String> {
    if !PROXY_ENABLED {
        return None;
    }
    let mut http = if PROXY_HTTP.trim().is_empty() {
        None
    } else {
        Some(PROXY_HTTP.trim().to_string())
    };
    let mut https = if PROXY_HTTPS.trim().is_empty() {
        None
    } else {
        Some(PROXY_HTTPS.trim().to_string())
    };
    // 只填了一个时，另一个也用同一个代理
    if http.is_some() && https.is_none() {
        https = http.clone();
    }
    if https.is_some() && http.is_none() {
        http = https.clone();
    }
    let mut servers = Vec::new();
    if let Some(h) = &http {
        servers.push(format!("http={}", h));
    }
    if let Some(h) = &https {
        servers.push(format!("https={}", h));
    }
    if servers.is_empty() {
        return None;
    }
    let mut args = format!("--proxy-server={}", servers.join(";"));
    if !PROXY_BYPASS.trim().is_empty() {
        args.push_str(&format!(" --proxy-bypass-list={}", PROXY_BYPASS.trim()));
    }
    Some(args)
}
