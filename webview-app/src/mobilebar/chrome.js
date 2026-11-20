  // =====================================================================
  // 分片：自适应配色（顶部状态栏区 + 底栏），由 lib.rs 按序拼进同一 IIFE
  // 前置：core.js 的 CSS 变量（--w2m-bg / --w2m-fg / --w2m-border）与 Android 原生桥
  //       （patch-android-extras.py 注入的 __web2appAndroid.setChrome）。
  // =====================================================================
  // 为什么需要这一片（2026-11-20 真机反馈）：
  //   · 顶部那一格是 Activity 窗口层：insets 把内容视图按状态栏高度内缩后，WebView 从状态栏
  //     下方才开始画，露出来的永远是主题 windowBackground（#f7f8fa）—— 页面自己的渐变
  //     到不了那里，纯 CSS 也改不动 ⇒ 只能经原生桥上色（见 setChrome 的 Kotlin 实现）。
  //   · 底栏是页面里的一层，但底色是写死的中性灰 #eef0f2；在饱和的页面上像另一个 App 的 UI。
  //
  // 取色策略（刻意保守：认不出来就什么都不改，维持壳的默认中性色）：
  //   ① 顶部优先 <meta name="theme-color">（站点自己的品牌色意图）；
  //   ② 否则在「接缝」处取色：顶部 = 页面视口 y=0（正好是状态栏下沿），
  //      底部 = 底栏上沿再往上 2px —— 取那一行元素的 background；
  //   ③ 该元素的背景是渐变时，按渐变方向取靠近接缝那一侧的色标（近似）；
  //   ④ 一路向上找不到不透明背景（图片 / 视频头图等）⇒ 放弃，保持现状。
  //
  // 已知边界：跨域图片 / 视频当背景时拿不到像素（canvas 会被污染），只能用 theme-color
  // 或保持中性色；多色标渐变的中间段被近似成端点色（只取色标，不做像素采样）。
  //
  // 性能：所有入口合并成一个 120ms 去抖；每次只读 2 个接缝的计算样式（不写页面 DOM，
  // 只写底栏自己的 inline 变量），滚动 / DOM 变动再频繁也不会形成同步布局抖动。

  var CHROME_TINT = 0.15; // 底栏混入页面色的比例（轻混色：以壳的表面色为主）
  var chromeTimer = 0;
  var chromeSent = '';    // 上次下发给原生的 "hex|light"，值没变不重复调用

  function prefersDark() {
    try {
      return !!(window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches);
    } catch (e) {
      return false;
    }
  }

  /// "rgb(1, 2, 3)" / "rgba(1, 2, 3, .5)" → {r,g,b,a}；解析不了返回 null。
  function parseColor(v) {
    if (!v) return null;
    var m = /rgba?\(([^)]+)\)/i.exec(String(v));
    if (!m) return null;
    var p = m[1].split(',');
    if (p.length < 3) return null;
    var r = parseInt(p[0], 10);
    var g = parseInt(p[1], 10);
    var b = parseInt(p[2], 10);
    var a = p.length > 3 ? parseFloat(p[3]) : 1;
    if (isNaN(r) || isNaN(g) || isNaN(b)) return null;
    return { r: r, g: g, b: b, a: isNaN(a) ? 1 : a };
  }

  /// WCAG 相对亮度（0=黑 1=白）：只用来决定图标明暗与底栏文字色。
  function luminance(c) {
    function ch(x) {
      x = x / 255;
      return x <= 0.03928 ? x / 12.92 : Math.pow((x + 0.055) / 1.055, 2.4);
    }
    return 0.2126 * ch(c.r) + 0.7152 * ch(c.g) + 0.0722 * ch(c.b);
  }

  /// 从 computed background-image 取「靠近接缝那一侧」的色标。
  /// Chromium 的计算值里渐变颜色已是 rgb()/rgba()，这里只认色标文本。
  function gradientStop(img, wantTop) {
    if (!img || img.indexOf('gradient') < 0) return null;
    var stops = img.match(/rgba?\([^)]+\)/gi);
    if (!stops || !stops.length) return null;
    var start = img.slice(img.indexOf('gradient'));
    var head = start.slice(0, Math.min(start.length, 64));
    var startAtTop;
    if (/to\s+bottom/i.test(head)) startAtTop = true;
    else if (/to\s+top/i.test(head)) startAtTop = false;
    else if (/to\s+(left|right)/i.test(head)) startAtTop = true; // 水平渐变：上下同色
    else {
      // 0deg = to top（起点在底部）、180deg = to bottom（起点在顶部）。
      var deg = /(-?\d+(?:\.\d+)?)deg/i.exec(head);
      startAtTop = deg ? parseFloat(deg[1]) >= 90 && parseFloat(deg[1]) <= 270 : true;
    }
    var idx = startAtTop === !!wantTop ? 0 : stops.length - 1;
    return parseColor(stops[idx]);
  }

  /// 取某个元素的「可见底色」：优先渐变，其次背景色；半透明一律视为没有（交给父级）。
  function colorOf(el, wantTop) {
    var cs = null;
    try { cs = window.getComputedStyle(el); } catch (e) { return null; }
    if (!cs) return null;
    var g = gradientStop(cs.backgroundImage, wantTop);
    if (g && g.a > 0.5) return g;
    var c = parseColor(cs.backgroundColor);
    if (c && c.a > 0.5) return c;
    return null;
  }

  /// `<meta name="theme-color">` 的颜色（站点的品牌色意图）；没有 / 解析不了返回 null。
  /// 注意：它描述的是整站主色，用作「顶部」最稳；底部接缝色优先用实际采样。
  function themeColor() {
    try {
      var m = document.querySelector('meta[name="theme-color"]');
      if (!m) return null;
      var v = String(m.getAttribute('content') || '').trim();
      var h = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(v);
      if (h) {
        var s = h[1];
        if (s.length === 3) s = s[0] + s[0] + s[1] + s[1] + s[2] + s[2];
        return {
          r: parseInt(s.slice(0, 2), 16),
          g: parseInt(s.slice(2, 4), 16),
          b: parseInt(s.slice(4, 6), 16),
          a: 1
        };
      }
      return parseColor(v);
    } catch (e) {
      return null;
    }
  }

  /// 在接缝处采样：wantTop=true 取页面视口 y=0，否则取底栏上沿往上一行。
  function sampleSeam(wantTop) {
    var x = 0;
    var y = 0;
    try { x = Math.max(0, Math.round((window.innerWidth || 0) / 2)); } catch (e) { return null; }
    if (wantTop) {
      y = 1;
    } else {
      var bar = document.getElementById('__web2app_mobilebar__');
      var top = 0;
      try { top = bar ? bar.getBoundingClientRect().top : 0; } catch (e) { top = 0; }
      if (!top) {
        try { top = (window.innerHeight || 0) - BAR_H; } catch (e) { return null; }
      }
      y = Math.max(1, Math.round(top) - 2);
    }
    var el = null;
    try { el = document.elementFromPoint(x, y); } catch (e) { return null; }
    // 底部接缝：面板遮罩 / toast 也挂在底栏里（fixed），命中的可能是我们自己 ——
    // 那就不是页面颜色，直接放弃这一轮（否则会在面板开着时把自己的底色当成采样值）。
    if (!wantTop && el) {
      try {
        var host = document.getElementById('__web2app_mobilebar__');
        if (host && host.contains(el)) return null;
      } catch (e) { /* noop */ }
    }
    var depth = 0;
    while (el && depth++ < 12) {
      var c = colorOf(el, wantTop);
      if (c) return c;
      el = el.parentElement;
    }
    return null;
  }

  /// 按权重混合：src 占 w，dst 占 1-w。
  function mix(src, dst, w) {
    return {
      r: Math.round(src.r * w + dst.r * (1 - w)),
      g: Math.round(src.g * w + dst.g * (1 - w)),
      b: Math.round(src.b * w + dst.b * (1 - w)),
      a: 1
    };
  }

  function rgbCss(c) {
    return 'rgb(' + c.r + ',' + c.g + ',' + c.b + ')';
  }

  function hexCss(c) {
    function p(n) {
      n = Math.max(0, Math.min(255, Math.round(n)));
      return (n < 16 ? '0' : '') + n.toString(16);
    }
    return '#' + p(c.r) + p(c.g) + p(c.b);
  }

  /// 顶部状态栏区：经原生桥把窗口底色刷成页面接缝色 + 图标明暗（与桥的其它调用同一条
  /// 主框架门控；桥缺失 / 调用失败都静默 —— 顶部保持窗口底色，不影响页面）。
  function sendChrome(color) {
    var light = luminance(color) > 0.6;
    var sig = hexCss(color) + '|' + (light ? '1' : '0');
    if (sig === chromeSent) return;
    try {
      if (window.top !== window.self) return;
      var b = window.__web2appAndroid;
      var k = window.__web2appAndroidKey || '';
      if (!b || typeof b.setChrome !== 'function' || !k) return;
      b.setChrome(k, hexCss(color), !!light);
      chromeSent = sig;
    } catch (e) { /* noop */ }
  }

  /// 采一次色并落到「顶部 + 底栏」。认不出颜色时**什么都不改**（维持壳的默认中性色）。
  function applyChrome(bar) {
    if (!bar) return;
    var dark = prefersDark();
    var theme = themeColor();
    var top = theme || sampleSeam(true);
    var bottom = sampleSeam(false) || theme;
    if (top) sendChrome(top);

    var base = bottom || top;
    if (!base) return;
    // 轻混色：以壳的表面色为主，只掺一点页面色 —— 既不突兀，又保留「这是壳的 UI」边界。
    var surface = dark ? { r: 31, g: 37, b: 48 } : { r: 238, g: 240, b: 242 };
    var bg = mix(base, surface, CHROME_TINT);
    var lightBg = luminance(bg) > 0.55;
    bar.style.setProperty('--w2m-bg', rgbCss(bg));
    bar.style.setProperty('--w2m-fg', lightBg ? '#2c3440' : '#e6ebf2');
    bar.style.setProperty('--w2m-border', lightBg ? 'rgba(0,0,0,.08)' : 'rgba(255,255,255,.07)');
  }

  function scheduleChrome(bar) {
    if (chromeTimer) return;
    chromeTimer = setTimeout(function () {
      chromeTimer = 0;
      try { applyChrome(bar); } catch (e) { /* noop */ }
    }, 120);
  }

  /// 挂载自适应配色。触发时机：首帧 / 首屏补采（页面样式未落定时）、滚动、视口变化、
  /// 系统日夜切换、DOM 变动（SPA 换页 / 换肤）。都只是「重新采样」，没有常驻轮询。
  function installChromeColors(bar) {
    if (!bar) return;
    scheduleChrome(bar);
    [300, 1200].forEach(function (ms) {
      setTimeout(function () { scheduleChrome(bar); }, ms);
    });
    try {
      document.addEventListener('DOMContentLoaded', function () { scheduleChrome(bar); });
      window.addEventListener('load', function () { scheduleChrome(bar); });
    } catch (e) { /* noop */ }
    try {
      // scroll 不冒泡，但捕获阶段能收到内嵌滚动容器的滚动。
      document.addEventListener('scroll', function () { scheduleChrome(bar); }, { passive: true, capture: true });
      window.addEventListener('resize', function () { scheduleChrome(bar); }, { passive: true });
      window.addEventListener('orientationchange', function () { scheduleChrome(bar); }, { passive: true });
    } catch (e) { /* noop */ }
    try {
      var mq = window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)');
      if (mq && mq.addEventListener) mq.addEventListener('change', function () { scheduleChrome(bar); });
      else if (mq && mq.addListener) mq.addListener(function () { scheduleChrome(bar); });
    } catch (e) { /* noop */ }
    try {
      if (window.MutationObserver) {
        // 只观察 childList（换页 / 插内容）：底栏自己的 inline 变量不属于 childList，
        // 不会形成「写样式 → 触发观察 → 再写样式」的自激循环。
        var mo = new MutationObserver(function () { scheduleChrome(bar); });
        mo.observe(document.documentElement, { childList: true, subtree: true });
        // 页面换肤通常改 body/html 的 class 或 style：只盯这两个节点自身的属性。
        if (document.body) {
          mo.observe(document.body, { attributes: true, attributeFilter: ['class', 'style'] });
        }
      }
    } catch (e) { /* noop */ }
  }
