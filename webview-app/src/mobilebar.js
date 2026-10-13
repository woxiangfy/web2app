/*
 * web2app 移动端底部导航栏（mobilebar.js）
 * ---------------------------------------------------------------------------
 * 只在**移动端（Android / iOS）**注入，且仅当打包配置 `mobile_bar=true`（编译期常量
 * `config.rs::MOBILE_BAR`，移动端专属）时注入。桌面端（Windows 的 Tauri 壳与 Electron 绿色版）
 * **不注入也不编译这条路径** —— 桌面端有自绘顶部标题栏（`titlebar.js`），底部再长一条就是重复。
 *
 * 形态（屏幕底部常驻一条，左→右）：
 *      [首页] [后退] [前进] [列表]
 *   · 首页：回到入口地址 app_url；已在首页时**重新加载**；还没就绪（停在启动页）时重走一次
 *     bootstrap。⚠️ 它【不能】在页面里直接写 `location = app_url`：入口可能是【域名数组】
 *     （要 DNS TXT 解析 + 验签解密），也可能走本地反向代理（真实地址是 127.0.0.1:<随机端口>），
 *     页面手里根本没有最终地址 ⇒ 一律走核心事件 `web2app:nav-home`，由 Rust 侧导航。
 *   · 后退 / 前进：驱动当前文档的 `window.history`，但**先判定落点**（与桌面端标题栏、
 *     Android 系统返回键同一条规则）：上/下一条不是真实网页、或落点是内置启动页时【不动作】，
 *     并把按钮置灰。移动端绝不允许退回到内置页面。
 *   · 列表：展开已打开的页签并切换。**只在壳告诉它「真的有页签可切」时才渲染**
 *     （`window.__web2appMobileBarCfg.tabs`，真源 `mobilebar.rs::tabs_available`）——
 *     移动端目前没有多页签（见下），宁可少一个按钮，也不要一个点了没反应的死按钮。
 *
 * 为什么【不能】invoke 应用命令：主窗加载的是**远程页面**（直连上游域名，或本地反向代理的
 * `http://127.0.0.1:<port>`），Tauri 2 对非本地 origin 的 IPC 强制走 ACL，而应用命令没有
 * app 级 ACL 清单可授权 ⇒ 必被拒（与 `titlebar.js` / `tab_title.js` 同一条约束）。所以这里
 * 一律发**核心插件的 event 命令**（`core:event:allow-emit` 可授权给远程 origin，
 * 见 capabilities/default.json），由 Rust 侧 `app.listen(...)` 落地。
 *
 * ---------------------------------------------------------------------------
 * 底部空间契约（与桌面端「顶部空间契约」是镜像关系）
 * ---------------------------------------------------------------------------
 * 底栏占掉屏幕底部一格（52px + 手势导航条的安全区）。不处理的话页面自己的页脚 / 贴底操作条
 * 会被它盖住 —— 那正是桌面端顶部标题栏曾经踩过的同一类缺陷。做法与顶部那条对称：
 *   · `html{height:100%;overflow:hidden}` + `body{margin-bottom:H;height:calc(100vh - H);
 *     overflow-y:auto}` —— 让 **body 当滚动容器**，滚动条因此画不到底栏上；
 *   · 页面自己的「贴底固定条」（`position:fixed/absolute` 且底边贴视口底）→ 标记 + inline
 *     `bottom:Hpx!important`（上移一格，别被压住）；
 *   · 页面自己的「按视口高度定尺寸的容器」（`height:100vh` / `min-height:100vh`）→ 标记 +
 *     inline 高度减一格（缩高，别被顶出视口底）。
 * 认不出来就什么都不做，页面最多维持原样（识别条件刻意保守）。
 *
 * ---------------------------------------------------------------------------
 * 移动端交互约定（别按桌面端的习惯改）
 * ---------------------------------------------------------------------------
 *   · **没有 hover**：触摸设备上 `:hover` 要么不触发、要么点完一直粘着 ⇒ 反馈一律用 `:active`
 *     （按下瞬间给底色），并用 `aria-label` 承担无障碍语义；刻意**不画** tooltip 伪元素。
 *   · **触摸目标 ≥ 44px**（Apple HIG / Material 同为 44–48dp）：整格按钮 `flex:1` 铺满条高，
 *     而不是 34×28 的桌面按钮盒模型。
 *   · `touch-action:manipulation` + `-webkit-tap-highlight-color:transparent`：去掉双击缩放带来的
 *     ~300ms 点击延迟与系统高亮块。
 *   · 关闭页签列表 = 点遮罩 / 再点一次「列表」；面板本身从底栏上沿升起（移动端习惯的自下而上），
 *     不用桌面那种右上角浮层。
 *   · 启动页（内置 `splash`）**不画底栏**：那时还没有可前进后退的真实页面，画出来只会误导。
 *
 * 设计约束（沿用 loading.js / titlebar.js）：
 *   1. 零依赖、零网络，全部 try/catch：注入到第三方页面，任何异常都不能破坏宿主页面。
 *   2. 同一文档只安装一次（`window.__web2appMobileBar` 防重）。
 *   3. 关键样式带 !important 并走 ID 选择器，z-index 取极高位（仅次于加载层的 2147483647）。
 *   4. 样式优先走 `adoptedStyleSheets`（不受页面 CSP `style-src` 约束），回退 `<style>`。
 */
(function () {
  'use strict';
  if (window.__web2appMobileBar) return; // 同一文档只安装一次

  // ---------------------------------------------------------------------
  // 壳下发的配置（由 Rust 侧 `mobilebar.rs::config_script()` 在【本脚本之前】注入）
  //   h    : 底栏内容区高度（CSS px，不含安全区）
  //   tabs : 当前构建是否真的有「可切换的页签」（真源 mobilebar.rs::tabs_available）
  // ⚠️ 高度与按钮清单的真源都在**壳侧**，页面里不写第二份 —— 改高度只改 Rust 常量。
  // ---------------------------------------------------------------------
  var CFG = window.__web2appMobileBarCfg || {};
  var BAR_H = typeof CFG.h === 'number' && CFG.h > 0 ? CFG.h : 52;
  var HAS_TABS = CFG.tabs === true;

  var SAFE = 'env(safe-area-inset-bottom, 0px)';
  // 底栏实际占掉的高度（内容区 + 手势导航条安全区）。安全区在 CSS 里由 env() 展开，
  // JS 侧要用到同一数值时改读 `bar.getBoundingClientRect().height`（唯一实测来源）。
  var RESERVE = 'calc(' + BAR_H + 'px + ' + SAFE + ')';
  // 页面「按视口高度定尺寸」的容器该用的高度。
  var FIT_H = 'calc(100vh - ' + BAR_H + 'px - ' + SAFE + ')';

  var SHIFT_ATTR = 'data-web2app-mobile-shift';   // 已上移的贴底固定条
  var SHRINK_ATTR = 'data-web2app-mobile-shrink'; // 已缩高的整屏容器
  var VH_TOL = 1;

  var ICO_HOME =
    '<svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">' +
    '<path d="M10 20v-6h4v6h5v-8h3L12 3 2 12h3v8z"/></svg>';
  var ICO_BACK =
    '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" ' +
    'stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
    '<path d="M15 5 L8 12 L15 19"/></svg>';
  var ICO_FWD =
    '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" ' +
    'stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
    '<path d="M9 5 L16 12 L9 19"/></svg>';
  var ICO_LIST =
    '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" ' +
    'stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
    '<path d="M4 6h16M4 12h16M4 18h16"/></svg>';
  var ICO_CLOSE =
    '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" ' +
    'stroke-width="2" stroke-linecap="round" aria-hidden="true">' +
    '<path d="M6 6 L18 18 M18 6 L6 18"/></svg>';

  var CSS = [
    '#__web2app_mobilebar__{',
    'position:fixed!important;left:0!important;bottom:0!important;width:100vw!important;',
    'box-sizing:border-box!important;height:' + RESERVE + '!important;',
    'padding-bottom:' + SAFE + '!important;',
    'margin:0!important;',
    'z-index:2147483646!important;',
    'display:flex!important;align-items:stretch!important;gap:0!important;',
    'background:#eef0f2!important;color:#2c3440!important;',
    'border-top:1px solid rgba(0,0,0,.08)!important;',
    'font:12px/1 -apple-system,"Segoe UI","Microsoft YaHei",sans-serif!important;',
    '-webkit-user-select:none!important;user-select:none!important;',
    '}',
    '@media (prefers-color-scheme:dark){#__web2app_mobilebar__{',
    'background:#1f2530!important;color:#e6ebf2!important;border-top-color:rgba(255,255,255,.07)!important;}}',
    '#__web2app_mobilebar__ *{box-sizing:border-box!important;margin:0!important;padding:0!important;}',
    /* 按钮：整格铺满条高（触摸目标 ≥ 44px），反馈走 :active 而不是 :hover */
    '#__web2app_mobilebar__ .w2m-btn{',
    'flex:1 1 0!important;min-width:0!important;height:100%!important;min-height:44px!important;',
    'display:flex!important;flex-direction:column!important;align-items:center!important;' +
      'justify-content:center!important;gap:2px!important;',
    'border:0!important;border-radius:0!important;background:transparent!important;',
    'color:inherit!important;font:inherit!important;cursor:pointer!important;',
    '-webkit-tap-highlight-color:transparent!important;touch-action:manipulation!important;',
    '}',
    '#__web2app_mobilebar__ .w2m-btn:active{background:rgba(120,133,153,.22)!important;}',
    '#__web2app_mobilebar__ .w2m-btn:disabled{opacity:.35!important;}',
    '#__web2app_mobilebar__ .w2m-ico{',
    'display:flex!important;align-items:center!important;justify-content:center!important;height:18px!important;}',
    '#__web2app_mobilebar__ .w2m-txt{font-size:10px!important;line-height:1!important;opacity:.9!important;}',
    /* 页签面板：从底栏上沿升起（移动端习惯）。遮罩负责「点外面关掉」。 */
    '#__web2app_mobilebar__ .w2m-mask{',
    'position:fixed!important;left:0!important;right:0!important;top:0!important;bottom:0!important;',
    'background:rgba(0,0,0,.32)!important;z-index:2147483645!important;}',
    '#__web2app_mobilebar__ .w2m-sheet{',
    'position:fixed!important;left:0!important;right:0!important;bottom:' + RESERVE + '!important;',
    'max-height:50vh!important;overflow-y:auto!important;',
    'background:#eef0f2!important;color:#2c3440!important;',
    'border-top:1px solid rgba(0,0,0,.08)!important;',
    'z-index:2147483646!important;}',
    '@media (prefers-color-scheme:dark){#__web2app_mobilebar__ .w2m-sheet{',
    'background:#1f2530!important;color:#e6ebf2!important;border-top-color:rgba(255,255,255,.07)!important;}}',
    '#__web2app_mobilebar__ .w2m-row{',
    'display:flex!important;align-items:center!important;gap:8px!important;',
    'width:100%!important;min-height:44px!important;padding:10px 14px!important;',
    'border:0!important;border-bottom:1px solid rgba(0,0,0,.06)!important;',
    'background:transparent!important;color:inherit!important;font:inherit!important;text-align:left!important;',
    '-webkit-tap-highlight-color:transparent!important;touch-action:manipulation!important;}',
    '#__web2app_mobilebar__ .w2m-row:active{background:rgba(120,133,153,.22)!important;}',
    '#__web2app_mobilebar__ .w2m-row.active{font-weight:600!important;}',
    '#__web2app_mobilebar__ .w2m-row .w2m-label{',
    'flex:1!important;min-width:0!important;font-size:13px!important;',
    'white-space:nowrap!important;overflow:hidden!important;text-overflow:ellipsis!important;}',
    '#__web2app_mobilebar__ .w2m-row .w2m-ico{flex:none!important;opacity:.75!important;height:14px!important;}',
    '#__web2app_mobilebar__ .w2m-close{',
    'flex:none!important;width:32px!important;height:32px!important;',
    'display:flex!important;align-items:center!important;justify-content:center!important;',
    'border:0!important;border-radius:50%!important;background:transparent!important;color:inherit!important;}',
    '#__web2app_mobilebar__ .w2m-close:active{background:rgba(120,133,153,.22)!important;}',
    '#__web2app_mobilebar__ .w2m-empty{',
    'padding:14px!important;font-size:12px!important;opacity:.6!important;}',
    /* ================== 底部空间契约 ==================
     * 与桌面端顶部那条镜像：`html` 不滚、`body` 滚，滚动矩形从底部让出一格底栏。
     * ⚠️ 必须用 margin-bottom 而不是 padding-bottom：padding 在盒子内部，滚动条仍从盒子
     *    下沿开始画（底部留白会被滚动条穿过去）。
     * ⚠️ body 的 min-height 必须归零：页面常写 `min-height:100vh`，它会把 body 撑高一格底栏，
     *    于是最后一段内容滚不到。 */
    'html{box-sizing:border-box!important;height:100%!important;overflow:hidden!important;}',
    'body{box-sizing:border-box!important;margin-bottom:' + RESERVE + '!important;',
    'min-height:0!important;height:' + FIT_H + '!important;overflow-y:auto!important;',
    'scroll-behavior:inherit!important;}',
    '[' + SHIFT_ATTR + ']{bottom:' + RESERVE + '!important;}',
    '[' + SHRINK_ATTR + ']{height:' + FIT_H + '!important;min-height:' + FIT_H + '!important;}'
  ].join('\n');

  // 内置页面（启动页）判定：两种 URL 形态都要认 —— 原生 `splash://` scheme，与 wry 改写后的
  // `http://splash.localhost/`（custom_protocol_workaround，见 bootstrap.rs）。启动页不画底栏。
  function isBuiltinUrl(u) {
    if (!u) return false;
    try {
      return /^splash:/i.test(u) || /^https?:\/\/splash\.localhost(?:[/?#]|$)/i.test(u);
    } catch (e) {
      return false;
    }
  }

  /// 前进/后退落点判定：{ back: 上一条是真实网页, fwd: 下一条是真实网页 }。
  /// 与桌面端 `titlebar.js::navDirections()` **同一条规则**：没有 Navigation API（极老内核）
  /// 时无法证明落点不是内置页 ⇒ 两个方向一律按「不可走」处理（宁可按钮失效，也不退回启动页）。
  function navDirections() {
    try {
      var n = window.navigation;
      if (n && n.currentEntry && typeof n.currentEntry.index === 'number') {
        var i = n.currentEntry.index;
        var entries = n.entries();
        function realAt(j) {
          var e = entries[j];
          return !!e && !isBuiltinUrl(e.url);
        }
        return { back: i > 0 && realAt(i - 1), fwd: realAt(i + 1) };
      }
    } catch (e) { /* noop */ }
    return { back: false, fwd: false };
  }

  /// 发核心事件（远程 origin 唯一可用的通路）。
  /// 失败一律打到控制台：静默的话用户只看到「点了没反应」，排查得从零开始。
  function emit(event, payload) {
    var evt = event;
    try {
      var T = window.__TAURI__;
      if (T && T.event && typeof T.event.emit === 'function') {
        var r = T.event.emit(evt, payload);
        if (r && r.then) {
          r.then(null, function (err) {
            try { console.error('[web2app] 事件 ' + evt + ' 被拒：', err); } catch (e) { /* noop */ }
          });
        }
        return;
      }
      // 兜底：直接走底层 invoke（withGlobalTauri 关掉时 __TAURI__.event 不存在）。
      var I = window.__TAURI_INTERNALS__;
      if (I && typeof I.invoke === 'function') {
        I.invoke('plugin:event|emit', { event: evt, payload: payload });
        return;
      }
    } catch (e) {
      try { console.error('[web2app] 发事件 ' + evt + ' 抛异常：', e); } catch (e2) { /* noop */ }
      return;
    }
    try {
      console.error(
        '[web2app] 无法发事件 ' + evt +
          '：壳控制器未就绪。查 __TAURI__ 与 capabilities 的 core:event:allow-emit。'
      );
    } catch (e) { /* noop */ }
  }

  // ---------------------------------------------------------------------
  // DOM 宿主等待（与 titlebar.js 同源的问题）
  // ---------------------------------------------------------------------
  // 本脚本由 `AddScriptToExecuteOnDocumentCreated`（Android 侧同理）注入，执行时机是
  // 「全局对象已创建、**HTML 文档尚未解析**」——此刻 `document.body` / `documentElement`
  // 都可能为 null。历史实现直接 appendChild ⇒ 真实页面上必然抛 TypeError、被最外层
  // try/catch 静默吞掉 ⇒ 顶栏从不出现。这里改为「宿主节点一出现就挂载」。
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
      try { if (obs) obs.disconnect(); } catch (e) { /* noop */ }
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
    } catch (e) { /* 老引擎：靠下面的时机 + 定时器兜底 */ }
    try {
      document.addEventListener('readystatechange', attempt);
      document.addEventListener('DOMContentLoaded', attempt);
      window.addEventListener('load', attempt);
    } catch (e) { /* noop */ }
    iv = setInterval(attempt, 16);
  }

  function adoptSheet(sheet) {
    var next = [];
    var cur = document.adoptedStyleSheets || [];
    for (var i = 0; i < cur.length; i++) next.push(cur[i]);
    next.push(sheet);
    document.adoptedStyleSheets = next;
  }

  /// 装载样式表：优先「构造函数式样式表」（不受页面 CSP 的 style-src 约束），回退 <style>。
  function mountStyle() {
    try {
      if (typeof CSSStyleSheet === 'function' && document.adoptedStyleSheets) {
        var sheet = new CSSStyleSheet();
        sheet.replaceSync(CSS);
        adoptSheet(sheet);
        return { kind: 'adopted', sheet: sheet };
      }
    } catch (e) { /* 落到 <style> 回退 */ }
    var el = document.createElement('style');
    el.setAttribute('data-web2app', 'mobilebar');
    el.textContent = CSS;
    (document.head || document.documentElement).appendChild(el);
    return { kind: 'style', el: el };
  }

  function styleAttached(h) {
    try {
      if (!h) return true;
      if (h.kind === 'style') return !!h.el.parentNode;
      var list = document.adoptedStyleSheets || [];
      for (var i = 0; i < list.length; i++) if (list[i] === h.sheet) return true;
      return false;
    } catch (e) { return true; }
  }

  function styleReattach(h) {
    try {
      if (!h) return;
      if (h.kind === 'style') {
        (document.head || document.documentElement).appendChild(h.el);
      } else {
        adoptSheet(h.sheet);
      }
    } catch (e) { /* noop */ }
  }

  /// 把宽度钉到视口宽（CSS 里已有 100vw，这里再钉一次是为了「确定性」：
  /// 100vw == innerWidth 依赖引擎实现细节，显式写 px 之后它变成可测可断言的事实）。
  function syncBarWidth(bar) {
    try {
      var b = bar && bar.style ? bar : document.getElementById('__web2app_mobilebar__');
      if (!b || !b.style) return;
      var w = 0;
      try { w = window.innerWidth || 0; } catch (e) { w = 0; }
      if (w > 0) b.style.setProperty('width', w + 'px', 'important');
    } catch (e) { /* noop */ }
  }

  // ---------------------------------------------------------------------
  // 底部空间契约的 JS 半边
  // ---------------------------------------------------------------------
  // ① 探测契约是否真的生效；被页面压掉就退回「html 底部留白 + 页面自己滚」的旧模式。
  // ② 把页面自己的贴底固定条 / 100vh 容器一起适配（见 CSS 里的两条属性选择器）。
  //
  // ⚠️ 探测必须在样式表挂上之后跑：早一步跑，computed style 还是页面原样，探测必然判失败、
  //    并把退路写死在 inline style 上（inline !important 会永久压住我们的样式表）。
  var bodyScrollOK = false;

  function ensureScrollRoot(h) {
    try {
      var de = document.documentElement;
      var b = document.body;
      if (!de || !b) return;
      if (h && !styleAttached(h)) return;
      var hcs = window.getComputedStyle(de);
      var bcs = window.getComputedStyle(b);
      var rootHidden = hcs.overflowY === 'hidden' || hcs.overflowY === 'clip';
      var bodyScrolls = bcs.overflowY === 'auto' || bcs.overflowY === 'scroll';
      var want = (window.innerHeight || 0) - reservePx();
      var got = b.getBoundingClientRect().height;
      bodyScrollOK = rootHidden && bodyScrolls && want > 0 && Math.abs(got - want) <= 2;
      if (bodyScrollOK) return;
      // ---- 退路：宁可底栏压住页面底部的一小条，也不能让页面滚不动 ----
      b.style.setProperty('margin-bottom', '0px', 'important');
      b.style.setProperty('height', 'auto', 'important');
      b.style.setProperty('min-height', '0px', 'important');
      b.style.setProperty('overflow-y', 'visible', 'important');
      de.style.setProperty('height', 'auto', 'important');
      de.style.setProperty('overflow-y', 'auto', 'important');
      de.style.setProperty('padding-bottom', reservePx() + 'px', 'important');
      try {
        console.error('[web2app] 底部滚动根契约未生效（html overflow=' + hcs.overflowY +
          ' body overflowY=' + bcs.overflowY + '），已退回 html 底部留白模式。');
      } catch (e) { /* noop */ }
    } catch (e) { /* 探测本身失败：按现状继续，绝不打扰宿主页面 */ }
  }

  /// 底栏实际占掉的高度（内容区 + 安全区）。唯一实测来源：底栏自身的盒子高度。
  function reservePx() {
    try {
      var b = document.getElementById('__web2app_mobilebar__');
      if (b) {
        var r = b.getBoundingClientRect();
        if (r.height > 0) return Math.round(r.height);
      }
    } catch (e) { /* noop */ }
    return BAR_H;
  }

  /// 让页面自己的「贴底固定条」上移、把「按视口高度定尺寸的容器」缩高一格底栏。
  /// 两阶段（先只读收集、再统一写）：边读边写会让每次 getBoundingClientRect 触发重排。
  function applyChromeFit() {
    try {
      var root = document.body || document.documentElement;
      if (!root) return;
      var de = document.documentElement;
      var vw = de.clientWidth || window.innerWidth || 0;
      var vh = de.clientHeight || window.innerHeight || 0;
      if (!vw || !vh) return;
      var reserve = reservePx();
      var all = root.getElementsByTagName('*');
      var shifts = [];
      var shrinks = [];

      for (var i = 0; i < all.length; i++) {
        var el = all[i];
        var id = el.id;
        if (id === '__web2app_mobilebar__' || id === '__web2app_loading__') continue;
        if (el === de) continue;
        var needShift = !el.hasAttribute(SHIFT_ATTR);
        var needShrink = !el.hasAttribute(SHRINK_ATTR);
        if (!needShift && !needShrink) continue;

        var r = el.getBoundingClientRect();
        if (r.width < vw * 0.5 || r.height <= 0) continue;
        // 贴底：底边落在视口底附近（含已被系统键盘 / 安全区推上来的余量）
        var atBottom = needShift && r.bottom >= vh - Math.max(reserve + 2, 8) && r.bottom <= vh + 2;
        // 整屏：高度≈视口高且底边已到/越过视口底（100vh 容器被顶出可视区）
        var viewportTall = needShrink &&
          r.height >= vh - VH_TOL && r.height <= vh + VH_TOL && r.bottom >= vh - VH_TOL;
        if (!atBottom && !viewportTall) continue;

        var cs = window.getComputedStyle(el);
        var pos = cs.position;
        if (atBottom && (pos === 'fixed' || pos === 'sticky' ||
            // absolute 只认「底边真的压在视口底」的：有定位祖先时它的包含块不是视口，
            // 放宽到一整格底栏的范围内会把页面内部的元素误判成贴底条（与 titlebar.js
            // 处理顶部 absolute 时同一条谨慎规则）。
            (pos === 'absolute' && r.bottom >= vh - 2))) {
          // 声明式 bottom 已经比底栏高的（页面自己让开了）不动。
          if (cs.bottom === 'auto' || parseFloat(cs.bottom) < reserve) shifts.push(el);
        }
        if (needShrink && viewportTall && cs.bottom === 'auto') shrinks.push(el);
      }

      for (var j = 0; j < shrinks.length; j++) {
        try {
          shrinks[j].setAttribute(SHRINK_ATTR, '');
          shrinks[j].style.setProperty('height', FIT_H, 'important');
          shrinks[j].style.setProperty('min-height', FIT_H, 'important');
        } catch (e) { /* 单个元素失败不影响其余 */ }
      }
      for (var k = 0; k < shifts.length; k++) {
        try {
          shifts[k].setAttribute(SHIFT_ATTR, '');
          shifts[k].style.setProperty('bottom', RESERVE, 'important');
        } catch (e) { /* noop */ }
      }
    } catch (e) { /* 绝不把异常抛给宿主页面 */ }
  }

  function scheduleChromeFit() {
    var MAX_RUNS = 20;
    var MIN_GAP = 300;
    var runs = 0;
    var last = 0;
    var mo = null;

    function run() {
      if (runs >= MAX_RUNS) {
        try { if (mo) mo.disconnect(); } catch (e) { /* noop */ }
        return;
      }
      var now = Date.now();
      if (now - last < MIN_GAP) return;
      last = now;
      runs++;
      applyChromeFit();
    }

    run();
    try { document.addEventListener('DOMContentLoaded', run); } catch (e) { /* noop */ }
    try { window.addEventListener('load', function () { run(); }); } catch (e) { /* noop */ }
    try {
      mo = new MutationObserver(run);
      mo.observe(document.documentElement, { childList: true, subtree: true });
    } catch (e) { /* 老引擎：靠上面的时机 + 定时器兜底 */ }
    try {
      setTimeout(run, 1000);
      setTimeout(run, 3000);
    } catch (e) { /* noop */ }
  }

  // ---------------------------------------------------------------------
  // 底栏本体
  // ---------------------------------------------------------------------
  /// 按 cfg 生成按钮 HTML。⚠️「列表」按钮只在 HAS_TABS 时才出现 —— 移动端目前没有多页签，
  /// 无条件画它就是一个点了没反应的死按钮（本文件的核心约束之一）。
  function barHtml() {
    var items = [
      btn('home', ICO_HOME, '首页', '首页'),
      btn('back', ICO_BACK, '后退', '后退'),
      btn('fwd', ICO_FWD, '前进', '前进')
    ];
    if (HAS_TABS) items.push(btn('list', ICO_LIST, '列表', '已打开的标签'));
    return items.join('');
  }

  function btn(kind, icon, text, aria) {
    return '<button type="button" class="w2m-btn w2m-' + kind + '" aria-label="' + aria + '">' +
      '<span class="w2m-ico">' + icon + '</span>' +
      '<span class="w2m-txt">' + text + '</span>' +
      '</button>';
  }

  /// 页签行数据（壳侧经 eval 推送）：[[label, title, active, home], …]，
  /// 与多页签 `get_tabs` 的**四元组**同形（home = 首页标记：不可关闭、名字前带房子图标）。
  var tabRows = [];

  function setTabs(rows) {
    tabRows = Array.isArray(rows) ? rows : [];
    if (sheetOpen) renderSheet();
  }

  var sheetOpen = false;

  function closeSheet(bar) {
    sheetOpen = false;
    try {
      var mask = bar.querySelector('.w2m-mask');
      if (mask) mask.remove();
      var sheet = bar.querySelector('.w2m-sheet');
      if (sheet) sheet.remove();
      var listBtn = bar.querySelector('.w2m-list');
      if (listBtn) listBtn.style.removeProperty('background');
    } catch (e) { /* noop */ }
  }

  function renderSheet() {
    var bar = document.getElementById('__web2app_mobilebar__');
    if (!bar) return;
    var old = bar.querySelector('.w2m-sheet');
    if (old) old.remove();

    var sheet = document.createElement('div');
    sheet.className = 'w2m-sheet';
    if (!tabRows.length) {
      var empty = document.createElement('div');
      empty.className = 'w2m-empty';
      empty.textContent = '暂无其它标签';
      sheet.appendChild(empty);
      bar.appendChild(sheet);
      return;
    }

    tabRows.forEach(function (row) {
      var label = row && row[0];
      var title = (row && row[1]) || label || '';
      var active = !!(row && row[2]);
      var home = !!(row && row[3]);

      var item = document.createElement('div');
      item.className = 'w2m-row' + (active ? ' active' : '');

      // 切换：整行是一个大触摸目标（关闭按钮是行内独立的小目标）
      var tap = document.createElement('button');
      tap.type = 'button';
      tap.className = 'w2m-row-tap';
      tap.style.cssText =
        'display:flex!important;align-items:center!important;gap:8px!important;flex:1!important;' +
        'min-width:0!important;background:transparent!important;border:0!important;color:inherit!important;' +
        'font:inherit!important;text-align:left!important;padding:0!important;min-height:44px!important;';
      tap.addEventListener('click', function () {
        emit('web2app:switch-tab', { label: label });
        closeSheet(bar);
      });

      if (home) {
        var ico = document.createElement('span');
        ico.className = 'w2m-ico';
        ico.innerHTML = ICO_HOME;
        tap.appendChild(ico);
      }
      var text = document.createElement('span');
      text.className = 'w2m-label';
      text.textContent = title;
      tap.appendChild(text);
      item.appendChild(tap);

      // ⚠️ 首页不画关闭按钮（与桌面端标签栏同规则：首页是入口，关不掉）
      if (!home) {
        var close = document.createElement('button');
        close.type = 'button';
        close.className = 'w2m-close';
        close.setAttribute('aria-label', '关闭标签');
        close.innerHTML = ICO_CLOSE;
        close.addEventListener('click', function (e) {
          try { e.stopPropagation(); } catch (err) { /* noop */ }
          emit('web2app:close-tab', { label: label });
          closeSheet(bar);
        });
        item.appendChild(close);
      }

      sheet.appendChild(item);
    });

    bar.appendChild(sheet);
  }

  function openSheet(bar) {
    sheetOpen = true;
    try {
      var mask = document.createElement('div');
      mask.className = 'w2m-mask';
      mask.addEventListener('click', function () { closeSheet(bar); });
      bar.appendChild(mask);
      renderSheet();
      var listBtn = bar.querySelector('.w2m-list');
      if (listBtn) listBtn.style.setProperty('background', 'rgba(120,133,153,.22)', 'important');
    } catch (e) { /* noop */ }
  }

  function wire(bar) {
    var backBtn = bar.querySelector('.w2m-back');
    var fwdBtn = bar.querySelector('.w2m-fwd');

    function onNav(direction) {
      try {
        var dirs = navDirections();
        if (direction === 'back') {
          if (dirs.back) window.history.back();
        } else if (dirs.fwd) {
          window.history.forward();
        }
      } catch (e) { /* noop */ }
    }

    // 首页：走核心事件由壳导航（理由见文件头）。页面手里没有最终地址（可能要解密 / 走代理）。
    bar.querySelector('.w2m-home').addEventListener('click', function () {
      emit('web2app:nav-home', null);
    });
    backBtn.addEventListener('click', function () { onNav('back'); });
    fwdBtn.addEventListener('click', function () { onNav('forward'); });

    function refreshNavState() {
      var d = navDirections();
      backBtn.disabled = !d.back;
      fwdBtn.disabled = !d.fwd;
    }
    refreshNavState();
    try {
      var navApi = window.navigation;
      if (navApi && typeof navApi.addEventListener === 'function') {
        navApi.addEventListener('navigatesuccess', refreshNavState);
        navApi.addEventListener('navigateerror', refreshNavState);
      }
    } catch (e) { /* 不影响按钮点击本身 */ }

    var listBtn = bar.querySelector('.w2m-list');
    if (listBtn) {
      listBtn.addEventListener('click', function () {
        if (sheetOpen) closeSheet(bar);
        else openSheet(bar);
      });
    }
  }

  /// 宣告「底部已被底栏占用」（外部探针 / 同世界脚本读它，比全局变量更稳）。
  function markChromeReady() {
    try {
      var de = document.documentElement;
      if (de) {
        de.setAttribute('data-web2app-mobile-bar-h', String(reservePx()));
        de.setAttribute('data-web2app-mobile-bar-tabs', HAS_TABS ? '1' : '0');
      }
    } catch (e) { /* noop */ }
  }

  function mount() {
    var style = mountStyle();
    var bar = document.createElement('div');
    bar.id = '__web2app_mobilebar__';
    bar.innerHTML = barHtml();
    (document.body || document.documentElement).appendChild(bar);
    return { style: style, bar: bar };
  }

  function ensureHost(bar) {
    try {
      if (document.body && bar.parentNode !== document.body) document.body.appendChild(bar);
    } catch (e) { /* 挪动失败不影响 fixed 定位的显示效果 */ }
  }

  // 立刻置位防重入：挂载是异步的（要等 DOM 宿主节点），期间可能被再次注入。
  window.__web2appMobileBar = { setTabs: setTabs };

  // 启动页（内置页）不画底栏：那时还没有可前进 / 后退的真实页面，画出来只会误导。
  // ⚠️ 这里不 return 之前已置位的防重标记 —— 换文档后本脚本会被重新注入，新的 JS 环境里
  //    这段判断会重新跑一遍（导航到真实页面时 location 已不是内置页）。
  if (isBuiltinUrl(location.href)) return;

  whenHostReady(function () {
    var mounted = null;
    try {
      mounted = mount();
      wire(mounted.bar);
    } catch (e) {
      // 极端文档挂载失败：绝不破坏宿主页面。撤销防重标记，让后续注入仍有机会补挂。
      window.__web2appMobileBar = null;
    }

    ensureScrollRoot(mounted ? mounted.style : null);
    scheduleChromeFit();

    if (!mounted) return;
    var bar = mounted.bar;
    var style = mounted.style;

    syncBarWidth(bar);
    try {
      window.addEventListener('resize', function () { syncBarWidth(bar); }, { passive: true });
      window.addEventListener('orientationchange', function () { syncBarWidth(bar); }, { passive: true });
    } catch (e) { /* noop */ }

    try {
      if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', function () {
          ensureHost(bar);
          syncBarWidth(bar);
        }, { once: true });
      } else {
        ensureHost(bar);
        syncBarWidth(bar);
      }
    } catch (e) { /* noop */ }

    markChromeReady();

    // ---------------------------------------------------------------------
    // 自愈守护（SPA 清空并重建 <body> 时把底栏挂回去）
    // ---------------------------------------------------------------------
    // 与 titlebar.js 同一类问题：前端框架整片替换 DOM 会顺手把我们挂进去的节点抹掉，
    // 而**同文档**的 DOM 重建不会换掉 JS 环境 ⇒ 本脚本仍在、可以低频自检补挂。
    // 整页导航会换掉 JS 环境，由新文档的初始化脚本重新注入，不归这里管。
    (function guard() {
      var tries = 0;
      var iv = setInterval(function () {
        try {
          if (!document.documentElement) return;
          var root = document.body || document.head || document.documentElement;
          if (!root) return;
          var lostBar = !document.documentElement.contains(bar);
          var lostStyle = !styleAttached(style);
          if (!lostBar && !lostStyle) return;
          if (++tries > 20) {
            clearInterval(iv);
            return;
          }
          if (lostStyle) {
            styleReattach(style);
            ensureScrollRoot(style);
          }
          if (lostBar) {
            root.appendChild(bar);
            syncBarWidth(bar);
          }
          markChromeReady();
        } catch (e) { /* noop */ }
      }, 800);
    })();
  });
})();
