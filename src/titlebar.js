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

  function mount() {
    var styleEl = document.createElement('style');
    styleEl.setAttribute('data-web2app', 'titlebar');
    styleEl.textContent = CSS;
    (document.head || document.documentElement).appendChild(styleEl);

    var bar = document.createElement('div');
    bar.id = '__web2app_titlebar__';
    bar.innerHTML = HTML;

    // document-created 阶段 body 可能尚未出现，先挂到 <html>，body 就绪后挪进去。
    (document.body || document.documentElement).appendChild(bar);
    if (document.body && bar.parentNode !== document.body) {
      document.body.appendChild(bar);
    }

    return { styleEl: styleEl, bar: bar };
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

  try {
    var m = mount();
    wire(m.bar);
  } catch (e) {
    // 极端文档挂载失败：绝不破坏宿主页面。
    window.__web2appTitlebar = true;
    return;
  }
  window.__web2appTitlebar = true;
})();