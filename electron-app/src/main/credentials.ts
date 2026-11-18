/**
 * 账号密码存储（Electron 自研）—— 纯规则在 `credentials-logic.ts`，这里只做「加密 + 落盘 + 查找」。
 *
 * 为什么必须自研：Electron 不含 Chrome 的密码管理器 / 自动填充（见 `credentials-logic.ts`
 * 文件头与 `docs/changes/2026-10-09.md` 里那条被纠正的结论）。Tauri 壳用 WebView2 原生
 * 容量，本壳用 `safeStorage`：
 *
 *   · Windows ⇒ DPAPI（绑定当前用户账户，换机 / 换用户解密会失败，属预期）；
 *   · `safeStorage.isAsyncEncryptionAvailable() === false` 时**拒绝保存**（绝不退化成明文）；
 *   · 解密失败（文件被复制到另一台机器、DPAPI 记录损坏）只记日志并当作「没有保存过」，
 *     不删除用户文件 —— 换回原机器还能读。
 *
 * ⚠️ 安全纪律：本模块的任何日志 / 错误文本**不得**包含密码；用户名可展示（气泡里要显示）。
 *    IPC 侧只回传解密后的明文给**当前 origin 的内容页**，origin 由主进程从
 *    `event.senderFrame.url` 推导，渲染进程没有指定 origin 的能力（见 `app-ipc.ts`）。
 */
import { safeStorage } from 'electron'
import { mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs'
import { dirname } from 'node:path'

import {
  findCredential,
  normalizeOrigin,
  parseCredentials,
  removeCredentials,
  serializeCredentials,
  upsertCredential,
  type CredentialsDoc,
} from './credentials-logic'

/** 保存结果（IPC 原样回给 preload，preload 只在 ok=true 时弹「已保存」气泡）。 */
export interface SaveResult {
  ok: boolean
  /** 失败原因码（仅供 UI / 日志分流，不含任何凭据内容） */
  reason?: 'invalid' | 'no-crypto' | 'write-failed'
}

export interface CredentialStore {
  /** 按 `pageUrl` 的 origin 取最新一条；没有 / 解不开 / 非 http(s) 时返回 null。 */
  lookup(pageUrl: string): Promise<{ username: string; password: string } | null>
  /** 按 `pageUrl` 的 origin 保存（同用户名覆盖）。 */
  save(pageUrl: string, payload: { username: string; password: string }): Promise<SaveResult>
  /** 删除该 origin 的全部记录（气泡「删除」）。 */
  forget(pageUrl: string): Promise<void>
  /** 删除整个文件（设置页「清除全部浏览数据」）。 */
  clear(): void
}

export function createCredentialStore(file: string): CredentialStore {
  const read = (): CredentialsDoc => {
    try {
      return parseCredentials(JSON.parse(readFileSync(file, 'utf8')))
    } catch {
      // 文件不存在 / 坏 JSON：都当作空文档（与 settings.ts 同一宽容口径）。
      return parseCredentials(null)
    }
  }

  const write = (doc: CredentialsDoc): void => {
    mkdirSync(dirname(file), { recursive: true })
    const tmp = `${file}.tmp-${process.pid}`
    try {
      writeFileSync(tmp, serializeCredentials(doc), { encoding: 'utf8', flush: true })
      renameSync(tmp, file)
    } catch (e) {
      rmSync(tmp, { force: true })
      throw e
    }
  }

  return {
    async lookup(pageUrl) {
      const origin = normalizeOrigin(pageUrl)
      if (!origin) return null
      const doc = read()
      const entry = findCredential(doc, origin)
      if (!entry) return null
      // Electron 45 起同步版 safeStorage 已废弃（日志会刷 deprecated 警告），统一走异步版。
      if (!(await safeStorage.isAsyncEncryptionAvailable())) return null
      try {
        const dec = await safeStorage.decryptStringAsync(Buffer.from(entry.secret, 'base64'))
        if (dec.shouldReEncrypt) {
          // 系统密钥轮换：顺手把新密文写回，失败不影响本次填充。
          // ⚠️ 必须**重新读盘**再写：解密是 await，期间别的页签可能已经保存了新的凭据；
          //    拿开头那份 doc 去 upsert 会把它覆盖掉（还会把已删除的条目「复活」）。
          //    只在「该条目仍在、密文仍是本次解出来的那份」时才回写。
          try {
            const secret = (await safeStorage.encryptStringAsync(dec.result)).toString('base64')
            const fresh = read()
            const cur = fresh.entries.find(
              (e) => e.origin === entry.origin && e.username === entry.username
            )
            if (cur && cur.secret === entry.secret) {
              write(upsertCredential(fresh, { ...cur, secret }))
            }
          } catch {
            console.warn('[web2app] 账号密码重加密失败（本次仍使用已解出的值）')
          }
        }
        return { username: entry.username, password: dec.result }
      } catch {
        // ⚠️ 不删除文件：换回原机器 / 原账户仍可解密；只提示「读不出」。
        console.warn('[web2app] 已保存的账号密码无法解密（数据目录可能来自另一台设备 / 另一个账户）')
        return null
      }
    },

    async save(pageUrl, payload) {
      const origin = normalizeOrigin(pageUrl)
      if (!origin) return { ok: false, reason: 'invalid' }
      const username = payload.username
      const password = payload.password
      if (typeof username !== 'string' || typeof password !== 'string' || password === '') {
        return { ok: false, reason: 'invalid' }
      }
      if (!(await safeStorage.isAsyncEncryptionAvailable())) {
        console.warn('[web2app] 系统加密不可用（safeStorage），拒绝保存账号密码')
        return { ok: false, reason: 'no-crypto' }
      }
      try {
        const secret = (await safeStorage.encryptStringAsync(password)).toString('base64')
        write(upsertCredential(read(), { origin, username, secret, updatedAt: Date.now() }))
        return { ok: true }
      } catch (e) {
        console.error(`[web2app] 保存账号密码失败：${(e as Error).message}`)
        return { ok: false, reason: 'write-failed' }
      }
    },

    async forget(pageUrl) {
      const origin = normalizeOrigin(pageUrl)
      if (!origin) return
      const doc = read()
      if (!findCredential(doc, origin)) return
      try {
        write(removeCredentials(doc, origin))
      } catch (e) {
        console.error(`[web2app] 删除保存的账号密码失败：${(e as Error).message}`)
      }
    },

    clear() {
      rmSync(file, { force: true })
    },
  }
}
