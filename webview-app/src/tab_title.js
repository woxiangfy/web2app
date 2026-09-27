/*
 * web2app 多页签标题回传（tab_title.js）
 * ---------------------------------------------------------------------------
 * 仅多页签模式（TABS_ENABLED=true，Windows）注入到每个 tab 窗口。
 * 职责：监听当前网页的 document.title 变化，经**核心事件**回传给 Rust
 *       （事件名 web2app:tab-title），让宿主窗口的标签栏显示真实网页标题，
 *       而非默认应用名。
 *
 * ⚠️⚠️ 为什么是事件而不是直接 invoke 标题回传命令（2026-09-27 改，别改回去）：
 *   tab 窗口里跑的是**远程 origin**（本地代理的 http://127.0.0.1:<port> 或直连上游）。
 *   Tauri 2 对非本地 origin 的 IPC 强制走 ACL，而应用自定义命令没有 app 级 ACL 清单
 *   可授权（框架自带测试 remote_origin_blocked_for_custom_commands_without_app_manifest
 *   即为这条规则）⇒ invoke 必被 reject，而本文件原来 .catch 静默吞掉，标签栏永远显示
 *   默认名，CI 与编译期全绿。`core:event:allow-emit` 可以授权给远程 origin
 *   （capabilities/tabs-remote.json 覆盖 "tab-*"），所以与 titlebar.js 的「设置」按钮
 *   走同一条路：emit 核心事件，Rust 侧 app.listen 落地（见 tabs::report_tab_title）。
 *
 * 安全：payload 必须带建窗时注入的 window.__web2appTabToken（一次性随机令牌）。
 *   app.listen 的 Event 不携带发送窗口标识，没有令牌则任意外部页面都能伪造别的页签
 *   标题；Rust 侧对不上令牌一律丢弃。
 *
 * 设计约束：
 *   1. 零依赖、零网络；事件通路按 (__TAURI__.event → __TAURI_INTERNALS__) 顺序探测。
 *   2. 全程 try/catch，注入到第三方页面，任何异常都不影响宿主页面。
 *   3. 只回传有意义的标题（非空、与上次不同才发，避免无谓 IPC）。
 *   4. 兼容注入时机：本脚本经 initialization_script 在 document-created 阶段运行，
 *      早于页面脚本。
 */
(function () {
  'use strict';
  try {
    // 当前 tab 的 label / token 由 Rust 在注入本脚本前写入。
    var label = window.__web2appTabLabel;
    var token = window.__web2appTabToken;
    if (!label || !token) return;

    var EVENT = 'web2app:tab-title';
    var last = '';

    // 发核心事件：优先 withGlobalTauri 的 __TAURI__.event，兜底底层 __TAURI_INTERNALS__。
    // 与 titlebar.js 的 openSettings() 同一条通路（两处的原因也相同）。
    function emit(payload) {
      try {
        var T = window.__TAURI__;
        if (T && T.event && typeof T.event.emit === 'function') {
          var r = T.event.emit(EVENT, payload);
          if (r && r.then) r.then(null, function () {});
          return;
        }
        var I = window.__TAURI_INTERNALS__;
        if (I && typeof I.invoke === 'function') {
          I.invoke('plugin:event|emit', { event: EVENT, payload: payload });
        }
      } catch (e) { /* 忽略：回传失败不影响页面 */ }
    }

    function report() {
      try {
        var t = (document.title || '').trim();
        if (!t || t === last) return;
        last = t;
        emit({ label: label, token: token, title: t });
      } catch (e) { /* 忽略：回传失败不影响页面 */ }
    }

    // 页面脚本/SPA 路由会动态改 title：用 MutationObserver 监听 <title> 与 head 子树。
    var observer = new MutationObserver(function () {
      report();
    });
    function observe() {
      try {
        var head = document.head || document.documentElement;
        var target = document.querySelector('title') || head;
        observer.observe(target, { subtree: true, childList: true, characterData: true });
      } catch (e) { /* 忽略 */ }
    }

    // 初始上报一次（若 title 已有值）。
    report();

    // 页面可能尚未生成 <title>，等 DOM 就绪后再监听一次。
    if (document.readyState === 'loading') {
      document.addEventListener('DOMContentLoaded', function () {
        observe();
        report();
      }, { once: true });
    } else {
      observe();
    }

    // 兜底：定期轮询一次，捕获 MutationObserver 漏掉的场景（如直接改 document.title 未触发 DOM 变化）。
    setInterval(function () { report(); }, 1500);
  } catch (e) { /* 忽略 */ }
})();
