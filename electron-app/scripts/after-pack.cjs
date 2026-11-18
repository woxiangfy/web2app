/**
 * electron-builder `afterPack` 钩子：让包内 `resources/sidecar/w2a-proxy.exe` 的存在性
 * 与**打包期固化**的配置一致。
 *
 * 时机选择的理由：app 目录已铺好（extraResources 已拷完）、zip / portable / NSIS 尚未归档，
 * 而这三个目标都从同一个 `win-unpacked` 目录生成 —— 这是唯一可靠的「不随包分发」位置。
 * 出包后再删只会留下已经压进 zip / portable 的那一份。
 *
 * 判据复用运行期编译产物（`out/main/*.js`），不重写规则：
 * `proxy_enabled=false` 且 `app_url` 是字符串 ⇒ 运行期 `needsSidecar` 恒假 ⇒ sidecar 一次都不会
 * 被 spawn（也就不会触发「这个程序要联网」的提示）。
 *
 * ⚠️ 路径解析：electron-builder 用 `path.resolve(hook)`（相对 **process.cwd()**）加载本文件，
 * 三个出包入口（package.json 的 pack/dist、scripts/packlib/driver-electron.mjs、
 * .github/workflows/electron-build-custom.yml）都在 `electron-app/` 下执行，故
 * `./scripts/after-pack.cjs` 成立。
 */
const { join } = require('node:path')

const appDir = join(__dirname, '..')

/** 加载编译产物；缺失时给出「先 build」的明确文案，而不是一句 MODULE_NOT_FOUND。 */
function loadBuiltLogic() {
  try {
    return {
      loadConfig: require(join(appDir, 'out', 'main', 'config.js')).loadConfig,
      ...require(join(appDir, 'out', 'main', 'sidecar-ship.js')),
    }
  } catch (e) {
    throw new Error(`[after-pack] 无法加载 out/main 的编译产物（先跑 pnpm run build）：${e.message}`)
  }
}

module.exports = async function afterPack(context) {
  const { loadConfig, shouldShipSidecar, syncPackagedSidecar } = loadBuiltLogic()

  const { config } = loadConfig(join(appDir, 'out'))
  const need = shouldShipSidecar(config)
  const resourcesDir = join(context.appOutDir, 'resources')
  const action = syncPackagedSidecar(resourcesDir, need, process.platform)

  const shape = Array.isArray(config.app_url) ? '域名数组' : '字符串'
  console.log(
    `[after-pack] sidecar ${need ? '保留' : '剔除'}：` +
      `proxy_enabled=${config.proxy_enabled}，app_url=${shape} ⇒ ${action}`,
  )
}
