/**
 * 随 asar 分发的注入资源清单 —— **单一真源**（2026-11-07，架构审查 H-1）。
 *
 * `stage-assets.cjs` 按这份清单把文件拷进 `out/inject/`，`check-asar-config.cjs`
 * 出包后按同一份清单校验 asar 内是否齐全。此前两处各写一份手写数组，
 * 新增 `floaticon.html` / `icon.png` 时只改了 stage 侧，护栏就漏掉了它们
 * —— 漏件时构建与 CI 全绿，真机上表现为球页 404 / 托盘图标缺失。
 *
 * ⚠️ 为什么单独一个模块而不是从 stage-assets.cjs 导出：那个脚本顶层就会
 *    读文件、拷贝、`process.exit`，被 require 时会顺带执行一遍。清单单独成模块，
 *    两处 `require` 都只拿到纯数据。
 *
 * 条目格式：`[源（相对仓库根）, 目标文件名]`。
 */
const STAGED_INJECT = [
  ['webview-app/src/splash.html', 'splash.html'],
  ['webview-app/src/titlebar.js', 'titlebar.js'],
  ['webview-app/src/loading.js', 'loading.js'],
  ['webview-app/src/floaticon.html', 'floaticon.html'],
  // 悬浮球内的软件图标（与 floaticon.html 同目录，页面以相对路径 icon.png 引用）。
  // 与托盘 / exe 图标同源：打包时 icon_url 会先覆盖 webview-app/icons/icon.png（见
  // packlib/driver-electron.mjs），所以这里 stage 到的始终是本次配置的软件图标。
  ['webview-app/icons/icon.png', 'icon.png'],
  ['webview-app/src/shell.html', 'shell.html'],
  ['webview-app/dist/settings.html', 'settings.html'],
]

module.exports = { STAGED_INJECT }
