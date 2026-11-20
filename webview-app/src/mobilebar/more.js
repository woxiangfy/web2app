  // =====================================================================
  // 分片：「更多」能力（清除浏览数据 / 退出应用）
  // 前置：原生桥要素由 patch-android-extras.py 注入；toast / closeSheet 由 core.js 提供。
  // ⚠️ 面板按产品需求（2026-11-20）只保留两行：清除浏览数据 / 退出应用，且不再显示
  //    当前地址；刷新 / 复制链接 / 分享 / 在浏览器打开均已下线（原生桥的 share /
  //    openBrowser 方法仍在 Kotlin 侧，属预留能力，没有 UI 入口）。
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

  // ---------------------------------------------------------------------
  // 「更多」面板（清除浏览数据 / 退出应用）
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

    // ① 清除浏览数据：两步确认后调原生桥（Cookie/WebStorage/缓存/历史/表单/库全清，
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

    // ② 退出应用：finishAndRemoveTask + killProcess（原生侧兜底）。
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

