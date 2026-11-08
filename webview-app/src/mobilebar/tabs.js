  // =====================================================================
  // 分片：页签能力（列表面板 / 文档 url·标题上报 / _blank·window.open 拦截）
  // 前置：共享状态 tabRows / activePanel 与 emit / closeSheet 由 core.js 提供。
  // =====================================================================
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

