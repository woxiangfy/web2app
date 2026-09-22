//! 诊断日志（**默认关闭**）：开启时写 `<数据目录>\logs\startup.log`，并同时打 stderr。
//!
//! ## 为什么要有它
//!
//! 本程序在 Windows 上是 **GUI 子系统**程序（见 `main.rs` 的 `windows_subsystem = "windows"`），
//! 双击运行时没有控制台可挂，`eprintln!` 的输出**没有任何去处**。于是「双击没反应 /
//! 打开的是 example.com / 标题不对」这类反馈在用户侧**零证据**，只能远程猜。
//! 落一份 `logs\startup.log` 之后，用户发一条日志就能定位 —— 与 Electron 侧
//! （`electron-app/src/main/startup-log.ts`）**同一份契约**：同名文件、同目录约定、同轮转规则。
//!
//! ## ⚠️⚠️ 但它是可选的（`config.json` 的 `debug`，默认 false）
//!
//! 关闭时本模块什么都不做：不建 `logs\` 目录、不落文件，`dlog!` 也直接短路
//! （**连 `format!` 都不做**，见宏定义）。理由：每次启动都在用户数据目录里留一份日志不是
//! 期望行为。代价是上面那条「用户侧第一现场」在默认配置下**不存在**，所以留了运行时逃生口：
//! `web2app.exe --debug` 或环境变量 `WEB2APP_DEBUG=1` —— 交付出去的包也能临时取证，不必重编译。
//!
//! ## 设计取舍
//!
//!   1. **零外部依赖**：只用 std（本仓库不引 `log` / `env_logger`）。时间戳自己格式化成
//!      ISO-8601 UTC（与 Electron 侧 `new Date().toISOString()` 同形）。
//!   2. **fail-open**：目录建不了 / 文件写不进去**绝不影响启动**。为了记日志让应用起不来
//!      是本末倒置；写失败只是这次静默跳过。
//!   3. **同步追加**：每个进程内只打开一次文件句柄，逐行 `write_all`。异常退出（任务管理器
//!      结束 / 崩溃）时崩溃前最后几行已经落盘 —— 那正是最需要的东西。
//!   4. **绝不记凭据**：地址一律经 [`mask_url`] 去掉 `user:pass@`；Secret 只报「已配置 / 未配置」。
//!
//! ## 与 Electron 侧的唯一差别
//!
//! Electron 的配置是**运行期**读 `config.json`，Tauri 的配置是**编译期**常量
//! （改配置必须重新编译，见 `config.rs`）⇒ 这里没有「配置文件路径」可报，只报生效值。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::AppHandle;

use crate::config::DEBUG;

/// 日志文件名。⚠️ 与 Electron 侧 `startup-log.ts::LOG_FILE_NAME` 必须**逐字相同**。
pub(crate) const LOG_FILE_NAME: &str = "startup.log";
/// 轮转后的旧文件（只保留一代，启动日志不需要归档体系）。
pub(crate) const PREV_LOG_FILE_NAME: &str = "startup.prev.log";
/// 超过这个大小就在**启动时**轮转一次（运行期不检查，避免每行都 stat）。
/// ⚠️ 与 Electron 侧 `startup-log.ts::MAX_BYTES` 同值。
const MAX_BYTES: u64 = 1024 * 1024;

/// 落盘目标（`None` = 未开启 / 目录不可用 / 文件打不开 ⇒ 只打 stderr）。
static SINK: OnceLock<Option<Sink>> = OnceLock::new();

/// 已打开的日志文件。
///
/// `Mutex` 串行化写入：解析 / 代理 / 钩子各在自己的线程里调 `dlog!`，不串行化会出现
/// 交错的半行（Windows 上追加写并不保证「一次 write_all 不可分割」）。
struct Sink {
    path: PathBuf,
    file: Mutex<std::fs::File>,
}

/// 日志文件路径（未开启或不可写时 `None`）—— 供启动信息与排障文案使用。
pub(crate) fn log_file_path() -> Option<PathBuf> {
    SINK.get().and_then(|s| s.as_ref()).map(|s| s.path.clone())
}

/// 诊断日志是否开启（编译期 `DEBUG`、运行时 `--debug` 或 `WEB2APP_DEBUG`）。
///
/// ⚠️ 结果缓存：`dlog!` 在热路径（代理的每一条响应）上调用，每次去读环境变量/扫 argv 是浪费。
/// 三者的优先级没有意义（任一为真即开），因为都是「用户明确要求输出日志」。
pub(crate) fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        DEBUG
            || std::env::var("WEB2APP_DEBUG")
                .map(|v| flag_truthy(&v))
                .unwrap_or(false)
            || std::env::args().any(|a| a == "--debug")
    })
}

/// 环境变量取值判断：只认 `1 / true / yes`（大小写不敏感、允许首尾空白）。
///
/// ⚠️ 白名单与 `config.rs::env_bool!` **逐字保持一致**：那里刻意不认 `"on"`，
/// 因为两壳曾对 `"on"` 判定相反（见 docs/packaging-config.md §3.3）。多认一个值 = 重开那个坑。
fn flag_truthy(v: &str) -> bool {
    let v = v.trim().to_ascii_lowercase();
    v == "1" || v == "true" || v == "yes"
}

/// 初始化（幂等）：`AppHandle` 可用后**尽早**调用，越早越能捕获启动早期的日志。
///
/// 未开启 / 目录不可用 / 文件打不开时**静默降级**（只打 stderr），绝不 panic、绝不阻断启动。
pub(crate) fn init(app: &AppHandle) {
    if !enabled() {
        let _ = SINK.set(None);
        return;
    }

    let Some(dir) = log_dir(app) else {
        let _ = SINK.set(None);
        return;
    };

    // ⚠️ `set` 只成功一次（幂等）：重复调用不会换掉已打开的句柄。
    let _ = SINK.set(open_log_file(&dir));
}

/// 日志目录：Windows 与 Electron 同为 `<数据目录>\logs`；其余平台用配置目录下的 `logs`
/// （Android 没有 WebView2 数据目录这个概念，`config_dir` 是必然可写的应用私有目录）。
fn log_dir(app: &AppHandle) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        crate::config::data_dir(app).map(|d| d.join("logs"))
    }
    #[cfg(not(windows))]
    {
        crate::config::config_dir(app).map(|d| d.join("logs"))
    }
}

/// 建目录 → 轮转 → 打开追加句柄。任一步失败都返回 `None`（调用方降级为只打 stderr）。
fn open_log_file(dir: &Path) -> Option<Sink> {
    if std::fs::create_dir_all(dir).is_err() {
        return None;
    }
    let path = dir.join(LOG_FILE_NAME);
    rotate_if_too_large(&path);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()?;
    Some(Sink {
        path,
        file: Mutex::new(file),
    })
}

/// 超过上限就把当前文件挪成 `startup.prev.log`。**失败不抛**（轮转只是防无限增长，
/// 写不进去才是真故障 —— 两者是独立的失败域，与 Electron 侧同一条约定）。
fn rotate_if_too_large(path: &Path) {
    let Ok(meta) = std::fs::metadata(path) else {
        return; // 文件不存在（首次启动）→ 无需轮转
    };
    if meta.len() < MAX_BYTES {
        return;
    }
    let prev = path.with_file_name(PREV_LOG_FILE_NAME);
    let _ = std::fs::remove_file(&prev); // 删不掉也无所谓：下面的 rename 失败就只是这次不轮转
    let _ = std::fs::rename(path, &prev);
}

/// 写一行（带时间戳）。未开启 / 无文件句柄时静默跳过。
///
/// ⚠️ 这是 [`crate::dlog!`] 的底层：**不要在热路径直接调用**，用宏短路 `format!` 的开销。
pub(crate) fn log_line(text: &str) {
    if !enabled() {
        return;
    }
    // 先打 stderr：开发态（`cargo run`）与控制台启动时立刻可见，且落盘失败也不影响它。
    eprintln!("{text}");

    let Some(Some(sink)) = SINK.get() else {
        return;
    };
    // 中毒锁：另一个线程在写日志时 panic 过。日志本身不值得让调用方也崩，取回内部引用继续写。
    let mut file = sink.file.lock().unwrap_or_else(|p| p.into_inner());
    let _ = writeln!(file, "{} {}", iso_now(), text);
}

/// 当前时间的 ISO-8601 UTC（如 `2026-09-22T10:24:35Z`，与 Electron 的 `toISOString()` 同形）。
pub(crate) fn iso_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    iso_from_unix_secs(secs)
}

/// Unix 秒 → ISO-8601 UTC（秒级精度）。**不依赖任何日期库**，纯整数运算便于单测。
fn iso_from_unix_secs(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, mo, d) = civil_from_days(days);
    let (h, mi, s) = (rem / 3_600, (rem % 3_600) / 60, rem % 60);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// 自 1970-01-01 起的天数 → `(年, 月, 日)`（Howard Hinnant 的 `civil_from_days`，
/// 全整数运算、对格里高利历闰年规则精确）。
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468; // 平移到 0000-03-01 起的「纪元日」
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 打印文本前隐去所有 `user:pass@`（URL 与 `--proxy-server=...;...` 这类拼接串都适用）。
///
/// ⚠️⚠️ 日志会随用户的截图 / 粘贴四处传播，凭据绝不能进去。判据：`://` 之后、
/// **第一个 `/` 之前**出现的 `@` 才是 userinfo（`https://h.com/a@b` 里的 `@` 属于路径）。
pub(crate) fn mask_url(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let Some(i) = rest.find("://") else {
            out.push_str(rest);
            break;
        };
        let (head, tail) = rest.split_at(i + 3);
        out.push_str(head);
        match tail.find('@') {
            Some(at) if !tail[..at].contains('/') => {
                out.push_str("***@");
                rest = &tail[at + 1..];
            }
            // 没有 `@`，或 `@` 在路径里：原样保留，但要跳过 `://` 以免死循环
            _ => rest = tail,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_truthy_matches_env_bool_whitelist() {
        for yes in ["1", "true", "TRUE", "Yes", " yes "] {
            assert!(flag_truthy(yes), "{yes} 应视为真");
        }
        // ⚠️ "on" 必须为假：config.rs 的 env_bool! 不认它，两壳曾因它判定相反。
        for no in ["", "0", "false", "no", "on", "enable", "2"] {
            assert!(!flag_truthy(no), "{no} 不应视为真");
        }
    }

    #[test]
    fn iso_formats_known_timestamps() {
        assert_eq!(iso_from_unix_secs(0), "1970-01-01T00:00:00Z");
        // 2000-02-29：闰年规则（能被 400 整除）—— 最容易写错的一天
        assert_eq!(iso_from_unix_secs(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(iso_from_unix_secs(1_700_000_000), "2023-11-14T22:13:20Z");
        // 跨日边界：23:59:59 → 次日 00:00:00
        assert_eq!(iso_from_unix_secs(86_399), "1970-01-01T23:59:59Z");
        assert_eq!(iso_from_unix_secs(86_400), "1970-01-02T00:00:00Z");
    }

    #[test]
    fn mask_url_hides_credentials_only_in_userinfo() {
        assert_eq!(mask_url("https://u:p@h.com/x"), "https://***@h.com/x");
        assert_eq!(mask_url("https://u:p@h.com"), "https://***@h.com");
        // 没有凭据：原样
        assert_eq!(mask_url("https://h.com/x"), "https://h.com/x");
        // `@` 在路径里（不是 userinfo）：原样，别把路径截掉
        assert_eq!(mask_url("https://h.com/a@b"), "https://h.com/a@b");
        // 域名数组文本形态：不是 URL，原样返回
        assert_eq!(mask_url(r#"["a.com","b.com"]"#), r#"["a.com","b.com"]"#);
        // 端口与查询串不受影响
        assert_eq!(
            mask_url("http://u@h.com:8080/p?q=1"),
            "http://***@h.com:8080/p?q=1"
        );
        // ⚠️ 一格里可能有多段（`build_proxy_args()` 拼出来的 --proxy-server 串）：全部要脱敏，
        //    只处理第一处会让第二段的密码照样进日志。
        assert_eq!(
            mask_url("--proxy-server=http://u1:p1@h1:8080;https=http://u2:p2@h2"),
            "--proxy-server=http://***@h1:8080;https=http://***@h2"
        );
        // 无凭据的拼接串不该被改坏
        assert_eq!(
            mask_url("--proxy-server=http://h1:8080;https=http://h2:8080"),
            "--proxy-server=http://h1:8080;https=http://h2:8080"
        );
    }

    #[test]
    fn rotate_renames_only_when_too_large() {
        let dir = std::env::temp_dir().join("w2a-ut-debug-log");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join(LOG_FILE_NAME);
        let prev = dir.join(PREV_LOG_FILE_NAME);

        // 小文件：不轮转
        std::fs::write(&path, b"tiny").expect("写小文件");
        rotate_if_too_large(&path);
        assert!(path.exists() && !prev.exists(), "小文件不该被轮转");

        // 超过上限：改名成 startup.prev.log，原文件消失（下次 append 会重建）
        {
            let f = std::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .expect("打开");
            f.set_len(MAX_BYTES + 1).expect("撑到超过上限");
        }
        rotate_if_too_large(&path);
        assert!(!path.exists(), "超限文件应被挪走");
        assert!(prev.exists(), "超限文件应出现在 startup.prev.log");

        // 文件不存在（首次启动）：不 panic
        let missing = dir.join("nope.log");
        rotate_if_too_large(&missing);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
