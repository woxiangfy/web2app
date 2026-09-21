/**
 * 打包前置：把「构建期需要、但不进仓库」的两样东西铺到 build/ 下。
 *
 * 1. `build/icon.ico`  ← `../webview-app/icons/icon.ico`
 *    图标是**单一真源**，两壳共用一份。复制而非提交副本，避免出现两套图标各自演变。
 *    （`build/icon.ico` 已在 .gitignore 里，就是因为它只是复制品。）
 *
 * 2. `build/config.json` ← 项目根的 `config.json`（存在时原样复制）
 *    没有时生成一份**全字段默认模板**。
 *
 * ⚠️ 为什么必须生成：`electron-builder.yml` 用 `extraFiles` 把它放到 exe 同级。
 *    extraFiles 指向不存在的文件会让 electron-builder 直接报错退出；
 *    而 `config.json` 是运行期用户配置（gitignore），干净仓库里本来就没有。
 *    所以「一定有这个文件」这件事必须由本脚本保证。
 *
 * ⚠️ 为什么必须随包发一份：不预置的话，zip/portable 解压出来双击会连着内置默认的
 *    example.com —— 用户看到的是「打包了个空壳」，而不是「少了份配置」。
 *    （这也是 docs/electron-plan.md §7 第 1 条的结论：预置，预制一份可编辑的模板。）
 */
const { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } = require('node:fs')
const { dirname, join, resolve } = require('node:path')

const here = __dirname
const appDir = resolve(here, '..')
const buildDir = join(appDir, 'build')
mkdirSync(buildDir, { recursive: true })

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

// ---------- 2. config.json ----------
const userConfig = join(appDir, 'config.json')
const staged = join(buildDir, 'config.json')

if (existsSync(userConfig)) {
  // 校验一次 JSON：构建期就报错，好过用户双击后看到「读取配置失败」的启动页。
  const text = readFileSync(userConfig, 'utf8')
  try {
    JSON.parse(text)
  } catch (e) {
    console.error(`[stage] ✗ config.json 不是合法 JSON：${e.message}`)
    process.exit(1)
  }
  writeFileSync(staged, text, 'utf8')
  console.log(`[stage] 配置文件  : config.json → build/config.json（随包分发）`)
} else {
  const tpl = {
    app_url: 'https://www.example.com',
    app_name: 'Web2App',
    title: '',
    extra: {},
    icon_url: '',
    proxy_enabled: false,
    proxy_http: '',
    proxy_https: '',
    proxy_bypass: '',
    tabs_enabled: false,
    public_key: '',
    resolve_secret: '',
    data_dir_default: false,
  }
  writeFileSync(staged, JSON.stringify(tpl, null, 2) + '\n', 'utf8')
  console.log('[stage] 配置文件  : 未找到 config.json，已生成全字段默认模板 → build/config.json')
}
