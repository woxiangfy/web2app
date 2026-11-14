  // =====================================================================
  // 分片：底部空间契约的 JS 半边（由 lib.rs 按序拼进同一 IIFE，共享 core 作用域）
  // 职责：底栏让位探测、页面贴底元素上移、100vh 容器缩高、重排调度。
  // =====================================================================
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
  // 软键盘让位（visualViewport，#12）
  // ---------------------------------------------------------------------
  // 三种情况要分开看：
  //   · `adjustResize`（WebView 默认 resize 模式）：键盘弹起时 **layout viewport 变矮**，
  //     `innerHeight` 跟着变小、fixed 底栏自然被顶上来 ⇒ 这里算出的差值是 0，不动作。
  //   · `adjustPan` / edge-to-edge 沉浸 / `adjustNothing`：layout viewport 不变，键盘是
  //     **盖在页面上**的图层，fixed bottom:0 的底栏会被整个挡住 ⇒ 把底栏抬到键盘上沿。
  //   · 地址栏自动收起：visualViewport 也会变大，但幅度远小于键盘（一般 < 60px）。
  // 所以判据是「可视视口比 layout 视口矮 60px 以上」—— 只认键盘，不认浏览器 UI 抖动。
  // 抬升只改底栏的 inline bottom；页面 body 的底部让位（layout 坐标）不跟着改：
  // 键盘覆盖的是 layout 视口下沿，那是页面自身滚动的事，不该由底栏替它补偿。
  var W2M_KB_MIN_GAP = 60;
  var w2mKbTimer = 0;

  function applyKeyboardInset() {
    try {
      var bar = document.getElementById('__web2app_mobilebar__');
      if (!bar || !bar.style) return;
      var vv = window.visualViewport;
      if (!vv) {
        bar.style.removeProperty('bottom');
        bar.removeAttribute('data-w2m-keyboard');
        return;
      }
      var gap = Math.round((window.innerHeight || 0) - (vv.height + vv.offsetTop));
      if (gap >= W2M_KB_MIN_GAP) {
        bar.style.setProperty('bottom', gap + 'px', 'important');
        bar.setAttribute('data-w2m-keyboard', '1');
      } else {
        bar.style.removeProperty('bottom');
        bar.removeAttribute('data-w2m-keyboard');
      }
    } catch (e) { /* 绝不把异常抛给宿主页面 */ }
  }

  function installKeyboardAvoid() {
    try {
      var vv = window.visualViewport;
      if (!vv) return; // 老内核没有 visualViewport：保持现状（adjustResize 仍然可用）
      var schedule = function () {
        if (w2mKbTimer) clearTimeout(w2mKbTimer);
        // 80ms 去抖：键盘弹出动画期间 visualViewport 会连发十几帧，逐帧写 style 会抖。
        w2mKbTimer = setTimeout(applyKeyboardInset, 80);
      };
      vv.addEventListener('resize', schedule);
      vv.addEventListener('scroll', schedule);
      applyKeyboardInset();
    } catch (e) { /* noop */ }
  }

