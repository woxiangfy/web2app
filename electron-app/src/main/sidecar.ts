/**
 * w2a-proxy sidecar 客户端。
 *
 * 为什么是独立进程而不是塞进主进程（决定见 docs/electron-plan.md §2）：本地反向代理（741 行）与加密解析
 * （DNS TXT + Ed25519 + AES-GCM）都已在 `webview-app` 里写就并测过，用 JS 重写等于把
 * 鉴权注入 / 流式转发 / WS 隧道的验证重开一遍，且加密格式必须与 `resolve-core` 字节级一致。
 * 所以 Electron 侧 spawn 一个复用同一份 Rust 源码的 sidecar，只做「传参进去、读回一行 JSON」。
 *
 * 与 sidecar 的契约（对应 crates/proxy-server/src/main.rs 的头注释）：
 *   - 入参走命令行；
 *   - 就绪/失败都往 stdout 打**且仅打一行 JSON**，本模块按第一行 `\n` 切分解析；
 *   - sidecar 常驻服务，靠 **stdin EOF** 感知宿主退出。所以这里**绝不能提前关掉 stdin**
 *     （`stdio` 保持 'pipe'、不要调 `child.stdin.end()`，除非就是要它退出）。
 *
 * ⚠️⚠️ `windowsHide: true` 不是可选项：sidecar 是 console 子系统程序，不加的话每次启动都会
 * 在用户屏幕上闪一个黑色命令行窗口。绿色版尤其忌讳这个「一看就像病毒」的观感。
 * ⚠️ 环境变量必须**删掉 `ELECTRON_RUN_AS_NODE`** 再传下去：宿主 IDE / agent 沙箱会给所有
 * 子进程注入它（见 scripts/launch.cjs 的说明）。它虽然不影响这个纯 Rust 二进制，但同一套
 * env 将来若透传给别的 Node 子进程就会复发那个极难排查的「ExitCode 0 却没输出」。
 */
import { spawn, type ChildProcess } from 'node:child_process'
import { existsSync } from 'node:fs'
import { join } from 'node:path'

/** 请求 sidecar 做什么。`domains` 与 `url` 二选一，同时给则以 `domains` 为准（对齐 Tauri 侧）。 */
export interface SidecarRequest {
  /** 直连上游地址，可含 `user:pass@`（代理会强制注入 Basic 头） */
  url?: string
  /** 域名数组 → DNS TXT 解析 + 验签解密 */
  domains?: string[]
  /** Ed25519 公钥（裸 base64 / PEM / SPKI），域名模式必填 */
  publicKey?: string
  /** 只解析地址、不起本地代理（宿主改用 Chromium 的 login 事件注入凭据） */
  noProxy?: boolean
}

/** sidecar 握手成功后的句柄。 */
export interface SidecarHandle {
  /** 本地代理入口（如 `http://127.0.0.1:53211/`）；`noProxy` 时是解析后的上游地址 */
  proxyUrl: string
  /** 解析后的上游地址（**已剥掉 userinfo**，可安全写日志） */
  resolvedUrl: string
  upstreamHost: string
  hadAuth: boolean
  /**
   * ⚠️⚠️ 明文凭据，**仅 `noProxy` 模式（DNS 解析 + 直连）有值**。
   * 用途只有一个：喂给 Chromium 的 `login` 事件（对应 Tauri 侧 WebView2 的原生鉴权事件）。
   * **绝不能**写进日志 / 错误弹窗 / 崩溃报告 —— 这直接就是站点的 Basic 账号密码。
   * 走本地代理时为空：那时由代理给每个请求注入 `Authorization`，宿主不碰凭据。
   */
  authUser: string
  authPass: string
  pid: number
  /** 关掉 sidecar：先关 stdin 让它自己退，1s 后仍在则强杀。 */
  stop(): void
}

/** 握手超时。DNS 解析要走网络（可能几秒），代理启动本身很快，30s 足够宽松。 */
const HANDSHAKE_TIMEOUT_MS = 30_000

export class SidecarError extends Error {
  constructor(
    message: string,
    /** 区分失败阶段，便于上层给出准确文案（对齐 sidecar 的 kind 字段） */
    readonly kind: 'missing' | 'spawn' | 'args' | 'resolve' | 'proxy' | 'protocol' | 'timeout',
  ) {
    super(message)
    this.name = 'SidecarError'
  }
}

/** sidecar 可执行文件名（Windows 带 .exe）。 */
function exeName(): string {
  return process.platform === 'win32' ? 'w2a-proxy.exe' : 'w2a-proxy'
}

/**
 * 定位 sidecar 可执行文件。返回 null 表示「本次构建里根本没有它」。
 *
 * 顺序有讲究：
 *   1. `process.resourcesPath/sidecar/` —— 打包后的正式位置（electron-builder 的
 *      extraResources）。**绝不能放 asar 内**：asar 里的 exe 无法 spawn。
 *   2. `webview-app/target/release/` —— 开发态（`npm start`），且优先 release：
 *      代理是 CPU 敏感路径（流式转发 / 验签），debug 版在真实站点上会明显拖慢。
 *   3. `webview-app/target/debug/` —— 只编了 debug 时的兜底。
 */
export function locateSidecar(appPath: string, resourcesPath: string): string | null {
  const name = exeName()
  const candidates = [
    join(resourcesPath, 'sidecar', name),
    join(appPath, '..', 'webview-app', 'target', 'release', name),
    join(appPath, '..', 'webview-app', 'target', 'debug', name),
  ]
  for (const c of candidates) {
    if (existsSync(c)) return c
  }
  return null
}

/** 组装命令行参数（对齐 sidecar 的 `--help`）。 */
export function buildSidecarArgs(req: SidecarRequest): string[] {
  const args: string[] = []
  if (req.domains && req.domains.length > 0) {
    args.push('--domains', req.domains.join(','))
    args.push('--public-key', req.publicKey ?? '')
  } else if (req.url) {
    // ⚠️ 上游地址走 **stdin** 而不是命令行：URL 里可能内嵌 `user:pass@`，而 Windows 上
    // 命令行对同用户的其它进程是可见的（`GetCommandLine` / WMI 都能拿到）。
    // 域名模式没有这个问题（域名列表不含凭据），所以照常走命令行。
    args.push('--url-stdin')
  }
  if (req.noProxy) args.push('--no-proxy')
  return args
}

export interface StartSidecarOptions extends SidecarRequest {
  /** sidecar 可执行文件绝对路径（`locateSidecar` 的结果） */
  exe: string
  /** sidecar 的 stderr 日志回调（宿主写进自己的日志文件） */
  log: (line: string) => void
}

/**
 * 启动 sidecar 并等待握手。
 *
 * 失败一律 reject `SidecarError` —— **绝不静默降级**。这里的每一步失败都意味着
 * 「要么走不了代理、要么加密解析没生效」，静默降级成直连会让用户以为一切正常，
 * 却是把明文请求打到了错误的地方（最危险的一类回归）。
 */
export function startSidecar(opts: StartSidecarOptions): Promise<SidecarHandle> {
  const args = buildSidecarArgs(opts)
  if (args.length === 0) {
    return Promise.reject(new SidecarError('sidecar 参数为空：既没有 --url 也没有 --domains', 'args'))
  }
  // 需要经 stdin 送进去的地址（只有「直连/代理字符串模式」有；域名模式不含凭据）。
  const reqUrl = !(opts.domains && opts.domains.length > 0) && opts.url ? opts.url : null

  return new Promise<SidecarHandle>((resolve, reject) => {
    let child: ChildProcess
    try {
      const env = { ...process.env }
      // 见文件头：宿主注入的这个变量会污染子进程的 Node 行为。
      delete env.ELECTRON_RUN_AS_NODE

      child = spawn(opts.exe, args, {
        stdio: ['pipe', 'pipe', 'pipe'],
        windowsHide: true,
        env,
      })
    } catch (e) {
      reject(new SidecarError(`spawn sidecar 失败：${(e as Error).message}`, 'spawn'))
      return
    }

    let settled = false
    let buf = ''
    /** 握手之后 stdout 还剩的内容（sidecar 目前不打，留个口子便于将来输出进度）。 */
    let stdoutRest = ''

    const finish = (fn: () => void) => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      fn()
    }

    const timer = setTimeout(() => {
      finish(() => {
        killHard(child)
        reject(
          new SidecarError(
            `sidecar 握手超时（${HANDSHAKE_TIMEOUT_MS / 1000}s）：未收到 stdout JSON 行`,
            'timeout',
          ),
        )
      })
    }, HANDSHAKE_TIMEOUT_MS)

    child.stdout?.on('data', (chunk: Buffer) => {
      buf += chunk.toString('utf8')
      if (settled) {
        stdoutRest += buf
        buf = ''
        return
      }
      const nl = buf.indexOf('\n')
      if (nl < 0) return
      const line = buf.slice(0, nl).trim()
      buf = buf.slice(nl + 1)
      if (!line) return

      let v: Record<string, unknown>
      try {
        v = JSON.parse(line) as Record<string, unknown>
      } catch {
        finish(() => {
          killHard(child)
          reject(new SidecarError(`sidecar 握手不是合法 JSON：${line.slice(0, 200)}`, 'protocol'))
        })
        return
      }

      if (v.ok !== true) {
        const kind = (typeof v.kind === 'string' ? v.kind : 'protocol') as SidecarError['kind']
        finish(() => {
          // 失败时 sidecar 自己就退出了，这里兜个底防止异常路径留下孤儿。
          killHard(child)
          reject(new SidecarError(String(v.error ?? '未知错误'), kind))
        })
        return
      }

      finish(() =>
        resolve({
          proxyUrl: String(v.proxyUrl ?? ''),
          resolvedUrl: String(v.resolvedUrl ?? ''),
          upstreamHost: String(v.upstreamHost ?? ''),
          hadAuth: v.hadAuth === true,
          authUser: String(v.authUser ?? ''),
          authPass: String(v.authPass ?? ''),
          pid: Number(v.pid ?? child.pid ?? 0),
          stop: () => stopSidecar(child),
        }),
      )
    })

    child.stderr?.on('data', (chunk: Buffer) => {
      // sidecar 的日志走 stderr（stdout 留给握手）。按行拆开再转发，避免日志文件里
      // 出现一串没有换行的糊在一起的内容。
      for (const line of chunk.toString('utf8').split(/\r?\n/)) {
        if (line.trim()) opts.log(line.trimEnd())
      }
    })

    child.on('error', (e) => {
      finish(() => reject(new SidecarError(`sidecar 进程错误：${e.message}`, 'spawn')))
    })

    child.on('exit', (code, signal) => {
      finish(() =>
        reject(
          new SidecarError(
            `sidecar 在握手前退出（code=${code} signal=${signal}）；` +
              // ⚠️ 命令行里可能有 domains，但**不会有凭据**（凭据走 stdin，见 buildSidecarArgs）。
              `命令行：${args.join(' ')}`,
            'spawn',
          ),
        ),
      )
    })

    // 凭据经 stdin 传入（见 buildSidecarArgs）。**只写一行、绝不 end()**：
    // sidecar 靠这根 stdin 的 EOF 判断宿主是否还活着，提前关掉它会让代理刚起来就自杀。
    if (reqUrl !== null) {
      try {
        child.stdin?.write(reqUrl + '\n')
      } catch (e) {
        finish(() => {
          killHard(child)
          reject(new SidecarError(`写入 sidecar stdin 失败：${(e as Error).message}`, 'spawn'))
        })
      }
    }
  })
}

/** 关掉 sidecar：先关 stdin（它的退出信号），1s 后仍在则强杀。 */
export function stopSidecar(child: ChildProcess): void {
  if (child.exitCode !== null || child.signalCode !== null) return

  try {
    // 关 stdin = 告诉 sidecar「宿主走了」。它读到的就是 EOF。
    child.stdin?.end()
  } catch {
    /* 管道可能已经断了，忽略 */
  }

  const t = setTimeout(() => killHard(child), 1000)
  // 用 once 而不是 on：避免多次 stop() 时监听器堆积。
  child.once('exit', () => clearTimeout(t))
  // 不让这个定时器吊住 Electron 的退出流程。
  t.unref?.()
}

/** 强杀。Windows 上 `kill()` 落到 TerminateProcess，没有「优雅」这一说。 */
function killHard(child: ChildProcess): void {
  try {
    child.kill()
  } catch {
    /* 已经退了 */
  }
  // stdin 也一并断开，防止 sidecar 已死而管道还开着。
  try {
    child.stdin?.destroy()
  } catch {
    /* noop */
  }
}
