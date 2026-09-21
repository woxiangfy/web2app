//! 临时探针：确证 WebView2 在 document-created 阶段 DOM 的可访问性。
//!
//! 运行：`cargo run --example probe_dom_ready`
//!
//! 背景：`loading.js` / `titlebar.js` 都是经 `AddScriptToExecuteOnDocumentCreated`
//! 注入的。MSDN 说这些脚本「runs after the global object has been created, but before
//! the HTML document has been parsed」——如果此时 `document.documentElement` /
//! `document.head` / `document.body` 全都是 null，那么在它们上面 `appendChild` 就会抛
//! TypeError 并被最外层 try/catch 静默吞掉，表现为「顶部栏与加载动画双双不出现」。
//!
//! 本探针创建**不可见**窗口，注入一段探测脚本，把「三个节点是否存在 + appendChild 是否
//! 成功 + readyState」经 IPC 回传并打印，3 秒后自动退出，不影响正常使用。

fn main() {
    #[cfg(windows)]
    run();
    #[cfg(not(windows))]
    println!("probe_dom_ready 仅用于 Windows WebView2");
}

#[cfg(windows)]
fn run() {
    use std::time::Duration;
    use tao::event_loop::{ControlFlow, EventLoop};
    use tao::window::WindowBuilder;
    use wry::WebViewBuilder;

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("web2app-probe")
        .with_visible(false)
        .build(&event_loop)
        .expect("创建探针窗口失败");

    // 探测脚本：与生产注入脚本完全相同的执行时机。
    // ① 记录 document-created 一刻 DOM 是否存在、直接 appendChild 是否失败；
    // ② 用与 loading.js / titlebar.js 相同的 whenHostReady 等待宿主，验证它能挂载成功。
    let probe_js = r#"
(function () {
  var t0 = Date.now();
  var atCreated = {
    de: !!document.documentElement,
    head: !!document.head,
    body: !!document.body,
    rs: document.readyState
  };
  var naive;
  try {
    document.documentElement.appendChild(document.createElement('div'));
    naive = 'ok';
  } catch (e) {
    naive = 'ERR: ' + (e && e.message);
  }

  function hostNode() { return document.body || document.head || document.documentElement; }
  function whenHostReady(fn) {
    if (hostNode()) { fn(); return; }
    var done = false, obs = null, iv = 0;
    function attempt() {
      if (done || !hostNode()) return;
      done = true;
      try { if (obs) obs.disconnect(); } catch (e) {}
      try { document.removeEventListener('readystatechange', attempt); } catch (e) {}
      if (iv) clearInterval(iv);
      fn();
    }
    try { obs = new MutationObserver(attempt); obs.observe(document, { childList: true }); } catch (e) {}
    try { document.addEventListener('readystatechange', attempt); } catch (e) {}
    iv = setInterval(attempt, 16);
    setTimeout(attempt, 3000);
  }

  whenHostReady(function () {
    var mounted;
    try {
      var d = document.createElement('div');
      d.id = '__probe_bar__';
      var host = hostNode();
      host.appendChild(d);
      mounted = host.nodeName + ' -> ' + (d.parentNode && d.parentNode.nodeName);
    } catch (e) {
      mounted = 'ERR: ' + (e && e.message);
    }
    try {
      window.chrome.webview.postMessage('PROBE2 ' + JSON.stringify({
        at_created: atCreated,
        naive_append: naive,
        mounted_after_wait: mounted,
        waited_ms: Date.now() - t0,
        rs: document.readyState
      }));
    } catch (e) { /* noop */ }
  });
})();
"#;

    let _webview = WebViewBuilder::new()
        .with_initialization_script(probe_js)
        .with_ipc_handler(|req| {
            println!("RESULT {}", req.body());
            std::process::exit(0);
        })
        .with_html("<!doctype html><html><head><title>probe</title></head><body><h1>probe</h1></body></html>")
        .build(&window)
        .expect("创建 WebView 失败");

    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(3));
        println!("RESULT TIMEOUT(未收到回传)");
        std::process::exit(1);
    });

    event_loop.run(move |_event, _target, control_flow| {
        *control_flow = ControlFlow::Wait;
    });
}
