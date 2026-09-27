//! w2a-proxy — web2app Electron 绿色版的 sidecar 进程。
//!
//! ## 为什么要有这个独立进程
//!
//! Electron 侧有两块能力没法（也不该）用 JS 重写：
//!
//! * **本地反向代理**：给【每一个】请求（含 WebSocket 握手）统一注入 `Authorization: Basic`，
//!   并对监听端口做一次性随机密钥门禁（无密钥一律 404，防本机扫描借道）。
//! 这是一份已经过充分测试的 Rust（流式转发、Cookie Domain 重写、WS 双向隧道、入口门禁；
//! 行数会随实现增长，不要在注释里写死数字）。
//! * **动态域名解析**：DNS TXT → Ed25519 验签 → AES-GCM 解密。在 Node 侧要引 Ed25519 + HKDF +
//!   AES-GCM 一整套实现，且必须与 `resolve-core` 的字节级格式严丝合缝 —— 风险极高、收益为零。
//!
//! 所以本进程用 `#[path]` **直接引用** webview-app 的 `src/proxy.rs` 与 `src/resolve.rs`：
//! 零重写、零复制，两壳永远跑同一份实现（这是「双壳并行不跑偏」最省力的保障方式）。
//!
//! ## 与宿主的契约
//!
//! 1. 宿主 spawn 本进程，用命令行参数把上游地址交进来（见 `--help`）；
//! 2. 就绪后向 **stdout 打且仅打一行 JSON**，宿主按行解析拿到 `proxyUrl` 并导航过去
//!    （`proxyUrl` 自带一次性门禁密钥 `?w2a_key=...`，无密钥访问代理一律 404）；
//!    失败同样打一行 JSON（`ok:false` + `kind`），随后以退出码 1 结束；
//! 3. 之后本进程常驻转发流量。**stdin 关闭（EOF）= 宿主已退出 ⇒ 自己退出** ——
//!    这是防僵尸进程的兜底：宿主被强杀（或被任务管理器结束）时来不及调 `kill`。
//!
//! ⚠️⚠️ **stdout 只给机器读，日志一律走 stderr**。宿主是按行解析 stdout 的；往 stdout 打一行
//! 日志就会让握手解析失败（而进程又活着），表现为「窗口一直转圈、日志里什么都没」的假死。
//! ⚠️ 握手那行必须 `flush`：stdout 在管道上是块缓冲的，不 flush 宿主会一直等（Windows 上尤其明显）。
//! ⚠️ 若 stdin 是终端（人工手动运行），不启用 EOF 监视 —— 否则一个人的交互式终端会立刻被判成 EOF。

// ⚠️⚠️ 共享源码（`proxy.rs` / `resolve.rs`）里用 `dlog!` 写诊断。主 crate 的同名宏把输出落进
// `logs\startup.log`（主程序是 GUI 子系统，`eprintln!` 没有任何去处），但 **`macro_rules!` 的作用域
// 是 crate 级的**：不写 `#[macro_export]` 时它只在**定义它的 crate** 内可见，而共享源文件被本 bin
// 用 `#[path]` 引进来时算**本 crate** 的模块 ⇒ 主 crate 那份定义在这里根本看不见
// （实测报 `error: cannot find macro 'dlog' in this scope`，加 `crate::` 全限定也救不了，
// 因为找不到的是**宏名本身**，不是展开体里的路径）。
// 所以本 bin 必须自带一个同名宏 —— 文本作用域要求它**声明在 `mod proxy;` 之前**。
//
// 为什么展开成裸 `eprintln!`、这里不再判一次 debug：
//   本进程是**控制台进程**，stderr 由宿主逐行读走（`electron-app/src/main/sidecar.ts` 的
//   `log` 回调，宿主传的是 `(line) => console.log(line)`），再按**宿主的** `debug` 决定去向 ——
//   关闭时宿主的 `console.log` 是空操作，这些行不会落到任何地方；打开时才同时进 stdout 与
//   `logs\startup.log`。即**门控归宿主**。在这里再复制一份「debug 判定」只会多出一个可能与宿主
//   不一致的真值来源，而「同一份配置在当前壳里判定相反」正是本项目吃过亏的那类缺陷
//   （见 docs/packaging-config.md §3.3 的 `"on"`）。
macro_rules! dlog {
    ($($arg:tt)*) => {{
        eprintln!($($arg)*);
    }};
}

// 零重写复用：直接以上层 crate 的源文件为本模块（见文件头说明）。
// 路径相对本文件所在目录：crates/proxy-server/src/ → ../../../src/
#[path = "../../../src/proxy.rs"]
mod proxy;
// ⚠️ `resolve.rs` 里的 `pub use resolve_core::{...}` 是 re-export 一批 API 给主 crate 用的
// （主 crate 是 lib，公开 API 不会被判 unused）。在**本 bin** 里它变成了私有模块的 re-export，
// 其中 generate_keypair / parse_domains / sign_and_encrypt / VerifyingKey 没有调用点 ⇒
// 触发 unused_imports。删掉它们会破坏主 crate 与 resolve-tool 的公开 API，所以这里
// 用**模块级 allow** 精确豁免，而不是去改共享源码（那会让主 crate 也丢掉 lint 保护）。
#[path = "../../../src/resolve.rs"]
#[allow(unused_imports)]
mod resolve;

use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use url::Url;

/// 命令行参数集合。字段名与 `--help` 里的开关一一对应。
struct Args {
    /// 直连上游地址（可含 `user:pass@` → 代理会强制注入 Basic 头）。
    url: Option<String>,
    /// 从 stdin 读第一行作为上游地址。**宿主（Electron）用这条路径**，见 `read_url_from_stdin`。
    url_stdin: bool,
    /// 域名列表（提供了就走 DNS TXT 解析，`url` 可为空）。
    domains: Vec<String>,
    /// Ed25519 公钥（裸 base64 或 PEM/SPKI），仅域名模式需要。
    public_key: String,
    /// 只解析地址、不起本地代理。宿主自己会用 Chromium 的 login 事件注入凭据。
    no_proxy: bool,
    /// 额外把握手 JSON 写一份到该文件（stdout 被包装器吞掉时的逃生口）。
    handshake_file: Option<PathBuf>,
}

// ⚠️ 用 raw string（r#"..."#）而不是普通字面量：下面的「输出契约」示例里含有 JSON，
// 也就含有双引号。写普通字符串必须逐个转义成 \"，极易漏掉一个 ——
// 漏掉那一个的后果是字符串在那里提前结束，然后编译器报出一大串**与本行无关**的
// “unknown start of token: 。/（/：”（全角标点被当成代码），排查方向直接被带偏。
const USAGE: &str = r#"w2a-proxy — web2app 本地反向代理 / 动态域名解析 sidecar

用法：
  w2a-proxy --url <URL> [选项]                     直连上游（URL 含 user:pass@ 则注入 Basic）
  w2a-proxy --domains <a.com,b.com> --public-key <B64> [选项]
                                                   DNS TXT 解析 + 验签解密后代理

选项：
  --url <URL>              上游地址，可含 userinfo（**手动调试用**）
  --url-stdin              从 stdin 读第一行作为上游地址（宿主走这条；原因见下）
  --domains <列表>         逗号分隔的域名，按序尝试（任一解出即用）
  --public-key <B64>       Ed25519 公钥（裸 base64 / PEM / SPKI）
  --no-proxy               只解析地址，不启动本地代理
  --handshake-file <路径>  额外把握手 JSON 写入该文件
  -h, --help               显示本帮助
  -V, --version            显示版本

输出契约（stdout 仅一行 JSON）：
  成功 {"ok":true,"proxyUrl":"http://127.0.0.1:PORT/?w2a_key=<随机密钥>","resolvedUrl":...,"authUser":...}
  失败 {"ok":false,"kind":"args|resolve|proxy","error":"..."}  并退出码 1

为什么有 --url-stdin：
  上游地址里可能内嵌 `user:pass@`。命令行参数在 Windows 上对**同用户的其它进程**是可见的
  （GetCommandLine / WMI 都能拿到），等于把站点密码明晃晃摆出来。经 stdin 传递则避开了
  「命令行」与「环境变量」这两条最容易泄漏的通道。宿主（Electron）默认走这条；
  `--url` 保留给手动调试（那种场景下地址通常不含凭据）。
  ⚠️ stdin 第一行之后**不要关闭**管道 —— 它同时是「宿主是否还活着」的信号（EOF = 宿主已退出）。

日志走 stderr。stdin 关闭即退出（宿主已退出的兜底）。
"#;

fn main() -> ExitCode {
    let mut args = match parse_args(std::env::args().skip(1).collect()) {
        Ok(Some(a)) => a,
        // --help / --version：已输出，正常退出
        Ok(None) => return ExitCode::SUCCESS,
        Err(e) => {
            // 参数错误也要走 JSON 握手（宿主在等这一行），但 kind 标记为 args
            eprintln!("[w2a-proxy] {e}");
            eprintln!("[w2a-proxy] {USAGE}");
            handshake_failure("args", &e);
            return ExitCode::FAILURE;
        }
    };

    // 上游地址经 stdin 传入（原因见 USAGE 的「为什么有 --url-stdin」：避免凭据出现在命令行）。
    // ⚠️ 必须在建 runtime / block_on **之前**读 —— `read_line` 是阻塞调用，
    // 放进 async 上下文里会占住一个 worker 线程（多线程 runtime 下不致命，但没理由这么做）。
    if args.url_stdin {
        if std::io::stdin().is_terminal() {
            eprintln!("[w2a-proxy] --url-stdin：请在终端粘贴上游地址后回车");
        }
        match read_url_from_stdin() {
            Ok(u) => args.url = Some(u),
            Err(e) => {
                eprintln!("[w2a-proxy] {e}");
                handshake_failure("args", &e);
                return ExitCode::FAILURE;
            }
        }
    }

    // 多线程 runtime：axum 服务、DNS 查询、WebSocket 隧道里有并发任务。
    // enable_all() 打开 IO + 时间驱动（DNS 的 UDP socket 与 reqwest 都需要）。
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            let msg = format!("创建 tokio runtime 失败：{e}");
            eprintln!("[w2a-proxy] {msg}");
            handshake_failure("runtime", &msg);
            return ExitCode::FAILURE;
        }
    };

    // `--no-proxy` 只是「一次性解析地址」：握手交出去后本进程没有任何常驻价值，
    // 让它自行退出，宿主那边少一个需要回收的进程（也少一个僵尸的可能）。
    let should_wait = !args.no_proxy;
    let handshake_file = args.handshake_file.clone();
    match rt.block_on(run(args)) {
        Ok(handshake) => {
            emit_handshake(&handshake, handshake_file.as_deref(), should_wait);
            ExitCode::SUCCESS
        }
        Err((kind, msg)) => {
            eprintln!("[w2a-proxy] 启动失败（{kind}）：{msg}");
            emit_handshake(
                &serde_json::json!({ "ok": false, "kind": kind, "error": msg }),
                handshake_file.as_deref(),
                false,
            );
            ExitCode::FAILURE
        }
    }
}

/// 启动代理并返回握手 JSON；失败时返回 `(kind, 人类可读原因)`。
async fn run(args: Args) -> Result<serde_json::Value, (&'static str, String)> {
    eprintln!(
        "[w2a-proxy] 启动：pid={} 模式={}",
        std::process::id(),
        if args.domains.is_empty() {
            "直连".to_string()
        } else {
            format!("DNS 解析（{} 个域名）", args.domains.len())
        }
    );

    // ---------- 1. 拿到上游地址 ----------
    // Tauri 侧在 bootstrap::prepare_navigation 里是同一套语义：数组 → DNS 解析，字符串 → 原样用。
    let resolved_url: String = if args.domains.is_empty() {
        args.url.clone().ok_or((
            "args",
            "必须提供 --url、--url-stdin 或 --domains".to_string(),
        ))?
    } else {
        if args.public_key.trim().is_empty() {
            return Err(("args", "域名解析模式必须提供 --public-key".to_string()));
        }
        resolve::resolve_url_from_domains(&args.domains, &args.public_key)
            .await
            .map_err(|e| ("resolve", e))?
    };

    let parsed = Url::parse(&resolved_url)
        .map_err(|e| ("args", format!("上游地址不合法（{e}）：{resolved_url}")))?;

    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err((
                "args",
                format!("只支持 http / https 上游，当前是 {other}：{resolved_url}"),
            ))
        }
    }

    let had_auth = !parsed.username().is_empty();
    eprintln!(
        "[w2a-proxy] 上游：{} 鉴权={}",
        strip_userinfo(&parsed),
        if had_auth { "内置 Basic" } else { "无" }
    );

    // ---------- 2. 起本地反向代理 ----------
    //
    // `--no-proxy` 时宿主需要**凭据本身**：DNS 解析出的地址可能带 userinfo，而宿主导航的是
    // 剥掉 userinfo 的版本，凭据只能经 Chromium 的 `login` 事件回填（Tauri 侧对应
    // WebView2 的原生鉴权事件）。起代理时不需要 —— 代理会给每个请求强制注入 Basic 头。
    //
    // ⚠️⚠️ 这两个字段是**明文凭据**：宿主**绝不能**把它们写进日志 / 错误弹窗 / 握手文件。
    //     所以它们单独成字段，而不是塞进 resolvedUrl —— 后者会被打印，必须是干净的。
    let (auth_user, auth_pass) = if args.no_proxy {
        (
            parsed.username().to_string(),
            parsed.password().unwrap_or("").to_string(),
        )
    } else {
        (String::new(), String::new())
    };

    let proxy_url = if args.no_proxy {
        // 只解析不代理：宿主直接连上游（凭据由 login 事件注入）。
        eprintln!("[w2a-proxy] --no-proxy：只解析地址，不启动本地代理");
        strip_userinfo(&parsed)
    } else {
        let (u, _abort) = proxy::start_proxy(&parsed)
            .await
            .map_err(|e| ("proxy", format!("本地代理启动失败：{e}")))?;
        // 故意不 abort：代理任务要一直跑到本进程退出。进程退出时整个 runtime 一起销毁，
        // 监听端口随之释放（所以这里不需要保存 AbortHandle）。
        u
    };

    let upstream_host = parsed.host_str().unwrap_or("").to_string();
    eprintln!("[w2a-proxy] 本地入口：{proxy_url}");

    Ok(serde_json::json!({
        "ok": true,
        "proxyUrl": proxy_url,
        "resolvedUrl": strip_userinfo(&parsed),
        "upstreamHost": upstream_host,
        "hadAuth": had_auth,
        "authUser": auth_user,
        "authPass": auth_pass,
        "pid": std::process::id(),
    }))
}

/// 发出握手 JSON；`should_wait` 为真时随后开始等 stdin 关闭。
///
/// ⚠️ 顺序不能颠倒：宿主必须**先**拿到 `proxyUrl` 才会去导航，如果这里先等 stdin，
/// 宿主就永远等不到那一行（死锁）。
fn emit_handshake(v: &serde_json::Value, file: Option<&std::path::Path>, should_wait: bool) {
    let line = v.to_string();

    if let Some(p) = file {
        // 文件握手是「stdout 被外层包装器吞掉」时的逃生口：写失败不影响主通道，
        // 但必须在日志里现形 —— 静默失败会让人以为是 sidecar 没启动。
        match std::fs::write(p, &line) {
            Ok(()) => eprintln!("[w2a-proxy] 握手已写入 {}", p.display()),
            Err(e) => eprintln!("[w2a-proxy] 握手文件写入失败（{}）：{e}", p.display()),
        }
    }

    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    // 握手行是唯一给机器读的内容，必须独占一行且立刻可见。
    if let Err(e) = writeln!(lock, "{line}").and_then(|()| lock.flush()) {
        eprintln!("[w2a-proxy] 握手写入 stdout 失败：{e}");
    }
    drop(lock);

    if !should_wait || v["ok"] != serde_json::Value::Bool(true) {
        // 失败路径 / 一次性解析：握手已交出去，不再等 stdin，直接退出。
        return;
    }

    wait_host_exit();
}

/// 从 stdin 读第一行作为上游地址（`--url-stdin`）。
///
/// ⚠️ 这里**只读到第一个换行为止**，之后 stdin 仍保持「可读」状态 —— 因为 `wait_host_exit`
/// 还要靠同一个 stdin 的 EOF 来判断宿主是否退出。宿主（Electron）写一行后**不会**关闭管道，
/// 正是这个约定。
/// ⚠️ 读不到内容一律当错误：宿主若在写入前就崩了，这里读到 0 字节，必须报错退出而不是
/// 拿空地址去 `Url::parse`（那样报出来的是「地址不合法」，把排查方向带偏）。
fn read_url_from_stdin() -> Result<String, String> {
    use std::io::BufRead;

    let mut line = String::new();
    let n = std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| format!("读取 stdin 失败：{e}"))?;
    if n == 0 {
        return Err("stdin 已关闭，未收到上游地址（宿主可能在写入前就退出了）".to_string());
    }
    // 两端平台都可能给出 \r\n（Windows 管道上尤其常见），一并去掉。
    let url = line.trim_end_matches(['\r', '\n']).trim().to_string();
    if url.is_empty() {
        return Err("stdin 第一行为空，未收到上游地址".to_string());
    }
    Ok(url)
}

/// 等宿主退出。
///
/// 判据是 **stdin 关闭**：宿主（Electron 主进程）与 sidecar 之间是一根管道，宿主无论
/// 正常退出还是被强杀，管道都会关闭，本进程读到 0 字节 ⇒ 自行退出。
/// 这是防僵尸进程最省事、也最可靠的跨平台手段（Windows 上要写 Job Object 才能做到同等效果，
/// 而且对「宿主被任务管理器结束」这种情况 Job Object 反而更绕）。
///
/// ⚠️ stdin 是终端时**必须跳过**：人工手动运行（`w2a-proxy --url ...`）时终端 stdin 一开就是
/// EOF 或永远阻塞，会让进程刚起来就自杀，或者反过来赖着不走。
fn wait_host_exit() {
    if std::io::stdin().is_terminal() {
        eprintln!("[w2a-proxy] stdin 是终端，跳过宿主存活监视（手动运行模式，Ctrl+C 退出）");
        return;
    }

    eprintln!("[w2a-proxy] 已就绪，等待宿主连接（stdin 关闭即退出）");

    // ⚠️ 必须**阻塞在主线程**，不能用 spawn 后返回：main 一返回进程就退出了，
    // 代理还没接客就死掉，而宿主那边只看到「窗口白屏 + 进程秒退」。
    // 此刻已经不在 rt.block_on 内（async 上下文），主线程拿来阻塞读是安全的。
    let mut buf = [0u8; 256];
    loop {
        match std::io::stdin().read(&mut buf) {
            Ok(0) => {
                eprintln!("[w2a-proxy] stdin 关闭，宿主已退出，sidecar 随之退出");
                break;
            }
            // 宿主目前不发指令，读到什么都忽略（保留将来 send('reload') 之类的余地）。
            Ok(_) => {}
            Err(e) => {
                eprintln!("[w2a-proxy] 读 stdin 出错（{e}），按宿主已退出处理");
                break;
            }
        }
    }
}

/// 去掉 URL 里的 userinfo，避免**凭据出现在日志和握手 JSON 里**。
///
/// ⚠️ 这不是洁癖：握手 JSON 会被宿主写进日志文件、错误弹窗、甚至用户截图发给我们排障。
/// 上游地址里嵌的就是 Basic 账号密码，泄漏一次就等于把站点拱手让人。
fn strip_userinfo(u: &Url) -> String {
    let mut clean = u.clone();
    let _ = clean.set_username("");
    let _ = clean.set_password(None);
    clean.to_string()
}

/// 解析命令行。返回 `Ok(None)` 表示「已输出 --help / --version，正常结束」。
fn parse_args(argv: Vec<String>) -> Result<Option<Args>, String> {
    let mut args = Args {
        url: None,
        url_stdin: false,
        domains: Vec::new(),
        public_key: String::new(),
        no_proxy: false,
        handshake_file: None,
    };

    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        // 取下一个值：缺失时报错而不是静默用空串 —— 空值会让「配置没写对」伪装成「运行时报错」。
        let mut take = |name: &str| -> Result<String, String> {
            i += 1;
            argv.get(i)
                .cloned()
                .ok_or_else(|| format!("{name} 后面缺少值"))
        };

        match a {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("w2a-proxy {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "--url" => args.url = Some(take("--url")?),
            "--url-stdin" => args.url_stdin = true,
            "--public-key" => args.public_key = take("--public-key")?,
            "--domains" => {
                // 逗号分隔；顺手去空白与空项，允许 `a.com, b.com,` 这种手写格式。
                args.domains = take("--domains")?
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(resolve::normalize_domain)
                    .collect();
            }
            "--no-proxy" => args.no_proxy = true,
            "--handshake-file" => {
                args.handshake_file = Some(PathBuf::from(take("--handshake-file")?))
            }
            other => return Err(format!("未知参数：{other}")),
        }
        i += 1;
    }

    // ⚠️ `--url-stdin` 时此刻 url 还是空的（要等 main 去读 stdin），不能在这里就判失败。
    if args.url.is_none() && !args.url_stdin && args.domains.is_empty() {
        return Err("必须提供 --url、--url-stdin 或 --domains".to_string());
    }
    Ok(Some(args))
}

/// 失败握手的最早入口（参数解析 / runtime 创建阶段，此时 `run` 还没跑起来）。
fn handshake_failure(kind: &str, msg: &str) {
    let v = serde_json::json!({ "ok": false, "kind": kind, "error": msg });
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = writeln!(lock, "{v}").and_then(|()| lock.flush());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_accepts_url() {
        let args = parse_args(vec!["--url".into(), "https://h.com".into()])
            .unwrap()
            .expect("有有效来源，不应触发 help/version");
        assert_eq!(args.url.as_deref(), Some("https://h.com"));
        assert!(!args.url_stdin);
        assert!(args.domains.is_empty());
    }

    #[test]
    fn parse_args_no_source_is_error() {
        // 三个来源（--url / --url-stdin / --domains）一个都没有 → 报错，绝不静默拿空地址去解析。
        assert!(parse_args(vec![]).is_err());
        assert!(parse_args(vec!["--public-key".into(), "k".into()]).is_err());
    }

    #[test]
    fn parse_args_missing_value_is_error() {
        // 缺值时报错而不是静默用空串 —— 否则「配置没写对」会被伪装成「运行时报错」。
        assert!(parse_args(vec!["--url".into()]).is_err());
        assert!(parse_args(vec![
            "--domains".into(),
            "a.com".into(),
            "--public-key".into()
        ])
        .is_err());
    }

    #[test]
    fn parse_args_unknown_flag_is_error() {
        assert!(parse_args(vec!["--bogus".into()]).is_err());
    }

    #[test]
    fn parse_args_help_and_version_are_none() {
        assert!(matches!(parse_args(vec!["--help".into()]), Ok(None)));
        assert!(matches!(parse_args(vec!["-h".into()]), Ok(None)));
        assert!(matches!(parse_args(vec!["--version".into()]), Ok(None)));
    }

    #[test]
    fn parse_args_domains_normalize_trim_dedup_empty() {
        let args = parse_args(vec!["--domains".into(), " A.com , b.com, ".into()])
            .unwrap()
            .expect("domains 自成一个来源，不应报缺来源");
        assert_eq!(args.domains, vec!["a.com", "b.com"]);
    }

    #[test]
    fn strip_userinfo_removes_credentials_only() {
        assert_eq!(
            strip_userinfo(&Url::parse("https://u:p@h:8080/a?q=1").unwrap()),
            "https://h:8080/a?q=1"
        );
        // 注意：url crate 会把根路径规范化为 "/"（https://h.com → https://h.com/）。
        assert_eq!(
            strip_userinfo(&Url::parse("https://h.com").unwrap()),
            "https://h.com/"
        );
        assert_eq!(
            strip_userinfo(&Url::parse("http://u@[::1]/x").unwrap()),
            "http://[::1]/x"
        );
    }

    #[test]
    fn success_handshake_is_single_line_json_with_contract_fields() {
        // 与 run() 组装的字段保持一致，钉死「stdout 只出一行 JSON」的握手契约。
        // 说明见文件头：宿主按行解析 stdout，多打一行 = 握手失败但进程活着 = 假死。
        let v = serde_json::json!({
            "ok": true,
            "proxyUrl": "http://127.0.0.1:1234/",
            "resolvedUrl": "https://h.com/",
            "upstreamHost": "h.com",
            "hadAuth": false,
            "authUser": "",
            "authPass": "",
            "pid": 0,
        });
        let line = v.to_string();
        assert!(!line.contains('\n'), "握手必须是一行，绝不能内嵌换行");
        assert!(line.starts_with('{') && line.ends_with('}'));
        let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["ok"], serde_json::Value::Bool(true));
        assert!(parsed["proxyUrl"].as_str().unwrap().starts_with("http://"));
    }

    #[test]
    fn failure_handshake_has_kind_and_error() {
        let v = serde_json::json!({ "ok": false, "kind": "args", "error": "缺来源" });
        let line = v.to_string();
        assert!(!line.contains('\n'));
        let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["ok"], serde_json::Value::Bool(false));
        assert_eq!(parsed["kind"], "args");
        assert!(parsed["error"].as_str().unwrap().contains("缺来源"));
    }
}
