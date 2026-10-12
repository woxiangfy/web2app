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
 *       tauri-cli（`cargo tauri`）在 cargo fetch 成功后**默认自动安装**：
 *         `cargo install tauri-cli --version "^2"` —— 与 CI 的 Install Tauri CLI 完全同一步。
 *         它是本地打 Tauri 包（pack-local.mjs --target tauri-*）的硬依赖，但普通
 *         `cargo build` / sidecar 编译不需要；首次安装要编译几分钟，不想等可加 --no-tauri-cli
 *         跳过（打包前再手动装）。--check 永远只检查不安装。装完会用 `cargo tauri --version`
 *         复验（判据与 pack-local 一致）。
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
 *        `node node_modules/electron/install.js` 兜底（`electron-app/AGENTS.md` 记载的补救法，实测约 3 秒）。
 *
 *   · 子进程输出与失败排查（两侧共用）
 *       每个子进程的输出实时上屏的同时 tee 一份完整原文到系统临时目录日志（路径开头打印）；
 *       失败时最终汇总处会**重放该命令输出的尾部**。cargo/pnpm 的安装进度有几百行，真正的
 *       错误（下载失败 / TLS / 编译错误）会被顶出可视区，曾导致「只知道失败、看不到原因」。
 *
 * 为什么是 Node：与 scripts/pack-local.mjs 同样理由 —— 弄 electron-app 必然已有 node
 * （electron-builder、pnpm run * 全是 node 程序），node 是「拿这个仓库做开发」的必要条件；
 * PowerShell 之外保留一份在 macOS/Linux 也能跑的入口，成本为零。
 *
 * 用法：
 *   node scripts/init-env.mjs                 # 两侧都初始化（cargo fetch + pnpm install + tauri-cli）
 *   node scripts/init-env.mjs --webview       # 只初始化 webview-app（cargo fetch + tauri-cli）
 *   node scripts/init-env.mjs --electron      # 只初始化 electron-app（pnpm）
 *   node scripts/init-env.mjs --update        # cargo 先 update 再 fetch（会改写 Cargo.lock）
 *   node scripts/init-env.mjs --all-targets   # cargo 拉全平台依赖（默认只拉宿主平台；交叉编译前用）
 *   node scripts/init-env.mjs --frozen        # pnpm install --frozen-lockfile（CI 语义，lockfile 不符即失败）
 *   node scripts/init-env.mjs --no-mirror     # 不注入国内镜像（境外网络 / 直连 GitHub 更快时）
 *   node scripts/init-env.mjs --no-tauri-cli  # 不自动装 tauri-cli（默认缺失自动装，首次约几分钟）
 *   node scripts/init-env.mjs --check         # 只检查工具链与当前状态，不下载、不安装
 *   node scripts/init-env.mjs --help
 *
 * 退出码：0 = 选中的全部成功（--check 下无缺失）；1 = 有失败/缺失；2 = 参数错误。
 */

import { spawn, spawnSync } from 'node:child_process'
import { createWriteStream, existsSync } from 'node:fs'
import { tmpdir } from 'node:os'
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
const TAIL_BYTES = 8192 // 子进程失败时在汇总处重放的输出尾部上限（字节），完整输出在日志文件里

// 子进程的完整原始输出 tee 到系统临时目录（不污染工作区）。失败时汇总处会重放尾部 + 给出此路径，
// 解决「cargo/pnpm 进度刷了几百行，最后只剩一句退出码，看不到真正错误」的问题。
const logStamp = new Date().toISOString().replace(/[-:T]/g, '').slice(0, 14)
const LOG_FILE = join(tmpdir(), `web2app-init-env-${logStamp}-${process.pid}.log`)
let logStream = null
function logWrite(chunk) {
  if (!logStream) logStream = createWriteStream(LOG_FILE, { flags: 'w' }) // 懒创建：--help / 参数错误不留空文件
  if (logStream.writable) logStream.write(chunk)
}
const closeLog = () => (logStream ? new Promise((r) => logStream.end(() => r())) : Promise.resolve())

const rel = (p) => relative(ROOT, p).replace(/\\/g, '/') || '.'

// ===========================================================================
// 参数
// ===========================================================================
const KNOWN_FLAGS = new Set(['--webview', '--electron', '--update', '--all-targets', '--frozen', '--no-mirror', '--no-tauri-cli', '--check', '--help', '-h'])
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
  noTauriCli: flags.has('--no-tauri-cli'),
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
      '  --no-tauri-cli    不自动安装 tauri-cli（默认缺失时自动 cargo install，首次约几分钟）',
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
 * 执行子进程，返回 Promise。
 *
 * 输出三条路同时走：
 *   1) 实时转发到本进程 stdout/stderr —— 安装可能几分钟，必须看得见进度；
 *   2) 完整原始输出 tee 到 LOG_FILE —— 事后可全文排查；
 *   3) 内存保留最后 TAIL_BYTES 字节 —— 失败时挂到 Error.tail，由主流程在最终汇总处重放。
 *
 * ⚠️ 历史教训：旧实现用 spawnSync + stdio:'inherit'，输出虽实时，但 cargo/pnpm 的进度行有
 *    几百行，真正的错误（下载失败、TLS、编译错误）被顶出可视区，最终只剩
 *    「完成，但以下目标失败：webview-app」+ 退出码，无从定位。故改成 pipe 手动 tee。
 *
 * ⚠️ 不加 `shell: true`（与 pack-local.mjs 同一实测结论）：本机裸 `pnpm` / `cargo` 都解析到
 *    .exe shim，无 shell 直接 spawn 即可；shell 模式还会错误拆分含空格/分号/^ 的参数
 *    （`cargo install --version ^2` 经 cmd 会把 ^ 当转义符吞掉）。
 */
function run(cmd, { cwd, env } = {}) {
  const printable = cmd.map((c) => (c.includes(' ') ? `"${c}"` : c)).join(' ')
  const header = `\n$ (cd ${rel(cwd)}/) ${printable}\n`
  console.log(header)
  logWrite(header)

  return new Promise((resolveRun, rejectRun) => {
    const child = spawn(cmd[0], cmd.slice(1), {
      cwd,
      env,
      stdio: ['ignore', 'pipe', 'pipe'],
      windowsHide: true,
    })
    let tail = ''
    const ingest = (buf, toStderr) => {
      ;(toStderr ? process.stderr : process.stdout).write(buf)
      const text = buf.toString('utf8')
      logWrite(text)
      tail += text
      // 控制内存：远超窗口时先砍一刀，最终 close 时再精确截断。
      if (tail.length > TAIL_BYTES * 2) tail = tail.slice(-TAIL_BYTES)
    }
    child.stdout.on('data', (b) => ingest(b, false))
    child.stderr.on('data', (b) => ingest(b, true))
    child.on('error', (err) => rejectRun(new Error(`命令无法执行（${cmd[0]}）：${err.message}`)))
    child.on('close', (status) => {
      if (status === 0) {
        resolveRun()
        return
      }
      let t = tail.slice(-TAIL_BYTES)
      const nl = t.indexOf('\n') // 截掉窗口里被腰斩的第一行，重放时从完整行开始
      if (nl >= 0 && nl < t.length - 1) t = t.slice(nl + 1)
      const err = new Error(`命令失败（退出码 ${status}）：${printable}`)
      err.tail = t.trimEnd()
      rejectRun(err)
    })
  })
}

/**
 * Electron 侧子进程环境。
 *
 * ⚠️ 剥掉 ELECTRON_RUN_AS_NODE：部分宿主（IDE / agent 沙箱）给所有子进程注入它（=1），
 *    泄漏进任何 electron 可执行文件会让其退化成纯 node 模式（见 electron-app/AGENTS.md
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
  // 查 `cargo tauri --version`，缺失直接退出）。普通 `cargo build` / sidecar 编译不需要它。
  // 策略：仅在选中 webview 侧时检查；默认缺失即自动安装（见 ensureTauriCli，与 CI 同一步），
  // --no-tauri-cli 显式跳过时只提示、不算问题；--check 永远只报告不安装。
  if (wantWebview) {
    const tauriVersion = tauriCliVersion()
    if (tauriVersion) {
      console.log(`  tauri   ${tauriVersion}（cargo tauri 子命令，本地打 Tauri 包用）`)
    } else if (opts.noTauriCli) {
      console.log('  tauri   — 未安装（--no-tauri-cli 已跳过；本地打 Tauri 包前需手动执行 cargo install tauri-cli --version "^2"）')
    } else {
      const hint = opts.checkOnly
        ? '未安装 tauri-cli（cargo tauri）：本地打 Tauri 包需要（普通 cargo build 不需要）。' +
          '不带 --check 运行本脚本会自动安装，或手动执行 `cargo install tauri-cli --version "^2"`（首次约几分钟）'
        : '未安装 tauri-cli（cargo tauri）：本地打 Tauri 包需要；本次将自动安装（与 CI 的 Install Tauri CLI 同一步，首次约几分钟），加 --no-tauri-cli 可跳过'
      console.log('  tauri   ✗ 未找到 cargo tauri 子命令（本地打 Tauri 包需要；普通 cargo build 不需要）')
      problems.push(hint)
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

async function initWebview() {
  const manifest = join(WEBVIEW, 'Cargo.toml')
  if (!existsSync(manifest)) throw new Error(`找不到 ${rel(manifest)}，请在仓库根运行本脚本`)
  if (!toolVersion('cargo')) {
    throw new Error('未找到 cargo。请先安装 Rust 工具链：https://rustup.rs （webview-app 与 electron sidecar 都需要）')
  }

  if (opts.update) {
    console.log('\n── [webview-app] cargo update（更新 Cargo.lock 到 semver 允许的新版本）──────────')
    await run(['cargo', 'update', '--manifest-path', manifest], { cwd: ROOT })
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
  await run(fetchCmd, { cwd: ROOT })
  console.log('  ✓ cargo 依赖已就绪')
}

/**
 * 确保 tauri-cli（`cargo tauri` 子命令）可用；缺失时自动安装，与 CI 的 Install Tauri CLI
 * 完全同一步：`cargo install tauri-cli --version "^2"`（首次要编译几分钟）。
 *
 * 设计要点：
 *   · 判据前后都用 `cargo tauri --version`（tauriCliVersion），与 pack-local.mjs 的
 *     have(['cargo','tauri','--version']) 保持一致，避免「init 说装了、打包却说没装」；
 *   · 装完复验一次：cargo install 退出 0 但 PATH 里的 cargo bin 目录未生效时（极少见），
 *     给出可操作提示而不是默默成功；
 *   · 版本约束 "^2" 走参数数组传给 spawn，不经 shell，^ 不会被 cmd 转义吞掉。
 */
async function ensureTauriCli() {
  const version = tauriCliVersion()
  if (version) {
    console.log(`  ✓ tauri-cli ${version} 已安装，跳过`)
    return
  }
  console.log('\n── [webview-app] 安装 tauri-cli（cargo install tauri-cli --version "^2"）────────')
  console.log('  本地打 Tauri 包（pack-local.mjs --target tauri-*）的硬依赖；首次编译约几分钟，请耐心等待 …')
  await run(['cargo', 'install', 'tauri-cli', '--version', '^2'], { cwd: ROOT })
  const after = tauriCliVersion()
  if (!after) {
    throw new Error(
      'tauri-cli 安装命令已成功返回，但 `cargo tauri --version` 仍不可用：' +
        '多半是 cargo bin 目录（~/.cargo/bin）未在当前终端 PATH 生效，新开一个终端后重跑本脚本复验',
    )
  }
  console.log(`  ✓ tauri-cli ${after} 安装完成`)
}

async function initElectron() {
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
  await run(installCmd, { cwd: ELECTRON, env })

  // `electron-app/AGENTS.md` 反复强调的判据：退出码 0 不算数，只认平台二进制真的在 dist/ 里。
  const exe = electronExePath()
  if (!existsSync(exe)) {
    console.log('\n  ⚠️ pnpm install 已返回，但未发现 electron 平台二进制；执行 install.js 兜底 …')
    await run(['node', join('node_modules', 'electron', 'install.js')], { cwd: ELECTRON, env })
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
async function main() {
  console.log('web2app 环境初始化')
  console.log(`仓库根：${ROOT}`)
  console.log(`子进程完整日志：${LOG_FILE}（仅在真正执行子进程时创建）`)
  console.log(
    `目标：${[wantWebview ? 'webview-app(cargo)' : null, wantElectron ? 'electron-app(pnpm)' : null].filter(Boolean).join(' + ')}` +
      `${opts.update ? '；cargo --update' : ''}${opts.allTargets ? '；cargo --all-targets' : ''}${opts.frozen ? '；pnpm --frozen-lockfile' : ''}${opts.noTauriCli ? '；--no-tauri-cli' : ''}${opts.checkOnly ? '；--check（只检查）' : ''}`,
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
  let anyTail = false
  // 失败时在此处重放子进程输出尾部 —— 错误不再只存在于滚走的进度屏里。
  const reportFailure = (label, e) => {
    console.error(`\n✗ [${label}] ${e.message}`)
    if (e.tail) {
      anyTail = true
      console.error('  ┌─ 子进程输出尾部（完整输出见本次日志文件）──────────')
      for (const line of e.tail.split(/\r?\n/)) console.error(`  │ ${line}`)
      console.error('  └────────────────────────────────────────────────────')
    }
    failures.push(label)
  }

  // 工具链整体先报一遍（两个壳的版本要求一起看），但不因此短路：
  // 只做 electron 的人机器上暂时没装 Rust 时，仍应让 electron 侧跑完。
  const toolProblems = checkToolchain()
  if (toolProblems.length) console.log(`\n（以上 ${toolProblems.length} 项工具链告警；仍继续执行所选目标，失败会在对应步骤报出）\n`)

  let webviewBaseOk = false
  if (wantWebview) {
    try {
      await initWebview()
      webviewBaseOk = true
    } catch (e) {
      reportFailure('webview-app', e)
    }
  }

  // tauri-cli 紧跟 webview 基础依赖之后安装；fetch 都失败时不再跑 cargo install
  //（网络/工具链有问题时它只会重复失败、白等几分钟编译）。
  if (wantWebview && webviewBaseOk && !opts.noTauriCli) {
    try {
      await ensureTauriCli()
    } catch (e) {
      reportFailure('tauri-cli', e)
    }
  }

  if (wantElectron) {
    try {
      await initElectron()
    } catch (e) {
      reportFailure('electron-app', e)
    }
  }

  console.log('\n────────────────────────────────────────────────────────')
  if (failures.length) {
    console.error(`完成，但以下目标失败：${failures.join('、')}`)
    if (anyTail) console.error(`排查：完整子进程输出见日志 ${LOG_FILE}`)
    await closeLog()
    process.exit(1)
  }
  console.log('✓ 环境初始化完成')
  if (wantElectron) console.log('  后续：electron-app/ 下 pnpm start（开发）或 pnpm run probe（契约自检）')
  if (wantWebview) console.log('  后续：webview-app/ 下 cargo build，或 electron-app/ 下 pnpm run sidecar 编 sidecar')
  if (wantWebview && !opts.noTauriCli) console.log('  本地打 Tauri 包：node scripts/pack-local.mjs --config <配置.json> --target tauri-msi')
  await closeLog()
}

main().catch(async (e) => {
  console.error(`\n✗ init-env 发生未预期错误：${e?.stack || e}`)
  await closeLog()
  process.exit(1)
})
