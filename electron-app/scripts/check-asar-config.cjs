/**
 * 出包后护栏：`app.asar` 里**必须**有 `out/config.json` 与 `out/inject/*`。
 *
 * ## 为什么值得单独写一个脚本
 *
 * 「配置进 asar」这件事**失败了也不会报错**：electron-builder 照样出包、exe 照样双击打开，
 * 只是读不到 config.json，于是静默退回内置默认配置（`https://www.example.com`）——
 * 用户看到的是「打包打了个空壳」，而构建日志全绿。这正是本项目反复强调的那类
 * 「错了也静默」的缺陷，必须用一条会亮红灯的判据盯住。
 *
 * 注入资源（titlebar.js / loading.js / splash.html / shell.html / settings.html）同理：
 * 它们若漏进包，标题栏 / 加载层 / 多页签壳 / 设置页至少一样会静默退化（白屏 / 兜底页 /
 * 按钮点了没反应），比 config.json 缺失更难从表象反推。所以一起纳入护栏。
 *
 * 最容易踩的坑：把待打包的 config.json 放进 `build/`（`directories.buildResources`）。
 * electron-builder 会把 buildResources 目录**整个排除**在包外，见
 * `app-builder-lib/out/fileMatcher.js` 里那条基于 `relativeBuildResourceDir` 的排除规则。
 *
 * 用法：
 *   node scripts/check-asar-config.cjs [app.asar 路径]
 * 默认路径：release/win-unpacked/resources/app.asar（未给参数时会在 release/ 下自动找）
 */
const { existsSync, readdirSync, statSync } = require('node:fs')
const { join, resolve } = require('node:path')

const appDir = resolve(__dirname, '..')
const releaseDir = join(appDir, 'release')

/** 必须随 asar 分发的注入资源（stage-assets.cjs 拷贝到 out/inject/ 的文件名）。 */
const REQUIRED_INJECT = ['titlebar.js', 'loading.js', 'splash.html', 'shell.html', 'settings.html']

/** 在 release/ 下找任意一份 app.asar（zip/portable/dir 三种目标的中间产物路径可能不同）。 */
function findAsar() {
  const explicit = process.argv[2]
  if (explicit) {
    const p = resolve(explicit)
    return existsSync(p) ? p : null
  }
  const direct = join(releaseDir, 'win-unpacked', 'resources', 'app.asar')
  if (existsSync(direct)) return direct
  if (!existsSync(releaseDir)) return null
  // 退一步：递归找（目录名可能因 productName 变化）
  const stack = [releaseDir]
  while (stack.length > 0) {
    const dir = stack.pop()
    for (const name of readdirSync(dir)) {
      const p = join(dir, name)
      let st
      try {
        st = statSync(p)
      } catch {
        continue
      }
      if (st.isDirectory()) {
        if (name === 'node_modules') continue
        stack.push(p)
      } else if (name === 'app.asar') {
        return p
      }
    }
  }
  return null
}

const asarPath = findAsar()
if (!asarPath) {
  console.error('::error::未找到 app.asar，无法校验（先跑 pnpm run pack / dist）')
  process.exit(1)
}

let listPackage
try {
  ;({ listPackage } = require('@electron/asar'))
} catch {
  // @electron/asar 是 electron-builder 的依赖，正常会被提升到顶层；
  // 拿不到时不要静默通过 —— 那会让这条护栏形同虚设。
  console.error('::error::无法加载 @electron/asar，无法校验 asar 内容（请确认已 pnpm install）')
  process.exit(1)
}

const entries = listPackage(asarPath).map((f) => f.replace(/\\/g, '/'))
const hit = entries.filter((f) => f === '/out/config.json' || f.endsWith('/out/config.json'))

console.log(`asar: ${asarPath}`)
console.log(`条目数: ${entries.length}`)
console.log(`配置: ${hit.length > 0 ? hit.join(', ') : '（缺失！）'}`)

if (hit.length === 0) {
  console.error(
    '::error::app.asar 内没有 out/config.json —— 包会以【内置默认配置】启动。\n' +
      '  排查顺序：① scripts/stage-assets.cjs 是否已跑（pnpm run stage）；\n' +
      '            ② 不要把 config.json 放 build/（buildResources 会被整体排除在包外）；\n' +
      '            ③ electron-builder.yml 的 files 是否仍包含 out/ 下的文件。',
  )
  process.exit(1)
}

// 注入资源护栏：out/inject/ 下的 5 个文件缺一不可（否则标题栏 / 加载层 / 壳 / 设置页静默退化）。
const missingInject = REQUIRED_INJECT.filter(
  (name) => !entries.includes(`/out/inject/${name}`)
)
if (missingInject.length > 0) {
  console.error(
    `::error::app.asar 内缺少注入资源：${missingInject.join(', ')}\n` +
      '  排查顺序：① stage-assets.cjs 是否已跑（pnpm run stage）；\n' +
      '            ② 源文件是否存在于 webview-app/src 与 webview-app/dist；\n' +
      '            ③ electron-builder.yml 的 files 是否仍包含 out/ 下的文件。',
  )
  process.exit(1)
}
console.log(`注入资源: ${REQUIRED_INJECT.length} 项齐全 ✓`)

console.log('asar 配置护栏通过 ✓')
