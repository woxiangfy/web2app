/*
 * web2app 启动加载过渡层（loading.js）
 * ---------------------------------------------------------------------------
 * 同一份脚本以两种模式运行，视觉与状态机完全一致，保证「启动页 → 目标页」之间观感连续：
 *
 *   • splash  模式：内嵌在启动页 splash://localhost/ 里。窗口创建后立刻显示（纯本地、零网络），
 *                   覆盖「进程已启动但首个页面还没开始加载」这段原本完全无反馈的等待。
 *   • overlay 模式：经 WebviewWindowBuilder::initialization_script 注入到【每一次】导航的目标
 *                   文档，运行在 document-created 阶段（早于页面任何脚本与首次绘制），
 *                   因此能盖住目标页从空白到渲染出内容之间的白屏，内容就绪后淡出。
 *
 * 模式由注入方在脚本执行前设置 window.__web2appLoadingMode 决定，缺省为 overlay。
 *
 * 状态机：loading（转圈 + 进度条）→ slow（慢提示，仍可自动完成）→ done（淡出移除）
 *                                                              ↘ failed（错误 + 重试）
 *
 * 设计约束（务必保持）：
 *   1. 零依赖、零网络请求、无外部字体/图片，脚本体积控制在几 KB，不影响真实启动速度。
 *   2. 全程包在 try/catch 里：注入到第三方页面，任何异常都不能破坏宿主页面。
 *   3. 关键样式带 !important 并走 ID 选择器，避免被宿主页面的后加载样式覆盖。
 *   4. 动画只用 transform/opacity（合成层），淡出后立刻从 DOM 移除，不留残留开销。
 */
(function () {
  'use strict';
  if (window.__web2appLoading) return; // 同一文档只安装一次

  var MODE = window.__web2appLoadingMode === 'splash' ? 'splash' : 'overlay';
  var IS_SPLASH = MODE === 'splash';

  // —— 时间参数（毫秒）——
  // splash 阶段可能包含域名解析等网络操作，阈值放宽；overlay 阶段是真实页面加载，收紧。
  var SLOW_MS = IS_SPLASH ? 5000 : 8000; // 超过则提示“仍在加载”，不阻断
  var FAIL_MS = IS_SPLASH ? 40000 : 30000; // 超过则判定失败，给出重试入口
  var CAP_MS = 15000; // 目标页：内容迟迟检测不到时的硬上限，防止永远卡住
  var MIN_MS = 220; // 最短展示时长：内容秒开时避免“闪一下”造成突兀
  var FADE_MS = 280; // 淡出时长（与 CSS transition 保持一致）
  var POLL_MS = 120; // 内容就绪检测间隔
  var DCL_DELAY = 240; // DOMContentLoaded 后给浏览器留出首帧绘制时间

  var RETRY_URL = 'splash://localhost/retry';

  var state = 'loading'; // loading | slow | failed | done
  var t0 = Date.now();
  var root, styleEl, elRing, elBar, elTitle, elDetail, elActions, elBtn;

  // ---------------------------------------------------------------------
  // 样式：ID 选择器 + !important，抵御宿主页面样式污染
  // ---------------------------------------------------------------------
  var CSS = [
    '#__web2app_loading__{',
    'position:fixed!important;top:0!important;right:0!important;bottom:0!important;left:0!important;',
    'z-index:2147483647!important;display:flex!important;flex-direction:column!important;',
    'align-items:center!important;justify-content:center!important;gap:15px!important;',
    'margin:0!important;padding:0!important;border:0!important;outline:0!important;',
    'background:#f7f8fa!important;color:#4b5563!important;',
    'font:14px/1.7 -apple-system,BlinkMacSystemFont,"Segoe UI","PingFang SC","Microsoft YaHei",sans-serif!important;',
    'opacity:1!important;visibility:visible!important;',
    '-webkit-user-select:none!important;user-select:none!important;cursor:default!important;',
    'transition:opacity .28s ease,visibility .28s ease!important;}',
    '#__web2app_loading__ *{box-sizing:border-box!important;margin:0!important;padding:0!important;}',
    '#__web2app_loading__.w2l-out{opacity:0!important;visibility:hidden!important;pointer-events:none!important;}',
    '@media (prefers-color-scheme:dark){',
    '  #__web2app_loading__{background:#14181f!important;color:#cbd5e1!important;}',
    '}',
    /* 转圈指示：ring 匀速旋转，传达“正在工作” */
    '#__web2app_loading__ .w2l-ring{width:36px!important;height:36px!important;border-radius:50%!important;',
    'border:3px solid rgba(120,133,153,.22)!important;border-top-color:#2563eb!important;',
    'border-right-color:#38bdf8!important;animation:w2l-spin .85s linear infinite!important;}',
    '@media (prefers-color-scheme:dark){#__web2app_loading__ .w2l-ring{',
    'border-color:rgba(148,163,184,.22)!important;border-top-color:#60a5fa!important;border-right-color:#22d3ee!important;}}',
    '@keyframes w2l-spin{to{transform:rotate(360deg)}}',
    /* 不确定进度条：无法获知真实百分比时，用它表达“持续推进中” */
    '#__web2app_loading__ .w2l-bar{position:relative!important;width:168px!important;height:3px!important;',
    'border-radius:99px!important;overflow:hidden!important;background:rgba(120,133,153,.18)!important;}',
    '#__web2app_loading__ .w2l-bar i{position:absolute!important;top:0!important;left:0!important;height:100%!important;',
    'width:38%!important;border-radius:99px!important;',
    'background:linear-gradient(90deg,#2563eb,#22d3ee)!important;',
    'animation:w2l-slide 1.15s ease-in-out infinite!important;}',
    '@keyframes w2l-slide{0%{transform:translateX(-105%)}100%{transform:translateX(268%)}}',
    '#__web2app_loading__ .w2l-title{font-size:14px!important;letter-spacing:.02em!important;}',
    '#__web2app_loading__ .w2l-detail{font-size:12px!important;opacity:.62!important;text-align:center!important;',
    'max-width:min(420px,84vw)!important;}',
    /* 失败态：停掉转圈与进度条，换成警示图标 + 重试按钮 */
    '#__web2app_loading__.w2l-failed .w2l-ring,#__web2app_loading__.w2l-failed .w2l-bar{display:none!important;}',
    '#__web2app_loading__ .w2l-warn{display:none!important;width:40px!important;height:40px!important;',
    'border-radius:50%!important;border:2px solid #f59e0b!important;color:#f59e0b!important;',
    'font-size:24px!important;line-height:36px!important;text-align:center!important;font-weight:700!important;}',
    '#__web2app_loading__.w2l-failed .w2l-warn{display:block!important;}',
    '#__web2app_loading__ .w2l-actions{display:none!important;}',
    '#__web2app_loading__.w2l-failed .w2l-actions{display:block!important;}',
    '#__web2app_loading__ .w2l-btn{display:inline-block!important;padding:7px 22px!important;',
    'border:1px solid rgba(120,133,153,.35)!important;border-radius:8px!important;',
    'background:transparent!important;color:inherit!important;font:inherit!important;font-size:13px!important;',
    'cursor:pointer!important;transition:background-color .16s ease,border-color .16s ease!important;}',
    '#__web2app_loading__ .w2l-btn:hover{background:rgba(120,133,153,.12)!important;}',
    /* 尊重系统“减弱动态效果”偏好 */
    '@media (prefers-reduced-motion:reduce){',
    '  #__web2app_loading__{transition:opacity .12s linear,visibility .12s linear!important;}',
    '  #__web2app_loading__ .w2l-ring{animation:none!important;}',
    '  #__web2app_loading__ .w2l-bar i{animation:none!important;width:100%!important;}',
    '}'
  ].join('\n');

  var HTML = [
    '<div class="w2l-ring"></div>',
    '<div class="w2l-warn" aria-hidden="true">!</div>',
    '<div class="w2l-title">加载中…</div>',
    '<div class="w2l-detail"></div>',
    '<div class="w2l-bar"><i></i></div>',
    '<div class="w2l-actions"><button type="button" class="w2l-btn">重试</button></div>'
  ].join('');

  // ---------------------------------------------------------------------
  // 挂载
  // ---------------------------------------------------------------------
  function mount() {
    styleEl = document.createElement('style');
    styleEl.setAttribute('data-web2app', 'loading');
    styleEl.textContent = CSS;
    (document.head || document.documentElement).appendChild(styleEl);

    root = document.createElement('div');
    root.id = '__web2app_loading__';
    root.setAttribute('role', 'status');
    root.setAttribute('aria-live', 'polite');
    root.innerHTML = HTML;
    // document-created 阶段 body 可能还不存在，先挂到 <html>，等 body 就绪再挪进去，
    // 保证 fixed 定位的层始终处于正常渲染树中。
    (document.body || document.documentElement).appendChild(root);

    elRing = root.querySelector('.w2l-ring');
    elBar = root.querySelector('.w2l-bar');
    elTitle = root.querySelector('.w2l-title');
    elDetail = root.querySelector('.w2l-detail');
    elActions = root.querySelector('.w2l-actions');
    elBtn = root.querySelector('.w2l-btn');
    elBtn.addEventListener('click', onRetry);
    // 阻止点击穿透到宿主页面
    root.addEventListener('mousedown', function (e) { e.stopPropagation(); });
  }

  /** body 就绪后把层挪进 body，避免它作为 <html> 直接子节点时被部分页面样式影响。 */
  function ensureHost() {
    try {
      if (document.body && root && root.parentNode !== document.body) {
        document.body.appendChild(root);
      }
    } catch (e) { /* 忽略：挪动失败不影响 fixed 定位的显示效果 */ }
  }

  // ---------------------------------------------------------------------
  // 状态切换
  // ---------------------------------------------------------------------
  /** 页面是否已渲染出可见内容（排除本层自身与 script/style 等非视觉节点）。 */
  function bodyEmpty() {
    var b = document.body;
    if (!b) return true;
    var kids = b.children || [];
    for (var i = 0; i < kids.length; i++) {
      var e = kids[i];
      if (e === root) continue;
      var t = e.tagName;
      if (t === 'SCRIPT' || t === 'STYLE' || t === 'TEMPLATE' || t === 'LINK' ||
        t === 'META' || t === 'NOSCRIPT' || t === 'BASE') continue;
      return false;
    }
    // 没有任何可见元素时，再看是否存在非空文本节点（纯文本页面）
    var nodes = b.childNodes || [];
    var txt = '';
    for (var k = 0; k < nodes.length; k++) {
      var nd = nodes[k];
      if (nd === root) continue;
      if (nd.nodeType === 3) txt += nd.nodeValue;
    }
    return txt.trim().length === 0;
  }

  /** 内容已就绪 → 平滑淡出并移除，避免画面突兀跳变。 */
  function done() {
    if (state === 'done' || state === 'failed') return;
    state = 'done';
    var wait = Math.max(0, MIN_MS - (Date.now() - t0));
    setTimeout(function () {
      if (!root) return;
      root.className = 'w2l-out';
      setTimeout(function () {
        if (root && root.parentNode) root.parentNode.removeChild(root);
        if (styleEl && styleEl.parentNode) styleEl.parentNode.removeChild(styleEl);
        root = styleEl = null;
      }, FADE_MS);
    }, wait);
  }

  function toSlow(text) {
    if (state !== 'loading') return;
    state = 'slow';
    elDetail.textContent = text;
  }

  /** 加载失败/超时的容错态：Rust 侧也可调用 window.__web2appLoading.fail(t, d)。 */
  function toFail(title, detail) {
    if (state === 'done' || state === 'failed') return;
    state = 'failed';
    root.className = 'w2l-failed';
    elTitle.textContent = title || '加载失败';
    elDetail.textContent = detail || '请检查网络连接后重试。';
  }

  function onRetry() {
    try {
      // splash 阶段：回到启动页的 /retry，由 Rust 重新走一遍完整的启动流程
      // （重新解析地址、重启本地代理、再导航），而不是简单刷新当前页。
      // overlay 阶段：已经在目标页上，直接刷新当前地址即可重试。
      window.location.href = IS_SPLASH ? RETRY_URL : window.location.href;
    } catch (e) {
      try { window.location.reload(); } catch (e2) { /* noop */ }
    }
  }

  // ---------------------------------------------------------------------
  // 就绪判定（overlay 模式）
  // ---------------------------------------------------------------------
  function startReadyWatch() {
    function tick() {
      if (state === 'done' || state === 'failed') return;
      ensureHost();
      if (!bodyEmpty()) return done();
      if (Date.now() - t0 > CAP_MS) return done(); // 硬上限，避免极端页面把用户永久挡住
      setTimeout(tick, POLL_MS);
    }
    function onDcl() {
      ensureHost();
      setTimeout(tick, DCL_DELAY);
    }
    if (document.readyState === 'loading') {
      document.addEventListener('DOMContentLoaded', onDcl, { once: true });
    } else {
      onDcl();
    }
  }

  // ---------------------------------------------------------------------
  // 安装
  // ---------------------------------------------------------------------
  try {
    mount();
  } catch (e) {
    // 挂载失败（极端文档形态）时绝不影响宿主页面，也绝不让用户卡在无反馈的空白里：
    // 目标页加载完成会触发新的 load 事件，Rust 侧另有超时兜底。
    window.__web2appLoading = { hide: function () {}, fail: function () {}, slow: function () {}, state: function () { return 'error'; } };
    return;
  }

  // 对外 API：Rust 通过 window.eval 驱动异常态展示。
  window.__web2appLoading = {
    state: function () { return state; },
    hide: function () { done(); },
    slow: function (t) { toSlow(t || '网络较慢，仍在努力加载…'); },
    fail: function (t, d) { toFail(t, d); }
  };

  setTimeout(function () {
    toSlow(IS_SPLASH ? '正在准备应用环境…' : '网络较慢，仍在努力加载…');
  }, SLOW_MS);
  setTimeout(function () {
    toFail(
      IS_SPLASH ? '启动超时' : '加载超时',
      IS_SPLASH ? '应用初始化耗时过长，请检查网络后重试。' : '页面响应超时，请检查网络连接后重试。'
    );
  }, FAIL_MS);

  if (!IS_SPLASH) startReadyWatch();
})();
