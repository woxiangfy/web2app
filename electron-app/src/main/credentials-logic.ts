/**
 * 账号密码保存的**纯规则**（零 Electron 依赖，`pnpm run unit` 可直接覆盖）。
 *
 * 背景：Electron 只嵌入 Chromium 的 content 层，**不含** Chrome 浏览器层的密码管理器
 * （`chrome/browser/password_manager`）与自动填充 UI —— 既没有「保存密码？」气泡，也没有
 * `chrome://settings/passwords`。Tauri 壳走 WebView2 的
 * `ICoreWebView2Settings4::SetIsPasswordAutosaveEnabled(true)`（见
 * `webview-app/src/webview_hooks.rs`），Electron 侧只能自研：preload 里采集 / 填充，
 * 主进程用 `safeStorage`（Windows = DPAPI）加密落盘。本文件只管**文档形状与匹配规则**，
 * 加密与读写见 `credentials.ts`。
 *
 * 存储位置：`<data_dir>/credentials.json`（与 settings.json 同目录，跟着绿色版走）。
 *
 * ```json
 * {
 *   "version": 1,
 *   "entries": [
 *     { "origin": "https://example.com", "username": "alice", "secret": "<base64>", "updatedAt": 1700000000000 }
 *   ]
 * }
 * ```
 *
 * ⚠️ `secret` 只存 `safeStorage.encryptString()` 的 base64，**绝不落明文**；本模块的任何
 *   日志 / 错误信息也不得包含用户名以外的凭据内容（用户名可展示，密码不行）。
 */
/** 凭据文件名（与 settings.json 同目录；Tauri 侧不用它）。 */
export const CREDENTIALS_FILE = 'credentials.json'

/** 单条凭据。 */
export interface CredentialEntry {
  /** 归一化后的站点标识（见 [`normalizeOrigin`]） */
  origin: string
  /** 用户名（可为空串：只要密码就足够自动填充） */
  username: string
  /** `safeStorage.encryptString(password)` 的 base64 */
  secret: string
  /** 最后写入时间（毫秒），同一 origin 多条时取最新 */
  updatedAt: number
}

export interface CredentialsDoc {
  version: 1
  entries: CredentialEntry[]
}

/** 条目上限：防止配置文件无限增长（正常使用远到不了）。 */
export const MAX_ENTRIES = 64

/**
 * 站点标识归一化。
 *
 * - 只接受 `http:` / `https:`：内置页（splash / settings / shell / floaticon）与
 *   `about:` / `data:` 一律返回 null（绝不为它们保存凭据）。
 * - **回环地址丢弃端口**：走本地反向代理时入口是 `http://127.0.0.1:<随机端口>/…`，
 *   端口每次启动都可能变；不丢端口会导致「重启后自动填充失效」。回环上的不同站点由
 *   不同的应用数据目录隔离，不存在串用风险。
 * - 其余站点保留完整 origin（含非默认端口），与浏览器同源策略同一口径。
 */
export function normalizeOrigin(raw: string): string | null {
  let u: URL
  try {
    u = new URL(raw)
  } catch {
    return null
  }
  if (u.protocol !== 'http:' && u.protocol !== 'https:') return null
  const host = u.hostname.toLowerCase()
  if (host === '') return null
  if (host === '127.0.0.1' || host === 'localhost' || host === '::1' || host === '[::1]') {
    return `${u.protocol}//${host}`
  }
  return u.origin
}

/** 宽容解析：文件缺失 / 坏 JSON / 脏条目一律丢弃，绝不抛错。 */
export function parseCredentials(v: unknown): CredentialsDoc {
  const empty: CredentialsDoc = { version: 1, entries: [] }
  if (!v || typeof v !== 'object') return empty
  const raw = (v as { entries?: unknown }).entries
  if (!Array.isArray(raw)) return empty
  const entries: CredentialEntry[] = []
  for (const item of raw) {
    if (!item || typeof item !== 'object') continue
    const o = item as Record<string, unknown>
    if (typeof o.origin !== 'string' || typeof o.secret !== 'string') continue
    const origin = normalizeOrigin(o.origin)
    // 存进来时已归一；这里再验一次，挡住手改文件塞进无效 origin 的情况。
    if (!origin || origin !== o.origin || o.secret === '') continue
    if (typeof o.username !== 'string') continue
    const updatedAt = typeof o.updatedAt === 'number' && Number.isFinite(o.updatedAt) ? o.updatedAt : 0
    entries.push({ origin, username: o.username, secret: o.secret, updatedAt })
  }
  return { version: 1, entries: entries.slice(-MAX_ENTRIES) }
}

/**
 * 写入一条凭据（同 origin + 同用户名覆盖，其它用户的记录保留）。
 *
 * 返回新文档（不修改入参）。超出 [`MAX_ENTRIES`] 时丢弃最旧的。
 */
export function upsertCredential(
  doc: CredentialsDoc,
  entry: { origin: string; username: string; secret: string; updatedAt: number }
): CredentialsDoc {
  const entries = doc.entries.filter(
    (e) => !(e.origin === entry.origin && e.username === entry.username)
  )
  entries.push({ ...entry })
  entries.sort((a, b) => a.updatedAt - b.updatedAt)
  return { version: 1, entries: entries.slice(-MAX_ENTRIES) }
}

/** 取该 origin 最新的一条（多用户名时取最后保存的那个 —— 首版不做账号选择）。 */
export function findCredential(doc: CredentialsDoc, origin: string): CredentialEntry | null {
  let hit: CredentialEntry | null = null
  for (const e of doc.entries) {
    if (e.origin !== origin) continue
    if (!hit || e.updatedAt >= hit.updatedAt) hit = e
  }
  return hit
}

/** 删除该 origin 的全部记录（气泡上的「撤销」与清除浏览数据共用）。 */
export function removeCredentials(doc: CredentialsDoc, origin: string): CredentialsDoc {
  return { version: 1, entries: doc.entries.filter((e) => e.origin !== origin) }
}

/** 规范序列化（2 空格缩进，便于人工检查；文件里没有明文密码）。 */
export function serializeCredentials(doc: CredentialsDoc): string {
  return JSON.stringify({ version: 1, entries: doc.entries }, null, 2)
}
