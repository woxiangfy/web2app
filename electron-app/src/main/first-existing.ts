/**
 * 资源候选扫描（**纯 node:fs 规则**，零 Electron 依赖）。
 *
 * 全应用的「打包候选 vs 开发态 webview-app 目录」定位都走同一个形状：
 *   sidecar.ts::locateSidecar / tray.ts::locateTrayIcon /
 *   settings-window.ts::locateSettingsHtml
 * 此前各自抄一份 `for (const c of candidates) if (existsSync(c)) return c`，
 * 2026-10-01 收敛到这里 —— 循环本身没有差异，差异只在**候选列表怎么构造**
 * （asar 内 vs resourcesPath 外、开发态子目录不同），那部分刻意留在各 locator 里。
 *
 * ⚠️ 各 locator 用 `app.isPackaged` 而非「文件存在与否」决定候选集的纪律不变，
 *    本函数只负责在给定候选集里找第一个存在的，不参与打包/开发判定。
 */
import { existsSync } from 'node:fs'

/** 返回候选路径中第一个实际存在的；全部不存在（或候选为空）时返回 null。 */
export function firstExisting(candidates: readonly string[]): string | null {
  for (const c of candidates) {
    if (existsSync(c)) return c
  }
  return null
}
