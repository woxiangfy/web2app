// ⚠️ 本文件是 `src/mobilebar/` 的**分片**，不能单独运行：`lib.rs` 按
//    core → bottom_space → tabs → more → mount 的顺序 `include_str!` 拼进**同一个 IIFE**，
//    分片共享同一份闭包作用域（不要改成各自独立的 IIFE）。
// 职责（底板）：注入骨架与共享状态 —— 配置解析、常量/图标/样式、事件与 DOM 宿主工具、
//     底栏本体与面板宿主（openSheet / closeSheet / renderSheet）、防重入与自愈的公共状态。
/*
 * web2app 移动端底部导航栏（src/mobilebar/，入口约定见 lib.rs）
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
 *   · 列表：展开已打开的页签并切换 / 关闭，面板顶部还有「新建标签」。**只在壳告诉它
 *     「真的有页签可切」时才渲染**（`window.__web2appMobileBarCfg.tabs`，真源
 *     `mobilebar.rs::tabs_available` = 打包配置 `tabs_enabled`）。
 *     ⚠️ 移动端是【单 webview 页签记录】：Tauri/wry 在 Android 不支持同一窗口挂多个
 *     子 webview，壳侧只维护每个标签的 url / 标题记录，真正存活的页面只有当前这一个 ——
 *     切换标签 = 按记录的 url 重新加载（桌面端那种「多 webview 并行、状态保留」在
 *     手机上不存在）；首页固定在 0 号位、不可关闭，与桌面端同一约定。
 *     文档 url / 标题由本脚本经 `web2app:mobile-doc` 事件上报，面板数据由壳侧 eval
 *     `__web2appMobileBar.setTabs(...)` 推回（四元组 [label,title,active,home]）。
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
  // 「更多」：竖三点（Material/微信风格的「更多操作」通用符号）。
  var ICO_MORE =
    '<svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">' +
    '<circle cx="12" cy="5" r="1.8"/><circle cx="12" cy="12" r="1.8"/>' +
    '<circle cx="12" cy="19" r="1.8"/></svg>';
  // 「更多」面板各行图标（统一 18px 线性风格）。
  var ICO_REFRESH =
    '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" ' +
    'stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
    '<path d="M23 4v6h-6M1 20v-6h6"/>' +
    '<path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/></svg>';
  var ICO_COPY =
    '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" ' +
    'stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
    '<rect x="9" y="9" width="13" height="13" rx="2"/>' +
    '<path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>';
  var ICO_SHARE =
    '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" ' +
    'stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
    '<circle cx="18" cy="5" r="3"/><circle cx="6" cy="12" r="3"/><circle cx="18" cy="19" r="3"/>' +
    '<line x1="8.6" y1="13.5" x2="15.4" y2="17.5"/>' +
    '<line x1="15.4" y1="6.5" x2="8.6" y2="10.5"/></svg>';
  var ICO_EXTERNAL =
    '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" ' +
    'stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
    '<path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>' +
    '<polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>';
  var ICO_TRASH =
    '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" ' +
    'stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
    '<polyline points="3 6 5 6 21 6"/>' +
    '<path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>';
  var ICO_EXIT =
    '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" ' +
    'stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
    '<path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"/>' +
    '<polyline points="16 17 21 12 16 7"/><line x1="21" y1="12" x2="9" y2="12"/></svg>';
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
    /* 「新建标签」行：与列表行同高，但文案居中、不带关闭区（壳侧收到事件后新开一个标签）。 */
    '#__web2app_mobilebar__ .w2m-newrow{',
    'display:flex!important;align-items:center!important;justify-content:center!important;',
    'width:100%!important;min-height:44px!important;padding:10px 14px!important;',
    'border:0!important;border-bottom:1px solid rgba(0,0,0,.10)!important;',
    'background:transparent!important;color:inherit!important;font:inherit!important;font-size:13px!important;',
    '-webkit-tap-highlight-color:transparent!important;touch-action:manipulation!important;}',
    '#__web2app_mobilebar__ .w2m-newrow:active{background:rgba(120,133,153,.22)!important;}',
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
    /* 「更多」面板：顶部一行当前地址（可截断），其余复用 .w2m-row 行样式。 */
    '#__web2app_mobilebar__ .w2m-morehead{',
    'padding:10px 14px!important;font-size:11px!important;line-height:1.3!important;opacity:.6!important;',
    'white-space:nowrap!important;overflow:hidden!important;text-overflow:ellipsis!important;',
    'border-bottom:1px solid rgba(0,0,0,.06)!important;}',
    /* 危险动作（清除浏览数据二次确认态）：整行红色提示，暗色模式提亮。 */
    '#__web2app_mobilebar__ .w2m-row.danger .w2m-ico,',
    '#__web2app_mobilebar__ .w2m-row.danger .w2m-label{color:#d33f3f!important;opacity:1!important;}',
    '@media (prefers-color-scheme:dark){#__web2app_mobilebar__ .w2m-row.danger .w2m-ico,',
    '#__web2app_mobilebar__ .w2m-row.danger .w2m-label{color:#ff7a7a!important;}}',
    /* Toast：浮在底栏上沿 12px，不参与点击（纯提示），层级压过 sheet/遮罩。 */
    '#__web2app_mobilebar__ .w2m-toast{',
    'position:fixed!important;left:50%!important;transform:translateX(-50%)!important;',
    'bottom:calc(' + BAR_H + 'px + ' + SAFE + ' + 12px)!important;max-width:80vw!important;',
    'padding:8px 14px!important;border-radius:18px!important;',
    'background:rgba(20,24,30,.88)!important;color:#fff!important;',
    'font-size:12px!important;line-height:1.4!important;',
    'white-space:nowrap!important;overflow:hidden!important;text-overflow:ellipsis!important;',
    'opacity:0!important;transition:opacity .18s ease!important;',
    'pointer-events:none!important;z-index:2147483647!important;}',
    '#__web2app_mobilebar__ .w2m-toast[data-show="1"]{opacity:1!important;}',
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
  // 底栏本体
  // ---------------------------------------------------------------------
  /// 按 cfg 生成按钮 HTML。⚠️「列表」按钮只在 HAS_TABS 时才出现 —— 移动端目前没有多页签，
  /// 无条件画它就是一个点了没反应的死按钮（本文件的核心约束之一）。
  /// 「更多」无条件出现且固定在最后：刷新/复制/分享/浏览器打开/清数据/退出的唯一入口，
  /// 与是否多页签无关。
  function barHtml() {
    var items = [
      btn('home', ICO_HOME, '首页', '首页'),
      btn('back', ICO_BACK, '后退', '后退'),
      btn('fwd', ICO_FWD, '前进', '前进')
    ];
    if (HAS_TABS) items.push(btn('list', ICO_LIST, '列表', '已打开的标签'));
    items.push(btn('more', ICO_MORE, '更多', '更多操作'));
    return items.join('');
  }

  function btn(kind, icon, text, aria) {
    return '<button type="button" class="w2m-btn w2m-' + kind + '" aria-label="' + aria + '">' +
      '<span class="w2m-ico">' + icon + '</span>' +
      '<span class="w2m-txt">' + text + '</span>' +
      '</button>';
  }

  /// 页签行数据（壳侧经 eval 推送）：[[label, title, active, home], …]，
  /// 与多页签 `get_tabs` 的四元组同形（home = 首页标记：不可关闭、名字前带房子图标）。
  var tabRows = [];

  /// 当前打开的面板：'tabs'（页签列表）| 'more'（更多操作）| null。同一时刻最多一个。
  var activePanel = null;

  function setTabs(rows) {
    tabRows = Array.isArray(rows) ? rows : [];
    if (activePanel === 'tabs') renderSheet();
  }

  // ---------------------------------------------------------------------
  // Toast（面板操作的唯一反馈通道：原生侧不弹窗时由这里提示）
  // ---------------------------------------------------------------------
  var toastTimer = 0;
  function toast(msg) {
    try {
      var bar0 = document.getElementById('__web2app_mobilebar__');
      if (!bar0) return;
      var el = bar0.querySelector('.w2m-toast');
      if (!el) {
        el = document.createElement('div');
        el.className = 'w2m-toast';
        bar0.appendChild(el);
      }
      el.textContent = String(msg == null ? '' : msg);
      el.setAttribute('data-show', '1');
      if (toastTimer) clearTimeout(toastTimer);
      toastTimer = setTimeout(function () {
        try { el.removeAttribute('data-show'); } catch (e) { /* noop */ }
      }, 2200);
    } catch (e) { /* noop */ }
  }

  function closeSheet(bar) {
    activePanel = null;
    clearArmedClear();
    try {
      var mask = bar.querySelector('.w2m-mask');
      if (mask) mask.remove();
      var sheet = bar.querySelector('.w2m-sheet');
      if (sheet) sheet.remove();
      ['.w2m-list', '.w2m-more'].forEach(function (sel) {
        var btn0 = bar.querySelector(sel);
        if (btn0) btn0.style.removeProperty('background');
      });
    } catch (e) { /* noop */ }
  }

  function renderSheet() {
    if (activePanel === 'more') renderMorePanel();
    else renderTabsSheet();
  }

  function openSheet(bar, which) {
    activePanel = which;
    try {
      var mask = document.createElement('div');
      mask.className = 'w2m-mask';
      mask.addEventListener('click', function () { closeSheet(bar); });
      bar.appendChild(mask);
      if (which === 'tabs') {
        // 打开前先让壳侧推一份最新页签清单（tabRows 可能停留在上次打开时的旧标题）。
        emit('web2app:request-tabs', null);
      }
      renderSheet();
      var activeBtn = bar.querySelector(which === 'tabs' ? '.w2m-list' : '.w2m-more');
      if (activeBtn) activeBtn.style.setProperty('background', 'rgba(120,133,153,.22)', 'important');
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

    // 「列表」仅多页签模式存在；「更多」无条件存在（面板互斥：一个开时另一个先关）。
    var listBtn = bar.querySelector('.w2m-list');
    if (listBtn) {
      listBtn.addEventListener('click', function () {
        if (activePanel === 'tabs') closeSheet(bar);
        else openSheet(bar, 'tabs');
      });
    }
    var moreBtn = bar.querySelector('.w2m-more');
    if (moreBtn) {
      moreBtn.addEventListener('click', function () {
        if (activePanel === 'more') closeSheet(bar);
        else openSheet(bar, 'more');
      });
    }
  }

  /// 宣告「底部已被底栏占用」（外部探针 / 同世界脚本读它，比全局变量更稳）。
  function markChromeReady() {
    try {
      // 加载层（loading.js）按这两个全局把遮罩缩到「顶栏与底栏之间」：顶部取
      // __web2appChromeH（桌面 titlebar.js 下发），底部取本值。再广播 chrome-ready，
      // 让先挂载的 loading 层重算一次 —— 加载转圈期间底栏仍可见、可点。
      window.__web2appMobileBarH = reservePx();
      window.dispatchEvent(new Event('web2app:chrome-ready'));
    } catch (e) { /* noop */ }
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

