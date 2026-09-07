/*
 * web2app 多页签标题回传（tab_title.js）
 * ---------------------------------------------------------------------------
 * 仅多页签模式（TABS_ENABLED=true，Windows）注入到每个 tab 窗口。
 * 职责：监听当前网页的 document.title 变化，经 IPC 回传给 Rust（set_tab_title），
 *       让宿主窗口的标签栏显示真实网页标题，而非默认应用名。
 *
 * 设计约束：
 *   1. 零依赖、零网络；调用 Tauri 的 window.__TAURI__.core.invoke。
 *   2. 全程 try/catch，注入到第三方页面，任何异常都不影响宿主页面。
 *   3. 只回传有意义的标题（非空、与上次不同才 invoke，避免无谓 IPC）。
 *   4. 兼容 loading.js 的 overlay 注入时机：本脚本同样经 initialization_script
 *      注入，在 document-created 阶段运行，早于页面脚本。
 */
(function () {
  'use strict';
  try {
    // 当前 tab 的 label 由 Rust 在注入本脚本前以 window.__web2appTabLabel 写入。
    var label = window.__web2appTabLabel;
    var invoke = window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke;
    if (!label || !invoke) return;

    var last = '';

    function report() {
      try {
        var t = (document.title || '').trim();
        if (!t || t === last) return;
        last = t;
        invoke('set_tab_title', { label: label, title: t }).catch(function () {});
      } catch (e) { /* 忽略：回传失败不影响页面 */ }
    }

    // 页面脚本/SPA 路由会动态改 title：用 MutationObserver 监听 <title> 与 head 子树。
    var titleEl = document.querySelector('title');
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
