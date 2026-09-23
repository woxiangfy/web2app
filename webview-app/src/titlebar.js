/*
 * web2app 自绘窗口标题栏（titlebar.js）
 * ---------------------------------------------------------------------------
 * 两个壳共用这一份脚本：Tauri 版（webview-app，Windows/Android）与 Electron 绿色版
 * （electron-app，Windows）。差异全部收在下面两处能力探测里，其余逻辑一字不分叉。
 *
 * 提供的标题栏（高 38px，与多页签宿主 shell.html 的标签栏一致）：
 *      [← 后退] [→ 前进]        标题        [— 最小化] [⚙ 设置] [▢ 最大化] [✕ 关闭]
 *   - 拖动 / 双击最大化：
 *       · Tauri（Windows）：交给 drag region（`data-tauri-drag-region` 属性，由 tauri 注入的
 *         drag.js 处理）。WebView2 上 `-webkit-app-region` 不可靠，故以属性为准。
 *       · Electron：靠 CSS 的 `-webkit-app-region: drag`（Chromium 原生支持无边框窗口的自绘
 *         拖动区），双击最大化由 Chromium 把拖动区当作 caption 交给系统处理 —— 所以两个壳都
 *         【不要】自绑 dblclick：一次双击会触发两次切换，看起来像「点了没反应」。
 *   - 前进/后退直接驱动当前文档的 window.history（单窗口模式下与 Rust 的 history_nav 等价）。
 *   - 最小化/最大化/关闭经能力探测拿到的窗口控制器原生执行：
 *       · Electron：`window.web2app`（preload 白名单 API，minimize / toggleMaximize / close）
 *       · Tauri   ：`window.__TAURI__.window.getCurrentWindow()`
 *
 * ⚠️⚠️ 权限（capabilities，**仅 Tauri 版**）：主窗加载的是【远程页面】（直连上游域名，或本地
 * 反向代理的 http://127.0.0.1:<port>），Tauri 2 对非本地 origin 的 IPC 强制走 ACL ——
 * 未命中 capability 的调用会被一律拒绝，前端 catch 后静默，表现为「按钮点了没反应」。
 * 因此 capabilities/default.json 必须声明：
 *   core:window:allow-minimize / allow-toggle-maximize / allow-close /
 *   allow-start-dragging / allow-internal-toggle-maximize，并把 remote.urls 放开到实际站点。
 * 同理，本脚本【不要】去 invoke 应用自定义命令（history_nav / *_host 等）：
 * 那类命令无法在 capability 里授权给远程 origin（需要 build.rs 的 app_manifest），必被拒。
 * ⚠️ 这条约束同样管着「设置」按钮：主窗是远程页面，调不了 `open_settings` 这种应用命令，
 * 所以 Tauri 侧改发**核心插件的 event 命令**（`core:event:allow-emit` 可以授权给远程 origin），
 * Rust 侧 `app.listen("web2app:open-settings")` 收到后开设置窗 —— 见下面的 openSettings()。
 * （Electron 侧没有这套 ACL：`window.web2app` 由 preload 直接暴露给页面，远程 origin 同样可用。
 *   也因此【不要】把这段 ACL 结论套到 Electron 上排查「按钮没反应」，那边要先看 preload 是否注入。）
 *
 * 设计约束（沿用 loading.js 的约定，务必保持）：
 *   1. 零依赖、零网络，全部 try/catch：注入到第三方页面，任何异常都不能破坏宿主页面。
 *   2. 同一文档只安装一次（window.__web2appTitlebar 防重）。
 *   3. 关键样式带 !important 并走 ID 选择器，z-index 取极高位，避免被宿主页面样式覆盖。
 *   4. 顶部空间只留【一次】38px：把 `body` 的盒子整体推到 38px（`margin-top`）并让 body
 *      当滚动容器（`html` 不滚）—— 详见下面「顶部空间契约」。页面自己的「贴顶固定条」要一起下移。
 *
 * ---------------------------------------------------------------------------
 * 顶部空间契约（2026-09-14 重写；2026-09-17 改成「html 不滚、body 滚」）
 * ---------------------------------------------------------------------------
 * 症状（用户截图）：① 顶部出现一整条空白；② 页面自己的头部被切掉半截「显示不全」。
 * 根因两条，都在旧实现 `html,body{padding-top:38px}` 上：
 *   · `html` 与 `body` **各加一次** padding，而 body 位于 html 的内容框内 ⇒ 实际留白
 *     **76px**（不是 38），页面正文被整体推低一条，顶部就露出空白；
 *   · padding 只能移动**正常流**内容，对 `position:fixed` / `sticky` 的页面头部毫无作用：
 *     它仍锚在视口顶（top:0），于是被我们 38px 的标题栏压住 ⇒ 「显示不全」。
 *     （实测：fixed 页头 rect.top 恒为 0，标题栏 rect 0..38，恰好盖住它 38px。）
 *
 * 现在的做法（见 CSS 里那段「顶部空间契约」）：
 *   · `html{height:100%;overflow:hidden}` + `body{margin-top:38px;height:calc(100vh - 38px);
 *     overflow-y:auto}` —— **页面滚动的矩形从 38px 开始**，滚动条因此不可能越过标题栏。
 *     2026-09-17 逐像素对照实验（`scripts/scrollbar-lab.cjs`）确认这是唯一可行解：
 *     根滚动条永远绘制在视口最顶层、几何占满整个视口高，`border-top` / overlay 开关全都无效；
 *     而「把滚动条隐藏掉」又不是使用者要的（他要的是「滚动条在，只是别超过菜单栏」）。
 *   · 页面自己的「贴顶固定条」（fixed）→ 标记 + inline `top:38px!important`（下移，别被压住）。
 *     `sticky` 不再动：body 已是滚动容器，`top:0` 的 sticky 天然停在 y=38，再加就成 76px。
 *   · 页面自己的「按视口高度定尺寸的容器」→ 标记 + inline 高度减 38px（缩高，别被挤出视口底部）。
 *     只改这几条声明，不动页面结构、不动其它样式，随时可撤。
 *   · 换来这套契约的代价：`window` 级滚动 API 不再指向真正的滚动元素，由 shimWindowScroll()
 *     转发到 body（懒加载 / 吸顶导航 / 回到顶部 / 阅读进度都靠它）。若页面把契约压掉，
 *     ensureScrollRoot() 会探测并退回旧的 html 顶部留白模式。
 *
 * ---------------------------------------------------------------------------
 * 底部空间契约（2026-09-16 追加，别再退回「只补顶部」）
 * ---------------------------------------------------------------------------
 * 症状（用户反馈）：顶部修好之后，页面【底部】有一截被窗口下沿吞掉 —— 页面自己的页脚 /
 * 底栏显示不全（「页面底部存在部分被下面覆盖」）。
 * 根因：我们拿掉的是 38px 的【布局高度】，而 CSS 里的 `100vh` / `min-height:100vh` 量的
 * 是【视口高度】—— 视口并没有变小（仍是整窗高），于是：
 *   · 正常流里的 `min-height:100vh` 容器：起点被 padding 推到 38，高度仍是整个视口
 *     ⇒ 底边落到视口下方 38px（实测 #root: top=38 / bottom=688 / viewH=650），页脚被推出可视区；
 *   · `position:fixed;inset:0` 的整屏外壳：padding 对它无效，仍锚在 0..视口底
 *     ⇒ 顶部 38px 被标题栏压住（下面「已知边界」提到的那个）。
 * 做法：对这两类元素各补一条 inline 声明（内联 + !important，能压过页面自己带 !important 的规则）：
 *   · 底部锚定（computed `bottom` 不是 auto）→ 只把 `top` 改成 38px：元素被【缩短】而不是
 *     下移，底边仍钉在视口底 ⇒ 整屏外壳既不压头也不切尾；
 *   · 只给了高度（`height:100vh` / `min-height:100vh`）→ 改 `top` 的同时把高度也减 38px。
 * 只动 `top` / `height` / `min-height` 三个属性；识别条件保守（见 applyChromeFit），
 * 认不出来就什么都不做，页面最多维持原样。
 *
 * 仍存在的边界：真正的「原生内缩」（把 WebView2 的 bounds 本身下移 38px）能让 `100vh`
 * 天然等于「视口 - 38」，但那要求标题栏改由原生绘制（画在 webview 之外），是另一套架构；
 * 单窗口模式继续走「注入 + 逐元素补偿」这条路。
 */
(function () {
  'use strict';
  if (window.__web2appTitlebar) return; // 同一文档只安装一次

  /// 壳能力探测（唯一一处壳分支）。
  ///
  /// Electron 绿色版：preload 用 contextBridge 暴露 `window.web2app`
  /// （minimize / toggleMaximize / close / chromeH / isElectron）；
  /// Tauri 版：`window.__TAURI__`（window.getCurrentWindow()）。
  ///
  /// ⚠️ 探测必须**每次要用时现做**，不要在挂载时把结果缓存进闭包：注入发生在
  /// document-created 之后不久，个别环境下 preload/初始化脚本还没跑完；
  /// 旧实现就是在挂载时缓存句柄、拿不到就整段不绑定 → 「按钮点了没反应」且毫无日志。
  function electronApi() {
    try {
      var E = window.web2app;
      if (E && typeof E.minimize === 'function' && typeof E.close === 'function') return E;
    } catch (e) { /* noop */ }
    return null;
  }

  function shellName() {
    if (electronApi()) return 'electron';
    try { if (window.__TAURI__) return 'tauri'; } catch (e) { /* noop */ }
    return 'unknown';
  }

  var TITLEBAR_H = 38;
  // 标题栏高度以壳侧声明为准（单一真源在 preload / capabilities 里）。
  // 拿不到就沿用 38 —— 两个壳的 CSS 与加载层让位逻辑都按这个值走，绝不各写一份。
  try {
    var apiH = electronApi() && electronApi().chromeH;
    if (typeof apiH === 'number' && apiH > 0) TITLEBAR_H = apiH;
  } catch (e) { /* noop */ }
  // 页面自己的「贴顶固定条」被识别后打上的标记属性（见 applyChromeFit）：只加属性 + 一条
  // top 样式，不改结构，出问题时可整批撤销。
  var SHIFT_ATTR = 'data-web2app-chrome-shift';
  // 页面自己的「按视口高度定尺寸的容器」被识别后打上的标记属性：加属性 + 高度声明。
  var SHRINK_ATTR = 'data-web2app-chrome-shrink';
  // 给这类容器用的高度：视口高度减去标题栏（保持它们原本的「按视口定尺寸」语义）。
  var FIXED_H = 'calc(100vh - ' + TITLEBAR_H + 'px)';
  // 判定「高度≈视口高度」的容差：布局取整 + 滚动条换算，1px 足够。
  var VH_TOL = 1;

  var CSS = [
    '#__web2app_titlebar__{',
    /* ⚠️⚠️ 必须用 width:100vw，**不能**用 left:0;right:0。
     * fixed 元素的 left/right:0 只到 clientWidth（**不含**滚动条），而垂直滚动条属于视口、
     * 从 y=0 就开始画 ⇒ 标题栏右侧会空出一条滚动条宽度，滚动条的向上箭头与轨道顶段正好
     * 裸露在那条缝里，看起来像「标题栏被切掉一角 + 滚动条穿过标题栏」。
     * 实测（W2A-VWCHECK，400×340 窗口、经典 15px 滚动条）：
     *   innerWidth=399  clientWidth=384  scrollbarW=15
     *   固定定位 left+right:0 → 宽 384（差 15px）；width:100vw → 宽 399 ✓
     *   固定定位 width:100%   → 宽 384（同样差 15px）
     * 100vw 含滚动条宽度 ⇒ 标题栏铺满整窗宽（含滚动条本该占的那一条）。
     * 无滚动条时 100vw == clientWidth == 窗口宽，无副作用；WebView2 的 overlay 滚动条
     * 不占布局（clientWidth == innerWidth）时同样无副作用。
     * 注：「滚动条不横穿标题栏」这件事**不由这个宽度保证**，而由下面的「顶部空间契约」
     *（html 不滚、body 滚）保证 —— 见那里 2026-09-17 的实测记录。 */
    'position:fixed!important;top:0!important;left:0!important;width:100vw!important;',
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
    /* ================== 顶部空间契约：让 body 当滚动容器 ==================
     *
     * 目标（使用者原话）：「滚动条不应该强制隐藏，只是不要超过菜单栏而已」：
     * 滚动条**保持可见**，但它的最高点不得越过标题栏下沿（y = 38）。
     *
     * ⚠️⚠️ 为什么不能让 html 继续当滚动容器：Chromium 把**视口（根）滚动条**绘制在所有 DOM
     * 内容之上 —— 标题栏 `z-index:2147483647` 也压不住；而它的几何就是「视口右缘 × 整个视口高」，
     * 任何 CSS 都缩不动。2026-09-17 用逐像素 + 对照实验（`scripts/scrollbar-lab.cjs`）逐一否掉：
     *   · `::-webkit-scrollbar{border-top:38px}`        → thumb 仍从 y=0 起（border 不参与零件布局）
     *   · `::-webkit-scrollbar-track{border-top:38px}`  → 同上，无效
     *   · `--enable-features=OverlayScrollbar` / `FluentOverlayScrollbar` → Electron 44 上均已失效
     *   · `scrollbar-width:none`                        → 缝没了，但滚动条也没了（使用者要的不是这个）
     *   · html 不滚 + body 当滚动容器                   → y0..y32 纯标题栏色、y40 起才是滚动条 ✓
     * 也就是说：只要滚动条存在，它就从 y=0 开始画、必然横穿标题栏，还顺带盖住最右侧的关闭按钮
     *（`elementFromPoint(innerWidth-1, 20)` 返回 null 就是「此处被滚动条占住、点不到」）。
     * 所以只能反过来 —— 让**滚动的那块矩形**自己从 38px 开始：html 不滚，body 滚。
     *
     * body 的滚动条只在 body 盒子内绘制 ⇒ 物理上不可能高于 y=38，不依赖任何引擎语义。
     * 附带收益：关闭按钮右端重新可点；页面自己的 `position:sticky` 也天然停在 y=38
     *（见 applyChromeFit 里 sticky 分支的说明）。
     *
     * ⚠️ 必须用 margin-top 而不是 padding-top：padding 在盒子内部，滚动条仍从盒子上沿开始画。
     * ⚠️ box-sizing:border-box 让 body 自己的 padding/border 不把盒子撑出 38px 之外。
     * ⚠️ body 的 min-height 必须归零：页面常写 `min-height:100vh`，它会把 body 撑高一个标题栏，
     *    于是 body 的滚动口比可视区高 38px、最后 38px 内容滚不到。
     * ⚠️ 页面若用 !important 把这两条压掉，会落到「html 不滚 + body 也不滚 = 页面钉死滚不动」的
     *    死局（比原缺陷严重得多）⇒ JS 侧 `ensureScrollRoot()` 会探测并退回旧模式
     *   （html 顶部留白 + 页面自己滚；滚动条会横穿标题栏，但页面至少能用）。 */
    'html{box-sizing:border-box!important;height:100%!important;overflow:hidden!important;}',
    'body{box-sizing:border-box!important;margin-top:' + TITLEBAR_H + 'px!important;',
    'min-height:0!important;height:calc(100vh - ' + TITLEBAR_H + 'px)!important;',
    'overflow-y:auto!important;',
    /* 页面常写 `html{scroll-behavior:smooth}` 让锚点跳转平滑 —— 但 scroll-behavior 不继承，
     * 不镜像过来就会让平滑滚动悄悄变成瞬移。用 inherit 把 html 的值搬给 body：
     * html 没设时就是 auto，与改契约之前的行为完全一致。 */
    'scroll-behavior:inherit!important;}',
    /* 页面自己的贴顶固定条（fixed / sticky / 整屏外壳，且压在视口顶）由 applyChromeFit 标记后
     * 下移。属性选择器特异性低，页面自己写 `#id{top:0!important}` 就能压过它 —— 所以 JS 里
     * 同时写 inline `top:...!important`（内联 important 优先级最高），这两条只作兜底。 */
    '[' + SHIFT_ATTR + ']{top:' + TITLEBAR_H + 'px!important;}',
    /* 页面自己的「按视口高度定尺寸的容器」由 applyChromeFit 标记后缩高一个标题栏：
     * 否则它们（100vh / min-height:100vh）的底边会落到视口下方 38px，页脚被窗口下沿吞掉。 */
    '[' + SHRINK_ATTR + ']{height:' + FIXED_H + '!important;min-height:' + FIXED_H + '!important;}'
  ].join('\n');

  // 设置按钮的图标用内联 SVG，而不是 "⚙" 字符：那个码位在部分系统上会带 emoji 的彩色字形，
  // 与旁边 — / ▢ / ✕ 的单色字形放一起会很跳。
  var GEAR =
    '<svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">' +
    '<path d="M19.14,12.94c0.04-0.3,0.06-0.61,0.06-0.94c0-0.32-0.02-0.64-0.07-0.94l2.03-1.58' +
    'c0.18-0.14,0.23-0.41,0.12-0.61l-1.92-3.32c-0.12-0.22-0.37-0.29-0.59-0.22l-2.39,0.96' +
    'c-0.5-0.38-1.03-0.7-1.62-0.94L14.4,2.81c-0.04-0.24-0.24-0.41-0.48-0.41h-3.84' +
    'c-0.24,0-0.43,0.17-0.47,0.41L9.25,5.35C8.66,5.59,8.12,5.92,7.63,6.29L5.24,5.33' +
    'c-0.22-0.08-0.47,0-0.59,0.22L2.74,8.87C2.62,9.08,2.66,9.34,2.86,9.48l2.03,1.58' +
    'C4.84,11.36,4.8,11.69,4.8,12s0.02,0.64,0.07,0.94l-2.03,1.58c-0.18,0.14-0.23,0.41-0.12,0.61' +
    'l1.92,3.32c0.12,0.22,0.37,0.29,0.59,0.22l2.39-0.96c0.5,0.38,1.03,0.7,1.62,0.94l0.36,2.54' +
    'c0.05,0.24,0.24,0.41,0.48,0.41h3.84c0.24,0,0.44-0.17,0.47-0.41l0.36-2.54' +
    'c0.59-0.24,1.13-0.56,1.62-0.94l2.39,0.96c0.22,0.08,0.47,0,0.59-0.22l1.92-3.32' +
    'c0.12-0.22,0.07-0.47-0.12-0.61L19.14,12.94z M12,15.6c-1.98,0-3.6-1.62-3.6-3.6' +
    's1.62-3.6,3.6-3.6s3.6,1.62,3.6,3.6S13.98,15.6,12,15.6z"/></svg>';

  // `data-tauri-drag-region`：Tauri 的 drag.js 只认这个属性（Windows WebView2 上
  // `-webkit-app-region` 不可靠）。空值语义是「只有直接点在本元素上才拖动」，
  // 所以按钮不加（避免点不动），标题与整条空白区各自带上。
  var HTML = [
    '<button type="button" class="w2t-btn w2t-back" title="后退">←</button>',
    '<button type="button" class="w2t-btn w2t-fwd" title="前进">→</button>',
    '<div class="w2t-title" data-tauri-drag-region> </div>',
    '<button type="button" class="w2t-btn w2t-min" title="最小化">—</button>',
    '<button type="button" class="w2t-btn w2t-settings" title="设置">' + GEAR + '</button>',
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
  // 让页面自己的「顶部条 / 整屏外壳 / 按视口高度定尺寸的容器」适配这 38px
  // ---------------------------------------------------------------------
  // 一个扫描里做三件事（识别条件都保守，认不出来就什么都不做，页面维持原样）：
  //   ① 贴顶固定条（fixed / sticky，声明 top < 38）→ inline `top:38px!important`，
  //      否则被标题栏压住上半截（「页面头部显示不全」）。
  //   ② 底部锚定的整屏外壳（fixed / absolute 且 computed `bottom` ≠ auto）→ 同样只改 top：
  //      因为 bottom 还钉在视口底，元素是被【缩短】而不是整体下移 —— 既不压头也不切尾。
  //      （旧版把这整类元素排除在外，它们的顶部 38px 就一直被标题栏压着。）
  //   ③ 高度≈视口高度的容器（`height:100vh` / `min-height:100vh`，且底边已经贴到/越过
  //      视口底）→ inline `height/min-height: calc(100vh - 38px)!important`：
  //      它们本来底边会落到视口下方 38px，把页面自己的页脚 / 底栏顶出窗口
  //      （用户反馈的「页面底部存在部分被下面覆盖」）。
  //
  // 判定依据全部来自实测几何，不猜页面结构：
  //   · 宽 ≥ 视口一半（顶部条、整屏容器、整页容器必然宽；顺带筛掉绝大多数小元素）
  //   · 顶部类：rect.top ∈ [-1, 38+8]（此刻确实贴着视口顶）
  //   · 缩高类：rect.height ≈ 视口高度（±1px，网格/取整余量）且 rect.bottom ≥ 视口底
  //   · 声明式 top 小于标题栏高度（页面自己已经让开的，别去动）
  //   · absolute 只认 rect.top ≤ 1 的：只有「包含块就是初始包含块」的绝对定位元素才会如此；
  //     有定位祖先时它会被 html 的 padding 推到 ~38，那时改 top 是错的。
  //   · 已经标记过的元素不再进入候选（属性检查），避免反复扫描反复写。
  //
  // ⚠️ 两阶段：先只读（收集命中项），再统一写。边读边写会让每次 getBoundingClientRect
  // 都触发重排，大页面上能把首屏拖慢几十毫秒。
  // ---------------------------------------------------------------------
  // 滚动根：html 不滚、body 滚（CSS 里「顶部空间契约」的 JS 半边）
  // ---------------------------------------------------------------------
  // 这个标志决定 applyChromeFit 怎么对待页面自己的 `position:sticky`，所以必须是模块级的。
  var bodyScrollOK = false;

  /// 架好滚动根，并**探测它是否真的生效**；被页面压掉就退回旧的 html 顶部留白模式。
  ///
  /// 三条探测都是有意义的：
  ///   ① `html` 的 overflow 必须已是 hidden（否则根滚动条还在，缺陷原样复现）；
  ///   ② `body` 必须是滚动容器（overflow-y:auto/scroll）；
  ///   ③ `body` 的实际高度必须 ≈ 视口 - 38（页面写 `body{height:auto!important}` 时前两条都成立，
  ///      但 body 会随内容长高、html 又裁掉了溢出 ⇒ 页面滚不动。只查 computed overflow 抓不到这种）。
  /// 任一不成立就退回旧模式，并且**不再装滚动 API 转发**（那时 body 不是滚动容器，
  /// 把它们转发到 body 反而会把页面读到的滚动位置弄错）。
  function ensureScrollRoot(h) {
    try {
      var de = document.documentElement;
      var b = document.body;
      if (!de || !b) return;
      // 契约样式表还没挂上时什么都不做：此刻 computed style 必然是页面原样，
      // 在这里判「失败」并把退路写进 inline style，会把新契约永久压住（见调用点说明）。
      if (h && !styleAttached(h)) return;
      var hcs = window.getComputedStyle(de);
      var bcs = window.getComputedStyle(b);
      var rootHidden = hcs.overflowY === 'hidden' || hcs.overflowY === 'clip';
      var bodyScrolls = bcs.overflowY === 'auto' || bcs.overflowY === 'scroll';
      var want = (window.innerHeight || 0) - TITLEBAR_H;
      var h = b.getBoundingClientRect().height;
      var heightOK = want > 0 && Math.abs(h - want) <= 2;
      bodyScrollOK = rootHidden && bodyScrolls && heightOK;
      if (bodyScrollOK) {
        shimWindowScroll(b);
        return;
      }
      // ---- 退路：把契约换回「html 顶部留白 + 页面自己滚」 ----
      // 用 inline !important（优先级最高），确保反转一定生效；宁可滚动条横穿标题栏，
      // 也不能让页面滚不动。
      b.style.setProperty('margin-top', '0px', 'important');
      b.style.setProperty('height', 'auto', 'important');
      b.style.setProperty('min-height', '0px', 'important');
      b.style.setProperty('overflow-y', 'visible', 'important');
      de.style.setProperty('height', 'auto', 'important');
      de.style.setProperty('overflow-y', 'auto', 'important');
      de.style.setProperty('padding-top', TITLEBAR_H + 'px', 'important');
      try {
        console.error('[web2app] 滚动根契约未生效（html overflow=' + hcs.overflowY +
          ' body overflowY=' + bcs.overflowY + ' bodyH=' + Math.round(h) + ' 期望≈' + Math.round(want) +
          '）已退回 html 顶部留白模式：标题栏右端可能被滚动条盖住，但页面可以正常滚动。');
      } catch (e) { /* noop */ }
    } catch (e) { /* 探测本身失败：按现状继续，绝不打扰宿主页面 */ }
  }

  /// body 当滚动容器之后，页面代码常用的「window 级滚动 API」会指向一个不再滚动的
  /// documentElement —— `scrollY` 恒 0、window 上的 `scroll` 事件不再触发、`scrollTo` 空转，
  /// 而懒加载 / 吸顶导航 / 回到顶部 / 阅读进度全靠它们。这里做一层**转发**：
  /// 读到的就是真实的 body 滚动位置，写下去也是真的滚 body。
  ///
  /// 只转发、不改语义；每一步都独立 try/catch —— 少转发一个 API 不影响页面运行。
  function shimWindowScroll(b) {
    try {
      if (window.__web2appScrollShim) return;
      window.__web2appScrollShim = true;
    } catch (e) { return; }

    // ① body 滚动 → 在 window / document 上补发 scroll 事件。
    //    body 的 scroll 事件**不冒泡**，所以两个目标各派发一次（页面多半监听其中之一）。
    try {
      b.addEventListener('scroll', function () {
        try { window.dispatchEvent(new Event('scroll')); } catch (e) { /* noop */ }
        try { document.dispatchEvent(new Event('scroll')); } catch (e) { /* noop */ }
      }, { passive: true });
    } catch (e) { /* noop */ }

    // ② 四个位置读法转发到 body（页面据此判断「滚到哪了 / 该不该回顶」）。
    var readers = {
      scrollY: 'scrollTop', pageYOffset: 'scrollTop',
      scrollX: 'scrollLeft', pageXOffset: 'scrollLeft'
    };
    for (var name in readers) {
      (function (prop, key) {
        try {
          Object.defineProperty(window, key, {
            configurable: true,
            get: function () { return b[prop] || 0; }
          });
        } catch (e) { /* noop */ }
      })(readers[name], name);
    }

    // ③ 两个写方法转发到 body（`window.scrollTo(0,0)` 这类回顶调用）。
    //    支持 `scrollTo({top:0,behavior:'smooth'})` 这种 options 形式（直接透传给 body）。
    try {
      var to = function (x, y) {
        return (x && typeof x === 'object') ? b.scrollTo(x) : b.scrollTo(x, y);
      };
      var by = function (x, y) {
        return (x && typeof x === 'object') ? b.scrollBy(x) : b.scrollBy(x, y);
      };
      window.scrollTo = to;
      window.scroll = to;
      window.scrollBy = by;
    } catch (e) { /* noop */ }

    // ④ `document.scrollingElement` 指向 body：现代页面写
    //    `document.scrollingElement.scrollTop = 0` 很常见。它是 Document 上的只读访问器，
    //    某些引擎标记为不可配置，改不动就跳过（那条路径本来也只是少数站点用）。
    try {
      Object.defineProperty(Document.prototype, 'scrollingElement', {
        configurable: true,
        get: function () { return this.body || null; }
      });
    } catch (e) { /* noop */ }

    // ⑤ `<html>` 元素自己的 scrollTop / scrollLeft 转发到 body：还有一批「回到顶部」写的是
    //    `document.documentElement.scrollTop = 0`。只给**这一个实例**挂自有访问器去遮蔽原型上的
    //    那对（不碰 Element.prototype），影响面最小：其它任何元素的行为一模一样。
    try {
      ['scrollTop', 'scrollLeft'].forEach(function (prop) {
        Object.defineProperty(document.documentElement, prop, {
          configurable: true,
          get: function () { return b[prop] || 0; },
          set: function (v) { b[prop] = v; }
        });
      });
    } catch (e) { /* noop */ }
  }

  function applyChromeFit() {
    try {
      var root = document.body || document.documentElement;
      if (!root) return;
      var de = document.documentElement;
      var vw = de.clientWidth || window.innerWidth || 0;
      var vh = de.clientHeight || window.innerHeight || 0;
      if (!vw || !vh) return;
      var scrollY = window.scrollY || window.pageYOffset || 0;

      var all = root.getElementsByTagName('*');
      var shifts = [];                 // 需要把 top 改成 38px 的元素
      var shrinks = [];                // 需要把高度减掉一个标题栏的元素
      var shrinkH = [];                // 与 shrinks 一一对应的高度值

      for (var i = 0; i < all.length; i++) {
        var el = all[i];
        var id = el.id;
        if (id === '__web2app_titlebar__' || id === '__web2app_loading__') continue; // 我们自己的
        if (el === de) continue;       // 不碰 <html>：它的高度由文件头「顶部空间契约」负责

        var needShift = !el.hasAttribute(SHIFT_ATTR);
        var needShrink = !el.hasAttribute(SHRINK_ATTR);
        if (!needShift && !needShrink) continue;

        // 先用几何特征筛掉绝大多数元素（getBoundingClientRect 便宜），
        // 只有少数候选才去读 computed style（那才是开销大头）。
        var r = el.getBoundingClientRect();
        if (r.width < vw * 0.5) continue;
        if (r.height <= 0) continue;

        var atTop = needShift && r.top >= -1 && r.top <= TITLEBAR_H + 8;
        var viewportTall = needShrink &&
          r.height >= vh - VH_TOL && r.height <= vh + VH_TOL && r.bottom >= vh - VH_TOL;
        if (!atTop && !viewportTall) continue;

        var cs = window.getComputedStyle(el);
        var pos = cs.position;
        var bottomAnchored = cs.bottom !== 'auto';   // 底边钉在包含块/视口底部

        var moved = false;   // 本次是否会把它的 top 改成 38px
        // ① 贴顶的 fixed / sticky（以及「包含块就是初始包含块」的 absolute）→ 下移
        //
        // ⚠️ sticky 只在**退路模式**（html 当滚动容器）下才需要下移：那时它贴的是视口顶 y=0。
        // 正常模式下 body 就是滚动容器、盒子上沿已在 38px，`top:0` 的 sticky 天然停在 y=38 ——
        // 再给它加 38 就成了 76px（页面自己的吸顶导航凭空多出一条空白）。所以正常模式不碰 sticky。
        if (atTop && (pos === 'fixed' || (pos === 'sticky' && !bodyScrollOK) ||
            (pos === 'absolute' && r.top <= 1))) {
          var t = cs.top;
          if (t === 'auto' ? r.top <= 2 : parseFloat(t) < TITLEBAR_H) {
            shifts.push(el);
            moved = true;
          }
        }
        if (!needShrink) continue;
        // 底部锚定的元素靠 bottom 定位，改 top 就够了，本身会被缩短，不需要动高度
        if (bottomAnchored) continue;
        // ② 正常流里按视口高度定尺寸的容器 → 高度减一个标题栏
        if (!viewportTall) continue;
        // 高度该减多少：元素被下移到 38 时减 38；本身就在 38 往下的（body 默认 8px 边距等）
        // 就按它离文档顶的实际距离减，但夹在 [38, 54] 内 —— 只补偿标题栏这一档，不做大改写。
        var c = TITLEBAR_H;
        if (!moved) {
          var off = Math.round(r.top + scrollY);
          if (off >= 0) c = Math.max(TITLEBAR_H, Math.min(off, TITLEBAR_H + 16));
        }
        shrinks.push(el);
        shrinkH.push('calc(100vh - ' + c + 'px)');
      }

      for (var j = 0; j < shrinks.length; j++) {
        try {
          shrinks[j].setAttribute(SHRINK_ATTR, '');
          shrinks[j].style.setProperty('height', shrinkH[j], 'important');
          shrinks[j].style.setProperty('min-height', shrinkH[j], 'important');
        } catch (e) { /* 单个元素失败不影响其余 */ }
      }
      for (var k = 0; k < shifts.length; k++) {
        try {
          shifts[k].setAttribute(SHIFT_ATTR, '');
          shifts[k].style.setProperty('top', TITLEBAR_H + 'px', 'important');
        } catch (e) { /* 单个元素失败不影响其余 */ }
      }
    } catch (e) { /* 绝不把异常抛给宿主页面 */ }
  }

  // 调度：定时器在「被遮挡窗口」里会被 Chromium 节流到几乎不跑，所以主力是
  // 「DOM 事件 + MutationObserver」（观察者回调不依赖定时器），定时器只作兜底补扫。
  // 总次数封顶，避免与频繁改 DOM 的页面互相拉扯。
  //
  // ⚠️ 多轮扫描是有用的：嵌套的整页容器（外层被缩高后，内层才暴露出「底边越过视口底」）
  // 要靠下一轮才命中 —— 单轮只处理「当轮量得到」的那一层。
  function scheduleChromeFit() {
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

  /// 往 document 追加一张「构造函数式样式表」（adoptedStyleSheets 是 FrozenArray，
  /// 只能整体替换，且它不保证有 concat —— 用最笨的复制方式，兼容性最好）。
  function adoptSheet(sheet) {
    var next = [];
    var cur = document.adoptedStyleSheets || [];
    for (var i = 0; i < cur.length; i++) next.push(cur[i]);
    next.push(sheet);
    document.adoptedStyleSheets = next;
  }

  /// 装载样式表，返回句柄供自愈守护检查 / 重挂。
  ///
  /// ⚠️⚠️ 优先走「构造函数式样式表」（CSSStyleSheet + document.adoptedStyleSheets）：
  /// 它**不受页面 CSP 的 `style-src` 约束**。像声明 `style-src 'self'` 的站点会把我们
  /// 插入的 `<style>` 元素直接拦掉，故障表象是「标题栏在、按钮在、几何也对，但完全没样式」
  ///（透明背景、按钮挤成一团、38px 让位也丢了）—— 本项目里最难从表象反推原因的一类。
  /// 回退路径保留 `<style>`：WebView2 / Chromium 100+ 都支持 adoptedStyleSheets，
  /// 这条回退基本只在极老引擎上才走到。
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
    el.setAttribute('data-web2app', 'titlebar');
    el.textContent = CSS;
    (document.head || document.documentElement).appendChild(el);
    return { kind: 'style', el: el };
  }

  /// 样式是否仍然生效（document.write / 框架重建 body 时可能被整片抹掉）。
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

  /// 标题栏该有多宽（CSS px）：窗口宽度（**含**滚动条区域）再除以页面 zoom。
  ///
  /// 为什么 CSS 里已经有 `width:100vw!important` 还要在 JS 里再钉一次：
  /// 实测在 WebView2 / Electron（Chromium）上 `100vw == innerWidth`（含滚动条宽）成立，
  /// 所以平时这里写进去的值与 100vw 相同、什么也不改；但那条等式依赖「引擎把 vw 解析成
  /// 布局视口宽」这个实现细节。一旦不成立（换了渲染引擎、页面在 html/body 上用了 `zoom`
  /// 把整棵子树按倍数缩放），标题栏就会比窗口窄**正好一条滚动条**，滚动条从 y=0 裸露在
  /// 标题栏右侧（2026-09-17 使用者截图反馈的现象）。显式写 px 之后，这件事从
  /// 「寄希望于引擎语义」变成「我们能测、能断言的事实」。
  ///
  /// ⚠️ 除 zoom 之外不加别的修正：任何「凭感觉补偿几个像素」的写法都会在不同 DPI / 不同
  /// 滚动条宽度下变成新的错位来源。
  function barWidthPx() {
    var w = 0;
    try { w = window.innerWidth || 0; } catch (e) { w = 0; }
    if (!(w > 0)) return 0;
    var z = 1;
    try {
      var cz = parseFloat(window.getComputedStyle(document.documentElement).zoom);
      if (isFinite(cz) && cz > 0 && cz !== 1) z = cz;
    } catch (e) { /* 没有 zoom 的引擎：1 就是正解 */ }
    return w / z;
  }

  /// 把宽度钉到窗口宽。任何异常都吞掉 —— 注入脚本绝不给宿主页面抛错。
  function syncBarWidth(bar) {
    try {
      var b = bar && bar.style ? bar : document.getElementById('__web2app_titlebar__');
      if (!b || !b.style) return;
      var w = barWidthPx();
      if (w > 0) b.style.setProperty('width', w + 'px', 'important');
    } catch (e) { /* noop */ }
  }

  function mount() {
    var style = mountStyle();

    var bar = document.createElement('div');
    bar.id = '__web2app_titlebar__';
    // 整条空白区可拖动：drag.js 的 isDragRegion 对「空属性」要求点击目标就是本元素，
    // 按钮因不属于 drag region（且属 clickable）仍可正常点击。
    bar.setAttribute('data-tauri-drag-region', '');
    bar.innerHTML = HTML;

    // document-created 阶段 DOM 尚未解析，`<body>` 往往还不存在，先挂到 `<html>`，
    // 等 body 就绪后再挪进去（见 ensureHost）。
    (document.body || document.documentElement).appendChild(bar);

    return { style: style, bar: bar };
  }

  /// body 就绪后把标题栏挪进 body，避免它作为 <html> 直接子节点时被部分页面样式影响。
  function ensureHost(bar) {
    try {
      if (document.body && bar.parentNode !== document.body) {
        document.body.appendChild(bar);
      }
    } catch (e) { /* 忽略：挪动失败不影响 fixed 定位的显示效果 */ }
  }

  /// 宣告「顶部 38px 已被标题栏占用」（loading.js 靠它让出该区域；壳侧也用它判进度）。
  ///
  /// 两条通路都给：
  ///   · 全局变量 + 事件 —— 给同世界的 loading.js（两个脚本总被一起注入，同一世界）；
  ///   · `documentElement` 上的属性 —— 给**外部探针**断言用。Electron 侧探针在渲染进程的
  ///     主世界里 `executeJavaScript` 读它；Tauri 侧探针走 beacon 回传。DOM 跨世界共享，
  ///     属性比全局变量更「稳」，是外部唯一该依赖的接口。
  ///     （注：这里曾写「属性是主世界唯一可观测的口子，因为注入脚本跑在隔离世界」——
  ///      那个前提**已被推翻**：Electron 侧改用 webFrame.executeJavaScript 注入主世界，
  ///      理由见 electron-app/src/renderer/preload.ts。属性保留只为对外观测。）
  function markChromeReady() {
    try { window.__web2appChromeH = TITLEBAR_H; } catch (e) { /* noop */ }
    try {
      var de = document.documentElement;
      if (de) {
        de.setAttribute('data-web2app-chrome-h', String(TITLEBAR_H));
        de.setAttribute('data-web2app-shell', shellName());
      }
    } catch (e) { /* noop */ }
    try { window.dispatchEvent(new Event('web2app:chrome-ready')); } catch (e) { /* noop */ }
  }

  /// 取当前窗口控制器：**每次要用时现取**，绝不在挂载时缓存（理由见 electronApi 注释）。
  /// 两个壳暴露的控制器方法名一致：minimize / toggleMaximize / close。
  function currentWindow() {
    var E = electronApi();
    if (E) return E;
    try {
      var T = window.__TAURI__;
      if (!T) return null;
      if (T.window && T.window.getCurrentWindow) return T.window.getCurrentWindow();
      if (T.webviewWindow && T.webviewWindow.getCurrentWebviewWindow) {
        return T.webviewWindow.getCurrentWebviewWindow();
      }
    } catch (e) { /* noop */ }
    return null;
  }

  /// 执行窗口命令，并把失败原因打到控制台。
  /// ⚠️ **不要**退回 `.catch(function(){})` 那种静默写法：ACL 拒绝（Tauri）/ IPC 未注册
  ///（Electron）时前端一片安静，用户只看到「点了没反应」，排查得从零开始
  ///（2026-09-16 Tauri 侧就是这么被坑的）。控制台日志只是给人看的，仍然绝不向宿主页面抛异常。
  function callWindow(method) {
    var w = currentWindow();
    if (!w || typeof w[method] !== 'function') {
      try {
        console.error(
          '[web2app] 窗口命令 ' + method + ' 不可用：壳控制器未就绪（shell=' + shellName() + '）。' +
          'Electron 侧查 preload 是否注入 window.web2app；Tauri 侧查 __TAURI__ 与 capabilities 的 remote.urls。'
        );
      } catch (e) { /* noop */ }
      return;
    }
    try {
      var p = w[method]();
      if (p && p.then) {
        p.then(null, function (err) {
          try { console.error('[web2app] 窗口命令 ' + method + ' 被拒绝：', err); } catch (e) { /* noop */ }
        });
      }
    } catch (e) {
      try { console.error('[web2app] 窗口命令 ' + method + ' 抛异常：', e); } catch (e2) { /* noop */ }
    }
  }

  /// 打开设置窗口（标题栏「设置」按钮）。
  ///
  /// 两个壳走**不同通路**，因为主窗加载的是**远程页面**：
  ///   · Electron：`window.web2app.openSettings()`（preload 白名单 → 主进程 openSettingsWindow）。
  ///   · Tauri   ：**不能** invoke 应用命令 `open_settings` —— Tauri 2 对非本地 origin 的 IPC
  ///     强制走 ACL，而应用命令没有 app 级 ACL 清单可授权，必被拒（与 history_nav 同一条约束）。
  ///     改发**核心插件的事件命令**：`core:event:allow-emit` 是可以授权给远程 origin 的
  ///     （见 capabilities/default.json），Rust 侧 `app.listen("web2app:open-settings")` 收到后
  ///     调 open_settings_window。⚠️ 事件名必须与 lib.rs 里的字面量逐字一致。
  /// 失败一律打到控制台：静默的话用户只看到「点了没反应」，排查得从零开始。
  function openSettings() {
    var E = electronApi();
    if (E && typeof E.openSettings === 'function') {
      try {
        var p = E.openSettings();
        if (p && p.then) {
          p.then(null, function (err) {
            try { console.error('[web2app] 打开设置被拒绝：', err); } catch (e) { /* noop */ }
          });
        }
        return;
      } catch (e) {
        try { console.error('[web2app] 打开设置抛异常：', e); } catch (e2) { /* noop */ }
        return;
      }
    }
    var evt = 'web2app:open-settings';
    try {
      var T = window.__TAURI__;
      if (T && T.event && typeof T.event.emit === 'function') {
        var r = T.event.emit(evt);
        if (r && r.then) {
          r.then(null, function (err) {
            try { console.error('[web2app] 打开设置事件被拒：', err); } catch (e) { /* noop */ }
          });
        }
        return;
      }
      // 兜底：直接走底层 invoke（withGlobalTauri 关掉时 __TAURI__.event 不存在）。
      var I = window.__TAURI_INTERNALS__;
      if (I && typeof I.invoke === 'function') {
        I.invoke('plugin:event|emit', { event: evt, payload: null });
        return;
      }
    } catch (e) {
      try { console.error('[web2app] 打开设置抛异常：', e); } catch (e2) { /* noop */ }
      return;
    }
    try {
      console.error(
        '[web2app] 无法打开设置：壳控制器未就绪（shell=' + shellName() + '）。' +
        'Electron 侧查 preload 是否暴露 window.web2app.openSettings；' +
        'Tauri 侧查 __TAURI__ 与 capabilities 的 core:event:allow-emit。'
      );
    } catch (e) { /* noop */ }
  }

  function wire(bar) {
    // ⚠️ 前进/后退【直接】驱动当前文档的历史，不要改回 invoke 自定义命令：
    // 主窗加载的是远程页面（非 local origin），Tauri 2 会强制走 ACL 校验，而应用自定义命令
    // 无法授权给远程 origin → 调用必被拒；前端又不能把异常抛给宿主页面，只能 catch 掉，
    // 结果就是「点了没反应」。单窗口模式下用 window.history 与 Rust 的 history_nav 完全等价
    //（后者也只是在目标 webview 里执行同样的调用，多页签模式才需要它来定位目标 tab）。
    //
    // 最小化/最大化/关闭走的是 `plugin:window|*`（Tauri 核心插件命令）—— 这类命令**即使本地
    // origin 也要过 ACL**，所以 capabilities/default.json 必须同时满足两点：
    //   1. 列出窗口 label（`windows: ["main"]`）；
    //   2. `remote.urls` 覆盖真实页面 origin，**并且带上端口通配 `:*`**。
    //      踩坑记录：URLPattern 里「不写端口」＝只匹配该 scheme 的默认端口，`http://*`
    //      匹配不了 `http://127.0.0.1:51234/`（本地反向代理的端口是随机的）⇒ 只有直连
    //      https://域名（443 默认端口）时按钮才是好的，代理模式下按钮全废。
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
    bar.querySelector('.w2t-min').addEventListener('click', function () { callWindow('minimize'); });
    // 设置按钮：紧挨最小化按钮右侧（与多页签标签栏 shell.html 的同名按钮位置一致）。
    bar.querySelector('.w2t-settings').addEventListener('click', function () { openSettings(); });
    bar.querySelector('.w2t-max').addEventListener('click', function () { callWindow('toggleMaximize'); });
    bar.querySelector('.w2t-close').addEventListener('click', function () { callWindow('close'); });
    // 双击标题栏空白区最大化/还原由 Tauri 的 drag region 负责（drag.js 在 mousedown
    // detail===2 时调用 plugin:window|internal_toggle_maximize）。这里【不要】再自绑 dblclick：
    // 一次双击会触发两次切换 = 看起来没反应。

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
    var mounted = null;
    try {
      mounted = mount();
      wire(mounted.bar);
    } catch (e) {
      // 极端文档挂载失败：绝不破坏宿主页面。同时撤销防重标记，让后续注入
      //（页面重载、Rust 侧 `on_page_load` 的兜底重注入）仍有机会补挂，而不是从此彻底放弃。
      window.__web2appTitlebar = false;
    }

    // ⚠️ 滚动根契约（html 不滚、body 滚）必须**在样式表挂上之后**探测。
    // 早一步跑，computed style 还是页面原样（html auto / body visible），探测必然判失败、
    // 并把退路写死在 inline style 上 —— 而 inline `!important` 会永久压住我们的样式表，
    // 新契约再也起不来（2026-09-17 自己踩过一次：现象是 body 高度仍是内容高、根滚动条照旧）。
    // 挂载真失败时它也顺手把「html 顶部留白」的旧契约兜上，页面至少能滚。
    ensureScrollRoot(mounted ? mounted.style : null);

    // 页面自己的贴顶固定条 / 整屏外壳 / 按视口高度定尺寸的容器都要一起适配（见 applyChromeFit）：
    // ① 下移，别被标题栏压住；② 缩高，别被挤出视口底部。
    // 这一步不依赖标题栏挂载成功与否，所以无论如何都启动（内部自带 try/catch 与次数上限）。
    scheduleChromeFit();

    if (!mounted) return;
    var bar = mounted.bar;
    var style = mounted.style;

    // 宽度钉一次（见 barWidthPx 的说明）：CSS 的 100vw 负责首帧，这里负责「确定性」。
    syncBarWidth(bar);
    // 窗口尺寸变化（拖动 / 最大化 / DPI 与系统缩放改动）后重新钉。
    // 只监听 resize：这是唯一会改变窗口宽度的途径，页面自身滚动条出现/消失不影响 innerWidth。
    try {
      window.addEventListener('resize', function () { syncBarWidth(bar); }, { passive: true });
    } catch (e) { /* noop */ }

    // body 出现后把标题栏挪进 body（首次挂载时 body 常常还不存在，先挂在 <html> 上）。
    try {
      if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', function () {
          ensureHost(bar);
          syncBarWidth(bar);   // 换宿主后 zoom 上下文可能变，跟着重钉一次
        }, { once: true });
      } else {
        ensureHost(bar);
        syncBarWidth(bar);
      }
    } catch (e) { /* noop */ }

    // 通告加载层（loading.js）：顶部已被占用 38px，请它让出该区域，避免把标题栏盖住。
    // 两脚本的注入顺序不固定，故「全局标记 + 事件」双保险，谁先谁后都能同步上。
    markChromeReady();

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
          var lostStyle = !styleAttached(style);
          if (!lostBar && !lostStyle) return;
          if (++tries > 20) {
            clearInterval(iv);
            return;
          }
          if (lostStyle) {
            styleReattach(style);
            // 文档被重写时 <html>/<body> 可能也是新的 ⇒ 滚动根契约与滚动 API 转发一起重放
            //（bodyScrollOK 会重新探测；已被转发过的 window 属性靠 __web2appScrollShim 幂等跳过）。
            ensureScrollRoot(style);
          }
          if (lostBar) {
            root.appendChild(bar);
            syncBarWidth(bar);   // 文档被重写后宿主与 zoom 上下文都可能变了
          }
          // 文档被重写时 documentElement 可能也是新的：契约标记（属性 / 全局 / 让位事件）
          // 得跟着重放，否则 loading.js 会把加载层重新盖到标题栏上。
          markChromeReady();
        } catch (e) { /* noop */ }
      }, 800);
    })();
  });
})();