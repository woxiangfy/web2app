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
 * ⚠️⚠️ 「脚本执行前」必须是**真的更早**：本脚本经 document-start 注入（WebView2 的
 *   AddScriptToExecuteOnDocumentCreated / Electron preload）⇒ 写在页面 <body> 里的内联标记
 *   一定晚于本脚本、**来不及生效**。启动页在 Tauri 侧因此曾被当成目标页跑（overlay 语义），
 *   命中 15s 空白硬上限而误报失败 —— 现在由建窗时的初始化脚本在 OVERLAY_SCRIPT【之前】
 *   注入标记（lib.rs::build_main_window / tabs.rs::create_tab_window），静态守卫在
 *   contract_tests.rs::loading_mode_contract。
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
  // 目标页：文档到达但正文【持续空白】的失败硬上限。注意只对“空白文档”生效——
  // 有任何正文内容都会在检测到的那一拍立即完成，不受此限。
  var CAP_MS = 15000;
  var MIN_MS = 220; // 最短展示时长：内容秒开时避免“闪一下”造成突兀
  var FADE_MS = 280; // 淡出时长（与 CSS transition 保持一致）
  var POLL_MS = 120; // 内容就绪检测间隔
  var DCL_DELAY = 240; // DOMContentLoaded 后给浏览器留出首帧绘制时间

  // 启动页「重试」的落点。⚠️ 必须用**相对路径**，不能写 `splash://` 开头的绝对 URL：
  // wry/tauri 在 Windows 与 Android 上会把自定义协议改写成 `http://splash.localhost/`
  //（custom_protocol_workaround），此时内核根本不认识 `splash:` 这个 scheme，导航直接失败。
  // 相对路径按当前文档 origin 解析，两种形态都落到 `<origin>/retry`，两侧处理器
  //（Tauri `loading.rs::handle_splash`、Electron `protocols.ts`）都按 pathname 判 `/retry`。
  var RETRY_URL = '/retry';
  // tab 首屏占位文档（本地资产页，被 tabs.rs 的模式标记强制为 splash 语义）上的
  // 重试事件。该页不是 splash: 协议页，不能导航 RETRY_URL；改由 Rust 监听此事件
  // 为对应 tab 重走 bootstrap（见 tabs::retry_tab_bootstrap）。
  // ⚠️ 事件名必须与 tabs.rs 的 TAB_RETRY_EVENT 逐字一致（lib.rs 有静态守卫）。
  var TAB_RETRY_EVENT = 'web2app:retry-bootstrap';

  var state = 'loading'; // loading | slow | failed | done
  var t0 = Date.now();
  // 壳侧在加载层【挂载前】推来的调用（失败 / 慢提示 / 补充信息）：先缓冲，挂载后回放。
  // ⚠️⚠️ 旧实现这里挂的是一份**纯 no-op 桩**（fail 是空实现），于是「壳在挂载前推来的
  // 错误」被静默吃掉，用户只看到 40s 的「启动超时」兜底卡，真因就此丢失。而直连配置的启动流程
  // 只需毫秒级就完成（无域名解析、无本地代理），它的失败推送**必然**落在挂载之前 ——
  // 也就是说这条通路是 Tauri 侧唯一的错误出口（Electron 侧另有烤进 HTML 的
  // `__web2appPendingFail`，见下方 applyPendingFail）。
  var pending = [];
  // 壳侧在挂载【后】推来的补充信息行（各阶段进度 / 可达性探测结论）：随失败卡一起渲染。
  // 兜底卡 40s 才出现，而探测结论可能 8s 就到了；若只写进 detail，会被后来的兜底卡覆盖掉，
  // 所以单独存一份，由 toFail 统一拼出来（见 pushNote）。
  var notes = [];
  var root, styleHandle, elRing, elBar, elTitle, elDetail, elActions, elBtn;

  // 最近一条页面侧错误（脚本异常 / 资源加载失败）：失败卡诊断行里带上，便于定位
  // 「文档到了但页面脚本在老版本 WebView 上崩掉」这类问题。document-start 立刻挂，
  // 才能赶在宿主页面脚本之前捕获。全部 try/catch，不影响宿主页面。
  var lastErr = '';
  try {
    window.addEventListener('error', function (ev) {
      try {
        var m = ev && (ev.message || (ev.target && (ev.target.src || ev.target.href)));
        if (m) lastErr = String(m).slice(0, 160);
      } catch (e) { /* noop */ }
    }, true);
    window.addEventListener('unhandledrejection', function (ev) {
      try { if (ev && ev.reason) lastErr = String(ev.reason).slice(0, 160); } catch (e) { /* noop */ }
    });
  } catch (e) { /* noop */ }

  // ---------------------------------------------------------------------
  // 样式：ID 选择器 + !important，抵御宿主页面样式污染
  // ---------------------------------------------------------------------
  var CSS = [
    '#__web2app_loading__{',
    /* 整屏遮罩同理（见 titlebar.js 里的那段说明）：fixed 的 right/bottom:0 只到 clientWidth/clientHeight，
     * 页面有滚动条时右侧/底部会漏出 15px 缝隙。改成 100vw/100vh 才能真盖满。 */
    'position:fixed!important;top:0!important;left:0!important;width:100vw!important;height:100vh!important;',
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
    'max-width:min(420px,84vw)!important;white-space:pre-wrap!important;word-break:break-word!important;}',
    /* 失败态的布局：⚠️⚠️ 这一条是「诊断能不能被看见」的成败所在，别删。
     * 失败卡的内容可能很长（四条阶段进度 + 地址 / 状态 / HTTP / 整条 UA / 脚本错误），
     * 窄屏手机上光是 UA 一行就能折成四五行，总高度轻易超过视口。而本层是
     * `display:flex + justify-content:center`：内容一旦高于 100vh，**顶部的元素会被推出
     * 屏幕上方**，而 splash 页的 html/body 是 overflow:hidden ⇒ 既看不见、也滚不回来
     *（「地址」行恰好在 detail 最上面，正是最容易被裁掉的位置）。
     * 所以失败态改为**顶部对齐 + 整层可滚动**；同时放开文本选择（根规则里的
     * user-select:none 会让用户选不中、复制不了 —— 而设计意图正是「把下面的信息反馈给打包者」）。 */
    '#__web2app_loading__.w2l-failed{',
    'justify-content:flex-start!important;overflow-y:auto!important;-webkit-overflow-scrolling:touch!important;',
    'padding:22px 14px!important;-webkit-user-select:text!important;user-select:text!important;}',
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
  // DOM 宿主等待（关键，别删）
  // ---------------------------------------------------------------------
  // ⚠️ 本脚本由 WebView2 的 `AddScriptToExecuteOnDocumentCreated` 注入，执行时机是
  // 「全局对象已创建、**HTML 文档尚未解析**」——MSDN 原文 "runs after the global object
  // has been created, but before the HTML document has been parsed"。也就是说此刻
  // `document.documentElement` / `document.head` / `document.body` **都可能为 null**，
  // 而 `document` 对象本身已存在。
  //
  // 历史实现直接 `(document.head || document.documentElement).appendChild(...)`，在真实
  // 页面上必然抛 TypeError、被最外层 try/catch 静默吞掉 → 加载层从不出现。这正是用户
  // 反馈「首次打开没有加载动画」的根因。（对照组：dnd_polyfill.js 只用
  // `document.addEventListener`，不碰 DOM 节点，所以一直正常工作。）
  //
  // 这里的做法：宿主节点一出现就挂载。MutationObserver 最先（`<html>` 被插入 document
  // 时立即触发，且不受隐藏窗口的定时器节流影响——tab 窗口建窗时是 hidden 的），
  // readystatechange / DOMContentLoaded / load 与 interval 兜底。
  //
  // ⚠️ 兜底**不要**写成「setTimeout 3s 后只试一次」：那一次尝试若仍拿不到宿主节点就直接返回，
  // 加载层从此不会出现（用户反馈的「首次打开没有加载动画」）。现在持续重试到 20s。
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
    var stopAt = Date.now() + 20000;

    function stop() {
      try {
        if (obs) obs.disconnect();
      } catch (e) { /* noop */ }
      try {
        document.removeEventListener('readystatechange', attempt);
        document.removeEventListener('DOMContentLoaded', attempt);
        window.removeEventListener('load', attempt);
      } catch (e) { /* noop */ }
      if (iv) clearInterval(iv);
    }

    function attempt() {
      if (done) return;
      if (!hostNode()) {
        if (Date.now() > stopAt) {
          done = true;
          stop();
        }
        return;
      }
      done = true;
      stop();
      fn();
    }

    try {
      obs = new MutationObserver(attempt);
      obs.observe(document, { childList: true });
    } catch (e) { /* 老引擎不支持时靠下面的兜底 */ }
    try {
      document.addEventListener('readystatechange', attempt);
      document.addEventListener('DOMContentLoaded', attempt);
      window.addEventListener('load', attempt);
    } catch (e) { /* noop */ }
    iv = setInterval(attempt, 16);
  }

  /// 自绘标题栏（titlebar.js）占据顶部 38px：加载层让出该区域，避免把它盖住。
  /// 两脚本的注入顺序不固定，故用「全局标记 + 事件」双向同步。
  function applyChromeOffset() {
    if (!root || !root.style) return;
    var h = window.__web2appChromeH || 0;
    try {
      root.style.setProperty('top', h ? h + 'px' : '0px', 'important');
    } catch (e) { /* noop */ }
  }

  // ---------------------------------------------------------------------
  // 挂载
  // ---------------------------------------------------------------------
  /// 装载样式表，返回句柄。优先「构造函数式样式表」（**不受页面 CSP 的 style-src 限制**），
  /// 回退 `<style>` 元素 —— 缘由与细节见 titlebar.js 的 mountStyle，两侧必须保持一致，
  /// 否则会出现「标题栏有样式、加载层没样式」这种半截状态。
  function mountStyle() {
    try {
      if (typeof CSSStyleSheet === 'function' && document.adoptedStyleSheets) {
        var sheet = new CSSStyleSheet();
        sheet.replaceSync(CSS);
        var next = [], cur = document.adoptedStyleSheets;
        for (var i = 0; i < cur.length; i++) next.push(cur[i]);
        next.push(sheet);
        document.adoptedStyleSheets = next;
        return { kind: 'adopted', sheet: sheet };
      }
    } catch (e) { /* 落到 <style> 回退 */ }
    var el = document.createElement('style');
    el.setAttribute('data-web2app', 'loading');
    el.textContent = CSS;
    (document.head || document.documentElement).appendChild(el);
    return { kind: 'style', el: el };
  }

  function unmountStyle(h) {
    try {
      if (!h) return;
      if (h.kind === 'style') {
        if (h.el.parentNode) h.el.parentNode.removeChild(h.el);
        return;
      }
      var cur = document.adoptedStyleSheets, next = [];
      for (var i = 0; i < cur.length; i++) if (cur[i] !== h.sheet) next.push(cur[i]);
      document.adoptedStyleSheets = next;
    } catch (e) { /* noop */ }
  }

  function mount() {
    styleHandle = mountStyle();

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
      // 自绘标题栏也是我们注入的，不能算「页面内容已就绪」——否则单窗口模式下
      // 加载层会在页面还空着时就误判为完成、立刻淡出（等于没有加载动画）。
      if (e.id === '__web2app_titlebar__') continue;
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

  /**
   * 失败卡「现场诊断」：把决定故障层级的关键事实直接显示在屏幕上，免去 adb/logcat。
   * 各行含义：
   *   · 地址    —— 仍是 splash.localhost = 导航根本没提交/没离开启动页；
   *                已是目标域名 = 外链文档已开始加载，问题在网络/TLS/渲染；
   *   · 状态    —— document.readyState + body 内元素数（页面正文是否真的解析出来）；
   *   · HTTP    —— PerformanceNavigationTiming.responseStatus（Chromium 123+ 支持，
   *                老版本 WebView 显示 ?；net 错误时为 0）+ 子资源数 + 导航耗时；
   *   · UA      —— Android System WebView 的真实版本（老版本 WebView 是「浏览器正常、
   *                壳内白屏」的高发原因：TLS 证书链 / HTTP2 不兼容）；
   *   · 脚本错误 —— document-start 起捕获到的最后一条页面错误（如有）。
   * 只展示在用户自己的屏幕上，不上报、不外发。
   */
  /// 抹掉地址里的凭据再显示。
  ///
  /// 为什么必须做：启用本地反向代理时，WebView 实际加载的入口形如
  /// `http://127.0.0.1:<port>/auth.html?w2a_key=<门禁密钥>`，而这张诊断卡的设计用途
  /// 恰恰是「**截图发给打包者**排障」—— 密钥会随截图一起外发。诊断本身只需要知道
  /// 「是 127.0.0.1 还是目标域名」，值毫无用处，因此统一把这类查询参数的值换成 `***`。
  function redact(u) {
    try {
      return String(u).replace(
        /([?&](?:w2a_key|key|token|access_token)=)[^&#]*/gi,
        function (_m, p1) { return p1 + '***'; });
    } catch (e) {
      return String(u);
    }
  }

  function diagText() {
    try {
      var L = [];
      L.push('地址: ' + redact(location.href));
      var els = document.body ? document.body.getElementsByTagName('*').length : 0;
      L.push('状态: ' + document.readyState + '，元素 ' + els);
      try {
        var nav = performance.getEntriesByType('navigation')[0];
        var res = performance.getEntriesByType('resource').length;
        var st = nav ? (('responseStatus' in nav) ? nav.responseStatus : '?') : '无记录';
        L.push('HTTP: ' + st + '，资源 ' + res + (nav ? '，' + Math.round(nav.duration) + 'ms' : ''));
      } catch (e2) { L.push('HTTP: 时序不可用'); }
      L.push('UA: ' + navigator.userAgent);
      if (lastErr) L.push('脚本错误: ' + lastErr);
      return L.join('\n');
    } catch (e) {
      return '诊断信息不可用: ' + e;
    }
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
        unmountStyle(styleHandle);
        root = null;
        styleHandle = null;
      }, FADE_MS);
    }, wait);
  }

  function toSlow(text) {
    if (state !== 'loading') return;
    state = 'slow';
    elDetail.textContent = text;
  }

  /**
   * 追加一条壳侧补充信息（阶段进度 / 可达性探测结论 / 后到的失败结论）。
   *
   * ⚠️ 这些行**必须单独存一份**（`notes`）：兜底卡是 40s 才渲染的，而壳侧结论常在几秒内就到；
   * 只写进 detail 会被后来的 toFail 整体覆盖 —— 卡片上就只剩通用文案了（真机取证时正是如此：
   * 一张「启动超时」卡上没有任何现场信息，完全无从判断卡在哪一步）。
   */
  function pushNote(line) {
    if (!line) return;
    try {
      if (notes.indexOf(line) >= 0) return; // 幂等：重试 / 多次探测不重复堆同一行
      notes.push(line);
      if (state === 'done') return;
      elDetail.textContent = elDetail.textContent ? elDetail.textContent + '\n' + line : line;
    } catch (e) { /* noop */ }
  }

  /** 加载失败/超时的容错态：Rust 侧也可调用 window.__web2appLoading.fail(t, d)。 */
  function toFail(title, detail) {
    if (state === 'done' || state === 'failed') return;
    state = 'failed';
    root.className = 'w2l-failed';
    elTitle.textContent = title || '加载失败';
    var d = detail || '请检查网络连接后重试。';
    // 壳侧补充信息一并渲染（它们比兜底卡先到，且不能被兜底卡覆盖掉）。
    // ⚠️ 放在**最前面**而不是末尾：这几行是「走到哪一步」的分诊线索 —— 尤其「已发起导航」，
    // 它是区分「根本没导航」与「导航了但没提交」的**唯一**线索；而 diagText 的地址 / UA
    // 属于技术细节，是第二步才看的。末尾拼接会让它们随长诊断一起沉到滚动区最底部。
    if (notes.length) d = notes.join('\n') + '\n' + d;
    elDetail.textContent = d;
  }

  /**
   * 失败卡统一入口（壳侧 `fail` 与「挂载前缓冲」的回放共用）：
   *   · `soft` 为真时只在页面正文仍空白才接管 —— 供壳侧可达性探测使用，避免探测假阴性
   *     （探测端不认可的企业根证书等）把其实已经渲染成功的页面盖成失败卡；
   *   · 接管时附上现场诊断（地址 / readyState / HTTP / UA / 脚本错误）；
   *   · **已经有过卡片**（多为 40s 兜底卡）时不再静默丢弃，而是把结论追加成一行 ——
   *     否则「兜底卡先出现」这一个时序差别就会把可达性探测结论整个吃掉。
   */
  function failCard(title, detail, soft) {
    if (soft) {
      try { if (!bodyEmpty()) return; } catch (e) { /* 判定失败则正常接管 */ }
    }
    if (state === 'failed') {
      pushNote('【' + (title || '加载失败') + '】' + (detail || ''));
      return;
    }
    toFail(title, (detail || '请检查网络连接后重试。') + '\n' + diagText());
  }

  function emitTauriEvent(evt, payload) {
    // 与 titlebar.js / tab_title.js 同一条通路：withGlobalTauri 的 __TAURI__.event
    // 优先，__TAURI_INTERNALS__ 兜底。只在 Tauri 环境存在；Electron 侧两者皆无。
    try {
      var T = window.__TAURI__;
      if (T && T.event && typeof T.event.emit === 'function') {
        var r = T.event.emit(evt, payload);
        if (r && r.then) r.then(null, function () {});
        return true;
      }
      var I = window.__TAURI_INTERNALS__;
      if (I && typeof I.invoke === 'function') {
        I.invoke('plugin:event|emit', { event: evt, payload: payload });
        return true;
      }
    } catch (e) { /* 忽略 */ }
    return false;
  }

  // 是否【单窗口启动页】（自定义协议 splash://localhost/）。
  //
  // ⚠️ 不能只看 `location.protocol === 'splash:'`：wry/tauri 在 Windows 与 Android 上会把
  // 自定义协议改写成 `http://splash.localhost/`（wry `custom_protocol_workaround::
  // apply_uri_work_around`，`use_https_scheme` 默认 false），页面里读到的 protocol 是 `http:`。
  // 只按 protocol 判定 ⇒ 下面的重试分支永不成立、落到 location.reload()，而启动页的
  // `handle_splash` / `protocols.ts` 只有命中 `/retry` 才重新走 bootstrap ⇒ 「重试」退化成
  // 无限重载启动页，永远起不来（Windows 同样中招；Electron 侧 scheme 未被改写，protocol
  // 仍是 `splash:`，两条判据都成立）。
  function isSplashDoc() {
    try {
      return location.protocol === 'splash:' || location.hostname === 'splash.localhost';
    } catch (e) { return false; }
  }

  // 是否 Tauri【本地资产页】（tab 首屏占位文档；区别于远程目标页与 splash: 协议启动页）。
  function isTauriLocalAssetDoc() {
    try {
      return location.hostname === 'tauri.localhost' ||
        location.protocol === 'tauri:' ||
        location.protocol === 'asset:';
    } catch (e) { return false; }
  }

  function onRetry() {
    try {
      if (IS_SPLASH && isSplashDoc()) {
        // 单窗口启动页：回到 /retry，由壳侧重新走完整启动流程
        //（重新解析地址、重启本地代理、再导航）。
        window.location.href = RETRY_URL;
      } else if (IS_SPLASH && isTauriLocalAssetDoc()) {
        // 多页签 tab 的本地占位页：不导航（没有 tab 版 splash 协议页），发事件让壳
        // 针对当前 tab 重走 bootstrap；label/token 由建窗时的初始化脚本注入。
        if (!emitTauriEvent(TAB_RETRY_EVENT, {
          label: window.__web2appTabLabel,
          token: window.__web2appTabToken
        })) {
          try { console.error('[web2app] 重试事件发送失败：Tauri 事件通道不可用'); } catch (e2) {}
        }
      } else {
        // overlay 阶段（真实目标页加载失败）：直接刷新当前地址即可重试。
        window.location.reload();
      }
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
      // 正文持续空白超过硬上限：按【加载失败】处理而不是无条件淡出。
      // 关键背景：Android WebView 网络失败时停在一个“空白错误文档”（不像桌面端
      // Chromium / WebView2 会画出带错误码的页面）——旧实现到点 done() 淡出后，
      // 用户只看到纯白，且 30s 的 FAIL_MS 失败卡还会因 state==='done' 被静默吞掉，
      // 表现为「转圈十几秒 → 白屏，没有原因也没有重试」。改成失败卡后可直接点重试；
      // 内容在这之后才到达的极端慢页，bodyEmpty() 会在之前任一拍提前 done()，不受影响。
      if (Date.now() - t0 > CAP_MS) {
        // 诊断行直接拼在卡片上：用户截图即可判断「停在启动页 / 外链空白错误文档 /
        // 页面脚本崩溃」哪一种，无需 adb。文案不再把锅单方面推给网络——手机浏览器
        // 能打开而壳内空白时，常见根因是系统内置 WebView 版本过旧（见 UA 行）。
        toFail(
          '加载失败',
          '页面长时间为空白。若手机浏览器能打开同一网址，请把下方信息反馈给打包者：\n' +
          diagText()
        );
        return;
      }
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
  /// 取走壳侧「烤进 HTML」的失败态（Electron 侧启动页走这条路）。
  ///
  /// 为什么需要：启动页把加载层内联在**主世界**，而 preload 的 IPC 通道在**隔离世界**
  /// （读不到 window.__web2appLoading），所以启动页阶段的错误没法用 IPC 推 —— 壳侧改为在
  /// 生成的 HTML 末尾直接写 `window.__web2appPendingFail = {…}`，这里在加载层真正挂好后
  /// 取走。放在 API 安装【之后】调用是关键：挂载前 window.__web2appLoading 还是 no-op 桩，
  /// 那时调用 fail() 会被丢掉。
  function applyPendingFail() {
    try {
      var pf = window.__web2appPendingFail;
      if (!pf || !pf.title) return;
      window.__web2appPendingFail = null;
      toFail(pf.title, pf.detail || '');
    } catch (e) { /* noop */ }
  }

  /// 回放「挂载前」缓冲的壳侧调用（失败 / 慢提示 / 补充信息）——见文件上方 `pending` 的说明。
  /// 挂载前 `window.__web2appLoading` 是缓冲桩，桩里的每次调用都记在 `pending` 里，这里补上。
  function drainPending() {
    var list = pending;
    pending = [];
    for (var i = 0; i < list.length; i++) {
      try {
        var c = list[i];
        if (c[0] === 'fail') failCard(c[1], c[2], c[3]);
        else if (c[0] === 'slow') toSlow(c[1]);
        else pushNote(c[1]);
      } catch (e) { /* 单条回放失败不影响其余 */ }
    }
  }

  // 先挂一份**缓冲**API：壳侧可能在任何时刻 eval
  // `window.__web2appLoading.fail/slow/note`（解析失败、代理启动失败、阶段进度、导航已发起…），
  // 挂载完成前调用必须无害 —— 但**绝不能丢掉**（细节见文件上方 `pending` 的说明）。
  window.__web2appLoading = {
    state: function () { return state; },
    hide: function () {},
    slow: function (t) { pending.push(['slow', t]); },
    fail: function (t, d, soft) { pending.push(['fail', t, d, soft]); },
    note: function (l) { pending.push(['note', l]); }
  };

  whenHostReady(function () {
    try {
      mount();
    } catch (e) {
      // 挂载失败（极端文档形态）时绝不影响宿主页面，也绝不让用户卡在无反馈的空白里：
      // 目标页加载完成会触发新的 load 事件，Rust 侧另有超时兜底。
      return;
    }

    applyChromeOffset();
    try {
      window.addEventListener('web2app:chrome-ready', applyChromeOffset);
    } catch (e) { /* noop */ }

    // 对外 API：壳通过 window.eval 驱动异常态展示（Tauri），
    // 或经 IPC 转交（Electron 真实页面：加载层与 preload 同在隔离世界）。
    // fail(t, d, soft)：soft 为真时，仅当页面正文仍为空白才接管 —— 供 Rust 侧
    // 可达性探测使用，避免探测假阴性（如系统信任库认可但探测端不信任的企业根证书）
    // 把其实已经渲染成功的页面盖成失败卡。
    window.__web2appLoading = {
      state: function () { return state; },
      hide: function () { done(); },
      slow: function (t) { toSlow(t || '网络较慢，仍在努力加载…'); },
      // 壳侧补充信息（各阶段进度 / 可达性探测结论）：不改卡片状态，只往详情里加行。
      note: function (l) { pushNote(l); },
      // fail(t, d, soft)：语义见上面的 failCard。
      fail: function (t, d, soft) { failCard(t, d, soft); }
    };

    // 先回放「挂载前」缓冲的壳侧调用，再取走壳侧烤进 HTML 的失败态 —— 两者都是最后一手，
    // 后到的会覆盖先到的（说明见各自函数内）。
    drainPending();
    applyPendingFail();

    setTimeout(function () {
      toSlow(IS_SPLASH ? '正在准备应用环境…' : '网络较慢，仍在努力加载…');
    }, SLOW_MS);
    setTimeout(function () {
      // ⚠️ 这张兜底卡以前是**唯一不带现场诊断**的失败卡（CAP 卡与 fail API 卡都带），于是真机上
      // 只有一句「应用初始化耗时过长」，完全看不出卡在哪一步。现在带上 diagText()，其中
      // 「地址」行是最短判据：
      //   · 仍是 splash.localhost ⇒ 目标地址的导航**根本没提交**（Android 上 navigate 恒返回 Ok，
      //     且 wry 不向壳层透出 onReceivedError，壳侧无从得知，只能靠这里自证）；
      //   · 已是目标域名 ⇒ 文档层问题（HTTP 状态 / UA 版本 / 脚本错误三行分流）。
      toFail(
        IS_SPLASH ? '启动超时' : '加载超时',
        (IS_SPLASH ? '应用初始化耗时过长，请检查网络后重试。' : '页面响应超时，请检查网络连接后重试。') +
          (isSplashDoc() ? '\n仍停在启动页：目标地址的导航没有提交。' : '') +
          '\n' + diagText()
      );
    }, FAIL_MS);

    if (!IS_SPLASH) startReadyWatch();
  });
})();
