/**
 * 多页签模式下首个页签失败时，向壳页面（shell.html）注入的**可见**失败遮罩。
 *
 * 为什么必须有它：内容视图不注入 loading.js（见 index.ts::createWindow 的 onSpawn），
 * 首个页签 spawn 失败（sidecar 缺失 / DNS 解析失败 / 代理启动失败）时视图已被
 * spawnTab 回收，壳上只剩标签栏下面一片空白；而默认 `debug=false` 时 console 三路
 * 又被静默 —— 用户侧零证据、零入口，正是本项目最忌的失败形态（单窗口模式有
 * splash 的 pendingFail + 重试按钮，这里必须有等价物）。
 *
 * 从 index.ts 抽出：两个函数只依赖传入的宿主窗口与 TAB_BAR_HEIGHT 常量，
 * 不触碰任何模块级状态。
 */
import type { BrowserWindow } from 'electron'

import { TAB_BAR_HEIGHT } from './tabs'

const SHELL_FAIL_ID = '__w2a_shell_fail'

/** 在壳页面注入失败遮罩（错误文案 + 「重试」按钮）。幂等：重复调用先撤旧遮罩。 */
export function showShellStartupError(win: BrowserWindow, detail: string): void {
  if (win.isDestroyed()) return
  // 文案经 JSON.stringify 嵌入（天然 JS 字符串转义，杜绝引号/换行注入断脚本）；
  // 按钮经 shell-preload 白名单里的 retry_first_tab 重试，主进程侧还有 assertShell 校验。
  // 遮罩的 top 与标签栏高度同源（TAB_BAR_HEIGHT 插值）：写死 38 会在改栏高时盖住标签栏。
  const js = `(function () {
    var OLD = document.getElementById(${JSON.stringify(SHELL_FAIL_ID)});
    if (OLD) OLD.parentNode.removeChild(OLD);
    var box = document.createElement('div');
    box.id = ${JSON.stringify(SHELL_FAIL_ID)};
    box.style.cssText =
      'position:fixed;top:${TAB_BAR_HEIGHT}px;left:0;right:0;bottom:0;display:flex;flex-direction:column;' +
      'gap:14px;align-items:center;justify-content:center;padding:24px;' +
      'background:#f7f8fa;color:#1f2530;font:14px/1.6 "Segoe UI","Microsoft YaHei",sans-serif;' +
      'z-index:2147483646;user-select:none';
    var msg = document.createElement('div');
    msg.style.cssText = 'max-width:560px;text-align:center;word-break:break-word';
    msg.textContent = ${JSON.stringify(detail)};
    var btn = document.createElement('button');
    btn.type = 'button';
    btn.textContent = '重试';
    btn.style.cssText =
      'padding:7px 22px;border:1px solid #c3cbd8;border-radius:6px;background:#fff;' +
      'color:#1f2530;font:14px "Segoe UI","Microsoft YaHei",sans-serif;cursor:pointer';
    btn.onclick = function () {
      btn.disabled = true;
      btn.textContent = '重试中…';
      window.__TAURI_INTERNALS__.invoke('retry_first_tab').catch(function (e) {
        btn.disabled = false;
        btn.textContent = '重试';
        alert('重试失败：' + (e && e.message ? e.message : String(e)));
      });
    };
    box.appendChild(msg);
    box.appendChild(btn);
    document.body.appendChild(box);
  })();`
  win.webContents.executeJavaScript(js, true).catch(() => {
    /* 壳页面尚未就绪：首签失败只可能发生在壳加载完成之后，此处静默即可 */
  })
}

/** 撤掉壳页面的失败遮罩（重试成功 / 正常启动路径）。幂等。 */
export function hideShellStartupError(win: BrowserWindow): void {
  if (win.isDestroyed()) return
  win.webContents
    .executeJavaScript(
      `try{var e=document.getElementById(${JSON.stringify(SHELL_FAIL_ID)});if(e)e.parentNode.removeChild(e);true}catch(e){false}`,
      true,
    )
    .catch(() => {
      /* 同上 */
    })
}
