/*
 * web2app 多页签快捷键（tab_keys.js）—— 目前只有一条：Ctrl+W 关闭当前页签。
 * ---------------------------------------------------------------------------
 * 仅多页签模式（TABS_ENABLED=true，Windows）注入到每个 tab 窗口，与 tab_title.js 同一套
 * 「注入脚本 + 核心事件」姿势：
 *   按键 → emit(web2app:close-tab, {label, token}) → Rust `tabs::request_close_tab`
 *   → 校验令牌 → close_tab_inner（**首页那一签在这里被拒绝**，见 tabs/window.rs）。
 *
 * ⚠️⚠️ 为什么是事件而不是 invoke 应用命令：tab 窗口加载的是【远程 origin】（本地代理的
 *   http://127.0.0.1:<port> 或直连上游），Tauri 2 对远程 origin 的应用自定义命令一律拒
 *   （没有 app 级 ACL 清单可授权），而 `core:event:allow-emit` 可以授权给远程 origin
 *   （capabilities/tabs-remote.json 覆盖 "tab-*"）。理由与 tab_title.js 头注释完全相同。
 *
 * ⚠️ 为什么不用 WebView2 的 AcceleratorKeyPressed：那个事件只对「WebView2 自己会处理的
 *   加速键」触发（F5 / Ctrl+F 之类）。WebView2 没有页签概念，Ctrl+W 不在它的加速键表里
 *   ⇒ 事件根本不来，写了也是「看似生效、实际从不触发」。页面级 keydown 是确定的通路。
 *
 * 安全：payload 带建窗时注入的一次性令牌 window.__web2appTabToken，Rust 侧对不上就丢弃
 *   （app.listen 的 Event 不携带发送窗口标识，没有令牌则任何页面都能关掉别的页签）。
 *   本脚本只发「关闭自己」这个意图，label 取自本窗口被注入的 label —— 页面伪造不了别的
 *   页签的令牌，最多关掉自己那一签。
 *
 * 设计约束：
 *   1. 零依赖；事件通路按 (__TAURI__.event → __TAURI_INTERNALS__) 顺序探测；
 *   2. 全程 try/catch，注入进第三方页面，任何异常都不影响页面；
 *   3. 只在【主框架】运行（initialization_script 会注入每一个 iframe，子框架里按键不关本签）；
 *   4. 用 **capture** 阶段监听：站点自己的 Ctrl+W（编辑器类页面常见）不能抢在前面。
 */
(function () {
  'use strict';
  try {
    if (window.self !== window.top) return;

    var label = window.__web2appTabLabel;
    var token = window.__web2appTabToken;
    if (!label || !token) return;

    var EVENT = 'web2app:close-tab';

    function emit() {
      var payload = { label: label, token: token };
      try {
        var T = window.__TAURI__;
        if (T && T.event && typeof T.event.emit === 'function') {
          var r = T.event.emit(EVENT, payload);
          if (r && r.then) r.then(null, function () {});
          return;
        }
        var I = window.__TAURI_INTERNALS__;
        if (I && typeof I.invoke === 'function') {
          I.invoke('plugin:event|emit', { event: EVENT, payload: payload });
        }
      } catch (e) { /* 忽略：快捷键失效不影响页面 */ }
    }

    window.addEventListener('keydown', function (e) {
      try {
        // 只认最窄的组合：Ctrl+W（不带 Shift / Alt / Meta），与 Electron 侧
        // page-behavior.ts::isCloseTabKey 逐条对齐（双壳同一判据）。
        if (e.ctrlKey !== true || e.shiftKey || e.altKey || e.metaKey) return;
        // 只认**逻辑**键位（'w' / 'W'，大写锁定开着时给 'W'）。
        // ⚠️ 不要用 e.code（物理键位）做兜底：e.code 按 US 布局命名，AZERTY 上「生出 'w'
        //   的那个物理键」的 code 是 KeyZ ⇒ 用户按 Ctrl+Z（撤销）会被当成 Ctrl+W 关掉页签。
        //   Electron 侧 isCloseTabKey 同样只看 key —— 双壳判据必须字面一致。
        var isW = e.key === 'w' || e.key === 'W';
        if (!isW) return;
        // 长按不连关（与 Electron 侧 isAutoRepeat 排除同一取舍）。
        if (e.repeat) return;
        // 吃掉这次按键：页面自己的 Ctrl+W 不再触发，也不再冒泡到页面别处的监听。
        e.preventDefault();
        e.stopPropagation();
        emit();
      } catch (err) { /* 忽略 */ }
    }, true);
  } catch (e) { /* 忽略 */ }
})();
