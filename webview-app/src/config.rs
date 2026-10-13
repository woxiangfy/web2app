//! 编译时配置：环境变量注入的常量与宏。
//!
//! 这些常量在编译期读取环境变量（`option_env!`），只需在 `cargo build` 前设置对应环境变量、
//! 或 CI 的 `env:` 中注入，即可生成不同配置的应用，无需修改源码。
//! 未设置 / 为空时回落到默认值，保证任何环境都能直接编译。
//!
//! 可注入的变量：
//!   APP_URL          要嵌入的网址。两种形态：普通 URL（支持 https://user:pass@host 形式），
//!                    或【域名数组】的 JSON 文本（如 `["a.com","b.com"]`）——后者运行时做
//!                    DNS TXT 查询 + 验签解密得到真实 URL
//!   APP_NAME         【安装名】Windows 安装包名 / 数据目录名 / 设置目录名 / Android 包名（派生）
//!   TITLE            【显示名】窗口标题 / 托盘提示 / 设置页标题（缺省等于 APP_NAME）
//!                    字段名与打包配置 JSON 的 `title` 一致（旧名 WINDOW_TITLE 已移除）
//!   DATA_DIR         数据目录。"" = 系统默认程序目录（%LOCALAPPDATA%\<app_name>，不受程序位置影响）；
//!                    其余按路径处理，相对路径以【exe 所在目录】为基准（默认 "data"）
//!   PROXY_ENABLED    true/false 是否启用本地反向代理
//!   PROXY_HTTP       外部 HTTP 代理地址（留空=走自身代理）；仅 Windows
//!   PROXY_HTTPS      外部 HTTPS 代理地址（留空=同 HTTP）
//!   PROXY_BYPASS     不走代理的地址（分号分隔）
//!   TABS_ENABLED     true/false 是否启用多页签（tab）模式（仅 Windows）
//!   MOBILE_BAR       true/false 是否启用移动端底部导航栏（仅移动端；桌面目标无此常量）
//!   PUBLIC_KEY       Ed25519 公钥（APP_URL 为域名数组时必填）
//!   EXTRA            注入给页面的 JSON 文本（→ window.web2appExtra）
//!   DEBUG            true/false 诊断日志开关（默认 false：不落盘、也不打诊断）
//!
//! ⚠️ 这些变量的**来源**统一是打包配置 JSON，解析实现只有一份：
//!    `.github/scripts/parse-packaging-config.py`（字段表与多端行为见 docs/packaging-config.md）。
//! 例：APP_URL="https://u:p@example.com" APP_NAME="MyApp" cargo build --release

use std::path::PathBuf;
use tauri::{AppHandle, Manager};

// const 上下文可用的 ASCII 字母小写折叠（`u8::to_ascii_lowercase` 在 const fn 里不可用）。
const fn ascii_lower(b: u8) -> u8 {
    if b >= b'A' && b <= b'Z' {
        b + (b'a' - b'A')
    } else {
        b
    }
}

// const 上下文可用的 ASCII 大小写不敏感字符串相等判断：`str` 的 `==` 与 `match`
// 在常量求值中不可用（`PartialEq` 尚未稳定为 const trait，见 issue #143874），
// 这里用字节逐个比较实现。
const fn str_eq_ci(a: &str, b: &str) -> bool {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    if ab.len() != bb.len() {
        return false;
    }
    let mut i = 0;
    while i < ab.len() {
        if ascii_lower(ab[i]) != ascii_lower(bb[i]) {
            return false;
        }
        i += 1;
    }
    true
}

// 编译期布尔环境变量解析：`true/1/yes` => true，`false/0/no` => false（均 ASCII 大小写不敏感）；
// None / 空串 / 无法识别 => 默认值。抽成 const fn 是为了能直接单测（宏无法被单测调用）。
//
// ⚠️ 必须大小写不敏感：打包配置 JSON 的两个解析器（CI 的 Python 与本地 Node 复刻）都按
//    `str(bool)` 序列化布尔，注入的是 `TABS_ENABLED=True` / `PROXY_ENABLED=False`（Python
//    风格首字母大写，见 docs/packaging-config.md §3.3）。这里若只认小写 "true"，"True" 会
//    静默落入默认值 —— 多页签 / 代理 / 诊断开关会全部静默失效（曾实测：tabs_enabled=true
//    打出来的 Tauri 包仍是单窗口）。
//
// 白名单匹配走 const 循环助手 `any_ci`（const fn 里不能用 `match` 比 `str`，也不能直接
// `.iter().any()`），同时避免一串 then 块完全相同的 if-else（clippy::if_same_then_else）。
const fn parse_env_bool(value: Option<&str>, default: bool) -> bool {
    match value {
        None => default,
        Some(s) => {
            if s.is_empty() {
                default
            } else if any_ci(s, &["true", "1", "yes"]) {
                true
            } else if any_ci(s, &["false", "0", "no"]) {
                false
            } else {
                default
            }
        }
    }
}

// const 上下文可用的「s 与候选列表任一项大小写不敏感相等」。
const fn any_ci(s: &str, candidates: &[&str]) -> bool {
    let mut i = 0;
    while i < candidates.len() {
        if str_eq_ci(s, candidates[i]) {
            return true;
        }
        i += 1;
    }
    false
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

// 布尔型注入宏：true/1/yes => true；false/0/no => false（ASCII 大小写不敏感）；
// 未设置 / 空串 / 其余取值用默认值。解析逻辑在 const fn `parse_env_bool`（可单测）。
macro_rules! env_bool {
    ($name:literal, $default:expr) => {
        parse_env_bool(option_env!($name), $default)
    };
}

// 把名字清洗为 Windows 合法的文件/目录名：替换路径非法字符 < > : " / \ | ? * 与控制字符
// （0x00–0x1F）为下划线；`collapse_ws` 为 true 时额外把连续空白压缩为单个空格。
//
// 这里【只做】字符替换与空白压缩，不做首尾 trim 与默认值回落——那部分两个调用方语义不同
// （目录名 vs 下载文件名），由各自保留。与 `webview_hooks::sanitize_filename` 共用，消除两处
// 「Windows 非法字符→下划线」的重复实现。
pub(crate) fn sanitize_windows_name(name: &str, collapse_ws: bool) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_ws = false;
    for c in name.chars() {
        let mapped = match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 32 => '_',
            c => c,
        };
        if collapse_ws && mapped.is_whitespace() {
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
    out
}

// 把应用名清洗为合法的目录名（Windows/Linux/macOS 通用）：
//   - 替换路径非法字符 < > : " / \ | ? * 与控制字符为下划线
//   - 压缩连续空白为单个空格、去首尾空白与首尾句点
//   - 清洗后为空则回落 "Web2App"
// 用于让 Windows 的 WebView2 数据目录 / 设置目录以自定义 app_name 命名，
// 而非固定的 tauri.conf.json identifier（com.example.web2app）。
pub(crate) fn sanitize_dir_name(name: &str) -> String {
    let cleaned = sanitize_windows_name(name, true);
    let trimmed = cleaned.trim().trim_matches('.').to_string();
    if trimmed.is_empty() {
        "Web2App".to_string()
    } else {
        trimmed
    }
}

/// 配置根目录：桌面端取 `app_config_dir` 的父目录、Android 用自身 `dataDir`，再拼 `<app_name>`。
///
/// 桌面端的 `app_config_dir` 末段是 `tauri.conf.json` 的 identifier，必须取父目录后再拼自定义
/// app_name，让不同应用的配置互相隔离；Android 的 `app_config_dir` 直接就是应用私有根目录
/// （`/data/data/<package>`），**绝不能取 parent**（否则跳到 `/data/data`，既不可写又与其他包冲突）。
///
/// ⚠️ 本目录现在**只用来读旧位置的 `settings.json`**（升级兼容，见 [`crate::store`]）：
/// 2026-09-23 起运行期文档落在 [`store_dir`]（= 数据目录），不再落在这里。
pub(crate) fn config_dir(app: &AppHandle) -> Option<PathBuf> {
    let base = app.path().app_config_dir().ok()?;
    #[cfg(desktop)]
    let root = base.parent().unwrap_or(&base).to_path_buf();
    #[cfg(target_os = "android")]
    let root = base;
    Some(root.join(sanitize_dir_name(APP_NAME)))
}

/// 运行期文档（`settings.json`，内含窗口几何）所在目录 —— **与数据目录同一套规则**。
///
/// Windows：跟 [`data_dir`] 走，即由打包配置的 `data_dir` 字段决定
/// （`""` = 系统默认程序目录、相对路径 = exe 同级、绝对路径原样、不可写自动回退；
/// 见 `docs/packaging-config.md` §4.3）。这样「绿色版把整个文件夹搬走」成立：
/// 设置、几何、cookie、缓存**一起**跟着程序走，也与 Electron 绿色版
/// （`electron-app/src/main/settings.ts` 的 `storeLocation`）落在同一个文件上。
/// ⚠️ 反过来看：`data_dir` 一改，设置文件的位置**必须**跟着改 —— 它是「程序的数据」，
/// 不是「系统的配置」，这是使用者 2026-09-23 明确指出的行为一致性要求。
///
/// 非 Windows（Android）：没有 WebView2 数据目录这个概念，仍用应用私有配置目录
/// （`/data/data/<pkg>/<app_name>/settings.json`，见 `webview-app/AGENTS.md` §移动端）。
#[cfg(windows)]
pub(crate) fn store_dir(app: &AppHandle) -> Option<PathBuf> {
    data_dir(app)
}

/// 非 Windows：见 Windows 版的说明（Android 用应用私有目录）。
#[cfg(not(windows))]
pub(crate) fn store_dir(app: &AppHandle) -> Option<PathBuf> {
    config_dir(app)
}

/// WebView2 用户数据目录：`%LOCALAPPDATA%\<app_name>`（仅 Windows）。
///
/// 即 `DATA_DIR` 为空串（或配置的目录不可写）时使用的系统默认程序目录 —— 它与程序位置无关，
/// 始终可写，适合安装版与多用户场景。
/// `app_local_data_dir` 在极少数环境可能返回 Err，此时返回 None、由调用方退回系统默认目录。
#[cfg(windows)]
pub(crate) fn user_data_dir(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_local_data_dir().ok()?;
    let base = dir.parent().unwrap_or(&dir);
    Some(base.join(sanitize_dir_name(APP_NAME)))
}

/// 程序**自身所在目录**（exe 所在目录）。
///
/// ⚠️ 不能用 `std::env::current_dir()`：进程工作目录会随启动方式变化（快捷方式的「起始位置」、
/// 命令行 `cd` 后启动、从资源管理器拖拽/另存启动），同一份包在不同启动方式下会把数据落到
/// 不同地方 —— 用户表现为「设置丢了 / 又要重新登录」。「数据跟着程序走」必须以 exe 目录为基准。
#[cfg(windows)]
fn program_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.parent().map(|p| p.to_path_buf())
}

/// 可写性探测：真建目录 + 真写一个文件再删掉。
///
/// 只调 `create_dir_all` 是不够的：写保护 U 盘、只读网络盘、企业策略下的 `Program Files`，
/// 都可能「建目录成功但写文件失败」。Electron 侧用同一判据（`electron-app/src/main/portable.ts`）。
#[cfg(windows)]
fn is_writable(dir: &std::path::Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".web2app-write-probe");
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// 解析 `DATA_DIR` 得到实际使用的数据目录（仅 Windows）。
///
/// 规则（与打包配置契约一致，见 `docs/packaging-config.md` §4.3）：
///   · `""`    → 系统默认程序目录 `%LOCALAPPDATA%\<app_name>`，不受程序位置影响；
///   · 绝对路径 → 原样使用；
///   · 相对路径 → 程序所在目录（exe 目录）下的该路径（默认 `"data"`）。
///
/// ⚠️⚠️ 进程内**只解析一次并缓存**，两个理由：
///   1. 多页签模式下每个 tab 都会调用它。各算各的一旦结果不同，tab 之间就不再共享
///      cookie/localStorage —— 那正是多页签赖以成立的硬契约；
///   2. 可写探测要真写文件，没必要每建一个窗口就跑一次。
#[cfg(windows)]
pub(crate) fn data_dir(app: &AppHandle) -> Option<PathBuf> {
    static CACHE: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| resolve_data_dir(app)).clone()
}

#[cfg(windows)]
fn resolve_data_dir(app: &AppHandle) -> Option<PathBuf> {
    // 空串（= 系统默认目录）或拿不到 exe 目录时，都退回系统默认程序目录。
    let Some(candidate) = candidate_data_dir(DATA_DIR, program_dir().as_deref()) else {
        return user_data_dir(app);
    };

    if is_writable(&candidate) {
        return Some(candidate);
    }

    // 只读介质（写保护 U 盘 / Program Files / 只读网络盘）：退到系统默认目录，
    // 保证「能启动」优先于「够绿色」——否则 WebView2 初始化失败会让窗口**直接打不开**。
    let fallback = user_data_dir(app);
    match &fallback {
        Some(dir) => dlog!(
            "[config] 数据目录不可写，已回退系统默认目录：{} → {}",
            candidate.display(),
            dir.display()
        ),
        None => dlog!(
            "[config] 数据目录不可写且系统默认目录不可用：{}（改用 WebView2 自身默认位置）",
            candidate.display()
        ),
    }
    fallback
}

/// `DATA_DIR` 的**纯逻辑**解析（不依赖 `AppHandle`，因此可单测）：
/// 把配置值算成一个「候选目录」，是否需要回退由调用方再探测。
///
/// - 空串 / 纯空白 → `None`（调用方改用系统默认程序目录，由 `app_name` 命名）；
/// - 绝对路径       → 原样返回；
/// - 相对路径       → 以 `program_dir`（exe 所在目录）为基准拼接；基准拿不到时 `None`。
///
/// ⚠️ 基准只能是 exe 目录，**不能**用 `std::env::current_dir()`：见 `program_dir()` 的说明。
#[cfg(windows)]
fn candidate_data_dir(configured: &str, program_dir: Option<&std::path::Path>) -> Option<PathBuf> {
    let configured = configured.trim();
    if configured.is_empty() {
        return None;
    }
    let wanted = PathBuf::from(configured);
    if wanted.is_absolute() {
        return Some(wanted);
    }
    Some(program_dir?.join(wanted))
}

// 默认打开的网址。两种形态（打包配置 JSON 里分别是 `string` 与 `string[]`，
// 到编译期都收敛成一段文本，由 `resolve::parse_domains` 判别）：
//   ① 普通 URL：
//      "https://用户名:密码@www.example.com"
//      注意：现代 Chromium / WebView2 已废弃"URL 内嵌 userinfo 自动发 Basic 鉴权"的行为，
//      不会自动发送。程序会把账号密码解析出来，通过本地反向代理给所有 HTTP/WebSocket 请求统一
//      注入 `Authorization: Basic ...` 头，再转发到真实服务器。
//   ② 域名数组（JSON 数组文本，如 `["a.com","b.com"]`）：
//      运行时依次对这些域名做 DNS TXT 查询，用 PUBLIC_KEY 验签 + AES-GCM 解密 TXT 值得到真实 URL。
pub(crate) const APP_URL: &str = env_str!("APP_URL", "https://用户名:密码@www.example.com");

// Ed25519 公钥（PEM/SPKI 或裸 base64，32 字节原始公钥）。当 APP_URL 是【域名数组】时
// （如 `["a.com","b.com"]`），运行时依次对这些域名做 DNS TXT 查询，用本公钥验签 +
// AES-GCM 解密 TXT 值得到真实 URL。普通字符串 URL 模式下忽略本字段。
pub(crate) const PUBLIC_KEY: &str = env_str!("PUBLIC_KEY", "");

// 应用名称（**安装名**）：用于数据目录名 / 设置目录名 / 单实例标识。
// ⚠️ 语义边界：这里的 APP_NAME **不再**是界面显示名 —— 显示名见下方 TITLE。
//    两者分开是为了支持「安装名用英文、界面显示中文」。
pub(crate) const APP_NAME: &str = env_str!("APP_NAME", "Web2App");
// 显示名（**给用户看的名字**）：窗口标题、托盘悬停提示、设置窗标题。
//
// ⚠️ 编译期变量名与打包配置 JSON 的字段名**同为 `title`**（环境变量即 TITLE）——
//    历史上这里叫 WINDOW_TITLE、JSON 里还并存过一个已废弃的 `window_title`，
//    同一个概念有三个名字，改一处必漏另一处。现在只保留这一个。
// ⚠️ 取值链只有一跳：JSON 的 title 为空 ⇒ 在打包期就被填成 app_name（见解析器），
//    这里再回落一次 APP_NAME 只是「不依赖上游也自洽」的兜底。
pub(crate) const TITLE: &str = env_str!("TITLE", APP_NAME);

// 额外参数（编译期注入的 JSON 文本，默认 "{}"）。由手动打包 JSON 的 `extra` 字段提供，
// 会以 `window.web2appExtra = {...}` 注入到被包裹网页，前端 JS 可直接读取。
// 必须是合法 JSON（对象/数组/标量皆可），注入时原样内联为 JS 表达式。
pub(crate) const EXTRA: &str = env_str!("EXTRA", "{}");

/// 返回注入到页面的 EXTRA JSON 文本；若 EXTRA 不是合法 JSON（打包时手误配置），回退为 "{}"，
/// 避免 `window.web2appExtra = <非法 JSON>;` 因语法错误破坏宿主页面脚本。
///
/// EXTRA 是编译期常量，本函数在每次建窗 / 建 tab 时各调用一次（serde_json 解析开销可忽略），
/// 起到「注入前校验」的兜底作用。
pub(crate) fn extra_json() -> &'static str {
    if serde_json::from_str::<serde_json::Value>(EXTRA).is_ok() {
        EXTRA
    } else {
        dlog!("[config] EXTRA 不是合法 JSON，已回退为 {{}} 注入");
        "{}"
    }
}

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
// 空串 = 系统默认程序目录（%LOCALAPPDATA%\<清洗后的 app_name>），不受程序位置影响、始终可写；
// 其余 = 目录路径：相对路径以【exe 所在目录】为基准（默认 "data" ⇒ <exe 目录>\data\，跟着程序走），
//        绝对路径原样使用。路径解析与「不可写自动回退」见下方 `data_dir()`。
//
// ⚠️⚠️ 这里**不能**用 `env_str!`：那个宏把空串当成「未设置」并回落到默认值，而空串在本字段里
//    是**有含义的取值**（= 用系统默认目录）。必须用裸 `option_env!` 原样保留空串。
//    它与打包配置契约绑定：解析器把 `"data_dir": ""` 写成一行 `DATA_DIR=`（值就是空）。
pub(crate) const DATA_DIR: &str = match option_env!("DATA_DIR") {
    Some(s) => s,
    None => "data",
};

// ---------- 代理配置 ----------
// 不需要代理时把 PROXY_ENABLED 设为 false 即可。
pub(crate) const PROXY_ENABLED: bool = env_bool!("PROXY_ENABLED", false);
// 【重要】自实现本地反向代理（HTTP+WS 统一注入 Basic 鉴权，覆盖 WebSocket）的启用条件是：
//   PROXY_ENABLED = true 且 PROXY_HTTP / PROXY_HTTPS 均为空 ""。
// 因此下面两个必须保持空串作为默认；只有你【确实要用外部 HTTP 代理】时才填。
pub(crate) const PROXY_HTTP: &str = env_str!("PROXY_HTTP", "");
// HTTPS 代理地址；留空 "" 表示和 HTTP 代理用同一个（同样：留空即走自实现代理）。
pub(crate) const PROXY_HTTPS: &str = env_str!("PROXY_HTTPS", "");
// 不走代理的地址（分号分隔）。默认值与打包配置 JSON 的默认值、
// 以及 Electron 绿色版 config.ts::DEFAULTS 三处**必须一致**（同一个串）。
// 留空 = 不下发 --proxy-bypass-list（即沿用 Chromium 自身默认）。
pub(crate) const PROXY_BYPASS: &str = env_str!("PROXY_BYPASS", "localhost;127.0.0.1;<-loopback>");

// ---------- 多页签（tab）模式（仅 Windows）----------
// true  = 启用多页签：一个宿主窗口 + 顶部标签栏，每个 tab 是独立 WebView（同账号共享数据目录），
//         可随时新建/切换/关闭；target="_blank" / window.open() 直接新开一个 tab。
// false = 单窗口（默认），行为与历史版本一致。
// 该能力仅在 Windows 实现（依赖多 WebviewWindow 叠放 + WebView2 共享数据目录），移动端
// **显式忽略**：`cfg!(windows) &&` 让非 Windows 恒为 false —— 即使打包链路误注入
// `TABS_ENABLED=true` 也不会在 Android 上启用多页签（`mod tabs` 本身也是 `#[cfg(windows)]`）。
// 保留常量定义只为让 `#[cfg(windows)]` 分支的引用有处可依，且避免非 Windows 下的 dead_code 警告。
pub(crate) const TABS_ENABLED: bool = cfg!(windows) && env_bool!("TABS_ENABLED", false);

// ---------- 移动端底栏（mobile_bar，仅移动端）----------
// true  = 屏幕底部常驻一条导航条：**首页**（回到 app_url）/ 后退 / 前进 / 列表。
//         其中「列表」按钮只在**当前构建真的有页签可切**时才渲染（判定见
//         `mobilebar::tabs_available`，下发到页面后由 `mobilebar.js` 决定是否画它）——
//         否则会出现一个点了没有任何反应的死按钮。
// false = 移动端不画任何壳自绘控件（默认），与历史行为一致。
//
// ⚠️ 常量整体 `#[cfg(mobile)]`（而不是像 TABS_ENABLED 那样用 `cfg!(windows) &&` 把值压成
//    false）：本模式只在移动端实现，桌面目标上**连这个常量都不存在** —— 「打包时配了
//    mobile_bar=true，桌面端却静默长出一条底栏」这类「配了但不该生效」的情形因此不可能发生。
//    （TABS_ENABLED 在桌面端仍被引用，才需要保留常量并把值压成 false。）
#[cfg(mobile)]
pub(crate) const MOBILE_BAR: bool = env_bool!("MOBILE_BAR", false);

// ---------- 诊断日志开关 ----------
// true  = 每次启动写 `<数据目录>\logs\startup.log`，并把 `dlog!` 的诊断同时打到 stderr；
// false = 不落盘、也不输出任何诊断（默认）。
//
// ⚠️ 默认必须是 false：默认开启会在用户的数据目录里留下交付物以外的文件，那不是期望行为。
// ⚠️ 它是**开关**而不是「日志级别」：关闭时没有任何第一现场（用户报「双击没反应」时手上无证据），
//    因此留了运行时逃生口 —— 命令行 `--debug` 或环境变量 `WEB2APP_DEBUG=1`（见 debug_log 模块），
//    不必为了排障重新编译。
// ⚠️ 环境变量名就叫 DEBUG（与打包配置 JSON 的字段名 `debug` 同名，见
//    `.github/scripts/parse-packaging-config.py` 的 write_env）。它是个常见名字，
//    若开发机的环境里恰好存在 `DEBUG=1`，本地 release 构建会意外带上日志 —— 属于
//    「多一份日志、不影响功能」的偏差，可接受，不值得为它把契约名搞特殊。
pub(crate) const DEBUG: bool = env_bool!("DEBUG", false);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn str_eq_ci_matches_ignoring_ascii_case() {
        assert!(str_eq_ci("abc", "abc"));
        assert!(str_eq_ci("True", "true"));
        assert!(str_eq_ci("TRUE", "true"));
        assert!(str_eq_ci("YES", "yes"));
        // 非 ASCII 字母不折叠（白名单里也没有这类值），长度 / 内容不同仍为不等
        assert!(!str_eq_ci("abc", "abd"));
        assert!(!str_eq_ci("abc", "ab"));
        assert!(!str_eq_ci("", "a"));
        assert!(str_eq_ci("", ""));
    }

    // ---------- parse_env_bool ----------
    // 这是打包配置 JSON → 环境变量 → 编译期常量这条链上最容易静默断裂的一环：
    // 两个解析器（CI Python / 本地 Node 复刻）注入的都是 Python 风格 "True"/"False"。
    #[test]
    fn parse_env_bool_accepts_python_style_capitals() {
        assert!(parse_env_bool(Some("True"), false));
        assert!(!parse_env_bool(Some("False"), true));
        assert!(parse_env_bool(Some("TRUE"), false));
        assert!(!parse_env_bool(Some("FALSE"), true));
    }

    #[test]
    fn parse_env_bool_whitelist_is_case_insensitive() {
        for v in ["true", "TRUE", "True", "tRuE", "1", "yes", "Yes", "YES"] {
            assert!(parse_env_bool(Some(v), false), "{v} 应判真");
        }
        for v in ["false", "FALSE", "False", "0", "no", "No", "NO"] {
            // 默认值给 true：确认是「认出假值」而非「落进默认值」
            assert!(
                !parse_env_bool(Some(v), true),
                "{v} 应判假（即使默认值为 true）"
            );
        }
    }

    #[test]
    fn parse_env_bool_missing_empty_or_unknown_falls_back() {
        // None / 空串 → 默认值（env_str! 与 write_env 都把空串当「未设置」）
        assert!(!parse_env_bool(None, false));
        assert!(parse_env_bool(None, true));
        assert!(!parse_env_bool(Some(""), false));
        assert!(parse_env_bool(Some(""), true));
        // "on" 刻意不认：两壳曾对 "on" 判定相反（见 docs/packaging-config.md §3.3），只能回落默认值
        assert!(!parse_env_bool(Some("on"), false));
        assert!(parse_env_bool(Some("on"), true));
        // 其他无法识别的值同样回落，绝不猜真值
        assert!(!parse_env_bool(Some("garbage"), false));
        assert!(parse_env_bool(Some("2"), true));
    }

    #[test]
    fn sanitize_dir_name_replaces_illegal_chars() {
        assert_eq!(
            sanitize_dir_name(r#"a<b>c:d"e/f\g|h?i*j"#),
            "a_b_c_d_e_f_g_h_i_j"
        );
    }

    #[test]
    fn sanitize_dir_name_collapses_whitespace() {
        // 空格/全角空白压缩为单个空格，且去首尾。
        assert_eq!(sanitize_dir_name("  My   App  "), "My App");
        // 控制字符（tab/换行 < 32）走"替换为下划线"分支，不参与空白压缩。
        assert_eq!(sanitize_dir_name("a\tb\nc"), "a_b_c");
    }

    #[test]
    fn sanitize_dir_name_trims_dots_and_falls_back() {
        assert_eq!(sanitize_dir_name("..App.."), "App");
        // 非法字符映射为下划线后非空 → 不回退默认名（只有结果为空才回退）。
        assert_eq!(sanitize_dir_name("***"), "___");
        // 全部是空白 → 压缩后为空 → 回落默认名
        assert_eq!(sanitize_dir_name("   "), "Web2App");
        assert_eq!(sanitize_dir_name(""), "Web2App");
    }

    #[test]
    fn sanitize_dir_name_keeps_unicode() {
        assert_eq!(sanitize_dir_name("我的应用"), "我的应用");
        assert_eq!(sanitize_dir_name("我的/应用"), "我的_应用");
    }

    #[test]
    fn extra_is_valid_json() {
        // EXTRA 会原样内联为 `window.web2appExtra = <EXTRA>;`，必须是合法 JSON，否则宿主页面脚本报错。
        assert!(
            serde_json::from_str::<serde_json::Value>(EXTRA).is_ok(),
            "EXTRA 必须是合法 JSON"
        );
        // 合法默认值下 extra_json() 应原样返回，不回退。
        assert_eq!(extra_json(), EXTRA);
    }

    // build_proxy_args 依赖编译期常量 PROXY_ENABLED（默认 false），
    // 单测只能验证「未启用 → None」分支；启用分支由 CI 注入 env 编译验证。
    #[cfg(windows)]
    #[test]
    fn build_proxy_args_none_when_disabled() {
        assert!(build_proxy_args().is_none());
    }

    // ---------- data_dir ----------
    // 这些是「数据目录」契约里最容易错的部分：空串有含义、相对路径的基准是 exe 目录而非 cwd。
    // 依赖 AppHandle 的部分（系统默认目录）无法在单测里构造，由探针 / 真机验证。

    #[cfg(windows)]
    #[test]
    fn candidate_data_dir_empty_means_system_default() {
        let base = std::path::Path::new(r"C:\app");
        // 空串 ⇒ None（调用方改用系统默认程序目录），绝不能拼出一个叫 "" 的路径
        assert_eq!(candidate_data_dir("", Some(base)), None);
        // 纯空白视同空串
        assert_eq!(candidate_data_dir("   ", Some(base)), None);
    }

    #[cfg(windows)]
    #[test]
    fn candidate_data_dir_relative_uses_program_dir_not_cwd() {
        let base = std::path::Path::new(r"C:\app");
        assert_eq!(
            candidate_data_dir("data", Some(base)),
            Some(PathBuf::from(r"C:\app\data"))
        );
        assert_eq!(
            candidate_data_dir("sub/mydata", Some(base)),
            Some(PathBuf::from(r"C:\app\sub\mydata"))
        );
        // 首尾空白会被去掉（配置里手抖多打的空格不该变成一个带空格的目录名）
        assert_eq!(
            candidate_data_dir("  data  ", Some(base)),
            Some(PathBuf::from(r"C:\app\data"))
        );
        // 拿不到 exe 目录 ⇒ None（由调用方退回系统默认目录），不 panic
        assert_eq!(candidate_data_dir("data", None), None);
    }

    #[cfg(windows)]
    #[test]
    fn candidate_data_dir_absolute_passthrough() {
        let base = std::path::Path::new(r"C:\app");
        assert_eq!(
            candidate_data_dir(r"D:\appdata", Some(base)),
            Some(PathBuf::from(r"D:\appdata"))
        );
        // 绝对路径不依赖基准目录，基准缺失时依然可用
        assert_eq!(
            candidate_data_dir(r"D:\appdata", None),
            Some(PathBuf::from(r"D:\appdata"))
        );
        // 绝对路径不会被 base 污染（曾担心 join 会把它当相对路径拼在后面）
        assert_eq!(
            candidate_data_dir(r"D:\appdata", Some(base)),
            candidate_data_dir(r"D:\appdata", None)
        );
    }

    #[cfg(windows)]
    #[test]
    fn candidate_data_dir_default_is_relative_to_program() {
        // 编译期默认值是 "data" ⇒ 默认行为就是「数据跟着程序走」
        assert_eq!(DATA_DIR, "data");
        assert_eq!(
            candidate_data_dir(DATA_DIR, Some(std::path::Path::new(r"C:\app"))),
            Some(PathBuf::from(r"C:\app\data"))
        );
    }

    #[cfg(windows)]
    #[test]
    fn is_writable_accepts_fresh_dir_and_leaves_no_litter() {
        let dir = std::env::temp_dir().join("w2a-ut-writable");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(is_writable(&dir));
        // 探针文件必须被清掉，不能在用户数据目录里留垃圾
        assert!(!dir.join(".web2app-write-probe").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn is_writable_rejects_a_file_at_that_path() {
        // 「目标路径上已存在一个同名文件」时建目录会失败 ⇒ 必须判定不可写，而不是 panic。
        let path = std::env::temp_dir().join("w2a-ut-not-a-dir");
        let _ = std::fs::remove_dir_all(&path);
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, b"x").expect("写探针文件");
        assert!(!is_writable(&path));
        let _ = std::fs::remove_file(&path);
    }
}
