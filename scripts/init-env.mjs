#!/usr/bin/env node
/**
 * 环境初始化脚本（**零第三方依赖**，Node ≥ 18）：一条命令把两个壳的依赖都拉齐。
 *
 *   · webview-app/（Rust + Tauri cargo workspace，四个成员）
 *       默认 `cargo fetch --manifest-path webview-app/Cargo.toml --target <host>`
 *         —— 按 Cargo.lock 下载 crate，但**只拉当前宿主平台**（target triple 由 `rustc -vV`
 *         的 `host:` 行解析，如 Windows x64 = x86_64-pc-windows-msvc）。
 *       ⚠️ `cargo fetch` 不带 `--target` 时默认拉取**全平台**依赖 —— 本工程的 Tauri 依赖树里
 *          含 gtk / webkit2gtk（Linux）、objc2（macOS）、ndk（Android）等大量与本机无关的
 *          crate，墙内网络下纯属浪费。开发/构建只需要宿主目标。
 *          要准备 Android 交叉编译时用 `--all-targets`（或 `cargo fetch --target <android-triple>`）。
 *       `--update` 时先跑 `cargo update`：把 Cargo.lock 更新到 semver 允许的新版本，再 fetch。
 *         ⚠️ Cargo.lock 是**受版本控制**的文件，--update 会改动它（git diff 可见，自行决定是否提交）。
 *       tauri-cli（`cargo tauri`）**只检查不自动安装**：它是本地打 Tauri 包（pack-local.mjs
 *         --target tauri-*）的硬依赖，但普通 `cargo build` / sidecar 不需要，且首次编译要几分钟。
 *         `--check` 缺失时会报非 0 并给出安装命令：cargo install tauri-cli --version "^2"。
 *
 *   · electron-app/（Electron + TypeScript，pnpm）
 *       `pnpm install`，并**显式注入两个镜像环境变量**（默认开，--no-mirror 关）：
 *         ELECTRON_MIRROR                       装 electron ~158MB 平台二进制
 *         ELECTRON_BUILDER_BINARIES_MIRROR      electron-builder 的 winCodeSign / nsis 等工具包
 *     ⚠️ 这是本仓库踩过的坑（见 electron-app/.npmrc 与 pnpm-workspace.yaml 的文件头注释）：
 *        pnpm **不会**把 .npmrc 里的自定义键透传给依赖的 install 脚本，而 electron@44 已移除
 *        自带 postinstall（改由 package.json 的 `"postinstall": "install-electron"` 调它的 bin），
 *        install.js 只认环境变量；不设就去连 GitHub Releases，国内表现为**卡住不返回**（不是报错）。
 *       装完只认一个结果：`node_modules/electron/dist/electron.exe` 是否存在
 *        —— pnpm install 退出码 0 不代表二进制装上了。缺失时自动跑
 *        `node node_modules/electron/install.js` 兜底（README 记载的补救法，实测约 3 秒）。
 *
 * 为什么是 Node：与 scripts/pack-local.mjs 同样理由 —— 弄 electron-app 必然已有 node
 * （electron-builder、pnpm run * 全是 node 程序），node 是「拿这个仓库做开发」的必要条件；
 * PowerShell 之外保留一份在 macOS/Linux 也能跑的入口，成本为零。
 *
 * 用法：
 *   node scripts/init-env.mjs                 # 两侧都初始化（cargo fetch + pnpm install）
 *   node scripts/init-env.mjs --webview       # 只初始化 webview-app（cargo）
 *   node scripts/init-env.mjs --electron      # 只初始化 electron-app（pnpm）
 *   node scripts/init-env.mjs --update        # cargo 先 update 再 fetch（会改写 Cargo.lock）
 *   node scripts/init-env.mjs --all-targets   # cargo 拉全平台依赖（默认只拉宿主平台；交叉编译前用）
 *   node scripts/init-env.mjs --frozen        # pnpm install --frozen-lockfile（CI 语义，lockfile 不符即失败）
 *   node scripts/init-env.mjs --no-mirror     # 不注入国内镜像（境外网络 / 直连 GitHub 更快时）
 *   node scripts/init-env.mjs --check         # 只检查工具链与当前状态，不下载、不安装
 *   node scripts/init-env.mjs --help
 *
 * 退出码：0 = 选中的全部成功（--check 下无缺失）；1 = 有失败/缺失；2 = 参数错误。
 */

import { spawnSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import { join, relative, resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const WEBVIEW = join(ROOT, 'webview-app')
const ELECTRON = join(ROOT, 'electron-app')

const ELECTRON_MIRROR_URL = 'https://npmmirror.com/mirrors/electron/'
const ELECTRON_BUILDER_MIRROR_URL = 'https://npmmirror.com/mirrors/electron-builder-binaries/'
const MIN_NODE_MAJOR = 18
const MIN_PNPM_MAJOR = 11 // pnpm-workspace.yaml 用了 allowBuilds（11+ 键名），CI 固定 v12
const MIN_RUSTC = [1, 70] // webview-app/Cargo.toml 的 rust-version

const rel = (p) => relative(ROOT, p).replace(/\\/g, '/') || '.'

// ===========================================================================
// 参数
// ===========================================================================
const KNOWN_FLAGS = new Set(['--webview', '--electron', '--update', '--all-targets', '--frozen', '--no-mirror', '--check', '--help', '-h'])
const argv = process.argv.slice(2)
const unknown = argv.filter((a) => a.startsWith('-') && !KNOWN_FLAGS.has(a))
if (unknown.length) {
  console.error(`未知参数：${unknown.join(' ')}\n`)
  printUsage()
  process.exit(2)
}
const flags = new Set(argv)
if (flags.has('--help') || flags.has('-h')) {
  printUsage()
  process.exit(0)
}
const opts = {
  onlyWebview: flags.has('--webview') && !flags.has('--electron'),
  onlyElectron: flags.has('--electron') && !flags.has('--webview'),
  update: flags.has('--update'),
  allTargets: flags.has('--all-targets'),
  frozen: flags.has('--frozen'),
  mirror: !flags.has('--no-mirror'),
  checkOnly: flags.has('--check'),
}
const wantWebview = !opts.onlyElectron
const wantElectron = !opts.onlyWebview

function printUsage() {
  console.log(
    [
      '用法：node scripts/init-env.mjs [选项]',
      '',
      '  （无参数）        两侧都初始化：webview-app cargo fetch + electron-app pnpm install',
      '  --webview         只初始化 webview-app（cargo）',
      '  --electron        只初始化 electron-app（pnpm）',
      '  --update          cargo 先 update（改写 Cargo.lock）再 fetch',
      '  --all-targets     cargo 拉全平台依赖（默认只拉宿主平台；Android 交叉编译前用）',
      '  --frozen          pnpm install --frozen-lockfile（CI 语义）',
      '  --no-mirror       不注入 ELECTRON_MIRROR 等国内镜像环境变量',
      '  --check           只检查工具链与依赖现状，不下载、不安装',
      '  --help, -h        显示本帮助',
    ].join('\n'),
  )
}

// ===========================================================================
// 子进程工具
// ===========================================================================

/** 取版本号；命令不存在时返回 null。 */
function toolVersion(cmd) {
  const r = spawnSync(cmd, ['--version'], { encoding: 'utf8' })
  if (r.error || r.status !== 0) return null
  const m = String(r.stdout || '').match(/(\d+)\.(\d+)\.(\d+)/)
  return m ? `${m[1]}.${m[2]}.${m[3]}` : String(r.stdout || '').trim() || null
}

/**
 * 检测 tauri-cli（cargo 的 `tauri` 子命令，crate 包名是 tauri-cli，装完提供 cargo-tauri.exe）。
 *
 * ⚠️ 必须用 `cargo tauri --version` 探，不能 `toolVersion('cargo-tauri')`：
 *    cargo 子命令通过 PATH 上的 cargo-tauri 可执行文件分发，但不同安装方式
 *    （cargo install / 手动放 bin）落地路径一致性以 cargo 自己的解析为准；
 *    pack-local.mjs 的前置检测也是同一条命令（have(['cargo','tauri','--version'])），
 *    两边判据必须一致，否则会出现「init 说装了、打包却说没装」。
 * 未安装时 cargo 以非 0 退出并打印 "no such command"，这里归一化成 null。
 */
function tauriCliVersion() {
  const r = spawnSync('cargo', ['tauri', '--version'], { encoding: 'utf8' })
  if (r.error || r.status !== 0) return null
  const m = String(r.stdout || '').match(/(\d+)\.(\d+)\.(\d+)/)
  return m ? `${m[1]}.${m[2]}.${m[3]}` : String(r.stdout || '').trim() || null
}

/** 比较 "a.b.c" 与 [major, minor]：≥ 返回 true。 */
function atLeast(version, [needMajor, needMinor = 0]) {
  if (!version) return false
  const [a = 0, b = 0] = version.split('.').map((n) => parseInt(n, 10) || 0)
  return a > needMajor || (a === needMajor && b >= needMinor)
}

/**
 * 执行子进程并继承 stdio（安装过程可能几分钟，必须看得见进度）。
 *
 * ⚠️ 不加 `shell: true`：与 pack-local.mjs 同一实测结论 —— Node ≥ 22 无 shell 直接执行
 *    .cmd 会被拦，但把名字写成裸的 `pnpm` / `cargo` 反而能正常解析；shell 模式还会错误
 *    拆分含空格/分号的参数。
 */
function run(cmd, { cwd, env } = {}) {
  const printable = cmd.map((c) => (c.includes(' ') ? `"${c}"` : c)).join(' ')
  console.log(`\n$ (cd ${rel(cwd)}/) ${printable}`)
  const r = spawnSync(cmd[0], cmd.slice(1), { cwd, env, stdio: 'inherit' })
  if (r.error) throw new Error(`命令无法执行（${cmd[0]}）：${r.error.message}`)
  if (r.status !== 0) throw new Error(`命令失败（退出码 ${r.status}）：${printable}`)
}

/**
 * Electron 侧子进程环境。
 *
 * ⚠️ 剥掉 ELECTRON_RUN_AS_NODE：部分宿主（IDE / agent 沙箱）给所有子进程注入它（=1），
 *    泄漏进任何 electron 可执行文件会让其退化成纯 node 模式（见 electron-app/README
 *    「为什么必须经 launch.cjs」一节）。安装链路虽然不直接 spawn electron.exe，但照仓库
 *    既定纪律统一剥掉，免得 install.js 的解压/校验子步骤踩到。NODE_OPTIONS 不动
 *    （那是宿主的输出通道，清了会变哑巴）。
 */
function electronChildEnv() {
  const env = { ...process.env }
  delete env.ELECTRON_RUN_AS_NODE
  if (opts.mirror) {
    // 已显式设置过（如指到别的镜像）就尊重用户值，不覆盖。
    env.ELECTRON_MIRROR ??= ELECTRON_MIRROR_URL
    env.ELECTRON_BUILDER_BINARIES_MIRROR ??= ELECTRON_BUILDER_MIRROR_URL
  }
  return env
}

function electronExePath() {
  const bin = process.platform === 'win32' ? 'electron.exe' : 'electron'
  return join(ELECTRON, 'node_modules', 'electron', 'dist', bin)
}

// ===========================================================================
// 检查
// ===========================================================================
function checkToolchain() {
  const problems = []
  const nodeVersion = process.versions.node
  console.log('── 工具链 ──────────────────────────────────────────────')
  console.log(`  node    ${nodeVersion}${atLeast(nodeVersion, [MIN_NODE_MAJOR]) ? '' : `  ⚠️ 需要 ≥ ${MIN_NODE_MAJOR}`}`)
  if (!atLeast(nodeVersion, [MIN_NODE_MAJOR])) problems.push(`node 版本过低（${nodeVersion} < ${MIN_NODE_MAJOR}）`)

  const cargoVersion = toolVersion('cargo')
  const rustcVersion = toolVersion('rustc')
  if (cargoVersion) console.log(`  cargo   ${cargoVersion}`)
  else {
    console.log('  cargo   ✗ 未找到（安装 Rust 工具链：https://rustup.rs）')
    problems.push('未找到 cargo（webview-app 与 electron sidecar 都需要 Rust 工具链）')
  }
  if (rustcVersion) {
    const msrvOk = atLeast(rustcVersion, MIN_RUSTC)
    console.log(`  rustc   ${rustcVersion}${msrvOk ? '' : `  ⚠️ webview-app 要求 ≥ ${MIN_RUSTC.join('.')}`}`)
    if (!msrvOk) problems.push(`rustc ${rustcVersion} 低于 webview-app 的 rust-version ${MIN_RUSTC.join('.')}`)
  } else {
    console.log('  rustc   ✗ 未找到')
    problems.push('未找到 rustc（通常随 rustup 与 cargo 一起安装）')
  }

  // tauri-cli 是「本地打 Tauri 包」的硬依赖（pack-local.mjs --target tauri-* 在第一步就
  // 查 `cargo tauri --version`，缺失直接退出）。普通 `cargo build` / sidecar 编译不需要它，
  // 所以：① 仅在选中 webview 侧时检查；② init 默认不自动安装（首次要编译数分钟），
  // 只在缺失时给出与 CI / pack-local 完全一致的安装命令。
  if (wantWebview) {
    const tauriVersion = tauriCliVersion()
    if (tauriVersion) {
      console.log(`  tauri   ${tauriVersion}（cargo tauri 子命令，本地打 Tauri 包用）`)
    } else {
      console.log('  tauri   ✗ 未找到 cargo tauri 子命令（本地打 Tauri 包需要；普通 cargo build 不需要）')
      problems.push(
        '未安装 tauri-cli（cargo tauri）：执行 `cargo install tauri-cli --version "^2"` ' +
          '（与 CI 的 Install Tauri CLI 同一步，首次编译约几分钟）',
      )
    }
  }

  const pnpmVersion = toolVersion('pnpm')
  if (pnpmVersion) {
    const major = parseInt(pnpmVersion.split('.')[0], 10) || 0
    const ok = major >= MIN_PNPM_MAJOR
    console.log(`  pnpm    ${pnpmVersion}${ok ? '' : `  ⚠️ pnpm-workspace.yaml 的 allowBuilds 需要 pnpm ≥ ${MIN_PNPM_MAJOR}（CI 固定 v12）`}`)
    if (!ok) problems.push(`pnpm ${pnpmVersion} 过旧：需要 ≥ ${MIN_PNPM_MAJOR}（corepack enable 或 npm i -g pnpm@12）`)
  } else {
    console.log('  pnpm    ✗ 未找到（启用 corepack：`corepack enable`，或 `npm i -g pnpm`）')
    problems.push('未找到 pnpm（electron-app 的包管理器，见 electron-app/pnpm-workspace.yaml）')
  }
  return problems
}

function checkState() {
  const problems = []
  console.log('\n── 工程与依赖现状 ──────────────────────────────────────')

  const cargoToml = join(WEBVIEW, 'Cargo.toml')
  if (!existsSync(cargoToml)) {
    console.log(`  webview-app/Cargo.toml  ✗ 不存在（cwd=${rel(ROOT)}？请在仓库根运行本脚本）`)
    problems.push('找不到 webview-app/Cargo.toml')
  } else {
    // cargo fetch 的落地缓存：target/ 存在不代表全量拉过，但能给出直观状态。
    const fetched = existsSync(join(WEBVIEW, 'target')) || existsSync(join(ROOT, 'target'))
    console.log(`  webview-app/Cargo.toml  ✓${fetched ? '（已有 target/ 构建缓存）' : '（尚未 fetch / 编译过）'}`)
  }

  const pkg = join(ELECTRON, 'package.json')
  if (!existsSync(pkg)) {
    console.log('  electron-app/package.json  ✗ 不存在（请在仓库根运行本脚本）')
    problems.push('找不到 electron-app/package.json')
  } else {
    const modules = existsSync(join(ELECTRON, 'node_modules'))
    const exe = existsSync(electronExePath())
    console.log(`  electron-app/node_modules       ${modules ? '✓ 已安装' : '✗ 未安装（需要 pnpm install）'}`)
    console.log(`  electron/dist/${process.platform === 'win32' ? 'electron.exe' : 'electron'}        ${exe ? '✓ 存在' : modules ? '✗ 缺失（需要 node node_modules/electron/install.js 兜底）' : '✗ 缺失'}`)
    if (wantElectron) {
      if (!modules) problems.push('electron-app 依赖未安装')
      else if (!exe) problems.push('electron 平台二进制缺失')
    }
  }

  console.log(`\n  镜像环境变量注入：${opts.mirror ? `开（${ELECTRON_MIRROR_URL}）` : '关（--no-mirror）'}`)
  return problems
}

// ===========================================================================
// 两个初始化分支
// ===========================================================================
/**
 * 取 rustc 的宿主 target triple（`rustc -vV` 输出里的 `host: x86_64-pc-windows-msvc` 行）。
 * 不写死架构 —— ARM64 Windows 上应解析成 aarch64-pc-windows-msvc。
 */
function hostTarget() {
  const r = spawnSync('rustc', ['-vV'], { encoding: 'utf8' })
  if (r.error || r.status !== 0) return null
  for (const line of String(r.stdout || '').split(/\r?\n/)) {
    const m = line.match(/^host:\s*(\S+)\s*$/)
    if (m) return m[1]
  }
  return null
}

function initWebview() {
  const manifest = join(WEBVIEW, 'Cargo.toml')
  if (!existsSync(manifest)) throw new Error(`找不到 ${rel(manifest)}，请在仓库根运行本脚本`)
  if (!toolVersion('cargo')) {
    throw new Error('未找到 cargo。请先安装 Rust 工具链：https://rustup.rs （webview-app 与 electron sidecar 都需要）')
  }

  if (opts.update) {
    console.log('\n── [webview-app] cargo update（更新 Cargo.lock 到 semver 允许的新版本）──────────')
    run(['cargo', 'update', '--manifest-path', manifest], { cwd: ROOT })
  }

  // 默认只拉宿主平台：不带 --target 的 cargo fetch 会把 Linux/macOS/Android 的条件依赖
  // （gtk、webkit2gtk、objc2、ndk 等）全拉一遍，本机开发一个都用不到。
  const fetchCmd = ['cargo', 'fetch', '--manifest-path', manifest]
  if (opts.allTargets) {
    console.log('\n── [webview-app] cargo fetch（--all-targets：全平台依赖，含交叉编译目标）────────')
  } else {
    const target = hostTarget()
    if (!target) throw new Error('无法通过 `rustc -vV` 解析宿主 target triple（rustc 未装好？）')
    fetchCmd.push('--target', target)
    console.log(`\n── [webview-app] cargo fetch（仅宿主目标 ${target}；要全平台加 --all-targets）────`)
  }
  run(fetchCmd, { cwd: ROOT })
  console.log('  ✓ cargo 依赖已就绪')
}

function initElectron() {
  const pkg = join(ELECTRON, 'package.json')
  if (!existsSync(pkg)) throw new Error(`找不到 ${rel(pkg)}，请在仓库根运行本脚本`)
  if (!toolVersion('pnpm')) {
    throw new Error(
      '未找到 pnpm。electron-app 使用 pnpm 12（见 pnpm-workspace.yaml）：\n' +
        '    corepack enable            # 或 npm install -g pnpm@12',
    )
  }
  const env = electronChildEnv()

  const installCmd = ['pnpm', 'install']
  if (opts.frozen) installCmd.push('--frozen-lockfile')
  console.log('\n── [electron-app] pnpm install ──────────────────────────────────────────────────')
  if (opts.mirror) {
    console.log(`  ELECTRON_MIRROR=${env.ELECTRON_MIRROR}`)
    console.log(`  ELECTRON_BUILDER_BINARIES_MIRROR=${env.ELECTRON_BUILDER_BINARIES_MIRROR}`)
  }
  run(installCmd, { cwd: ELECTRON, env })

  // README 反复强调的判据：退出码 0 不算数，只认平台二进制真的在 dist/ 里。
  const exe = electronExePath()
  if (!existsSync(exe)) {
    console.log('\n  ⚠️ pnpm install 已返回，但未发现 electron 平台二进制；执行 install.js 兜底 …')
    run(['node', join('node_modules', 'electron', 'install.js')], { cwd: ELECTRON, env })
  }
  if (!existsSync(exe)) {
    throw new Error(
      `electron 平台二进制仍缺失：${rel(exe)}\n` +
        '排查：确认 ELECTRON_MIRROR 可访问后重跑（不带 --no-mirror），或手工执行\n' +
        '    node electron-app/node_modules/electron/install.js',
    )
  }
  console.log(`  ✓ ${rel(exe)} 已就位`)
}

// ===========================================================================
// 主流程
// ===========================================================================
function main() {
  console.log('web2app 环境初始化')
  console.log(`仓库根：${ROOT}`)
  console.log(
    `目标：${[wantWebview ? 'webview-app(cargo)' : null, wantElectron ? 'electron-app(pnpm)' : null].filter(Boolean).join(' + ')}` +
      `${opts.update ? '；cargo --update' : ''}${opts.allTargets ? '；cargo --all-targets' : ''}${opts.frozen ? '；pnpm --frozen-lockfile' : ''}${opts.checkOnly ? '；--check（只检查）' : ''}`,
  )

  if (opts.checkOnly) {
    const problems = [...checkToolchain(), ...checkState()]
    if (problems.length) {
      console.log(`\n结果：${problems.length} 项需要处理`)
      for (const p of problems) console.log(`  ✗ ${p}`)
      process.exit(1)
    }
    console.log('\n✓ 工具链与依赖均就绪')
    return
  }

  const failures = []

  // 工具链整体先报一遍（两个壳的版本要求一起看），但不因此短路：
  // 只做 electron 的人机器上暂时没装 Rust 时，仍应让 electron 侧跑完。
  const toolProblems = checkToolchain()
  if (toolProblems.length) console.log(`\n（以上 ${toolProblems.length} 项工具链告警；仍继续执行所选目标，失败会在对应步骤报出）\n`)

  if (wantWebview) {
    try {
      initWebview()
    } catch (e) {
      console.error(`\n✗ [webview-app] ${e.message}`)
      failures.push('webview-app')
    }
  }

  if (wantElectron) {
    try {
      initElectron()
    } catch (e) {
      console.error(`\n✗ [electron-app] ${e.message}`)
      failures.push('electron-app')
    }
  }

  console.log('\n────────────────────────────────────────────────────────')
  if (failures.length) {
    console.error(`完成，但以下目标失败：${failures.join('、')}`)
    process.exit(1)
  }
  console.log('✓ 环境初始化完成')
  if (wantElectron) console.log('  后续：electron-app/ 下 pnpm start（开发）或 pnpm run probe（契约自检）')
  if (wantWebview) console.log('  后续：webview-app/ 下 cargo build，或 electron-app/ 下 pnpm run sidecar 编 sidecar')
}

main()
