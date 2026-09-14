/*
 * web2app 自绘窗口标题栏（titlebar.js）
 * ---------------------------------------------------------------------------
 * 仅单窗口模式（TABS_ENABLED=false，Windows）注入到主窗【每一个】文档（document-created），
 * 因为主窗加载的是远程 Trilium 页，持久置顶的标题栏只能经 initialization_script 注入其页面。
 *
 * 提供的标题栏（高 38px，与多页签宿主 shell.html 的标签栏一致）：
 *      [← 后退] [→ 前进]          标题                [— 最小化] [▢ 最大化] [✕ 关闭]
 *   - 拖动/双击最大化交给 Tauri 的 drag region（`data-tauri-drag-region` 属性，
 *     由 tauri 注入的 drag.js 处理）；Windows WebView2 上 `-webkit-app-region` 不可靠，
 *     故以属性为准，CSS 里的 app-region 仅作兼容保留。
 *   - 前进/后退直接驱动当前文档的 window.history（单窗口模式下与 Rust 的 history_nav 等价）。
 *   - 最小化/最大化/关闭经 window.__TAURI__.window.getCurrentWindow() 原生执行。
 *
 * ⚠️⚠️ 权限（capabilities）：主窗加载的是【远程页面】（直连上游域名，或本地反向代理的
 * http://127.0.0.1:<port>），Tauri 2 对非本地 origin 的 IPC 强制走 ACL ——
 * 未命中 capability 的调用会被一律拒绝，前端 catch 后静默，表现为「按钮点了没反应」。
 * 因此 capabilities/default.json 必须声明：
 *   core:window:allow-minimize / allow-toggle-maximize / allow-close /
 *   allow-start-dragging / allow-internal-toggle-maximize，并把 remote.urls 放开到实际站点。
 * 同理，本脚本【不要】去 invoke 应用自定义命令（history_nav / *_host 等）：
 * 那类命令无法在 capability 里授权给远程 origin（需要 build.rs 的 app_manifest），必被拒。
 *
 * 设计约束（沿用 loading.js 的约定，务必保持）：
 *   1. 零依赖、零网络，全部 try/catch：注入到第三方页面，任何异常都不能破坏宿主页面。
 *   2. 同一文档只安装一次（window.__web2appTitlebar 防重）。
 *   3. 关键样式带 !important 并走 ID 选择器，z-index 取极高位，避免被宿主页面样式覆盖。
 *   4. 顶部空间只留【一次】38px（给 <html> 加 padding-top，body 不再重复加），
 *      并且要把页面自己的「贴顶固定条」一起下移 —— 详见下面「顶部空间契约」。
 *
 * ---------------------------------------------------------------------------
 * 顶部空间契约（2026-09-14 重写，别改回 html+body 双 padding）
 * ---------------------------------------------------------------------------
 * 症状（用户截图）：① 顶部出现一整条空白；② 页面自己的头部被切掉半截「显示不全」。
 * 根因两条，都在旧实现 `html,body{padding-top:38px}` 上：
 *   · `html` 与 `body` **各加一次** padding，而 body 位于 html 的内容框内 ⇒ 实际留白
 *     **76px**（不是 38），页面正文被整体推低一条，顶部就露出空白；
 *   · padding 只能移动**正常流**内容，对 `position:fixed` / `sticky` 的页面头部毫无作用：
 *     它仍锚在视口顶（top:0），于是被我们 38px 的标题栏压住 ⇒ 「显示不全」。
 *     （实测：fixed 页头 rect.top 恒为 0，标题栏 rect 0..38，恰好盖住它 38px。）
 *
 * 现在的做法：
 *   · 只在 `html` 上加 padding-top（配 `box-sizing:border-box`，让页面 `height:100%`
 *     的高度链仍然等于「视口 - 38」，不会多出 38px 溢出）；
 *   · 再用「标记 + 下移」处理页面自己的贴顶固定条（见 shiftPageTopBars）：
 *     给符合特征的元素加属性并 inline `top:38px!important`，让它们从标题栏下沿开始。
 *     只改 `top`，不动页面结构、不动其它样式，随时可撤。
 *
 * 已知边界：整屏式布局（`position:fixed;inset:0` 那种）不在此列 —— 它高过半个视口，
 * 被刻意排除在识别之外（否则整个应用会被推下去），其顶部 38px 仍会被标题栏遮住。
 * 要做到彻底不遮，只能把 WebView2 的 bounds 本身下移 38px（原生内缩），那是另一套架构，
 * 单窗口模式暂不采用。
 */
(function () {
  'use strict';
  if (window.__web2appTitlebar) return; // 同一文档只安装一次

  var TITLEBAR_H = 38;
  // 页面自己的「贴顶固定条」被识别后打上的标记属性（见 shiftPageTopBars）。
  // 只加属性 + 一条 top 样式，不改结构，出问题时可整批撤销。
  var SHIFT_ATTR = 'data-web2app-chrome-shift';

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
    /* 给远程页预留顶部空间，正文不被盖住。
     * ⚠️ 只给 html 加（旧实现 html,body 都加 → 实际留白 76px，页面顶部多出一条空白）。
     * 配 box-sizing:border-box：页面若用 height:100% 的高度链，内容高度 = 视口 - 38px，
     * 不会多出 38px 溢出；min-height:100% 保证 html 仍然是满高。 */
    'html{box-sizing:border-box!important;padding-top:' + TITLEBAR_H + 'px!important;',
    'min-height:100%!important;}',
    /* 页面自己的贴顶固定条（fixed / sticky 且压在视口顶）由 shiftPageTopBars 标记后下移。
     * 属性选择器特异性低，页面自己写 `#id{top:0!important}` 就能压过它 —— 所以 JS 里
     * 同时写 inline `top:...!important`（内联 important 优先级最高），这条只作兜底。 */
    '[' + SHIFT_ATTR + ']{top:' + TITLEBAR_H + 'px!important;}'
  ].join('\n');

  // `data-tauri-drag-region`：Tauri 的 drag.js 只认这个属性（Windows WebView2 上
  // `-webkit-app-region` 不可靠）。空值语义是「只有直接点在本元素上才拖动」，
  // 所以按钮不加（避免点不动），标题与整条空白区各自带上。
  var HTML = [
    '<button type="button" class="w2t-btn w2t-back" title="后退">←</button>',
    '<button type="button" class="w2t-btn w2t-fwd" title="前进">→</button>',
    '<div class="w2t-title" data-tauri-drag-region> </div>',
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
  // 且不受隐藏窗口的定时器节流影响），readystatechange / DOMContentLoaded / load 与 interval 兜底。
  //
  // ⚠️ 兜底**不要**写成「setTimeout 3s 后只试一次」：那一次尝试若仍拿不到宿主节点就直接返回，
  // 而 `__web2appTitlebar` 已经是 true → 本脚本从此再也不会挂载（标题栏永久消失）。
  // 现在的写法是「持续重试到 20s，期间任何一个宿主节点出现就立刻挂载」。
  // ---------------------------------------------------------------------
  // 把页面自己的「贴顶固定条」下移一个标题栏高度
  // ---------------------------------------------------------------------
  // 为什么必须做：我们只是往页面里插了一个 fixed 的标题栏（占 0..38px），
  // 而页面自己的头部若是 `position:fixed;top:0` / `sticky;top:0`，**padding 推不动它** ——
  // 它依旧锚在视口顶，被标题栏压住上半截（用户截图里的「显示不全」）。
  //
  // 识别条件刻意保守，只认「宽、扁、正贴着视口顶」的 fixed/sticky 元素：
  //   · 宽度 ≥ 视口一半（顶部条的基本特征）
  //   · 高度 >0 且 ≤ 视口一半（排除整屏浮层/遮罩，它们被推下去才是灾难）
  //   · rect.top 在 [-1, 38+8] 内（此刻确实在顶部；sticky 未贴顶时它会被我们的
  //     padding 推到 ~38，仍在窗口内，所以同样能识别到）
  //   · 声明式 top 小于标题栏高度（页面自己已经躲开的，别去动）
  // 命中后写 inline `top:38px!important`（内联 important 优先级最高，能压过页面
  // 自己带 !important 的规则），并打上 SHIFT_ATTR 便于排查/撤销。
  //
  // ⚠️ 两阶段：先只读（收集命中项），再统一写。边读边写会让每次 getBoundingClientRect
  // 都触发重排，大页面上能把首屏拖慢几十毫秒。
  function shiftPageTopBars() {
    try {
      var root = document.body || document.documentElement;
      if (!root) return;
      var de = document.documentElement;
      var vw = de.clientWidth || window.innerWidth || 0;
      var vh = de.clientHeight || window.innerHeight || 0;
      if (!vw || !vh) return;

      var all = root.getElementsByTagName('*');
      var hits = [];
      for (var i = 0; i < all.length; i++) {
        var el = all[i];
        if (el.hasAttribute(SHIFT_ATTR)) continue;          // 已经处理过
        var id = el.id;
        if (id === '__web2app_titlebar__' || id === '__web2app_loading__') continue; // 我们自己的
        // 先用几何特征筛掉绝大多数元素（getBoundingClientRect 便宜），
        // 只有少数候选才去读 computed style（那才是开销大头）。
        var r = el.getBoundingClientRect();
        if (r.width < vw * 0.5) continue;
        if (r.height <= 0 || r.height > vh * 0.5) continue;
        if (r.top < -1 || r.top > TITLEBAR_H + 8) continue;
        // 到这里候选已经很少，再读 computed style
        var cs = window.getComputedStyle(el);
        var p = cs.position;
        if (p !== 'fixed' && p !== 'sticky') continue;
        var t = cs.top;
        if (t === 'auto') {
          // 没声明 top：只有确实贴在视口顶才认（否则是按静态位置摆放的普通 fixed 元素）
          if (r.top > 2) continue;
        } else if (parseFloat(t) >= TITLEBAR_H) {
          continue;                                        // 页面自己已经把 top 让开了
        }
        hits.push(el);
      }

      for (var j = 0; j < hits.length; j++) {
        try {
          hits[j].setAttribute(SHIFT_ATTR, '');
          hits[j].style.setProperty('top', TITLEBAR_H + 'px', 'important');
        } catch (e) { /* 单个元素失败不影响其余 */ }
      }
    } catch (e) { /* 绝不把异常抛给宿主页面 */ }
  }

  // 调度：定时器在「被遮挡窗口」里会被 Chromium 节流到几乎不跑，所以主力是
  // 「DOM 事件 + MutationObserver」（观察者回调不依赖定时器），定时器只作兜底补扫。
  // 总次数封顶，避免与频繁改 DOM 的页面互相拉扯。
  function scheduleShiftPasses() {
    // 上限的取舍：SPA 换页/框架重建头部都发生在「结构性 DOM 变化」之后，观察者回调不受
    // 定时器节流影响，所以次数给够（40 次 × 最小间隔 300ms ≈ 覆盖最后 12s 内的变化）；
    // 又不能无限跑，否则与持续改 DOM 的页面互相拉扯。超过上限就不再补扫（已知边界）。
    var MAX_RUNS = 40;
    var MIN_GAP = 300;      // 两次扫描的最小间隔（ms），防抖动
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
      shiftPageTopBars();
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

  function mount() {
    var styleEl = document.createElement('style');
    styleEl.setAttribute('data-web2app', 'titlebar');
    styleEl.textContent = CSS;
    (document.head || document.documentElement).appendChild(styleEl);

    var bar = document.createElement('div');
    bar.id = '__web2app_titlebar__';
    // 整条空白区可拖动：drag.js 的 isDragRegion 对「空属性」要求点击目标就是本元素，
    // 按钮因不属于 drag region（且属 clickable）仍可正常点击。
    bar.setAttribute('data-tauri-drag-region', '');
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
    var win = null;
    try {
      win = window.__TAURI__ && window.__TAURI__.window && window.__TAURI__.window.getCurrentWindow();
    } catch (e) { /* 个别环境拿不到窗口句柄时，前进/后退仍可工作 */ }

    // ⚠️ 前进/后退【直接】驱动当前文档的历史，不要改回 invoke 自定义命令：
    // 主窗加载的是远程页面（非 local origin），Tauri 2 会强制走 ACL 校验，而应用自定义命令
    // 无法授权给远程 origin → 调用必被拒；前端又不能把异常抛给宿主页面，只能 catch 掉，
    // 结果就是「点了没反应」。单窗口模式下用 window.history 与 Rust 的 history_nav 完全等价
    //（后者也只是在目标 webview 里执行同样的调用，多页签模式才需要它来定位目标 tab）。
    function onNav(direction) {
      try {
        if (direction === 'back') {
          window.history.back();
        } else {
          window.history.forward();
        }
      } catch (e) { /* noop */ }
    }

    bar.querySelector('.w2t-back').addEventListener('click', function () { onNav('back'); });
    bar.querySelector('.w2t-fwd').addEventListener('click', function () { onNav('forward'); });

    if (win) {
      bar.querySelector('.w2t-min').addEventListener('click', function () { win.minimize().catch(function () {}); });
      bar.querySelector('.w2t-max').addEventListener('click', function () { win.toggleMaximize().catch(function () {}); });
      bar.querySelector('.w2t-close').addEventListener('click', function () { win.close().catch(function () {}); });
      // 双击标题栏空白区最大化/还原由 Tauri 的 drag region 负责（drag.js 在 mousedown
      // detail===2 时调用 plugin:window|internal_toggle_maximize）。这里【不要】再自绑 dblclick：
      // 一次双击会触发两次切换 = 看起来没反应。
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
    // 页面自己的贴顶固定条（fixed / sticky）必须一起下移，否则会被标题栏压住。
    // 这一步不依赖标题栏挂载成功与否，所以先启动它（内部自带 try/catch 与次数上限）。
    scheduleShiftPasses();

    var mounted;
    try {
      mounted = mount();
      wire(mounted.bar);
    } catch (e) {
      // 极端文档挂载失败：绝不破坏宿主页面。同时撤销防重标记，让后续注入
      //（页面重载、Rust 侧 `on_page_load` 的兜底重注入）仍有机会补挂，
      // 而不是从此彻底放弃。
      window.__web2appTitlebar = false;
      return;
    }
    var bar = mounted.bar;
    var styleEl = mounted.styleEl;

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

    // ---------------------------------------------------------------------
    // 自愈守护（用户反馈「点开其他页面后顶部菜单栏整条消失」的兜底）
    // ---------------------------------------------------------------------
    // 有几类页面会把我们挂进去的节点整片抹掉：
    //   · `document.write()` 重写文档（老式页面 / 某些 SPA 的兜底分支）；
    //   · 前端框架清空并重建 <body>（挂载根节点替换整棵子树）。
    // 这类「同文档」的 DOM 重建**不会**换掉 JS 环境，所以本脚本仍在运行、定时器仍在 ——
    // 正好可以在这里低频自检：节点一旦脱离文档就挂回去。（整页导航会换掉 JS 环境，
    // 新文档由初始化脚本重新注入，不归这里管。）
    // 上限 20 次：避免与持续删除节点的页面互相拉扯；超过即放弃，绝不打扰宿主页面。
    (function guard() {
      var tries = 0;
      var iv = setInterval(function () {
        try {
          if (!document.documentElement) return;
          var root = document.body || document.head || document.documentElement;
          if (!root) return;
          var lostBar = !document.documentElement.contains(bar);
          var lostStyle = !styleEl.parentNode;
          if (!lostBar && !lostStyle) return;
          if (++tries > 20) {
            clearInterval(iv);
            return;
          }
          if (lostStyle) {
            (document.head || document.documentElement).appendChild(styleEl);
          }
          if (lostBar) {
            root.appendChild(bar);
          }
        } catch (e) { /* noop */ }
      }, 800);
    })();
  });
})();