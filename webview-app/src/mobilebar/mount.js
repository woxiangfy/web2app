  // =====================================================================
  // 分片：入口生命周期（防重标记 → 内置页早退 → 首帧高度 → 挂载 → 自愈守护）
  // ⚠️ 必须是**最后一片**：它在结尾关闭 core.js 打开的 IIFE。
  // =====================================================================
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
