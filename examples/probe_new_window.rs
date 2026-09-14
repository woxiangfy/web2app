//! 临时探针：回答两个问题 ——
//!   ① wry 的 `with_initialization_script` 是否在【首个文档】与【导航后的新文档】都生效？
//!   ② 单窗口模式下点击 `target="_blank"` 到底发生什么（新窗口请求是被吞掉、还是同 webview 打开）？
//!
//! 运行（探针**自带**一个极简 HTTP 服务，不依赖外部进程 —— 之前用 python -m http.server
//! 反复掉线，导致页面压根没加载，结论全部失真）：
//!
//! ```text
//! cargo run --example probe_new_window auto            # 页面加载 2.5s 后 location.href 导航到 page B
//! cargo run --example probe_new_window blank           # 页面加载 2.5s 后点击 <a target="_blank">
//! PROBE_NO_INJECT=1 cargo run --example probe_new_window auto   # 对照组：不注入脚本
//! ```
//!
//! 上报通道刻意选 **document.title**（由 Rust 侧的 `with_document_title_changed_handler` 打印）：
//! `evaluate_script` 回调与 WebMessage 两条通道在本环境实测都收不到回调，title 是唯一可靠的。
//! 判据：
//!   · `[title] page A` 之后出现 `[title] [INJ] page A`  → 注入脚本在【首文档】执行了；
//!   · 导航后出现 `[title] page B` + `[title] [INJ] page B` → 新文档同样执行了注入脚本；
//!   · 只出现 `page B` 而没有 `[INJ] page B`            → 新文档【没有】拿到注入脚本（就是 bug）。
//!
//! ## 2026-09-14 实测记录（踩过的坑，别再重踩）
//!
//! **首个文档也会执行 `initialization_script`**：wry 是「先把脚本
//! `AddScriptToExecuteOnDocumentCreated` 完成，再 Navigate」，所以 `[INJ] page A` 必然出现。
//! 即 titlebar.js / splash.html 注释里「首个文档拿不到初始化脚本」的说法不成立
//! （splash 内联无害，可保留，但别再据此推断其它问题）。
//!
//! **隐藏窗口会毁掉结论**：`with_visible(false)` 时 WebView2 暂停文档解析与页面脚本，
//! title 会一直停在主机名（如 `127.0.0.1`），看起来像「页面没加载」。
//!
//! **Chromium 会节流被遮挡窗口的定时器**：探针里页面的 `setTimeout` 从不执行，
//! 改用 `meta refresh` / `window.open` 后仍受本机环境影响，所以「导航是否发生」这类结论
//! **不要**只靠本探针，最终以真机（真实 Tauri 应用）为准。
//!
//! **页面必须是 http://**：wry 的 IPC 处理器把 WebMessage 的 source 当 `http::Uri` 解析，
//! `file://` 页面一旦 `postMessage` 就 panic（`Result::unwrap()` on InvalidUri）。
//!
//! **上报通道**：`evaluate_script` / `evaluate_script_with_callback` 的回调在本环境收不到；
//! `with_document_title_changed_handler` 是唯一稳定的通道。
//!
//! **`blank` 组跑出了完整链路**（页面自己同步调用 `window.open` 时成功）：
//! `[title] page A` → `[title] [INJ] page A`（首文档注入生效）→
//! `[new-window] 我们的处理器被调用：uri=...page_b.html`（webview_hooks 的钩子确实被调用，
//! 即 wry 默认处理器的 `SetHandled(true)` 不会阻止后注册的处理器）→
//! `[title] [INJ]`（新文档创建，且初始化脚本在新文档里同样执行）→ `[title] page B`。
//! 结论：单窗口模式「`target="_blank"` 在当前窗口打开」的实现是有效的，别再当死代码删掉。
//!
//! 判断完即删，不进发布产物。

fn main() {
    #[cfg(windows)]
    run();
    #[cfg(not(windows))]
    println!("probe_new_window 仅用于 Windows WebView2");
}

#[cfg(windows)]
fn run() {
    const PAGE_B: &str =
        "<!doctype html><html><head><title>page B</title></head><body><h1>page B</h1></body></html>";
    // 触发方式刻意避开 setTimeout：Chromium 对「被遮挡/非前台」的窗口会把定时器节流到分钟级，
    // 实测 setTimeout 从不执行，会让实验看起来像「页面没加载」。meta refresh 是导航层行为，
    // window.open 在文档加载时同步调用，都不受该节流影响。
    const AUTO: &str = "<!doctype html><html><head><title>page A</title>\
        <meta http-equiv=\"refresh\" content=\"1; url=/page_b.html\"></head><body>\
        <h1>page A (meta refresh)</h1></body></html>";
    const BLANK: &str = "<!doctype html><html><head><title>page A</title></head><body>\
        <h1>page A (window.open)</h1>\
        <a id=\"t\" target=\"_blank\" href=\"/page_b.html\">open new window</a>\
        <script>window.open('/page_b.html');</script>\
        </body></html>";

    use std::io::{Read, Write};
    use std::time::{Duration, Instant};
    use tao::event_loop::{ControlFlow, EventLoop};
    use tao::window::WindowBuilder;
    use webview2_com::NewWindowRequestedEventHandler;
    use windows::core::PWSTR;
    use windows::Win32::System::Com::CoTaskMemFree;
    use wry::{WebViewBuilder, WebViewExtWindows};

    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "auto".to_string());
    let no_inject = std::env::var("PROBE_NO_INJECT").is_ok();
    let port: u16 = 18091;
    let url_a = format!("http://127.0.0.1:{port}/{mode}.html");

    // ---- 自带极简 HTTP 服务（每请求一连接，Connection: close）----
    std::thread::spawn(move || {
        let listener = std::net::TcpListener::bind(("127.0.0.1", port)).expect("绑定探针端口失败");
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
            let body = match path.as_str() {
                "/auto.html" => AUTO,
                "/blank.html" => BLANK,
                "/page_b.html" => PAGE_B,
                _ => "<!doctype html><html><head><title>404</title></head><body>404</body></html>",
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = s.write_all(resp.as_bytes());
            let _ = s.flush();
        }
    });

    // 与 titlebar.js 相同的注入时机与等待逻辑，挂载成功即改写标题作为「脚本确实跑了」的信号。
    let marker = r#"
(function () {
  function hostNode() { return document.body || document.head || document.documentElement; }
  function whenHostReady(fn) {
    if (hostNode()) { fn(); return; }
    var done = false, obs = null, iv = 0;
    function attempt() {
      if (done || !hostNode()) return;
      done = true;
      try { if (obs) obs.disconnect(); } catch (e) {}
      if (iv) clearInterval(iv);
      fn();
    }
    try { obs = new MutationObserver(attempt); obs.observe(document, { childList: true }); } catch (e) {}
    try { document.addEventListener('readystatechange', attempt); } catch (e) {}
    iv = setInterval(attempt, 16);
    setTimeout(attempt, 3000);
  }
  whenHostReady(function () {
    try {
      document.documentElement.setAttribute('data-web2app-probe', 'injected');
      var d = document.createElement('div');
      d.id = '__probe_bar__';
      (document.body || document.documentElement).appendChild(d);
      document.title = '[INJ] ' + document.title;
    } catch (e) { /* noop */ }
  });
})();
"#;

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("web2app-probe-new-window")
        // ⚠️ 必须可见：隐藏窗口下 WebView2 会暂停文档解析与页面脚本执行，结论会全部失真。
        .with_visible(true)
        .build(&event_loop)
        .expect("创建探针窗口失败");

    let builder = WebViewBuilder::new()
        .with_document_title_changed_handler(|title| println!("[title] {title}"))
        .with_url(url_a.clone());
    let builder = if no_inject {
        println!("[probe] 对照组：不注入初始化脚本");
        builder
    } else {
        builder.with_initialization_script(marker)
    };
    let webview = builder.build(&window).expect("创建 WebView 失败");

    // 复刻 src/webview_hooks.rs 的单窗口分支：SetNewWindow(当前 core) + SetHandled(true)
    let core = webview.webview();
    let core_for_nw = core.clone();
    let handler = NewWindowRequestedEventHandler::create(Box::new(move |_sender, args| {
        if let Some(args) = args {
            unsafe {
                let mut p = PWSTR::null();
                let uri = args.Uri(&mut p).ok().and_then(|_| p.to_string().ok());
                if !p.is_null() {
                    CoTaskMemFree(Some(p.0 as *const core::ffi::c_void));
                }
                println!("[new-window] 我们的处理器被调用：uri={uri:?}");
                let _ = args.SetNewWindow(&core_for_nw);
                let _ = args.SetHandled(true);
            }
        }
        Ok(())
    }));
    let mut token = 0i64;
    unsafe {
        let _ = core.add_NewWindowRequested(&handler, &mut token);
    }

    println!("[probe] 加载 {url_a}（页面 2.5s 后自行导航/点击）");
    println!("[probe] 判据：[title] 前缀 `[INJ]` = 该文档执行了注入脚本");

    let start = Instant::now();
    let mut done = false;
    event_loop.run(move |_event, _target, control_flow| {
        if !done && start.elapsed() > Duration::from_secs(7) {
            done = true;
            println!("[probe] 结束");
            std::process::exit(0);
        }
        *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(100));
    });
}
