/*
 * web2app 多页签「真实文档已提交」标记（tab_doc_ready.js）
 * ---------------------------------------------------------------------------
 * 仅多页签模式（TABS_ENABLED=true，Windows）注入到每个 tab 窗口。职责只有一件：
 * 当该 tab 里**创建了一个真实网页文档**时，发一条核心事件
 *   emit(web2app:tab-doc-ready, {label, token}) → Rust `tabs::mark_tab_doc_ready`
 * 把该 tab 标记为「已提交过文档」。
 *
 * 为什么需要它（用户场景）：站点点击后 `window.open` 一个附件地址，浏览器在下载开始后
 * 会把这个只为下载而开的空页签自动关掉、回到点击时的页面。附件响应
 * （`Content-Disposition: attachment`）**不会创建文档** ⇒ 本脚本不执行 ⇒ Rust 侧该 tab
 * 一直是「未提交」状态；下载落盘 / 取消后据此自动关页签（见 tabs/window.rs::
 * close_tab_after_download）。已加载过页面的 tab 会收到本事件，因而**不会**被误关。
 *
 * ⚠️ 为什么不用 WebView2 的 NavigationCompleted：它对「转成下载」的导航也会派发
 *   （带失败状态），无法区分「提交了文档」与「这次导航变成了下载」；初始化脚本只在
 *   文档真正创建时执行，是这里最确定、也最窄的信号。
 *
 * ⚠️ 本地占位页（tab 建窗时加载的 `http://tauri.localhost/index.html`）必须排除：
 *   它也是一个真实文档，若不早退，每个 tab 一建出来就被标成「已提交」，自动关页签失效。
 *   判定与建窗时的 loading 模式标记（tabs/window.rs）保持同一写法。
 *
 * 安全：payload 带建窗时注入的一次性令牌 window.__web2appTabToken，Rust 侧对不上就丢弃
 *   （app.listen 的 Event 不携带发送窗口标识）。本脚本只标记自己这一签，页面伪造不了
 *   别的页签的令牌。
 *
 * 设计约束：
 *   1. 零依赖；事件通路按 (__TAURI__.event → __TAURI_INTERNALS__) 顺序探测，
 *      与 tab_title.js / tab_keys.js 同一姿势；
 *   2. 全程 try/catch：注入进第三方页面，任何异常都不影响页面；
 *   3. 只在【主框架】标记（initialization_script 会注入每一个 iframe，子框架不算数）；
 *   4. 桥未就绪时短暂重试（最多约 1s），就绪后只成功标记一次。
 */
(function () {
  'use strict';
  try {
    if (window.self !== window.top) return;

    var label = window.__web2appTabLabel;
    var token = window.__web2appTabToken;
    if (!label || !token) return;

    // 本地占位页不算「提交过真实文档」：download 场景的判定依赖它保持未提交。
    if (location.hostname === 'tauri.localhost' || location.protocol === 'tauri:') return;

    var EVENT = 'web2app:tab-doc-ready';

    function emit() {
      var payload = { label: label, token: token };
      try {
        var T = window.__TAURI__;
        if (T && T.event && typeof T.event.emit === 'function') {
          var r = T.event.emit(EVENT, payload);
          if (r && r.then) r.then(null, function () {});
          return true;
        }
        var I = window.__TAURI_INTERNALS__;
        if (I && typeof I.invoke === 'function') {
          I.invoke('plugin:event|emit', { event: EVENT, payload: payload });
          return true;
        }
      } catch (e) { /* 忽略：标记失效只影响「下载后自动关页签」，不影响页面 */ }
      return false;
    }

    if (emit()) return;
    // 桥尚未注入时的兜底：短暂轮询，成功一次即收工。
    var tries = 0;
    var timer = setInterval(function () {
      tries += 1;
      if (emit() || tries >= 20) clearInterval(timer);
    }, 50);
  } catch (e) { /* 忽略 */ }
})();
