/**
 * 导航相关文本 / 判定的小工具（零 Electron 依赖，2026-11-07 架构审查 M-4）。
 *
 * 「无法打开 …（desc，code）」这条文案此前在 `index.ts` 与 `tab-spawn.ts` 各写一份，
 * 改一处漏一处时用户看到的提示就会分叉；内置页判定同理（单窗口只认 `splash:`，
 * 而 wry 在 Windows/Android 会把启动页改写成 `https://splash.localhost/`）。
 */

/** 加载失败的统一文案（单窗口与页签共用，改文案只改这里）。 */
export function navFailText(url: string, desc: string, code: number): string {
  return `无法打开 ${url}（${desc}，${code}）`
}

/**
 * 内置启动页判定：原生 `splash://` scheme，与 wry 在 Windows/Android 改写后的
 * `https://splash.localhost/` 两形态都要认（与 `webview-app/src/titlebar.js`
 * 的 isBuiltinUrl 逐字对齐）。
 */
export function isSplashUrl(url: string): boolean {
  return /^splash:/i.test(url) || /^https?:\/\/splash\.localhost(?:[/?#]|$)/i.test(url)
}

/**
 * 内部页面判定（不该被当成用户可访问的目标地址）：启动页 + `about:` / `chrome:` / `devtools:`
 * + 本应用自己的 `settings:` / `shell:` / `floaticon:`。用于「新窗口请求一律拒绝」这类闸门 ——
 * 这些地址没有可独立显示的实体页（与 `preload.ts::isInternalPage` 的 scheme 表保持一致）。
 */
export function isInternalUrl(url: string): boolean {
  if (!url) return true
  return isSplashUrl(url) || /^(about|chrome|devtools|settings|shell|floaticon):/i.test(url)
}
