/*
 * web2app 自绘窗口标题栏（titlebar.js）
 * ---------------------------------------------------------------------------
 * 仅单窗口模式（TABS_ENABLED=false，Windows）注入到主窗【每一个】文档（document-created），
 * 因为主窗加载的是远程 Trilium 页，持久置顶的标题栏只能经 initialization_script 注入其页面。
 *
 * 提供的标题栏（高 38px，与多页签宿主 shell.html 的标签栏一致）：
 *      [← 后退] [→ 前进]          标题                [— 最小化] [▢ 最大化] [✕ 关闭]
 *   - 整条 `-webkit-app-region: drag` 空白处可拖动窗口；按钮为 no-drag 可点击；双击最大化。
 *   - 前进/后退调用 Rust 命令 history_nav(direction)，由 Rust 控制当前窗体对应 webview 的历史。
 *   - 最小化/最大化/关闭经 window.__TAURI__.window.getCurrentWindow() 原生执行。
 *
 * 设计约束（沿用 loading.js 的约定，务必保持）：
 *   1. 零依赖、零网络，全部 try/catch：注入到第三方页面，任何异常都不能破坏宿主页面。
 *   2. 同一文档只安装一次（window.__web2appTitlebar 防重）。
 *   3. 关键样式带 !important 并走 ID 选择器，z-index 取极高位，避免被宿主页面样式覆盖。
 *   4. 标题栏落位在 <body>，并用 CSS 给 html/body 预留 38px 顶部空间，正文不被盖住。
 */
(function () {
  'use strict';
  if (window.__web2appTitlebar) return; // 同一文档只安装一次

  var TITLEBAR_H = 38;

  var CSS = [
    '#__web2app_titlebar__{',
    'position:fixed!important;top:0!important;left:0!important;right:0!important;',
    'height:' + TITLEBAR_H + 'px!important;',
    'z-index:2147483647!important;',
    'display:flex!important;align-items:center!important;',
    'box-sizing:border-box!important;margin:0!important;padding:0 8px!important;',
    'gap:4px!important;',
    'background:#eef0f2!important;color:#2c3440!important;',
    'border-bottom:1px solid rgba(0,0,0,.08)!important;',
    'font:13px/1 -apple-system,"Segoe UI","Microsoft YaHei",sans-serif!important;',
    '-webkit-user-select:none!important;user-select:none!important;',
    '-webkit-app-region:drag!important;',
    '}',
    '@media (prefers-color-scheme:dark){#__web2app_titlebar__{',
    'background:#1f2530!important;color:#e6ebf2!important;border-bottom-color:rgba(255,255,255,.07)!important;}}',
    '#__web2app_titlebar__ *{box-sizing:border-box!important;margin:0!important;padding:0!important;}',
    /* 按钮：no-drag 才可点击 */
    '#__web2app_titlebar__ .w2t-btn{',
    '-webkit-app-region:no-drag!important;',
    'display:flex!important;align-items:center!important;justify-content:center!important;',
    'min-width:34px!important;height:28px!important;padding:0 8px!important;',
    'border:0!important;border-radius:6px!important;background:transparent!important;',
    'color:inherit!important;font:13px/1 "Segoe UI","Microsoft YaHei",sans-serif!important;',
    'cursor:default!important;outline:none!important;',
    '}',
    '#__web2app_titlebar__ .w2t-btn:hover{background:rgba(120,133,153,.16)!important;}',
    /* 中央标题 */
    '#__web2app_titlebar__ .w2t-title{',
    'flex:1!important;min-width:0!important;padding:0 8px!important;',
    'font-size:12px!important;opacity:.72!important;',
    'white-space:nowrap!important;overflow:hidden!important;text-overflow:ellipsis!important;',
    '-webkit-app-region:drag!important;',
    '}',
    /* 关闭按钮 hover 用红底，符合系统惯例 */
    '#__web2app_titlebar__ .w2t-close:hover{background:#e81123!important;color:#fff!important;}',
    /* 给远程页预留顶部空间，正文不被盖住 */
    'html{min-height:100%!important;}',
    'html,body{padding-top:' + TITLEBAR_H + 'px!important;}'
  ].join('\n');

  var HTML = [
    '<button type="button" class="w2t-btn w2t-back" title="后退">←</button>',
    '<button type="button" class="w2t-btn w2t-fwd" title="前进">→</button>',
    '<div class="w2t-title"> </div>',
    '<button type="button" class="w2t-btn w2t-min" title="最小化">—</button>',
    '<button type="button" class="w2t-btn w2t-max" title="最大化/还原">▢</button>',
    '<button type="button" class="w2t-btn w2t-close" title="关闭">✕</button>'
  ].join('');

  // ---------------------------------------------------------------------
  // DOM 宿主等待（关键，别删）
  // ---------------------------------------------------------------------
  // ⚠️ 本脚本（单窗口模式）由 WebView2 的 `AddScriptToExecuteOnDocumentCreated` 注入，
  // 执行时机是「全局对象已创建、**HTML 文档尚未解析**」——MSDN 原文 "runs after the global
  // object has been created, but before the HTML document has been parsed"。此刻
  // `document.documentElement` / `document.head` / `document.body` **都可能为 null**。
  //
  // 历史实现直接 `(document.body || document.documentElement).appendChild(...)` → 真实页面上
  // 必然抛 TypeError、被最外层 try/catch 静默吞掉 → **顶部栏从不出现**（用户反馈的
  // 「无多标签时没有顶部的菜单栏」）。对照 splash 页是把它内联进完整 HTML，DOM 已就绪，
  // 所以只在真实页面上暴露。
  //
  // 做法：宿主节点一出现就挂载。MutationObserver 最先（`<html>` 插入 document 时立即触发，
  // 且不受隐藏窗口的定时器节流影响），interval 与 readystatechange 兜底。
  function hostNode() {
    return document.body || document.head || document.documentElement;
  }

  function whenHostReady(fn) {
    if (hostNode()) {
      fn();
      return;
    }
    var done = false;
    var obs = null;
    var iv = 0;
    function attempt() {
      if (done || !hostNode()) return;
      done = true;
      try {
        if (obs) obs.disconnect();
      } catch (e) { /* noop */ }
      try {
        document.removeEventListener('readystatechange', attempt);
      } catch (e) { /* noop */ }
      if (iv) clearInterval(iv);
      fn();
    }
    try {
      obs = new MutationObserver(attempt);
      obs.observe(document, { childList: true });
    } catch (e) { /* 老引擎不支持时靠下面的兜底 */ }
    try {
      document.addEventListener('readystatechange', attempt);
    } catch (e) { /* noop */ }
    iv = setInterval(attempt, 16);
    setTimeout(attempt, 3000); // 硬兜底：极端文档形态下 3s 后再强制试一次
  }

  function mount() {
    var styleEl = document.createElement('style');
    styleEl.setAttribute('data-web2app', 'titlebar');
    styleEl.textContent = CSS;
    (document.head || document.documentElement).appendChild(styleEl);

    var bar = document.createElement('div');
    bar.id = '__web2app_titlebar__';
    bar.innerHTML = HTML;

    // document-created 阶段 DOM 尚未解析，`<body>` 往往还不存在，先挂到 `<html>`，
    // 等 body 就绪后再挪进去（见 ensureHost）。
    (document.body || document.documentElement).appendChild(bar);

    return { styleEl: styleEl, bar: bar };
  }

  /// body 就绪后把标题栏挪进 body，避免它作为 <html> 直接子节点时被部分页面样式影响。
  function ensureHost(bar) {
    try {
      if (document.body && bar.parentNode !== document.body) {
        document.body.appendChild(bar);
      }
    } catch (e) { /* 忽略：挪动失败不影响 fixed 定位的显示效果 */ }
  }

  function wire(bar) {
    var invoke = window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke;
    var win = null;
    try {
      win = window.__TAURI__ && window.__TAURI__.window && window.__TAURI__.window.getCurrentWindow();
    } catch (e) { /* 个别环境拿不到窗口句柄时，仅前进/后退仍可工作 */ }

    function onNav(direction) {
      if (invoke) {
        invoke('history_nav', { direction: direction }).catch(function () {});
      } else if (direction === 'back') {
        try { window.history.back(); } catch (e) {}
      } else {
        try { window.history.forward(); } catch (e) {}
      }
    }

    bar.querySelector('.w2t-back').addEventListener('click', function () { onNav('back'); });
    bar.querySelector('.w2t-fwd').addEventListener('click', function () { onNav('forward'); });
    if (win) {
      bar.querySelector('.w2t-min').addEventListener('click', function () { win.minimize().catch(function () {}); });
      bar.querySelector('.w2t-max').addEventListener('click', function () { win.toggleMaximize().catch(function () {}); });
      bar.querySelector('.w2t-close').addEventListener('click', function () { win.close().catch(function () {}); });
      // 双击标题栏空白区最大化/还原
      bar.addEventListener('dblclick', function () { win.toggleMaximize().catch(function () {}); });
    }

    // 标题：跟随 document.title（SPA 路由会动态改 title）。
    var titleEl = bar.querySelector('.w2t-title');
    function refreshTitle() {
      var t = (document.title || '').trim();
      if (t) titleEl.textContent = t;
    }
    refreshTitle();
    try {
      var t0 = document.querySelector('title');
      var obs = new MutationObserver(function () { refreshTitle(); });
      obs.observe(t0 || document.head || document.documentElement,
        { subtree: true, childList: true, characterData: true });
    } catch (e) { /* 忽略：标题跟不跟随不影响按钮 */ }
  }

  // 立刻置位防重入：挂载是异步的（要等 DOM 宿主节点），期间可能被再次注入。
  window.__web2appTitlebar = true;

  whenHostReady(function () {
    var bar;
    try {
      bar = mount().bar;
      wire(bar);
    } catch (e) {
      // 极端文档挂载失败：绝不破坏宿主页面。
      return;
    }

    // body 出现后把标题栏挪进 body（首次挂载时 body 常常还不存在，先挂在 <html> 上）。
    try {
      if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', function () { ensureHost(bar); }, { once: true });
      } else {
        ensureHost(bar);
      }
    } catch (e) { /* noop */ }

    // 通告加载层（loading.js）：顶部已被占用 38px，请它让出该区域，避免把标题栏盖住。
    // 两脚本的注入顺序不固定，故「全局标记 + 事件」双保险，谁先谁后都能同步上。
    window.__web2appChromeH = TITLEBAR_H;
    try {
      window.dispatchEvent(new Event('web2app:chrome-ready'));
    } catch (e) { /* noop */ }
  });
})();