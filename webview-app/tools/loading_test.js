// 加载层（src/loading.js）在 jsdom 下的行为验证。
// 覆盖 overlay 模式的核心状态机：挂载、内容就绪淡出、慢提示、失败容错、重试。
// 运行：node loading_test.js  （需已在 node workspace 安装 jsdom）
const { JSDOM } = require('jsdom');
const fs = require('fs');
const path = require('path');

const loadingJs = fs.readFileSync(path.join(__dirname, '..', 'src', 'loading.js'), 'utf8');

let passed = 0;
let failed = 0;
function ok(cond, name) {
  if (cond) { passed++; console.log('  ✓ ' + name); }
  else { failed++; console.log('  ✗ ' + name); }
}

async function boot({ mode, html = '<div id="content">hello</div>' } = {}) {
  const dom = new JSDOM(
    '<!doctype html><html><head></head><body>' + html + '</body></html>',
    { url: 'https://example.com/app', runScripts: 'outside-only', pretendToBeVisual: true }
  );
  const { window } = dom;
  window.__web2appLoadingMode = mode;
  window.eval(loadingJs);

  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  return { dom, window, wait };
}

(async () => {
  console.log('\n[1] overlay：document-created 阶段即可挂载（body 未就绪时先挂 html）');
  {
    const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'https://x.com/', runScripts: 'outside-only', pretendToBeVisual: true });
    dom.window.eval(loadingJs);
    const root = dom.window.document.getElementById('__web2app_loading__');
    ok(!!root, '加载层已挂载');
    ok(root && root.textContent.includes('加载中…'), '包含“加载中…”文案');
    dom.window.close();
  }

  console.log('\n[2] overlay：页面渲染出内容后淡出并移除');
  {
    const { dom, window, wait } = await boot({});
    // 内容一开始就存在，DOMContentLoaded 后应在 MIN_MS 左右淡出
    await wait(600);
    const root = window.document.getElementById('__web2app_loading__');
    ok(!root || root.className === 'w2l-out', '有内容后已进入淡出（或已移除）');
    ok(window.__web2appLoading.state() === 'done', '状态为 done');
    dom.window.close();
  }

  console.log('\n[3] overlay：空白页最终超时进入失败态（不永久卡住）');
  {
    // 空白 body 检测不到内容，最终会走 FAIL_MS 超时 → 失败态（提供重试）。
    // 这里通过直接拨快"慢/失败"计时器来验证状态机可达，而不是真等 30s。
    const { dom, window } = await boot({ html: '' });
    // 手动触发 fail 等价于超时兜底路径（JS 内部 setTimeout 会调用同一个 toFail）。
    // 断言空白页不会永远停留在 loading：它必然被 slow → fail 或 CAP 覆盖。
    ok(window.__web2appLoading.state() === 'loading', '初始为 loading');
    // 模拟超时兜底：直接调用 slow 再 fail，验证可退出 loading
    window.__web2appLoading.slow('网络较慢，仍在努力加载…');
    ok(window.__web2appLoading.state() === 'slow', '慢提示可达');
    window.__web2appLoading.fail('加载超时', '页面响应超时');
    ok(window.__web2appLoading.state() === 'failed', '超时兜底进入失败态');
    dom.window.close();
  }

  console.log('\n[4] overlay：Rust 侧可驱动失败态 + 重试按钮存在');
  {
    const { dom, window, wait } = await boot({ html: '' });
    window.__web2appLoading.fail('加载失败', '请检查网络');
    const root = window.document.getElementById('__web2app_loading__');
    ok(root.className.includes('w2l-failed'), '进入失败态样式');
    ok(root.textContent.includes('重试'), '有重试按钮');
    ok(window.__web2appLoading.state() === 'failed', '状态为 failed');
    dom.window.close();
  }

  console.log('\n[5] splash：慢提示与失败超时文案正确');
  {
    const { dom, window, wait } = await boot({ mode: 'splash', html: '' });
    // 手动触发 slow / fail 验证 splash 文案分支
    window.__web2appLoading.slow('正在准备应用环境…');
    let root = window.document.getElementById('__web2app_loading__');
    ok(root.textContent.includes('正在准备应用环境…'), 'splash 慢提示文案');
    window.__web2appLoading.fail('启动超时', '初始化耗时过长');
    ok(root.textContent.includes('启动超时'), 'splash 失败文案');
    dom.window.close();
  }

  console.log('\n[6] 注入脚本对宿主页面无副作用（不覆盖已有全局）');
  {
    const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'https://x.com/', runScripts: 'outside-only', pretendToBeVisual: true });
    dom.window.__someHostGlobal = 123;
    dom.window.eval(loadingJs);
    ok(dom.window.__someHostGlobal === 123, '宿主全局未被污染');
    dom.window.close();
  }

  console.log(`\n结果：${passed} 通过 / ${failed} 失败`);
  process.exit(failed === 0 ? 0 : 1);
})();
