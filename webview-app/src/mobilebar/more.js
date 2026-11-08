  // =====================================================================
  // 分片：「更多」能力（刷新 / 复制链接 / 分享 / 浏览器打开 / 清除浏览数据 / 退出）
  // 前置：原生桥要素由 patch-android-extras.py 注入；toast / closeSheet 由 core.js 提供。
  // =====================================================================
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

