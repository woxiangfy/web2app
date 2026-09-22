/**
 * `config.json` 解析（与 webview-app 共用同一份文件契约）。
 *
 * 契约的**唯一真源**是 `.github/scripts/parse-packaging-config.py`（含内置自测），
 * 字段表与多端行为对照见 `docs/packaging-config.md`。本文件是它在运行期的对等实现
 * —— 两边的**校验规则逐条对齐**，任何一边放宽都会让「CI 通过、双击却起不来」或反过来。
 *
 * 与 Tauri 版的关键差异（有意为之，见 docs/electron-plan.md §4）：
 *   - Tauri 版：`config.json` 只是【打包期模板】，值被写成编译期常量（改配置必须重编译）；
 *   - Electron 绿色版：`config.json` 随包打进 **app.asar**，启动时读
 *     `app.getAppPath()/out/config.json` —— 同样必须重新打包才能改，但不必重编译。
 *
 * ⚠️ 约定（改动前先读这段，历史上每一条都踩过）：
 *   1. **布尔字段只接受真 bool**。曾经为了兼容上游模板的 `"false"` 字符串，这里写了一套
 *      宽容规则（`"1"` / `"yes"` / `"on"` 都算 true），而 Tauri 侧的规则不同 —— 于是同一份
 *      配置里写 `"on"`，两个壳的判定**相反**。现在字符串一律拒绝并给出改法。
 *   2. **未知字段直接失败**（`android_*` 除外，它们属于同一份契约、只是本壳不实现）。
 *      曾经只打一条 warning 就跳过，结果是「字段名拼错」与「字段不生效」表现完全一样。
 *   3. **`app_url` 接受 string 或 string[]**（真 JSON 类型），数组模式必须带 `public_key`。
 *      `"[\"a.com\"]"` 这种「JSON 数组文本」写法仍接受，但会告警提示改成真数组。
 *   4. 已移除的字段出现即报错，并给出改法：`window_title` → `title`；
 *      `data_dir_default`（布尔）→ `data_dir`（字符串，`false`⇒`"data"` / `true`⇒`""`）。
 *   5. **`data_dir` 的空串是有含义的取值**（= 系统默认程序目录），不是「未设置」。
 *      解析时必须原样保留空串，Rust 侧同理（那边不能用把空串当未设置的 `env_str!`）。
 */
import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'

/** Windows 壳可用的 13 个字段；`android_*` 4 个字段在本壳中忽略（见 ANDROID_ONLY_KEYS）。 */
export interface AppConfig {
  /**
   * 目标地址。两种形态（与打包配置 JSON 的类型一一对应）：
   *   - `string`   → 直连（可含 `user:pass@`，走本地代理时由代理注入 Basic 鉴权）；
   *   - `string[]` → 一组跳板域名，走 DNS TXT 解析 + Ed25519 验签 + AES-GCM 解密得到真实 URL。
   */
  app_url: string | string[]
  /** 【安装名】数据目录名 / `%APPDATA%` 子目录名（`data_dir` 为空串时） */
  app_name: string
  /** 【显示名】窗口标题、托盘悬停提示；缺省回落 app_name */
  title: string
  /** 注入给页面的全局对象 `window.web2appExtra` */
  extra: Record<string, unknown>
  /** 应用图标 PNG 的 URL；空 = 用打包时生成的内置图标（本壳在打包期消费，运行期只读不写） */
  icon_url: string
  /** true 且 proxy_http/proxy_https 为空 → 启用自身实现的本地反向代理 */
  proxy_enabled: boolean
  proxy_http: string
  proxy_https: string
  /** 不走代理的地址（分号分隔）。默认值与 Tauri / 解析器三处一致 */
  proxy_bypass: string
  /** 多页签开关：true 时走多页签模式（壳页面 + 子 WebContentsView） */
  tabs_enabled: boolean
  /** DNS TXT 解析模式下的 Ed25519 公钥（数组模式必填） */
  public_key: string
  /**
   * AES 对称加密 Secret。
   *
   * ⚠️ 它是**编译期**输入：值在打包时经环境变量 `RESOLVE_SECRET` 注入到
   * `w2a-proxy.exe`（编译 `resolve-core` 时读取），**运行期不再参与任何计算**
   * —— 与 Tauri 版语义完全相同（那边是编进主程序的常量）。
   * 所以改这个字段必须重新打包 sidecar，改完只重启是**不生效**的。
   */
  resolve_secret: string
  /**
   * 诊断日志开关（默认 false = 完全不输出日志）。
   *
   * `false`：不写 `data\logs\startup.log`，stdout/stderr 的诊断也一并静默
   *          （终端启动同样安静）——默认值必须是 false，否则每次启动都在用户的
   *          数据目录里留下交付物以外的文件，而那不是期望行为。
   * `true` ：落盘 `<data_dir>\logs\startup.log`，并把同一批诊断打到 stdout/stderr。
   *
   * ⚠️ 它是**开关**不是「日志级别」：关掉时没有任何第一现场，排障要么重新打包，
   *    要么用命令行临时打开（`--debug` / 环境变量 `WEB2APP_DEBUG=1`，无需重打包）。
   */
  debug: boolean
  /**
   * 数据目录。⚠️ **空串是有含义的取值**（= 系统默认程序目录），不是「未设置」。
   *   · `"data"`（默认）→ 程序所在目录（exe 目录）下的 `data\`，跟着程序走（真绿色）；
   *   · `""`           → `%APPDATA%\<app_name>`，不受程序位置影响；
   *   · 其它相对路径    → 同上，基准仍是 exe 目录（**不是** shell 的 cwd）；
   *   · 绝对路径        → 原样使用。
   * 实际解析与「不可写自动回退」在 portable.ts（与 webview-app config.rs 同规则）。
   */
  data_dir: string
}

export interface LoadedConfig {
  config: AppConfig
  /** 实际读取的文件路径；未找到时为 null（使用内置默认值） */
  path: string | null
  /** 读取过程中的非致命提示，用于启动日志 */
  warnings: string[]
}

/** 配置错误：消息必须**可操作**（直接告诉用户改成什么），因为它会出现在启动日志/控制台上。 */
export class ConfigError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'ConfigError'
  }
}

/** 仅 Android 使用的字段：同一份契约里存在，本壳忽略但**不报错**，只提示。 */
const ANDROID_ONLY_KEYS = [
  'android_keystore_base64',
  'android_key_alias',
  'android_keystore_password',
  'android_key_password',
] as const

/** 已移除的字段 → **可直接照做的改法说明**。命中的报错要说清具体怎么改，而不是只说「未知字段」。 */
const REMOVED_KEYS: Record<string, string> = {
  window_title: '该字段**已移除**，请改用 title（语义相同；移除原因是同一概念曾有三个名字）',
  data_dir_default:
    '该字段**已移除**（旧的是布尔，新的是字符串），请改用 data_dir：\n' +
    '       旧值 false ⇒ "data_dir": "data"  （默认：程序所在目录下的 data\\，跟着程序走）\n' +
    '       旧值 true  ⇒ "data_dir": ""      （空串 = 系统默认程序目录，不受程序位置影响）',
}

export const DEFAULTS: AppConfig = {
  app_url: 'https://www.example.com',
  app_name: 'Web2App',
  title: '',
  extra: {},
  icon_url: '',
  proxy_enabled: false,
  proxy_http: '',
  proxy_https: '',
  // ⚠️ 与 `webview-app/src/config.rs` 的 `PROXY_BYPASS` 默认值、以及解析器的 DEFAULTS
  //    必须**逐字相同**（三处同一个串）。留空 = 不下发 proxy-bypass-list。
  proxy_bypass: 'localhost;127.0.0.1;<-loopback>',
  tabs_enabled: false,
  public_key: '',
  resolve_secret: '',
  // 诊断日志开关：默认关（开着就会在用户数据目录里留下 startup.log，那不是期望行为）
  debug: false,
  // 数据目录：默认 "data" = exe 所在目录下的 data\，跟着程序走（真绿色）；
  // 空串 = %APPDATA%\<app_name>（系统默认程序目录，不受程序位置影响）。
  data_dir: 'data',
}

/** 布尔字段：只接受真 bool。字符串一律拒绝（见文件头第 1 条）。 */
function mustBool(v: unknown, key: string, dflt: boolean): boolean {
  if (v === undefined || v === null) return dflt
  if (typeof v === 'boolean') return v
  throw new ConfigError(
    `config.json 字段 ${key} 必须是布尔值（true / false，**不带引号**），` +
      `当前是 ${JSON.stringify(v)}\n` +
      `  正确："${key}": true\n` +
      `  错误："${key}": "true"   ← 字符串，已不再接受`,
  )
}

function toStr(v: unknown, dflt = ''): string {
  return typeof v === 'string' ? v : v == null ? dflt : String(v)
}

/**
 * `data_dir`：目录路径。⚠️ 空串是**有含义的取值**（= 系统默认程序目录），必须原样保留，
 * 绝不能被当成「未设置」而回落到默认值 —— 否则用户再也表达不出「用系统目录」这个意图。
 *
 * 非字符串一律拒绝：`"data_dir": true` 若经 `toStr` 宽容地转成 `"true"`，会静默建出一个名叫
 * `true` 的目录 —— 「看起来配置生效了、其实完全不是想要的」是最难查的一类。
 */
function toDataDir(v: unknown): string {
  if (v === undefined || v === null) return DEFAULTS.data_dir
  if (typeof v !== 'string') {
    throw new ConfigError(
      `config.json 字段 data_dir 必须是字符串（目录路径），当前是 ${typeof v}：${JSON.stringify(v)}\n` +
        '  正确："data_dir": "data"   ← 默认：程序所在目录下的 data\\\n' +
        '  正确："data_dir": ""       ← 空串 = 系统默认程序目录（由 app_name 命名）',
    )
  }
  return v.trim()
}

/** 清洗成 Windows 合法目录名（与 webview-app 的 `sanitize_dir_name` 同规则）。 */
export function sanitizeDirName(name: string): string {
  const cleaned = name.replace(/[<>:"/\\|?*\u0000-\u001f]/g, '_').trim()
  return cleaned.length > 0 ? cleaned : 'Web2App'
}

/** extra 必须是 JSON 对象；非法时回落 {}，避免注入非法 JSON 破坏页面脚本。 */
function toExtra(v: unknown, warnings: string[]): Record<string, unknown> {
  let val: unknown = v
  if (typeof val === 'string') {
    try {
      val = JSON.parse(val)
    } catch {
      warnings.push('config.json 的 extra 不是合法 JSON，已回退为 {}')
      return {}
    }
  }
  if (val === undefined || val === null) return {}
  if (typeof val === 'object' && !Array.isArray(val)) {
    return val as Record<string, unknown>
  }
  warnings.push('config.json 的 extra 不是对象，已回退为 {}')
  return {}
}

/**
 * `app_url`：接受 string 或 string[]，统一收敛成真正的 JSON 类型。
 * 数组模式（域名列表）必须提供 public_key —— 没有公钥就解不开 TXT 密文。
 */
function toAppUrl(v: unknown, publicKey: string, warnings: string[]): string | string[] {
  if (v === undefined || v === null) return DEFAULTS.app_url

  const asDomains = (list: unknown[]): string[] =>
    list.map((d) => String(d).trim()).filter((d) => d.length > 0)

  const checkDomains = (domains: string[]): string[] => {
    if (domains.length === 0) {
      throw new ConfigError('config.json 的 app_url 为数组时不能为空')
    }
    if (publicKey.trim().length === 0) {
      throw new ConfigError(
        'config.json 的 app_url 是域名数组时必须提供 public_key（Ed25519 公钥）——' +
          '域名模式下地址由 DNS TXT 密文解密得到，没有公钥无法验签',
      )
    }
    return domains
  }

  if (Array.isArray(v)) return checkDomains(asDomains(v))

  if (typeof v !== 'string') {
    throw new ConfigError(
      `config.json 的 app_url 必须是字符串或字符串数组，当前是 ${typeof v}`,
    )
  }
  const text = v.trim()
  if (text.length === 0) throw new ConfigError('config.json 缺少必填字段 app_url')

  // 兼容旧写法：字符串形态但内容是 JSON 数组（`"[\"a.com\"]"`）。
  if (text.startsWith('[')) {
    try {
      const parsed = JSON.parse(text) as unknown
      if (Array.isArray(parsed)) {
        warnings.push(
          'config.json 的 app_url 是「JSON 数组文本」写法，建议直接写真的 JSON 数组：' +
            'app_url: ["a.com","b.com"]（去掉外层引号）',
        )
        return checkDomains(asDomains(parsed))
      }
    } catch {
      /* 落到下面的报错 */
    }
    throw new ConfigError(
      'config.json 的 app_url 以 "[" 开头但不是合法的 JSON 字符串数组；' +
        '若要使用域名数组模式请写 ["a.com","b.com"]',
    )
  }
  return text
}

/** 未知字段 → 硬失败（并在可能时给出近似字段名）。 */
function rejectUnknownKeys(raw: Record<string, unknown>): void {
  const known = new Set(Object.keys(DEFAULTS))
  for (const key of Object.keys(raw)) {
    if (known.has(key)) continue
    if ((ANDROID_ONLY_KEYS as readonly string[]).includes(key)) continue
    // ⚠️ 用 hasOwnProperty 而不是 `key in`：后者会命中原型链，于是配置里一个叫
    //    `toString` / `constructor` 的键会被当成「已移除字段」并打印出 undefined。
    if (Object.prototype.hasOwnProperty.call(REMOVED_KEYS, key)) {
      throw new ConfigError(`config.json 字段 ${key}：${REMOVED_KEYS[key]}`)
    }
    const near = nearestKey(key, [...known])
    throw new ConfigError(
      `config.json 有无法识别的字段 ${key}${near ? `，是否想写 ${near}？` : ''}` +
        '（拼错即静默失效，故在此直接失败）',
    )
  }
}

/** 极简近似名查找（编辑距离 ≤ 2 或前缀包含），只为给出「是不是想写 X」的提示。 */
function nearestKey(key: string, candidates: string[]): string | null {
  let best: string | null = null
  let bestScore = Infinity
  for (const c of candidates) {
    const d = editDistance(key.toLowerCase(), c.toLowerCase())
    if (d < bestScore) {
      bestScore = d
      best = c
    }
  }
  return best !== null && bestScore <= 2 && bestScore < key.length ? best : null
}

function editDistance(a: string, b: string): number {
  const prev = Array.from({ length: b.length + 1 }, (_, i) => i)
  for (let i = 1; i <= a.length; i++) {
    let diag = prev[0]
    prev[0] = i
    for (let j = 1; j <= b.length; j++) {
      const tmp = prev[j]
      prev[j] = Math.min(prev[j] + 1, prev[j - 1] + 1, diag + (a[i - 1] === b[j - 1] ? 0 : 1))
      diag = tmp
    }
  }
  return prev[b.length]
}

/**
 * title 缺省回落 app_name（对齐 webview-app：TITLE 未填时用 APP_NAME）。
 * ⚠️ 必须对【所有】返回路径生效——只在「读到文件」的分支里做回落，
 * 会让「没有 config.json」和「config.json 里 title 为空」两种情况表现不一致。
 */
function withTitleFallback(c: AppConfig): AppConfig {
  return c.title.trim().length > 0 ? c : { ...c, title: c.app_name }
}

/**
 * 按顺序在多个目录里找 `config.json`，用第一个存在的。
 *
 * ⚠️ 打包后配置在 asar 内，正常情况下只会命中第一个目录。
 * 保留多目录能力是为了开发态/排查时能临时指到别处。
 *
 * ⚠️ 与 loadConfig 一致：文件存在但读不出来仍然抛错（致命），不静默回落 ——
 * 否则用户/开发者改坏一个字符，得到的是「配置像是没生效」，最难查。
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
      return { config: withTitleFallback({ ...DEFAULTS }), path: null, warnings }
    }
    // 配置存在但读不出来属于致命错误：静默回落会让用户以为配置生效了
    throw new ConfigError(`读取配置失败 ${path}：${err.message}`)
  }

  if (raw === null || typeof raw !== 'object' || Array.isArray(raw)) {
    throw new ConfigError(`config.json 必须是一个对象（{...}）：${path}`)
  }

  rejectUnknownKeys(raw)
  for (const key of ANDROID_ONLY_KEYS) {
    if (key in raw) warnings.push(`字段 ${key} 仅 Android 使用，本壳已忽略`)
  }

  const appName = toStr(raw.app_name, DEFAULTS.app_name)
  const publicKey = toStr(raw.public_key)
  const config = withTitleFallback({
    app_url: toAppUrl(raw.app_url, publicKey, warnings),
    app_name: appName,
    title: toStr(raw.title),
    extra: toExtra(raw.extra, warnings),
    icon_url: toStr(raw.icon_url),
    proxy_enabled: mustBool(raw.proxy_enabled, 'proxy_enabled', DEFAULTS.proxy_enabled),
    proxy_http: toStr(raw.proxy_http),
    proxy_https: toStr(raw.proxy_https),
    proxy_bypass: toStr(raw.proxy_bypass, DEFAULTS.proxy_bypass),
    tabs_enabled: mustBool(raw.tabs_enabled, 'tabs_enabled', DEFAULTS.tabs_enabled),
    public_key: publicKey,
    resolve_secret: toStr(raw.resolve_secret),
    debug: mustBool(raw.debug, 'debug', DEFAULTS.debug),
    data_dir: toDataDir(raw.data_dir),
  })

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
