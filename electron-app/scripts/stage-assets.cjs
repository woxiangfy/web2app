/**
 * 打包前置：把「构建期需要、但不进仓库」的东西铺好。
 *
 * 1. `build/icon.ico` ← `../webview-app/icons/icon.ico`
 *    图标是**单一真源**，两壳共用一份。复制而非提交副本，避免出现两套图标各自演变。
 *    （`build/icon.ico` 已在 .gitignore 里，就是因为它只是复制品。）
 *
 * 2. `out/config.json` ← 项目根的 `config.json`（存在时原样复制；没有则生成全字段默认模板）
 *    ⚠️ 它落在 `out/` 而不是 `build/`：`electron-builder.yml` 的 `files` 已覆盖 `out/` 下的
 *       全部文件，而 **`build/` 是 `directories.buildResources`，会被 electron-builder
 *       整个排除在包外**（`app-builder-lib/out/fileMatcher.js` 里那条 buildResources 排除）
 *       —— 放那儿会静默不打包。最终它在包内的位置是 `app.asar/out/config.json`。
 *
 * ⚠️ 为什么配置必须随包发一份：不预置的话，zip/portable 解压出来双击会连着内置默认的
 *    example.com —— 用户看到的是「打包了个空壳」，而不是「少了份配置」。
 *
 * ⚠️ 为什么不再放 exe 同级：那份是可读可改的明文，等于配置形同虚设（详见 electron-builder.yml
 *    里 extraFiles 位置的说明）。现在只有重新打包才能改配置。
 */
const { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } = require('node:fs')
const { join, resolve } = require('node:path')

const here = __dirname
const appDir = resolve(here, '..')
const buildDir = join(appDir, 'build')
const outDir = join(appDir, 'out')
mkdirSync(buildDir, { recursive: true })
mkdirSync(outDir, { recursive: true })

// ---------- 1. 图标 ----------
const iconSrc = join(appDir, '..', 'webview-app', 'icons', 'icon.ico')
if (existsSync(iconSrc)) {
  copyFileSync(iconSrc, join(buildDir, 'icon.ico'))
  console.log(`[stage] 图标      : ${iconSrc} → build/icon.ico`)
} else {
  // 不静默跳过：没有 icon.ico 时 electron-builder 会退回 Electron 默认图标，
  // 而「任务栏/托盘图标不对」是最不容易联想到构建问题的一类反馈。
  console.warn(`[stage] ⚠️ 未找到图标源 ${iconSrc}，将使用 Electron 默认图标`)
}

// ---------- 2. config.json（随 asar 分发） ----------
const userConfig = join(appDir, 'config.json')
const staged = join(outDir, 'config.json')

/**
 * 打包期浅校验：只挡「一眼就能看出写错」的两类 —— 字符串布尔与已移除的 window_title。
 *
 * ⚠️ 这里**不是**契约的第二个实现：完整契约（字段名、类型、默认值、app_url 形态）
 *    由 `.github/scripts/parse-packaging-config.py` 与 `src/main/config.ts` 负责，
 *    各自都有自测 / 探针。这里只求「构建时就炸」而不是「双击后才发现」。
 */
function assertShallowContract(text) {
  const raw = JSON.parse(text)
  if (raw === null || typeof raw !== 'object' || Array.isArray(raw)) {
    throw new Error('config.json 必须是一个对象（{...}）')
  }
  for (const key of ['window_title', 'data_dir_default']) {
    if (key in raw) {
      throw new Error(
        `字段 ${key} 已移除，请改用 ${key === 'window_title' ? 'title' : 'data_dir'}（见 docs/packaging-config.md）`,
      )
    }
  }
  for (const key of ['proxy_enabled', 'tabs_enabled', 'debug']) {
    if (key in raw && typeof raw[key] !== 'boolean') {
      throw new Error(
        `字段 ${key} 必须是布尔值（true / false，不带引号），当前是 ${JSON.stringify(raw[key])}`,
      )
    }
  }
  // data_dir 是**字符串**：空串有含义（系统默认程序目录），非字符串一律拒绝。
  if ('data_dir' in raw && typeof raw.data_dir !== 'string') {
    throw new Error(
      `字段 data_dir 必须是字符串（目录路径，空串 = 系统默认程序目录），当前是 ${JSON.stringify(raw.data_dir)}`,
    )
  }
}

if (existsSync(userConfig)) {
  // 校验一次 JSON：构建期就报错，好过用户双击后看到「读取配置失败」的启动页。
  const text = readFileSync(userConfig, 'utf8')
  try {
    assertShallowContract(text)
  } catch (e) {
    console.error(`[stage] ✗ config.json 不符合打包契约：${e.message}`)
    console.error('[stage]   字段表与多端行为见 docs/packaging-config.md')
    process.exit(1)
  }
  writeFileSync(staged, text, 'utf8')
  console.log(`[stage] 配置文件  : config.json → out/config.json（进 app.asar，运行期不可改）`)
} else {
  // ⚠️ 默认模板的字段/类型必须与解析器的 DEFAULTS 完全一致（含 proxy_bypass 的默认串）。
  const tpl = {
    app_url: 'https://www.example.com',
    app_name: 'Web2App',
    title: '',
    extra: {},
    icon_url: '',
    proxy_enabled: false,
    proxy_http: '',
    proxy_https: '',
    proxy_bypass: 'localhost;127.0.0.1;<-loopback>',
    tabs_enabled: false,
    public_key: '',
    resolve_secret: '',
    // 诊断日志开关：默认关（与解析器契约一致，见 docs/packaging-config.md）
    debug: false,
    data_dir: 'data',
  }
  writeFileSync(staged, JSON.stringify(tpl, null, 2) + '\n', 'utf8')
  console.log('[stage] 配置文件  : 未找到 config.json，已生成全字段默认模板 → out/config.json')
}
