/**
 * config.json 解析（与 webview-app 共用同一份文件契约）。
 *
 * 与 Tauri 版的关键差异（有意为之，见 docs/electron-plan.md §4）：
 *   - Tauri 版：config.json 只是【打包期模板】，值被写成编译期常量（改配置必须重编译）；
 *   - Electron 绿色版：启动时从 installRoot()\config.json 【运行期读取】，
 *     改完重启即生效，无需重新打包。这也是绿色版该有的体验。
 *
 * ⚠️ 因此解析必须【宽容】：上游 build-custom.yml 生成的模板里布尔值是字符串
 *    （如 "proxy_enabled": "false"），不能只接受 boolean。
 */
import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'

/** Windows 无关的 13 个字段；android_* 4 个字段在本壳中忽略（见 MOBILE_ONLY_KEYS）。 */
export interface AppConfig {
  /** 目标地址；字符串 = 直连，字符串数组 = 走 DNS TXT 解析 + 验签解密 */
  app_url: string | string[]
  /** 安装名 / 数据目录名（Win）；Android 侧为应用名 */
  app_name: string
  /** 显示名（窗口标题）；缺省回落 app_name */
  title: string
  /** 注入给页面的全局对象 window.web2appExtra */
  extra: Record<string, unknown>
  /** 托盘/任务栏图标 URL；空 = 用内置图标 */
  icon_url: string
  /** true 且 proxy_http/proxy_https 为空 → 启用自身实现的本地反向代理 */
  proxy_enabled: boolean
  proxy_http: string
  proxy_https: string
  proxy_bypass: string
  /** 多页签开关：true 时走多页签模式（壳页面 + 子 WebContentsView）。M5 已实现 */
  tabs_enabled: boolean
  /** DNS TXT 解析模式下的 Ed25519 公钥 */
  public_key: string
  /** ⚠️ 改这里两端必须同步；缺省 web2app-fixed-secret-2026 */
  resolve_secret: string
  /** false（默认）= 数据放 exe 同级 data\；true = 放 %APPDATA%\ */
  data_dir_default: boolean
}

export interface LoadedConfig {
  config: AppConfig
  /** 实际读取的文件路径；未找到时为 null（使用内置默认值） */
  path: string | null
  /** 读取过程中的非致命提示，用于启动日志 */
  warnings: string[]
}

/** 仅 Android 使用的字段：在 Windows 壳里存在即忽略，不报错（向后兼容同一个 config.json） */
const MOBILE_ONLY_KEYS = [
  'android_keystore_base64',
  'android_key_alias',
  'android_keystore_password',
  'android_key_password',
] as const

export const DEFAULTS: AppConfig = {
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

/** 上游模板里布尔值可能是 "false" / "1" / "yes"，统一收敛；空串按默认值处理。 */
function toBool(v: unknown, dflt: boolean): boolean {
  if (typeof v === 'boolean') return v
  if (typeof v === 'string') {
    const s = v.trim().toLowerCase()
    if (s === '') return dflt
    return s === 'true' || s === '1' || s === 'yes' || s === 'on'
  }
  return dflt
}

function toStr(v: unknown, dflt = ''): string {
  return typeof v === 'string' ? v : v == null ? dflt : String(v)
}

/** 清洗成 Windows 合法目录名（与 webview-app 的 sanitize_dir_name 同规则）。 */
export function sanitizeDirName(name: string): string {
  const cleaned = name.replace(/[<>:"/\\|?*\u0000-\u001f]/g, '_').trim()
  return cleaned.length > 0 ? cleaned : 'Web2App'
}

/** extra 必须是合法 JSON 对象；非法时回落 {}，避免注入非法 JSON 破坏页面脚本。 */
function toExtra(v: unknown): Record<string, unknown> {
  let val: unknown = v
  if (typeof val === 'string') {
    try {
      val = JSON.parse(val)
    } catch {
      return {}
    }
  }
  if (val && typeof val === 'object' && !Array.isArray(val)) {
    return val as Record<string, unknown>
  }
  return {}
}

/**
 * title 缺省回落 app_name（对齐 webview-app：WINDOW_TITLE 未填时用 APP_NAME）。
 * ⚠️ 必须对【所有】返回路径生效——只在「读到文件」的分支里做回落，
 * 会让「没有 config.json」和「config.json 里 title 为空」两种情况表现不一致。
 */
function withTitleFallback(c: AppConfig): AppConfig {
  return c.title.trim().length > 0 ? c : { ...c, title: c.app_name }
}

/**
 * 按顺序在多个目录里找 `config.json`，用第一个存在的。
 *
 * 为什么需要它：单文件自解压（portable）形态下 exe 被解压到 `%TEMP%`，
 * `installRoot()` 指向的是**用户放置 exe 的目录**。若用户只丢了一个 exe、
 * 没在旁边放 config.json，就必须回落到包内预置的那份，
 * 否则「绿色版双击就能打开目标站点」这件事在 portable 形态下不成立
 * （会静默用到内置默认的 example.com）。
 * 解压目录版（zip/dir）两个目录其实是同一个，此函数退化为单次查找。
 *
 * ⚠️ 与 loadConfig 一致：文件存在但读不出来仍然抛错（致命），不静默回落 ——
 * 否则用户改坏一个字符，得到的是「配置像是没生效」，最难查。
 */
export function loadConfigAny(roots: string[]): LoadedConfig {
  const tried: string[] = []
  for (const r of roots) {
    if (!r) continue
    const p = join(r, 'config.json')
    tried.push(p)
    if (!existsSync(p)) continue
    return loadConfig(r)
  }
  return {
    config: withTitleFallback({ ...DEFAULTS }),
    path: null,
    warnings: [`未找到 config.json（已查找：${tried.join(' 、 ')}），使用内置默认配置`],
  }
}

export function loadConfig(root: string): LoadedConfig {
  const path = join(root, 'config.json')
  const warnings: string[] = []

  let raw: Record<string, unknown>
  try {
    raw = JSON.parse(readFileSync(path, 'utf8')) as Record<string, unknown>
  } catch (e) {
    const err = e as NodeJS.ErrnoException
    if (err.code === 'ENOENT') {
      warnings.push(`未找到 ${path}，使用内置默认配置`)
    } else {
      // 配置存在但读不出来属于致命错误：静默回落会让用户以为配置生效了
      throw new Error(`读取配置失败 ${path}：${err.message}`)
    }
    return { config: withTitleFallback({ ...DEFAULTS }), path: null, warnings }
  }

  for (const key of MOBILE_ONLY_KEYS) {
    if (key in raw) warnings.push(`字段 ${key} 仅 Android 使用，本壳已忽略`)
  }

  const appName = toStr(raw.app_name, DEFAULTS.app_name)
  const config: AppConfig = withTitleFallback({
    app_url: (raw.app_url as string | string[]) ?? DEFAULTS.app_url,
    app_name: appName,
    title: toStr(raw.title),
    extra: toExtra(raw.extra),
    icon_url: toStr(raw.icon_url),
    proxy_enabled: toBool(raw.proxy_enabled, DEFAULTS.proxy_enabled),
    proxy_http: toStr(raw.proxy_http),
    proxy_https: toStr(raw.proxy_https),
    proxy_bypass: toStr(raw.proxy_bypass),
    tabs_enabled: toBool(raw.tabs_enabled, DEFAULTS.tabs_enabled),
    public_key: toStr(raw.public_key),
    resolve_secret: toStr(raw.resolve_secret),
    data_dir_default: toBool(raw.data_dir_default, DEFAULTS.data_dir_default),
  })

  if (config.tabs_enabled) {
    // M5 已实现：tabs_enabled=true 走多页签模式（壳页面 + 子视图）。保留这行日志便于排障。
    console.log('[web2app] tabs_enabled=true，使用多页签模式')
  }

  return { config, path, warnings }
}

/** 注入给页面的 window.web2appExtra 字面量（对齐 webview-app 同名全局变量）。 */
export function extraJson(cfg: AppConfig): string {
  try {
    return JSON.stringify(cfg.extra)
  } catch {
    return '{}'
  }
}

export interface ProxySwitches {
  proxyServer?: string
  proxyBypassList?: string
}

/**
 * 外部代理开关（对齐 webview-app `config::build_proxy_args()` 的语义）。
 *
 * Tauri 版把两者拼成一条字符串喂给 WebView2 的 additional_browser_args；
 * Electron 侧改用相同名字的 Chromium 原生 switch，拆成两个独立值。
 * 规则保持一致：只填了 http 或 https 之一时，另一个也复用同一个代理。
 */
export function buildProxySwitches(cfg: AppConfig): ProxySwitches {
  if (!cfg.proxy_enabled) return {}
  let http = cfg.proxy_http.trim()
  let https = cfg.proxy_https.trim()
  if (http && !https) https = http
  if (https && !http) http = https
  if (!http && !https) return {}

  const servers: string[] = []
  if (http) servers.push(`http=${http}`)
  if (https) servers.push(`https=${https}`)

  const out: ProxySwitches = { proxyServer: servers.join(';') }
  const bypass = cfg.proxy_bypass.trim()
  if (bypass) out.proxyBypassList = bypass
  return out
}
