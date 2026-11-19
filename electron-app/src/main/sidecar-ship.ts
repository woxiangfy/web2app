/**
 * 打包期 sidecar 随包判定与落盘动作（electron-builder `afterPack` 钩子使用）。
 *
 * 为什么需要它：`proxy_enabled=false` 且 `app_url` 是字符串时，运行期
 * [`needsSidecar`](./navigate.ts) 恒为假 —— `w2a-proxy.exe` 从启动到退出**一次都不会被执行**。
 * 但 electron-builder 的 `extraResources` 是静态声明，会无条件把它拷进
 * `resources/sidecar/`。包内留一个永不执行的 console 子系统 exe 不只是体积问题：它一旦被
 * 拉起（或被杀软扫描到首次联网行为）会触发系统 / 安全软件的「这个程序要联网」提示，
 * 而本壳在这种配置下根本用不到代理。
 *
 * ⚠️⚠️ 判据**不在这里重写**：直接复用运行期的 [`needsSidecar`]。打包期与运行期必须是同一条
 * 规则，否则会出现「打包按 A 判、运行期按 B 用」的静默分叉 —— 这是本项目反复踩过的坑
 * （见 `test/probe-isolation.test.ts` 对三个出包入口的守卫思路）。
 *
 * 本模块被两处以**编译产物**方式引用：
 *   - `scripts/after-pack.cjs`（electron-builder 钩子，真正执行裁剪）
 *   - `scripts/check-asar-config.cjs`（出包后双向护栏）
 * 所以必须零 Electron 依赖，且只能在 `pnpm run build` 之后 require。
 */
import { existsSync, readdirSync, rmSync } from 'node:fs'
import { dirname, join } from 'node:path'

import type { AppConfig } from './config'
import { needsSidecar } from './navigate'
// 「平台 → 可执行文件名」的唯一实现在 sidecar.ts（运行期定位与打包期裁剪同名同规则，
// 2026-11-18 架构复查：此前这里有一份逐字拷贝）。re-export 保持既有调用面不变。
// ⚠️ sidecar.ts 零 Electron 依赖（只用 node:child_process），对 after-pack / check-asar
//    的 require 链依然成立。
import { sidecarExeName } from './sidecar'

export { sidecarExeName }

/** 包内 sidecar 路径：`<resources>/sidecar/<exe>`（与 electron-builder.yml 的 `to` 一致）。 */
export function packagedSidecarPath(
  resourcesDir: string,
  platform: NodeJS.Platform = process.platform,
): string {
  return join(resourcesDir, 'sidecar', sidecarExeName(platform))
}

/** 纯判定：本次产物是否必须带 sidecar（= 运行期是否真的会用到它）。 */
export function shouldShipSidecar(cfg: AppConfig): boolean {
  return needsSidecar(cfg)
}

/** `syncPackagedSidecar` 的实际动作（便于日志与测试断言）。 */
export type SidecarSyncAction = 'kept' | 'removed' | 'absent'

/**
 * 让包内 sidecar 的存在性与 `need` 一致。
 *
 * - `need=true` 且文件缺失 → **抛错**（配置需要代理 / 域名解析却没有 sidecar，产物必然启动失败，
 *   必须在出包时就炸掉，而不是让用户双击后看到「缺少本地代理组件」）；
 * - `need=false` 且文件存在 → 删掉文件，顺带移除已空的 `sidecar/` 目录；
 * - 其余两种组合是幂等的 no-op。
 */
export function syncPackagedSidecar(
  resourcesDir: string,
  need: boolean,
  platform: NodeJS.Platform = process.platform,
): SidecarSyncAction {
  const exe = packagedSidecarPath(resourcesDir, platform)
  const present = existsSync(exe)

  if (need) {
    if (!present) {
      throw new Error(
        `配置需要 sidecar（proxy_enabled=true 且无外部代理，或 app_url 是域名数组），` +
          `但产物里没有 ${exe} —— 确认打包前跑过 pnpm run sidecar`,
      )
    }
    return 'kept'
  }

  if (!present) return 'absent'

  rmSync(exe, { force: true })
  // 目录按约定只装这一个文件；非空（将来多出别的资源）就留着，绝不递归删。
  const dir = dirname(exe)
  try {
    if (readdirSync(dir).length === 0) rmSync(dir, { recursive: true, force: true })
  } catch {
    /* 目录已不存在 / 不可读：文件删掉即达到目的 */
  }
  return 'removed'
}
