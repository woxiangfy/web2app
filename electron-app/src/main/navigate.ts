/**
 * 目标地址解析（对齐 webview-app `bootstrap::prepare_navigation` 的语义）。
 *
 * 三态，与 Tauri 版逐条对应：
 *   1. 字符串 `app_url` + 不走本地代理 → 直连（剥掉 userinfo，鉴权由 Chromium 的 login 事件注入）
 *   2. 字符串 `app_url` + 走本地代理   → sidecar 反向代理（给每个请求注入 Authorization，含 WebSocket）
 *   3. 域名数组 `app_url`              → sidecar 做 DNS TXT 解析 + Ed25519 验签 + AES-GCM 解密，
 *                                        再按 `proxy_enabled` 决定「解析后代理」还是「解析后直连」
 *
 * ⚠️ 2 与 3 都依赖 sidecar（`w2a-proxy.exe`）。拿不到 sidecar 时**明着报错，绝不静默降级成直连** ——
 * 「以为在走加密解析 / 以为鉴权由代理注入，其实直连了」是最危险的一类回归：请求会打到错误的
 * 地方，而界面看起来一切正常。
 */
import type { AppConfig } from './config'
import { SidecarError, type SidecarHandle, type SidecarRequest } from './sidecar'

/** 实际走的路径，写进日志便于排障。 */
export type ResolveHow = 'direct' | 'proxy' | 'dns' | 'dns+proxy'

export interface ResolvedTarget {
  ok: true
  /** 真正要导航过去的地址 */
  url: string
  /** userinfo 里的用户名 / 密码，供 Chromium 的 login 事件注入 */
  user: string
  pass: string
  how: ResolveHow
  /**
   * 走 sidecar 时的句柄。
   * ⚠️ 持有者**必须**在窗口关闭 / 重试 / 应用退出时调用 `stop()`，否则会留下一个仍在
   * 监听 127.0.0.1 的孤儿进程（宿主被强杀时由 sidecar 的 stdin-EOF 兜底，但正常退出路径
   * 不该指望那个兜底）。
   */
  sidecar?: SidecarHandle
}

export interface TargetFailure {
  ok: false
  title: string
  detail: string
}

export type TargetResult = ResolvedTarget | TargetFailure

/** 是否需要【本地反向代理】：启用代理，但没配外部代理（对齐 Tauri `local_proxy_enabled()`）。 */
export function needsLocalProxy(cfg: AppConfig): boolean {
  if (!cfg.proxy_enabled) return false
  return !cfg.proxy_http.trim() && !cfg.proxy_https.trim()
}

/** 是否需要 DNS TXT 解析（`app_url` 是数组即视为域名列表，对齐 `resolve::parse_domains`）。 */
export function needsDnsResolve(cfg: AppConfig): boolean {
  return Array.isArray(cfg.app_url)
}

/** 是否需要 sidecar：本地代理与 DNS 解析都靠它。 */
export function needsSidecar(cfg: AppConfig): boolean {
  return needsLocalProxy(cfg) || needsDnsResolve(cfg)
}

/** 去掉 URL 里的 userinfo，返回「干净」地址（对齐 Tauri `strip_userinfo`）。 */
function stripUserinfo(u: URL): string {
  const clean = new URL(u.toString())
  clean.username = ''
  clean.password = ''
  return clean.toString()
}

/** 校验字符串形式的 app_url；不合法时给出与 Tauri 版一致的文案。 */
function parseAppUrl(raw: string): { ok: true; parsed: URL } | TargetFailure {
  if (!raw) {
    return { ok: false, title: '加载失败', detail: '配置里的 app_url 为空' }
  }
  let parsed: URL
  try {
    parsed = new URL(raw)
  } catch (e) {
    // 文案与 Tauri 版保持一致，便于两端对着同一份日志排查
    return {
      ok: false,
      title: '加载失败',
      detail: `应用配置的网址不合法（${(e as Error).message}）：${raw}`,
    }
  }
  if (!/^https?:$/.test(parsed.protocol)) {
    return {
      ok: false,
      title: '加载失败',
      detail: `只支持 http / https 地址，当前是 ${parsed.protocol}：${raw}`,
    }
  }
  return { ok: true, parsed }
}

/**
 * sidecar 失败 → 用户可见文案。
 *
 * ⚠️ `resolve` 类失败沿用 Tauri 版的固定文案（「服务器连接失败，请检查网络后重试」），
 * 底层原因只进日志 —— 与 Tauri 侧 `map_err(|_| ...)` 的取舍一致，用户看到的提示两端相同。
 */
export function sidecarFailureText(e: unknown): string {
  const err = e instanceof SidecarError ? e : null
  const raw = err?.message ?? String(e)
  switch (err?.kind) {
    case 'resolve':
      return '服务器连接失败，请检查网络后重试'
    case 'missing':
      return `缺少本地代理组件（w2a-proxy），本可执行文件不完整：${raw}`
    case 'timeout':
      return `本地代理组件无响应：${raw}`
    default:
      return `本地代理组件启动失败：${raw}`
  }
}

export interface ResolveContext {
  /** 启动 sidecar。由 index.ts 注入真实实现（探针可换成假实现）。 */
  startSidecar: (req: SidecarRequest) => Promise<SidecarHandle>
}

/**
 * 解析出真正要导航的地址。
 *
 * 不需要 sidecar 的唯一情形是「字符串地址 + 不走本地代理」—— 那时纯 JS 就够了，
 * 连进程都不用起（外部代理由 Chromium 的 `--proxy-server` switch 处理）。
 */
export async function resolveTarget(
  cfg: AppConfig,
  ctx: ResolveContext
): Promise<TargetResult> {
  const dns = needsDnsResolve(cfg)
  const useProxy = needsLocalProxy(cfg)
  const raw = dns ? '' : String(cfg.app_url ?? '').trim()

  // ---------- 路径 1：直连 / 外部代理，完全不需要 sidecar ----------
  if (!dns && !useProxy) {
    const p = parseAppUrl(raw)
    if (!p.ok) return p
    return {
      ok: true,
      url: stripUserinfo(p.parsed),
      user: decodeURIComponent(p.parsed.username || ''),
      pass: decodeURIComponent(p.parsed.password || ''),
      how: 'direct',
    }
  }

  // ---------- 路径 2 / 3：交给 sidecar ----------
  // 本地能查出来的配置错误就先查掉，别丢给 sidecar 去报（那边报出来的文案更笼统）。
  if (!dns) {
    const p = parseAppUrl(raw)
    if (!p.ok) return p
  }
  if (dns && !cfg.public_key.trim()) {
    return {
      ok: false,
      title: '加载失败',
      detail: '配置使用域名解析模式（app_url 是数组），但没有提供 public_key',
    }
  }

  const domains = dns ? (cfg.app_url as string[]) : undefined
  try {
    const h = await ctx.startSidecar({
      url: dns ? undefined : raw,
      domains,
      publicKey: dns ? cfg.public_key : undefined,
      // 只做解析、不起代理 —— 此时凭据需要回到宿主手里（login 事件注入）
      noProxy: !useProxy,
    })

    return {
      ok: true,
      // 走代理时导航到本地入口；只解析时导航到解析结果
      url: useProxy ? h.proxyUrl : h.resolvedUrl,
      // 走代理时凭据由代理注入，宿主不持有（sidecar 也不会回传）
      user: useProxy ? '' : h.authUser,
      pass: useProxy ? '' : h.authPass,
      how: dns ? (useProxy ? 'dns+proxy' : 'dns') : 'proxy',
      sidecar: h,
    }
  } catch (e) {
    // 失败原因进日志（含 sidecar 的 stderr），界面用收敛后的文案
    console.error(`[web2app] sidecar 失败：${(e as Error).message}`)
    return { ok: false, title: '加载失败', detail: sidecarFailureText(e) }
  }
}
