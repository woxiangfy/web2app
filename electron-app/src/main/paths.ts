/**
 * 资源定位的**唯一出口**（2026-11-07 架构审查 M-3）。
 *
 * 此前「开发态读仓库 / 打包态读 asar」这件事有六处手写 `app.isPackaged` 三元
 * （`index.ts` 的 injectDir / floatIconPath / configRoot / 两个 preload 路径、
 * `settings-window.ts` 的 settings.html 候选），新增资源时很容易只改一处。
 * 现在所有「按打包态分叉」的判断都收口在这里，`first-existing.ts` 仍是底层扫描。
 *
 * ⚠️ 判据一律用 `app.isPackaged`，**不用「文件是否存在」**：`pnpm run stage` 会在开发目录
 *    生成 `out/inject/`，按存在性判会让开发态误读 stage 副本、失去 webview-app 单一真源。
 */
import { app } from 'electron'
import { join } from 'node:path'

/** 打包态（asar 内）为 true；开发态（`electron .`）为 false。 */
export function isPackaged(): boolean {
  return app.isPackaged
}

/** 仓库根（`out/main` 上三级）：开发态取 `webview-app/` 下的单一真源用。 */
function repoRoot(): string {
  return join(__dirname, '..', '..', '..')
}

/** 注入资源目录：打包态 `app.asar/out/inject/`，开发态 `webview-app/src/`。 */
export function injectDir(): string {
  if (isPackaged()) return join(app.getAppPath(), 'out', 'inject')
  return join(repoRoot(), 'webview-app', 'src')
}

/** 注入资源里的单个文件（titlebar.js / loading.js / shell.html / splash.html / settings.html / icon.png…）。 */
export function injectFile(name: string): string {
  return join(injectDir(), name)
}

/** 悬浮球内软件图标：打包态在 `out/inject/`，开发态直接读 `webview-app/icons/icon.png`。 */
export function floatIconPath(): string {
  if (isPackaged()) return injectFile('icon.png')
  return join(repoRoot(), 'webview-app', 'icons', 'icon.png')
}

/** 运行期配置根：打包态 `app.asar/out/`，开发态 `electron-app/`（`pnpm run stage` 会同步到 `out/`）。 */
export function configRoot(): string {
  if (isPackaged()) return join(app.getAppPath(), 'out')
  return join(__dirname, '..', '..')
}

/** renderer preload 的绝对路径（打包态在 asar 内，两种模式同一条表达式）。 */
export function preloadPath(name: string): string {
  return join(__dirname, '..', 'renderer', name)
}
