/**
 * 进程与文件工具（本地打包驱动用，与解析契约无关）。
 */
import { spawn, spawnSync } from 'node:child_process'
import { copyFileSync, cpSync, existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { createHash } from 'node:crypto'
import { tmpdir } from 'node:os'
import { isAbsolute, join, resolve } from 'node:path'
import { FatalError, emit } from './diagnostic.mjs'
import { ELECTRON, ROOT, rel } from './paths.mjs'

export function have(cmd) {
  const r = spawnSync(cmd[0], cmd.slice(1), { stdio: 'ignore' })
  return !r.error && r.status === 0
}

/**
 * 执行子进程。⚠️ **不要加 `shell: true`**：实测 `spawnSync('pnpm', ...)` 无 shell 即可工作，
 * 而 shell 模式下 Node 只做 `join(' ')` 不做转义 ⇒ `-c.productName=带空格的名字` 会被 cmd.exe 拆成两个参数。
 * （另外 Node ≥ 22 明确禁止无 shell 直接执行 `.cmd`，但把名字写成 `pnpm` 反而能正常解析。）
 */
export function run(cmd, cwd, env, dry) {
  const printable = cmd.map((c) => (c.includes(' ') ? `"${c}"` : c)).join(' ')
  emit(`\n$ (cd ${rel(cwd)}/) ${printable}`)
  if (dry) return
  const r = spawnSync(cmd[0], cmd.slice(1), { cwd, env, stdio: 'inherit' })
  if (r.error) throw new FatalError(`命令无法执行（${cmd[0]}）：${r.error.message}`)
  if (r.status !== 0) throw new FatalError(`命令失败（退出码 ${r.status}）：${printable}`)
}

export function sha256(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex')
}

/** `20260924-215530` —— 给自动降级目录用的时间戳（本地时区，够唯一）。 */
export function stamp() {
  const d = new Date()
  const p = (n) => String(n).padStart(2, '0')
  return (
    `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}` +
    `-${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}`
  )
}

/**
 * 跑子进程，输出**边跑边透传**给用户，同时留一份尾部用于判断失败类型。
 *
 * ⚠️ 必须边跑边透传：一次 electron-builder 要几分钟，若全缓冲到最后才打印，
 *    用户完全无法判断「卡住了」还是在干活。
 * 返回 `{ status, tail }`（`status` 为负 = 进程没能起来）。**不抛错** —— 由调用方决定怎么处理。
 */
export function runTee(cmd, cwd, env, dry) {
  const printable = cmd.map((c) => (c.includes(' ') ? `"${c}"` : c)).join(' ')
  emit(`\n$ (cd ${rel(cwd)}/) ${printable}`)
  if (dry) return Promise.resolve({ status: 0, tail: [] })
  return new Promise((done) => {
    const p = spawn(cmd[0], cmd.slice(1), { cwd, env })
    const tail = []
    const onData = (chunk) => {
      const s = chunk.toString()
      process.stdout.write(s) // 透传，保持「看得见进度」
      for (const line of s.split(/\r?\n/)) if (line.trim()) tail.push(line)
      if (tail.length > 400) tail.splice(0, tail.length - 400)
    }
    p.stdout.on('data', onData)
    p.stderr.on('data', onData)
    p.on('error', (e) => {
      emit(`  子进程无法启动：${e.message}`)
      done({ status: -1, tail })
    })
    p.on('close', (code) => done({ status: code === null ? -1 : code, tail }))
  })
}

/**
 * 失败是不是「这台机器不让删东西」这一类 —— 而不是配置写错 / 代码坏。
 * electron-builder 打包前要清空 appOutDir，清不掉时它给的就是这两种标记。
 */
export function isDeleteBlocked(tail) {
  return tail.some((l) => /safe-delete|emptyDir|trash operation/i.test(l))
}

/**
 * 记录「原文件 → 备份」，退出时无条件还原。
 *
 * ⚠️ 用 `finally` 而不是「成功才还原」：打包中途失败时更要还原，否则工作区留着一份被改过的
 * `tauri.conf.json`，下次 git diff 会莫名其妙。
 */
export class Restorer {
  constructor() {
    this.items = []
    this.dirs = []
    this.tmp = mkdtempSync(join(tmpdir(), 'w2a-pack-backup-'))
    // ⚠️⚠️ 把备份目录**打印出来**：`finally` 还原的前提是进程活着走到那里。
    //    实测踩过：构建被 kill ⇒ 不还原 ⇒ `electron-app/config.json` 留在生成态。
    //    备份本身是好的，只是没人知道它在哪 —— 这一行就是那个入口。
    emit(`（原文件备份目录：${this.tmp}）`)
  }

  file(path) {
    if (!existsSync(path)) return
    const bak = join(this.tmp, `${this.items.length}-${path.split(/[\\/]/).pop()}`)
    copyFileSync(path, bak)
    this.items.push([path, bak])
  }

  dir(path) {
    if (!existsSync(path)) return
    const bak = join(this.tmp, `dir-${this.dirs.length}-${path.split(/[\\/]/).pop()}`)
    cpSync(path, bak, { recursive: true })
    this.dirs.push([path, bak])
  }

  restore(keep) {
    if (keep) {
      emit(`⚠️ --keep-config：刻意保留改动，备份留在 ${this.tmp}`)
      return
    }
    for (const [path, bak] of this.items) {
      copyFileSync(bak, path)
      emit(`已还原 ${rel(path)}（sha256 ${sha256(path).slice(0, 12)}）`)
    }
    for (const [path, bak] of this.dirs) {
      if (existsSync(path)) rmSync(path, { recursive: true, force: true })
      cpSync(bak, path, { recursive: true })
      emit(`已还原目录 ${rel(path)}/`)
    }
    rmSync(this.tmp, { recursive: true, force: true })
  }
}

/** 与 CI 逐字一致的「安装名」清洗：去 Windows 文件名非法字符、压空白、去首尾空格与句点。 */
export function safeName(raw) {
  let s = raw ? raw.replace(/[\\/:*?"<>|]+/g, '_') : ''
  s = s ? s.replace(/\s+/g, ' ').replace(/^[ .]+|[ .]+$/g, '') : ''
  return s || 'Web2App'
}

/**
 * 两壳本地打包的默认产物目录：仓库根 `tests/pack/electron/` 与 `tests/pack/tauri/`。
 * **刻意分子目录** —— 两壳的绿色版都叫 `<name>.exe`（electron-builder portable 同样产出
 * `<name>.exe`），混放会互相覆盖。整棵 `/tests/` 被根 .gitignore 忽略，产物不进版本库。
 */
export const ELECTRON_OUT_DEFAULT = join(ROOT, 'tests', 'pack', 'electron')
export const TAURI_OUT_DEFAULT = join(ROOT, 'tests', 'pack', 'tauri')

/**
 * 解析 `--out`：相对路径一律以**对应壳工程目录**为基准（electron-builder 在 electron-app/ 下跑、
 * cargo 在 webview-app/ 下跑，与各自配置里的相对路径语义一致），不按调用方 cwd 解析；
 * 绝对路径原样使用。
 *
 * 本地打包的默认目录都在仓库根 `tests/pack/` 下（见 doElectron / doTauri），
 * 整棵 `/tests/` 被根 .gitignore 忽略，产物不进版本库；`--out ../tests/<名字>` 可在 tests/ 下另换目录。
 */
export function resolvePackOut(arg, baseDir) {
  return isAbsolute(arg) ? resolve(arg) : resolve(baseDir, arg)
}

/** Electron 侧的 `--out`：相对路径锚定 electron-app/。 */
export function resolveElectronOut(arg) {
  return resolvePackOut(arg, ELECTRON)
}

/**
 * 跨卷也安全的文件移动：先复制再删源（`rename` 在跨盘符时会 EXDEV）。
 * 打包产物是个位数的大文件，复制成本可接受；目标同名文件直接覆盖。
 */
export function moveFile(src, dest) {
  copyFileSync(src, dest)
  rmSync(src, { force: true })
}

/**
 * 把单张 PNG 封进 Windows ICO（单帧）。**零第三方依赖**，只写 ICO 容器头，图像体原样放 PNG。
 *
 * 为什么必须在本地做：electron-builder 26 收到 `win.icon=*.png` 时，要用它**运行时从
 * GitHub releases 下载**的 icons-bundle 工具集把 PNG 转成 ICO；直接给 `.ico` 则零下载。
 * 本机实测受限网络下 GitHub 不可达 ⇒ 锁重试约 8 分钟后 EPERM，打包必败。
 *
 * ICO 自 Vista 起允许图像体是 PNG 压缩流（微软官方 ICO 的 256×256 帧本来就是 PNG），
 * 单帧 PNG-in-ICO 合法，electron-builder 的 ICO 校验与 resedit 嵌入都能直接消费。
 *
 * 不做多尺寸：没有 PNG 解码库就无法缩放，256 帧由 Windows 自行缩放到 16~256 各档。
 * 故要求输入 ≥256 —— 与 electron-builder 的 ERR_ICON_TOO_SMALL 同一条判据，提前给出中文报错。
 */
export function pngToIco(png) {
  if (png.subarray(0, 8).toString('hex') !== '89504e470d0a1a0a') {
    throw new FatalError('生成 ICO 失败：icon_url 下载的文件不是 PNG')
  }
  // IHDR：宽在偏移 16、高在偏移 20，均为大端 4 字节。
  const width = png.readUInt32BE(16)
  const height = png.readUInt32BE(20)
  const dim = Math.max(width, height)
  if (!Number.isInteger(dim) || dim <= 0 || dim > 1024) {
    throw new FatalError(`生成 ICO 失败：PNG 尺寸非法（${width}x${height}）`)
  }
  if (dim < 256) {
    throw new FatalError(
      `icon_url 的 PNG 只有 ${width}x${height}，Windows 应用图标要求至少 256x256` +
        '（electron-builder 同样会以 ERR_ICON_TOO_SMALL 拒绝），请换一张大图',
    )
  }
  const header = Buffer.alloc(6)
  header.writeUInt16LE(0, 0) // reserved
  header.writeUInt16LE(1, 2) // type：1 = icon
  header.writeUInt16LE(1, 4) // 图像条目数 = 1
  const entry = Buffer.alloc(16)
  const sizeByte = dim >= 256 ? 0 : dim // ICO 尺寸字节里 0 即 256
  entry.writeUInt8(sizeByte, 0) // 宽
  entry.writeUInt8(sizeByte, 1) // 高
  entry.writeUInt8(0, 2) // 调色板颜色数（0）
  entry.writeUInt8(0, 3) // reserved
  entry.writeUInt16LE(1, 4) // 色彩平面
  entry.writeUInt16LE(32, 6) // 位深
  entry.writeUInt32LE(png.length, 8) // 图像体大小
  entry.writeUInt32LE(22, 12) // 图像体偏移 = 6（ICONDIR）+ 16（一条 ICONDIRENTRY）
  return Buffer.concat([header, entry, png])
}

/**
 * electron-builder 26 在 Windows 上会**按需**从 GitHub releases 另下几套工具集：
 * zip 目标的 7zip@1.0.0、portable / nsis 目标必需的 nsis@1.2.1 等。这与已用
 * `electronDist` 跳过的 223MB Electron 运行时是两套下载，受限网络下会在出包阶段卡到超时。
 *
 * 策略：用户已显式配置镜像环境变量时一律尊重；否则探一次**真实 release 资源**（短超时，
 * 跟随 302 到 objects 主机——只探 github.com 主站不准：主站能开不代表 release 能下），
 * 不通就自动换 npmmirror 镜像。镜像产物仍由 electron-builder 用内置 SHA256 校验，镜像无法投毒。
 */
export const BUILDER_BINARIES_PROBE_URL =
  'https://github.com/electron-userland/electron-builder-binaries/releases/download/nsis@1.2.1/nsis-bundle-3.12.tar.gz'
export const BUILDER_BINARIES_MIRROR = 'https://npmmirror.com/mirrors/electron-builder-binaries/'
export const BUILDER_BINARIES_MIRROR_ENV_KEYS = [
  'NPM_CONFIG_ELECTRON_BUILDER_BINARIES_MIRROR',
  'npm_config_electron_builder_binaries_mirror',
  'npm_package_config_electron_builder_binaries_mirror',
  'ELECTRON_BUILDER_BINARIES_MIRROR',
]

export async function resolveBuilderBinariesMirror(env, dry) {
  const existing = BUILDER_BINARIES_MIRROR_ENV_KEYS.map((k) => env[k]).find((v) => v && v.trim())
  if (existing) {
    emit(`    electron-builder 工具集下载源：沿用环境变量 ${existing}`)
    return
  }
  if (dry) {
    emit('    electron-builder 工具集下载源：dry-run，不做 GitHub 连通性探测')
    return
  }
  let reachable = false
  try {
    // HEAD 跟随重定向：能拿到任意 HTTP 响应（302/200）= release 下载链通；超时/连接失败才换镜像。
    await fetch(BUILDER_BINARIES_PROBE_URL, { method: 'HEAD', signal: AbortSignal.timeout(8000) })
    reachable = true
  } catch {
    reachable = false
  }
  if (reachable) {
    emit('    electron-builder 工具集下载源：GitHub release 可达，使用官方源')
    return
  }
  env.ELECTRON_BUILDER_BINARIES_MIRROR = BUILDER_BINARIES_MIRROR
  emit(`    GitHub release 8s 探测不通 ⇒ electron-builder 工具集自动改用镜像：${BUILDER_BINARIES_MIRROR}`)
}
