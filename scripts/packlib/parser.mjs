/**
 * 配置读取、校验与规范化 —— .github/scripts/parse-packaging-config.py 的 Node 复刻。
 * 改任何解析语义都必须同步改 Python 侧，并跑 `node scripts/pack-local.mjs --parity`。
 */
import { ConfigError, emit } from './diagnostic.mjs'
import { getCloseMatches, pyRepr, pyStr, pyTypeName } from './pycompat.mjs'
import { BOOL_KEYS, DATA_DIR_BAD, DEFAULTS, JSON_KEYS, REMOVED_KEYS, SIGNING_KEYS } from './constants.mjs'

// ===========================================================================
// 读取
// ===========================================================================
export async function readRaw(jsonText, url) {
  let raw = (jsonText || '').trim()
  const u = (url || '').trim()
  if (!raw && u) {
    let res
    try {
      res = await fetch(u, { headers: { 'User-Agent': 'web2app-ci' }, signal: AbortSignal.timeout(30000) })
    } catch (e) {
      throw new ConfigError(`拉取配置 URL 失败：${e.message}`)
    }
    // ⚠️ 与 Python 版有意不同：`urllib.request.urlopen` 对非 2xx 会抛 HTTPError（栈很丑），
    //    这里直接给出可读文案。
    if (!res.ok) throw new ConfigError(`拉取配置 URL 失败：HTTP ${res.status} ${res.statusText}`)
    raw = (await res.text()).trim()
  }
  if (!raw) throw new ConfigError('必须提供 config_json 或 config_url，二者至少填一个')
  return raw
}

export function parseJson(raw) {
  let data
  try {
    data = JSON.parse(raw)
  } catch (e) {
    throw new ConfigError(`配置不是合法 JSON：${e.message}`)
  }
  if (data === null || typeof data !== 'object' || Array.isArray(data)) {
    throw new ConfigError(`配置 JSON 必须是一个对象（{...}），当前是 ${pyTypeName(data)}`)
  }
  return data
}

// ===========================================================================
// 校验与规范化
// ===========================================================================

/**
 * 未知字段**硬失败**。
 *
 * 曾经只是 warning 后跳过 —— 后果是「字段名拼错」与「字段值不生效」看起来完全一样（都静默），
 * 用户只能靠猜。硬失败 + 近似名建议是唯一能让这类手误在打包时就现形的做法。
 */
function rejectUnknownKeys(data) {
  const unknown = Object.keys(data).filter((k) => !(k in DEFAULTS))
  if (unknown.length === 0) return
  const lines = []
  for (const k of [...unknown].sort()) {
    if (k in REMOVED_KEYS) {
      lines.push(`  ${k}：${REMOVED_KEYS[k]}`)
      continue
    }
    const near = getCloseMatches(k, Object.keys(DEFAULTS), 1, 0.5)
    const hint = near.length ? `，是否想写 ${near[0]}？` : ''
    lines.push(`  ${k}：未知字段${hint}`)
  }
  throw new ConfigError('配置里有无法识别的字段（拼错即静默失效，故在此硬失败）：\n' + lines.join('\n'))
}

/**
 * 布尔字段只接受真 bool。
 *
 * 旧模板里写的是 `"proxy_enabled": "false"`（字符串）。字符串 `"false"` 是**真值**，
 * 两壳各自用不同的宽容规则去猜它，结果 `"on"` 在 Tauri 是 false、在 Electron 是 true
 * —— 同一份配置两种行为。所以直接拒绝字符串，把歧义消灭在打包期。
 */
function normalizeBool(key, value) {
  if (typeof value === 'boolean') return value
  throw new ConfigError(
    `字段 ${key} 必须是布尔值（true / false，**不带引号**），当前是 ` +
      `${pyTypeName(value)}：${pyRepr(value)}\n` +
      `  正确写法："${key}": true\n` +
      `  错误写法："${key}": "true"   ← 字符串，已不再接受`,
  )
}

/**
 * app_url 支持两种形态，统一出口是**一段可以直接进环境变量的文本**：
 *   · string  → 原样（单个 URL，可含 `user:pass@`）
 *   · array   → JSON 数组文本（一组跳板域名，运行时做 DNS TXT + 验签解密）
 */
function normalizeAppUrl(value, publicKey) {
  if (Array.isArray(value)) {
    const domains = value.map((d) => pyStr(d).trim()).filter((d) => d.length > 0)
    if (domains.length === 0) throw new ConfigError('app_url 为数组时不能为空')
    if (!publicKey.trim()) {
      throw new ConfigError(
        'app_url 为域名数组时必须提供 public_key（Ed25519 公钥）——' +
          '域名模式下地址由 DNS TXT 密文解密得到，没有公钥无法验签',
      )
    }
    return JSON.stringify(domains)
  }

  if (typeof value === 'boolean' || !(typeof value === 'string' || typeof value === 'number')) {
    throw new ConfigError(`app_url 必须是字符串或字符串数组，当前是 ${pyTypeName(value)}`)
  }

  const text = pyStr(value).trim()
  if (!text) throw new ConfigError('缺少必填字段 app_url')

  // 兼容：字符串形态但内容是 JSON 数组文本（`"[\"a.com\"]"`）。这曾是唯一可行的数组写法，
  // 老配置里可能有；仍按数组处理，但提示改用真数组。
  if (text.startsWith('[')) {
    let parsed = null
    try {
      parsed = JSON.parse(text)
    } catch {
      parsed = null
    }
    if (Array.isArray(parsed)) {
      const domains = parsed.map((d) => pyStr(d).trim()).filter((d) => d.length > 0)
      if (domains.length === 0) throw new ConfigError('app_url 的数组内容为空')
      if (!publicKey.trim()) throw new ConfigError('app_url 为域名数组时必须提供 public_key（Ed25519 公钥）')
      emit(
        '::warning::app_url 是「JSON 数组文本」写法，建议直接写真的 JSON 数组：' +
          'app_url: ["a.com","b.com"]（去掉外层引号）',
      )
      return JSON.stringify(domains)
    }
    throw new ConfigError(
      "app_url 以 '[' 开头但不是合法的 JSON 字符串数组；" + '若要用域名数组模式请写 ["a.com","b.com"]',
    )
  }

  if (!/^https?:\/\//i.test(text)) {
    // 不硬失败：本地夹具 / 特殊环境可能用别的 scheme，最终由运行时 Url::parse 判定。
    // 但这条几乎总是手误（漏了 scheme），值得在打包日志里现形。
    emit(
      `::warning::app_url 未以 http:// 或 https:// 开头（${mask(text)}），` +
        '运行时若无法解析会提示「应用配置的网址不合法」',
    )
  }
  return text
}

/** 把 JSON 里手写的字面量 `\n` 还原成真换行，并压掉首尾空白（PEM 多行 / 单行都接受）。 */
function normalizePublicKey(value) {
  const text = value === null || value === undefined ? '' : pyStr(value)
  if (!text) return ''
  return text.replace(/\\n/g, '\n').replace(/\\r/g, '').trim()
}

/** extra 统一序列化为单行 JSON 文本（默认 {} 也要转成 "{}"，供 Rust 侧原样解析）。 */
function normalizeExtra(value) {
  try {
    return JSON.stringify(value)
  } catch (e) {
    throw new ConfigError(`extra 无法序列化为 JSON：${e.message}`)
  }
}

/**
 * 打包产物版本：只接受 "主.次.补丁" 三段纯数字字符串。
 *
 * 为什么比 semver 更严：两条打包链路的 Windows 产物版本最终都要过 Windows 自己的规则 ——
 *   · Tauri 的 MSI（WiX ProductVersion）：主、次 ∈ [0,255]，补丁 ∈ [0,65535]，
 *     只认三段数字（"1.2.3-beta" / "v1.2.3" / 四段都会在打包期报错）；
 *   · electron-builder 同样把版本归一进 exe 的 VERSIONINFO（16 位字段）。
 * 与 Python 侧 normalize_version 逐字对应（改规则必须两侧同步 + 跑 --parity）。
 */
const VERSION_RE = /^(\d+)\.(\d+)\.(\d+)$/
function normalizeVersion(value) {
  if (typeof value !== 'string') {
    throw new ConfigError(
      `字段 version 必须是字符串（三段数字版本号），当前是 ${pyTypeName(value)}：${pyRepr(value)}\n` +
        '  正确写法："version": "1.2.3"',
    )
  }
  const text = value.trim()
  const m = VERSION_RE.exec(text)
  if (!m) {
    throw new ConfigError(
      '字段 version 必须是三段数字版本号 "主.次.补丁"（如 "1.2.3"），' +
        '不带 v 前缀、不加第四段、不写预发布标签（-beta 等），' +
        `当前是 ${pyRepr(value)}`,
    )
  }
  const major = Number(m[1])
  const minor = Number(m[2])
  const patch = Number(m[3])
  if (major > 255 || minor > 255 || patch > 65535) {
    throw new ConfigError(
      '字段 version 超出 Windows 版本范围（主、次 ≤ 255，补丁 ≤ 65535）：' +
        `${text}（解析为主=${major} 次=${minor} 补丁=${patch}）`,
    )
  }
  return text
}

/**
 * 数据目录。**空串在这里是有含义的取值**（= 系统默认程序目录），不是「未设置」。
 *
 * 三种形态（两壳行为一致，见 docs/packaging-config.md §4.3）：
 *   · `""`      → 系统默认程序目录（`%LOCALAPPDATA%\<app_name>`），不受程序位置影响；
 *   · 相对路径   → 程序**自身所在目录**（exe 目录）下的该路径，跟着程序走；
 *   · 绝对路径   → 原样使用。
 *
 * 非字符串一律拒绝：`"data_dir": true` 若被宽容地 str() 成 `"True"`，会静默建出一个名叫
 * `True` 的目录 ——「看起来配置生效了、其实完全不是想要的」是最难查的一类。
 */
function normalizeDataDir(value) {
  if (value === null || value === undefined) return String(DEFAULTS.data_dir)
  if (typeof value !== 'string') {
    throw new ConfigError(
      `字段 data_dir 必须是字符串（目录路径），当前是 ${pyTypeName(value)}：${pyRepr(value)}\n` +
        '  正确写法："data_dir": "data"   ← 默认：程序所在目录下的 data\\\n' +
        '  正确写法："data_dir": ""       ← 空串 = 系统默认程序目录（由 app_name 命名）',
    )
  }
  const text = value.trim()
  if (text.includes('\u0000')) throw new ConfigError('data_dir 不能包含 NUL 字符')
  const bad = [...new Set(text.split('').filter((c) => DATA_DIR_BAD.has(c)))].sort()
  if (bad.length) {
    emit(
      `::warning::data_dir 含 Windows 目录名非法字符 ${bad.join('')}（${text}）` +
        '—— Windows 上创建该目录会失败，届时会自动回退到系统默认目录',
    )
  }
  return text
}

/** 打印时隐去 URL 里的 `user:pass@`。 */
function mask(u) {
  return pyStr(u).replace(/(:\/\/[^\/@\s]+)@/g, '://***@')
}

/** 把用户配置合并进默认值，输出「已规范化的字符串表」。 */
export function buildMerged(data) {
  rejectUnknownKeys(data)

  const merged = { ...DEFAULTS }
  // app_url / extra / data_dir 单独取：
  //   · app_url 是唯一允许「字符串或数组」两种 JSON 类型的字段；
  //   · extra 要保持 JSON 结构（先 str() 会丢形态）；
  //   · data_dir 必须按**原始类型**校验 —— 若先 str() 掉，`true` 会变成字符串 "True" 而被放行。
  let rawAppUrl = DEFAULTS.app_url
  let rawExtra = DEFAULTS.extra
  let rawDataDir = DEFAULTS.data_dir

  for (const [key, value] of Object.entries(data)) {
    if (value === null) continue // 显式 null = 用默认值
    if (key === 'app_url') {
      rawAppUrl = value
      continue
    }
    if (JSON_KEYS.has(key)) {
      rawExtra = value
      continue
    }
    if (key === 'data_dir') {
      rawDataDir = value
      continue
    }
    if (BOOL_KEYS.has(key)) merged[key] = normalizeBool(key, value)
    else merged[key] = typeof value === 'boolean' ? value : pyStr(value)
  }

  // ⚠️ 顺序有意义：下面的 warning 输出顺序与 Python 版逐字对应（app_url → data_dir → icon_url
  //    → resolve_secret），对等性比对会检查这个序列。
  // 先归一 public_key：normalizeAppUrl 要用它判断「数组模式是否带公钥」。
  merged.public_key = normalizePublicKey(merged.public_key)
  merged.app_url = normalizeAppUrl(rawAppUrl, pyStr(merged.public_key))
  merged.extra = normalizeExtra(rawExtra)
  merged.data_dir = normalizeDataDir(rawDataDir)
  merged.icon_url = pyStr(merged.icon_url).trim()
  merged.resolve_secret = pyStr(merged.resolve_secret).trim()

  if (merged.icon_url && !/^https?:\/\//i.test(pyStr(merged.icon_url))) {
    emit(
      `::warning::icon_url 不是 http(s) 地址（${merged.icon_url}），` +
        '打包时会下载失败并回落到仓库默认图标',
    )
  }

  for (const key of BOOL_KEYS) merged[key] = normalizeBool(key, merged[key])

  // title（显示名）与 app_name（安装名）分离；显式空值 → 回落 app_name。
  // 取值链只有一跳：title → app_name。（旧字段 window_title 已移除，见 REMOVED_KEYS）
  let title = pyStr(merged.title).trim()
  if (!title) title = pyStr(merged.app_name)
  merged.title = title

  // version 单独按版本规则校验（三段数字 + Windows 取值范围），不能走通用 str() 通道。
  merged.version = normalizeVersion(merged.version)

  // ⚠️⚠️ 告警的**触发条件**必须对齐「运行期到底哪条路会解密」：
  //    · Tauri：`bootstrap.rs` 里 `parse_domains(APP_URL)` 命中（文本以 `[` 开头）才走 DNS TXT → 解密；
  //    · Electron：`navigate.ts` 的 `needsDnsResolve` = `Array.isArray(app_url)`。
  //    `SECRET` 的唯一消费点是 `resolve-core::derive_key`（只被 `decrypt_txt` 用），所以单 URL
  //    字符串模式下这个字段在运行期**没有任何调用点** —— 此时告警是狼来了。
  if (!merged.resolve_secret) {
    if (pyStr(merged.app_url).trim().startsWith('[')) {
      emit(
        '::warning::resolve_secret 未配置：app_url 是域名数组，运行期要靠它解密 TXT 密文；' +
          '将使用代码内置默认 Secret（所有产物共享同一固定密钥串，安全门槛较低）。' +
          '建议配置独立的 resolve_secret，并同步服务端 encrypt_url.py / resolve-tool。',
      )
    } else {
      // 单 URL 模式：明确说明「当前不参与」，比默认告警更有用，且不产生 CI 注解。
      emit(
        'resolve_secret 未配置：app_url 是单 URL 字符串，运行期不解密 DNS TXT ' +
          '⇒ 该字段当前不参与，无需配置（将来改用域名数组模式时才需要）。',
      )
    }
  }

  // 全部收敛为字符串，供 env / 打印使用。
  const out = {}
  for (const [k, v] of Object.entries(merged)) out[k] = typeof v === 'string' ? v : pyStr(v)
  return out
}

/**
 * 把 merged 表转回**保持 JSON 类型**的对象（Electron 进 asar 的那一份 config.json）。
 *
 * ⚠️ 与 env 不同：这里 bool 就是 bool、app_url 数组就是数组。
 * Electron 的 config.ts 按真实类型解析，不做字符串宽容。
 */
export function toConfigJson(merged) {
  const out = {}
  for (const [key, value] of Object.entries(merged)) {
    if (JSON_KEYS.has(key)) out[key] = JSON.parse(value)
    else if (BOOL_KEYS.has(key)) {
      // merged 里是已规范化的字符串（"True"/"False"），这里转回真 bool。
      // ⚠️ 不能写 Boolean(value)：非空字符串恒为 true，会把 "False" 变成 true。
      out[key] = value.trim().toLowerCase() === 'true'
    } else if (key === 'app_url') {
      out[key] = value.trimStart().startsWith('[') ? JSON.parse(value) : value
    } else out[key] = value
  }
  // android_* 仅 Android 使用，不进 Windows 绿色版的 config.json ——
  // 那 4 个字段里含 keystore base64 与明文密码，没有任何理由随包分发。
  for (const key of SIGNING_KEYS) delete out[key]
  return out
}

/** 打卡日志时的字段展示（数值与敏感项做脱敏 / 摘要）。 */
export function display(key, value) {
  if (key === 'app_url' || key === 'icon_url') return mask(value)
  if (key === 'android_keystore_password' || key === 'android_key_password' || key === 'resolve_secret') {
    return value ? '***（已配置）' : '（未配置）'
  }
  if (key === 'android_keystore_base64') return value ? `（已配置，${value.length} 字符）` : '（未配置）'
  return value
}
