/**
 * 账号密码保存的纯规则单测（`src/main/credentials-logic.ts`）。
 *
 * 这层必须脱离 Electron 覆盖：`normalizeOrigin` 的「回环地址丢端口」是**功能可用性**的关键
 * （走本地代理时端口每次启动都变），写错了表现是「重启后自动填充静默失效」，而探针只测一次
 * 运行期、发现不了。加密 / 文件读写在 `credentials.ts`（需要 Electron，由契约测试与探针兜）。
 */
import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  findCredential,
  MAX_ENTRIES,
  normalizeOrigin,
  parseCredentials,
  removeCredentials,
  serializeCredentials,
  upsertCredential,
  type CredentialsDoc,
} from '../src/main/credentials-logic.ts'

test('normalizeOrigin：普通站点保留完整 origin（含非默认端口）', () => {
  assert.equal(normalizeOrigin('https://example.com/login?x=1'), 'https://example.com')
  assert.equal(normalizeOrigin('http://example.com:8080/a'), 'http://example.com:8080')
  assert.equal(normalizeOrigin('HTTPS://Example.COM/'), 'https://example.com')
})

test('normalizeOrigin：回环地址丢弃端口（本地代理端口每次启动都可能变）', () => {
  assert.equal(normalizeOrigin('http://127.0.0.1:54321/auth.html?w2a_key=x'), 'http://127.0.0.1')
  assert.equal(normalizeOrigin('http://localhost:1234/'), 'http://localhost')
  assert.equal(normalizeOrigin('http://[::1]:9000/'), 'http://[::1]')
})

test('normalizeOrigin：非 http(s) / 非法地址一律拒绝', () => {
  for (const bad of [
    'splash://localhost/',
    'settings://localhost/',
    'shell://localhost/',
    'floaticon://localhost/',
    'about:blank',
    'data:text/html,hi',
    'file:///C:/x.html',
    'not a url',
    '',
  ]) {
    assert.equal(normalizeOrigin(bad), null, `不应接受 ${bad}`)
  }
})

test('parseCredentials：宽容解析，脏条目丢弃而不是抛错', () => {
  const doc = parseCredentials({
    version: 1,
    entries: [
      { origin: 'https://a.com', username: 'u', secret: 'c2VjcmV0', updatedAt: 1 },
      { origin: 'not-a-url', username: 'u', secret: 'x', updatedAt: 1 },
      { origin: 'ftp://a.com', username: 'u', secret: 'x', updatedAt: 1 },
      { origin: 'https://b.com', username: 'u', secret: '' },
      { origin: 'https://c.com', secret: 'x', updatedAt: 1 },
      'garbage',
      null,
    ],
  })
  assert.deepEqual(
    doc.entries.map((e) => e.origin),
    ['https://a.com']
  )
  assert.deepEqual(parseCredentials('x'), { version: 1, entries: [] })
  assert.deepEqual(parseCredentials(null), { version: 1, entries: [] })
})

test('upsertCredential：同 origin + 同用户名覆盖，不同用户名保留', () => {
  let doc: CredentialsDoc = { version: 1, entries: [] }
  doc = upsertCredential(doc, { origin: 'https://a.com', username: 'alice', secret: 's1', updatedAt: 10 })
  doc = upsertCredential(doc, { origin: 'https://a.com', username: 'alice', secret: 's2', updatedAt: 20 })
  doc = upsertCredential(doc, { origin: 'https://a.com', username: 'bob', secret: 's3', updatedAt: 30 })
  assert.equal(doc.entries.length, 2)
  assert.equal(findCredential(doc, 'https://a.com')?.secret, 's3', '应取最后保存的那条')
  assert.equal(doc.entries.find((e) => e.username === 'alice')?.secret, 's2')
})

test('upsertCredential：超过上限时丢弃最旧的', () => {
  let doc: CredentialsDoc = { version: 1, entries: [] }
  for (let i = 0; i < MAX_ENTRIES + 6; i++) {
    doc = upsertCredential(doc, {
      origin: `https://s${i}.com`,
      username: 'u',
      secret: String(i),
      updatedAt: i,
    })
  }
  assert.equal(doc.entries.length, MAX_ENTRIES)
  assert.equal(doc.entries[0].origin, 'https://s6.com', '最旧的 6 条应被丢弃')
})

test('findCredential / removeCredentials：按 origin 精确匹配', () => {
  const doc = upsertCredential(
    upsertCredential({ version: 1, entries: [] }, {
      origin: 'https://a.com',
      username: 'u',
      secret: 's',
      updatedAt: 1,
    }),
    { origin: 'https://b.com', username: 'u', secret: 's', updatedAt: 2 }
  )
  assert.equal(findCredential(doc, 'https://c.com'), null)
  assert.equal(findCredential(doc, 'https://b.com')?.origin, 'https://b.com')
  const after = removeCredentials(doc, 'https://b.com')
  assert.deepEqual(after.entries.map((e) => e.origin), ['https://a.com'])
})

test('serializeCredentials：只输出密文，不出现明文密码字段', () => {
  const doc = upsertCredential({ version: 1, entries: [] }, {
    origin: 'https://a.com',
    username: 'alice',
    secret: 'ZW5jcnlwdGVk',
    updatedAt: 1,
  })
  const text = serializeCredentials(doc)
  assert.ok(text.includes('"secret": "ZW5jcnlwdGVk"'))
  assert.ok(!/password/i.test(text), '文件里不得出现 password 字段')
})
