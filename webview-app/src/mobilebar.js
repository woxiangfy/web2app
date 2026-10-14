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

  /// 贴底元素上移后的 bottom 值：在底栏预留高度之上【保留元素原本的底边距】。
  /// 悬浮按钮（FAB）一般写 `bottom:16px`，直接顶到 RESERVE 会让它贴死底栏上沿、丢掉
  /// 原本那口气；写成 calc 把原 px 数加回去。认不出（auto / 非 px 单位）就贴齐底栏上沿，
  /// 与整宽底条（bottom:0）的处理一致。
  function shiftedBottom(declared) {
    try {
      if (declared && declared !== 'auto') {
        var m = /^(\d+(?:\.\d+)?)px$/.exec(declared.trim());
        if (m) {
          var n = parseFloat(m[1]);
          if (n > 0) return 'calc(' + BAR_H + 'px + ' + SAFE + ' + ' + n + 'px)';
        }
      }
    } catch (e) { /* noop */ }
    return RESERVE;
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
        if (r.height <= 0) continue;
        // 贴底：底边落在视口底附近（含已被系统键盘 / 安全区推上来的余量）
        var atBottom = needShift && r.bottom >= vh - Math.max(reserve + 2, 8) && r.bottom <= vh + 2;
        // 整屏：高度≈视口高且底边已到/越过视口底（100vh 容器被顶出可视区）
        var viewportTall = needShrink && r.width >= vw * 0.5 &&
          r.height >= vh - VH_TOL && r.height <= vh + VH_TOL && r.bottom >= vh - VH_TOL;
        if (!atBottom && !viewportTall) continue;

        var cs = window.getComputedStyle(el);
        var pos = cs.position;
        // ⚠️⚠️ fixed 贴底元素【不设宽度门槛】：悬浮按钮（FAB）/ 客服球 / 回到顶部按钮
        //    通常只有 40–64px 宽，旧实现「窄于半屏一律跳过」把它们整类漏掉 —— 真机上被
        //    底栏切掉半截的正是这种按钮（fixed 的包含块就是视口，窄也能确定是贴底件）。
        //    sticky / absolute 仍要求半屏宽：它们的包含块可能是页面内部容器，窄元素极易
        //    误判（absolute 还额外要求底边真的压在视口底，与 titlebar.js 同一条谨慎规则）。
        var bottomFixed = atBottom && pos === 'fixed';
        var bottomWideBar = atBottom && r.width >= vw * 0.5 && (pos === 'sticky' ||
          (pos === 'absolute' && r.bottom >= vh - 2));
        if (bottomFixed || bottomWideBar) {
          // 声明式 bottom 已经比底栏高的（页面自己让开了）不动。
          if (cs.bottom === 'auto' || parseFloat(cs.bottom) < reserve) {
            shifts.push({ el: el, bottom: shiftedBottom(cs.bottom) });
          }
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
          shifts[k].el.setAttribute(SHIFT_ATTR, '');
          shifts[k].el.style.setProperty('bottom', shifts[k].bottom, 'important');
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
    // bfcache 恢复（后退回到本文档）：DOM 不重建，但贴底元素可能被页面脚本重置过 inline 样式。
    try { window.addEventListener('pageshow', run); } catch (e) { /* noop */ }
    try {
      mo = new MutationObserver(run);
      mo.observe(document.documentElement, { childList: true, subtree: true });
    } catch (e) { /* 老引擎：靠上面的时机 + 定时器兜底 */ }
    try {
      // 定长 20 拍由 MutationObserver 兜常规 SPA 改动；这两拍专门留给「首屏后很久才异步
      // 挂出来」的贴底悬浮按钮（轮播 / 广告条 / 客服 SDK 常见 3–8s 才到）。
      setTimeout(run, 1000);
      setTimeout(run, 3000);
      setTimeout(run, 5000);
      setTimeout(run, 9000);
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

  // ---------------------------------------------------------------------
  // Android 原生桥（.github/scripts/patch-android-extras.py 注入）
  // ---------------------------------------------------------------------
  // window.__web2appAndroid：@JavascriptInterface 对象（分享/浏览器打开/清数据/退出）。
  // window.__web2appAndroidKey：主框架每次 onPageFinished 轮换的一次性密钥；evaluateJavascript
  // 只注主框架，跨源 iframe 读不到顶层变量 ⇒ 危险动作天然挡住第三方 iframe。
  function inTopFrame() {
    try { return window.top === window.self; } catch (e) { return false; }
  }
  function nativeBridge() {
    try { return window.__web2appAndroid || null; } catch (e) { return null; }
  }
  function bridgeKey() {
    try { return window.__web2appAndroidKey || ''; } catch (e) { return ''; }
  }
  /// 取桥方法；桥缺失 / 密钥未轮换 / iframe 内调用时 toast 提示并返回 null。
  function requireNative(fnName) {
    var b = nativeBridge();
    if (!b || typeof b[fnName] !== 'function' || !bridgeKey()) {
      toast('当前版本不支持该操作');
      return null;
    }
    if (!inTopFrame()) {
      toast('请在主页面操作');
      return null;
    }
    return b;
  }

  /// 复制当前链接：Clipboard API 优先（HTTPS/WebView 现代内核），失败退回 execCommand。
  function legacyCopy(text) {
    try {
      var ta = document.createElement('textarea');
      ta.value = text;
      ta.setAttribute('readonly', '');
      ta.style.cssText =
        'position:fixed!important;top:-9999px!important;left:-9999px!important;opacity:0!important;';
      (document.body || document.documentElement).appendChild(ta);
      ta.focus();
      ta.select();
      try { ta.setSelectionRange(0, text.length); } catch (e) { /* noop */ }
      var ok = false;
      try { ok = document.execCommand('copy'); } catch (e) { ok = false; }
      ta.remove();
      return ok;
    } catch (e) { return false; }
  }
  function copyCurrentLink() {
    var url = '';
    try { url = location.href; } catch (e) { url = ''; }
    function done(ok) { toast(ok ? '链接已复制' : '复制失败，请长按页面自行复制'); }
    try {
      if (navigator.clipboard && typeof navigator.clipboard.writeText === 'function') {
        var p = navigator.clipboard.writeText(url);
        if (p && typeof p.then === 'function') {
          p.then(function () { done(true); }, function () { done(legacyCopy(url)); });
          return;
        }
      }
      done(legacyCopy(url));
    } catch (e) { done(legacyCopy(url)); }
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

  function renderTabsSheet() {
    var bar = document.getElementById('__web2app_mobilebar__');
    if (!bar) return;
    var old = bar.querySelector('.w2m-sheet');
    if (old) old.remove();

    var sheet = document.createElement('div');
    sheet.className = 'w2m-sheet';

    // 「新建标签」固定在面板顶部：移动端没有桌面那条常驻标签栏，新增入口只能放这里。
    var addBtn = document.createElement('button');
    addBtn.type = 'button';
    addBtn.className = 'w2m-newrow';
    addBtn.textContent = '＋ 新建标签';
    addBtn.addEventListener('click', function () {
      emit('web2app:new-tab', null);
      closeSheet(bar);
    });
    sheet.appendChild(addBtn);

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

  // ---------------------------------------------------------------------
  // 「更多」面板（刷新 / 复制链接 / 分享 / 浏览器打开 / 清除浏览数据 / 退出）
  // ---------------------------------------------------------------------
  // 清除浏览数据两步确认状态：首次点击进入 armed（红字「再点一次确认清除」），
  // 3s 内不二次点击自动撤防。清数据会删掉 Cookie / 登录态，不可逆，必须防误触。
  var clearArmed = false;
  var clearArmTimer = 0;
  function clearArmedClear() {
    clearArmed = false;
    if (clearArmTimer) {
      try { clearTimeout(clearArmTimer); } catch (e) { /* noop */ }
      clearArmTimer = 0;
    }
  }

  function moreRow(icon, text) {
    var row = document.createElement('button');
    row.type = 'button';
    row.className = 'w2m-row';
    var ic = document.createElement('span');
    ic.className = 'w2m-ico';
    ic.innerHTML = icon;
    var label = document.createElement('span');
    label.className = 'w2m-label';
    label.textContent = text;
    row.appendChild(ic);
    row.appendChild(label);
    return row;
  }

  function renderMorePanel() {
    var bar = document.getElementById('__web2app_mobilebar__');
    if (!bar) return;
    var old = bar.querySelector('.w2m-sheet');
    if (old) old.remove();

    var sheet = document.createElement('div');
    sheet.className = 'w2m-sheet';

    // 头部：当前地址（截断显示，长按可看全；原生动作也用这个地址）。
    var head = document.createElement('div');
    head.className = 'w2m-morehead';
    var urlNow = '';
    try { urlNow = location.href; } catch (e) { urlNow = ''; }
    head.textContent = urlNow;
    head.title = urlNow;
    sheet.appendChild(head);

    // ① 刷新 / ② 复制链接：纯页面内能力，不依赖原生桥。
    var refreshRow = moreRow(ICO_REFRESH, '刷新');
    refreshRow.addEventListener('click', function () {
      closeSheet(bar);
      try { window.location.reload(); } catch (e) { /* noop */ }
    });
    sheet.appendChild(refreshRow);

    var copyRow = moreRow(ICO_COPY, '复制链接');
    copyRow.addEventListener('click', copyCurrentLink);
    sheet.appendChild(copyRow);

    // ③ 分享 / ④ 在浏览器打开：原生桥（Intent.ACTION_SEND / ACTION_VIEW）。
    var shareRow = moreRow(ICO_SHARE, '分享');
    shareRow.addEventListener('click', function () {
      var b = requireNative('share');
      if (!b) return;
      var titleNow = '';
      try { titleNow = document.title || ''; } catch (e) { titleNow = ''; }
      closeSheet(bar);
      try {
        b.share(bridgeKey(), urlNow, titleNow);
      } catch (e) { toast('调起分享失败'); }
    });
    sheet.appendChild(shareRow);

    var browserRow = moreRow(ICO_EXTERNAL, '在浏览器打开');
    browserRow.addEventListener('click', function () {
      var b = requireNative('openBrowser');
      if (!b) return;
      closeSheet(bar);
      try {
        b.openBrowser(bridgeKey(), urlNow);
      } catch (e) { toast('无法打开浏览器'); }
    });
    sheet.appendChild(browserRow);

    // ⑤ 清除浏览数据：两步确认后调原生桥（Cookie/WebStorage/缓存/历史/表单/库全清，
    //    完成后原生侧 evaluateJavascript 触发一次整页 reload）。
    var clearRow = moreRow(ICO_TRASH, '清除浏览数据');
    clearRow.addEventListener('click', function () {
      if (!clearArmed) {
        clearArmed = true;
        clearRow.classList.add('danger');
        var label0 = clearRow.querySelector('.w2m-label');
        if (label0) label0.textContent = '再点一次确认清除';
        if (clearArmTimer) clearTimeout(clearArmTimer);
        clearArmTimer = setTimeout(function () {
          clearArmed = false;
          clearArmTimer = 0;
          try {
            clearRow.classList.remove('danger');
            var label1 = clearRow.querySelector('.w2m-label');
            if (label1) label1.textContent = '清除浏览数据';
          } catch (e) { /* noop */ }
        }, 3000);
        return;
      }
      clearArmedClear();
      var b = requireNative('clearData');
      if (!b) return;
      closeSheet(bar);
      try {
        b.clearData(bridgeKey());
        toast('正在清除浏览数据…');
      } catch (e) { toast('清除失败'); }
    });
    sheet.appendChild(clearRow);

    // ⑥ 退出应用：finishAndRemoveTask + killProcess（原生侧兜底）。
    var exitRow = moreRow(ICO_EXIT, '退出应用');
    exitRow.addEventListener('click', function () {
      var b = requireNative('quitApp');
      if (!b) return;
      closeSheet(bar);
      try { b.quitApp(bridgeKey()); } catch (e) { /* noop */ }
    });
    sheet.appendChild(exitRow);

    bar.appendChild(sheet);
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

  // ---------------------------------------------------------------------
  // 页签文档状态上报（仅 HAS_TABS）
  // ---------------------------------------------------------------------
  // 移动端是【单 webview 页签记录】：壳侧只持有每个标签的 url/标题记录，真正的页面
  // 只有当前这一个。标题 / URL 一变就得上报，壳侧才好把记录刷新、推回列表面板。
  var docReportTimer = 0;
  function reportDoc() {
    if (docReportTimer) return;
    docReportTimer = setTimeout(function () {
      docReportTimer = 0;
      try {
        emit('web2app:mobile-doc', { title: document.title || '', url: location.href });
      } catch (e) { /* noop */ }
    }, 150);
  }

  function watchDoc() {
    try { reportDoc(); } catch (e) { /* noop */ }
    try { document.addEventListener('DOMContentLoaded', reportDoc); } catch (e) { /* noop */ }
    try { window.addEventListener('load', reportDoc); } catch (e) { /* noop */ }
    try { window.addEventListener('pageshow', reportDoc); } catch (e) { /* noop */ }
    try { window.addEventListener('popstate', reportDoc); } catch (e) { /* noop */ }
    try { window.addEventListener('hashchange', reportDoc); } catch (e) { /* noop */ }
    // <title> 常被 SPA 在首屏后改写：盯住 title 元素本身，也盯 <head> 里 title 的插入。
    try {
      var titleMo = new MutationObserver(reportDoc);
      var titleEl = document.querySelector ? document.querySelector('title') : null;
      if (titleEl) titleMo.observe(titleEl, { childList: true, characterData: true, subtree: true });
      titleMo.observe(document.documentElement, { childList: true, subtree: false });
    } catch (e) { /* 老引擎：靠事件兜底 */ }
    // pushState/replaceState 不触发 popstate：包一层，URL 变了也报（标签落点记的是 url）。
    try {
      ['pushState', 'replaceState'].forEach(function (name) {
        var orig = history[name];
        if (typeof orig !== 'function') return;
        history[name] = function () {
          var ret = orig.apply(this, arguments);
          reportDoc();
          return ret;
        };
      });
    } catch (e) { /* 页面封死 history 时放弃 */ }
  }

  // ---------------------------------------------------------------------
  // target=_blank / window.open 拦截 → 新页签（仅 HAS_TABS）
  // ---------------------------------------------------------------------
  // wry Android 没有多窗口（未覆写 onCreateWindow 通道）：网页里 target="_blank" 的
  // <a> 点了静默没反应，window.open(url) 也开不出窗口。页签模式下把这两类请求改发
  // web2app:open-tab {url}，由壳侧压成一条新页签记录并主导航过去（见 mobilebar.rs
  // 的 create_tab_with_url）。非页签模式**不装**：没有承接记录，不假装支持多窗口。
  // ⚠️ 只承接 http(s)：tel:/mailto:/weixin: 等外跳不拦，交给 patch-android-schemes.py
  //    的主框架 shouldOverrideUrlLoading 拉系统应用打开。
  function installBlankGuard() {
    function isHttpUrl(u) {
      return typeof u === 'string' && /^https?:\/\//i.test(u);
    }

    // ① 捕获阶段点击：向上找最近的 <a>，target 含 _blank（大小写不敏感、允许前后空白）
    //    且 href 是 http(s) 时截下来发事件；在捕获阶段 stopPropagation，页面自己的
    //    开窗脚本也拿不到这次点击。
    try {
      document.addEventListener('click', function (ev) {
        var node = ev.target;
        while (node && node.nodeType === 1) {
          if (node.tagName === 'A') {
            var tgt = node.target ? String(node.target).trim().toLowerCase() : '';
            if (tgt && /(?:^|\s)_blank(?:\s|$)/.test(tgt) && isHttpUrl(node.href)) {
              ev.preventDefault();
              ev.stopPropagation();
              emit('web2app:open-tab', { url: node.href });
            }
            return; // 最近的 <a> 拍板，不继续向外找
          }
          node = node.parentNode;
        }
      }, true);
    } catch (e) { /* noop */ }

    // ② window.open(url)：安卓开不出新窗口，http(s) 直接转新页签并返回 null
    //    （Chromium 多窗口被拒时本来也返回 null）；非 http(s) 保留原调用，交给内核 /
    //    scheme 拦截走外跳。
    try {
      var origOpen = window.open;
      window.open = function (url) {
        var u = url == null ? '' : String(url);
        if (isHttpUrl(u)) {
          emit('web2app:open-tab', { url: u });
          return null;
        }
        if (typeof origOpen === 'function') {
          try { return origOpen.apply(window, arguments); } catch (e) { /* noop */ }
        }
        return null;
      };
    } catch (e) { /* 页面封死 window.open 时放弃 */ }
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

  // 立刻置位防重入：挂载是异步的（要等 DOM 宿主节点），期间可能被再次注入。
  // notify：供外部探针 / 同世界脚本弹同一个 toast（与「更多」面板操作反馈同款）。
  window.__web2appMobileBar = { setTabs: setTabs, notify: toast };

  // 启动页（内置页）不画底栏：那时还没有可前进 / 后退的真实页面，画出来只会误导。
  // ⚠️ 这里不 return 之前已置位的防重标记 —— 换文档后本脚本会被重新注入，新的 JS 环境里
  //    这段判断会重新跑一遍（导航到真实页面时 location 已不是内置页）。
  if (isBuiltinUrl(location.href)) return;

  // 首帧近似高度：loading.js 与本脚本同批 document-start 注入且注册更早，它挂载时底栏
  // 还没渲染、读不到实测高度（含安全区）。先给内容区高度 BAR_H 兜首帧，底栏挂好后
  // markChromeReady() 再用 getBoundingClientRect 的实测值纠正。启动页没底栏，不给值。
  try { window.__web2appMobileBarH = BAR_H; } catch (e) { /* noop */ }

  // _blank / window.open 拦截必须**尽早**装：document 上的捕获监听 document-start 即可用，
  // window.open 也要赶在页面脚本第一次调用它之前包好 ⇒ 不等 whenHostReady。
  if (HAS_TABS) installBlankGuard();

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
      window.addEventListener('resize', function () {
        syncBarWidth(bar);
        // 安全区 / 视口高度可能随旋转、折叠屏展开变化，重新上报给加载层与页面探针。
        markChromeReady();
      }, { passive: true });
      window.addEventListener('orientationchange', function () {
        syncBarWidth(bar);
        markChromeReady();
      }, { passive: true });
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

    // 页签模式才上报文档 url / 标题（壳侧靠它维护列表面板的记录）；没有列表按钮时
    // 这串事件没有消费者，一个都不发。
    if (HAS_TABS) watchDoc();

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
