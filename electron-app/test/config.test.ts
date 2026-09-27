/**
 * config.ts 纯函数单测（node:test，零 Electron 依赖，直接跑 TS 源码）。
 *
 * 为什么单独有这份：这些函数以前只能靠 GUI 探针（probe）间接覆盖 —— 探针断的是
 * 「应用整体行为」，而这里钉的是「错了也静默」的配置收敛规则（字符串布尔、空串
 * data_dir、未知字段硬失败、数组模式公钥），这类回归最值得在毫秒级单测里拦住。
 *
 * 运行：`pnpm run unit`（Node ≥ 22.6 内置类型擦除；本机 Node 24 直接执行 .ts）。
 */
import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, test } from 'node:test'
import {
  buildProxySwitches,
  ConfigError,
  DEFAULTS,
  debugFlagTruthy,
  editDistance,
  extraJson,
  loadConfig,
  mustBool,
  nearestKey,
  normalizePublicKey,
  rejectUnknownKeys,
  sanitizeDirName,
  toAppUrl,
  toDataDir,
} from '../src/main/config.ts'

// ---------- mustBool：只认真 bool（历史上字符串宽容规则让两个壳判定相反） ----------

test('mustBool：缺省回落、真 bool 原样、其它一律抛 ConfigError', () => {
  assert.equal(mustBool(undefined, 'debug', true), true)
  assert.equal(mustBool(null, 'debug', false), false)
  assert.equal(mustBool(true, 'debug', false), true)
  assert.equal(mustBool(false, 'debug', true), false)
  for (const bad of ['true', 'false', '1', 1, 0, 'yes']) {
    assert.throws(() => mustBool(bad, 'debug', false), ConfigError)
  }
})

// ---------- toDataDir：空串是有含义的取值，绝不能回落 ----------

test('toDataDir：缺省回落默认值；空串原样保留；trim；非字符串硬失败', () => {
  assert.equal(toDataDir(undefined), DEFAULTS.data_dir)
  assert.equal(toDataDir(null), 'data')
  assert.equal(toDataDir(''), '', '空串 = 系统默认程序目录，必须原样保留')
  assert.equal(toDataDir('  data  '), 'data')
  assert.equal(toDataDir('D:\\app\\store'), 'D:\\app\\store')
  assert.throws(() => toDataDir(true), ConfigError, '布尔不能被宽容转成 "true" 目录名')
  assert.throws(() => toDataDir(1), ConfigError)
})

// ---------- toAppUrl：string | string[]，数组必须带 public_key ----------

test('toAppUrl：缺省回落；字符串 trim；非字符串非数组报错', () => {
  const w: string[] = []
  assert.equal(toAppUrl(undefined, '', w), DEFAULTS.app_url)
  assert.equal(toAppUrl('  https://a.com  ', '', w), 'https://a.com')
  assert.throws(() => toAppUrl(123, '', w), ConfigError)
  assert.throws(() => toAppUrl('   ', '', w), ConfigError, '空字符串等于缺 app_url')
})

test('toAppUrl：数组模式 trim/去空项，且必须提供 public_key', () => {
  assert.deepEqual(toAppUrl([' a.com ', '', 'b.com'], 'pk', []), ['a.com', 'b.com'])
  assert.throws(() => toAppUrl([], 'pk', []), ConfigError, '空数组报错')
  assert.throws(() => toAppUrl(['a.com'], '   ', []), ConfigError, '无数组公钥报错')
})

test('toAppUrl：兼容「JSON 数组文本」但要发告警；坏 JSON 报错', () => {
  const w: string[] = []
  assert.deepEqual(toAppUrl('["a.com","b.com"]', 'pk', w), ['a.com', 'b.com'])
  assert.equal(w.length, 1, '应提示改成真数组')
  assert.throws(() => toAppUrl('[not-json', 'pk', []), ConfigError)
})

// ---------- 未知字段 / 近似名 / 编辑距离 ----------

test('rejectUnknownKeys：已知键与 android_* 放行；未知键硬失败并带近似提示', () => {
  assert.doesNotThrow(() => rejectUnknownKeys({ app_name: 'x', debug: false }))
  assert.doesNotThrow(() =>
    rejectUnknownKeys({ android_key_alias: 'a', android_keystore_base64: 'b' })
  )
  const err = (): Error => {
    try {
      rejectUnknownKeys({ proxy_enbled: true })
      throw new Error('应当失败')
    } catch (e) {
      return e as Error
    }
  }
  assert.match(err().message, /proxy_enabled/, '拼写字段应提示最近似键名')
  assert.throws(() => rejectUnknownKeys({ totally_unknown_xyz: 1 }), ConfigError)
})

test('rejectUnknownKeys：已移除字段给出可操作的改法，且不受原型链污染', () => {
  assert.throws(
    () => rejectUnknownKeys({ window_title: 'x' }),
    /已移除.*title/s
  )
  assert.throws(
    () => rejectUnknownKeys({ data_dir_default: false }),
    /data_dir/
  )
  // 原型链上的名字不能被当成「已移除字段」误报
  assert.throws(() => rejectUnknownKeys({ toString: 1 }), /无法识别/)
})

test('nearestKey / editDistance：编辑距离与阈值边界', () => {
  assert.equal(editDistance('abc', 'abc'), 0)
  assert.equal(editDistance('abc', ''), 3)
  assert.equal(editDistance('kitten', 'sitting'), 3)
  assert.equal(nearestKey('proxy_enbled', ['proxy_enabled', 'app_name']), 'proxy_enabled')
  assert.equal(nearestKey('xyz', ['app_name']), null, '距离超阈值不给提示')
  // bestScore < key.length：单字符键与任意单字符候选距离恒为 1，仍满足；超长差异不给提示
  assert.equal(nearestKey('a', ['bbbbbbbbbb']), null)
})

// ---------- sanitizeDirName：与 Rust sanitize_dir_name 同规则 ----------

test('sanitizeDirName：非法字符替换、trim、空名回落 Web2App', () => {
  assert.equal(sanitizeDirName('a/b\\c:d*e?f"g<h>i|j'), 'a_b_c_d_e_f_g_h_i_j')
  assert.equal(sanitizeDirName('  ok  '), 'ok')
  assert.equal(sanitizeDirName(''), 'Web2App')
  assert.equal(sanitizeDirName('///'), '___', '斜杠被替换成下划线，结果非空就不回落')
  assert.equal(sanitizeDirName('   '), 'Web2App', '纯空白 trim 后为空才回落')
  assert.equal(sanitizeDirName('正常名字'), '正常名字', '非 ASCII 合法字符不动')
})

test('sanitizeDirName：连续空白压缩、首尾句点剥离（对齐 Rust，2026-09-27 补齐）', () => {
  assert.equal(sanitizeDirName('My  App'), 'My App', '连续两个空格压成一个')
  assert.equal(sanitizeDirName('My    App'), 'My App', '多个空格压成一个')
  assert.equal(sanitizeDirName('  My  App  '), 'My App')
  assert.equal(sanitizeDirName('MyApp.'), 'MyApp', '尾句点必须剥离')
  assert.equal(sanitizeDirName('.MyApp'), 'MyApp', '首句点必须剥离')
  assert.equal(sanitizeDirName('My\tApp'), 'My_App',
    'Tab 是 <0x20 控制字符，在 Rust 匹配里先被映射成下划线（早于空白压缩）')
  assert.equal(sanitizeDirName('...'), 'Web2App', '全句点剥离后为空 → 回落')
})

test('sanitizeDirName whitespace matches Rust White_Space on NEL/BOM edges', () => {
  // U+0085 NEL：Rust is_whitespace 算空白（压成空格），裸 JS \s 不算
  assert.equal(sanitizeDirName('a\u0085b'), 'a b')
  // U+FEFF BOM：Rust 不算空白也不非法 → 原样保留；裸 JS \s 会把它压成空格
  assert.equal(sanitizeDirName('a\uFEFFb'), 'a\uFEFFb')
})

// ---------- normalizePublicKey：与打包解析器 normalize_public_key 同规则 ----------

test('normalizePublicKey：空值恒为空串；裸 base64 trim 后原样', () => {
  assert.equal(normalizePublicKey(undefined), '')
  assert.equal(normalizePublicKey(null), '')
  assert.equal(normalizePublicKey(''), '')
  assert.equal(normalizePublicKey('  abc=  '), 'abc=')
  assert.equal(normalizePublicKey(123 as unknown), '123', '非字符串按 String() 宽容收敛')
})

test('normalizePublicKey：PEM 手写的字面量 \\n 还原成真换行，\\r 剥离', () => {
  const pemLiteral =
    '-----BEGIN PUBLIC KEY-----\\n\\rAAAABBBBCCCC\\n-----END PUBLIC KEY-----\\n'
  const out = normalizePublicKey(pemLiteral)
  assert.ok(!out.includes('\\n'), '字面量反斜杠 n 必须被还原：' + JSON.stringify(out))
  assert.ok(!out.includes('\r'), '不得残留 CR')
  assert.equal(
    out,
    '-----BEGIN PUBLIC KEY-----\nAAAABBBBCCCC\n-----END PUBLIC KEY-----'
  )
  // 真换行 PEM 与首尾空白：trim
  assert.equal(normalizePublicKey('\n  pk-body  \n'), 'pk-body')
})

// ---------- debugFlagTruthy：WEB2APP_DEBUG 真值集合两壳逐字一致 ----------

test('debugFlagTruthy：只认 trim+小写后的 1/true/yes；on 刻意不认', () => {
  for (const v of ['1', 'true', 'yes', 'TRUE', ' Yes ', '\t1\t']) {
    assert.equal(debugFlagTruthy(v), true, v)
  }
  for (const v of [undefined, '', '0', 'false', 'no', 'on', '2', 'enable']) {
    assert.equal(debugFlagTruthy(v), false, String(v))
  }
})

// ---------- buildProxySwitches：开关、单边复用、bypass ----------

test('buildProxySwitches：未启用/无主机 → 空对象', () => {
  assert.deepEqual(buildProxySwitches({ ...DEFAULTS }), {})
  assert.deepEqual(
    buildProxySwitches({
      ...DEFAULTS,
      proxy_enabled: true,
      proxy_http: '',
      proxy_https: '',
    }),
    {}
  )
})

test('buildProxySwitches：单边填写两边复用；双边各自保留；bypass 跟随', () => {
  assert.deepEqual(
    buildProxySwitches({ ...DEFAULTS, proxy_enabled: true, proxy_http: 'http://p:8080' }),
    {
      proxyServer: 'http=http://p:8080;https=http://p:8080',
      proxyBypassList: DEFAULTS.proxy_bypass,
    }
  )
  assert.deepEqual(
    buildProxySwitches({ ...DEFAULTS, proxy_enabled: true, proxy_https: 'http://q:9090' }),
    {
      proxyServer: 'http=http://q:9090;https=http://q:9090',
      proxyBypassList: DEFAULTS.proxy_bypass,
    }
  )
  assert.deepEqual(
    buildProxySwitches({
      ...DEFAULTS,
      proxy_enabled: true,
      proxy_http: 'http://h1:1',
      proxy_https: 'http://h2:2',
      proxy_bypass: '   ',
    }),
    { proxyServer: 'http=http://h1:1;https=http://h2:2' },
    'bypass 为空白时不下发 proxyBypassList'
  )
})

test('extraJson：对象正常序列化', () => {
  assert.equal(extraJson({ ...DEFAULTS, extra: { a: 1, b: 'x' } }), '{"a":1,"b":"x"}')
})

// ---------- loadConfig 端到端：临时目录写真文件 ----------

let dirs: string[] = []
afterEach(() => {
  for (const d of dirs) rmSync(d, { recursive: true, force: true })
  dirs = []
})

function loadFromJson(obj: unknown): ReturnType<typeof loadConfig> {
  const d = mkdtempSync(join(tmpdir(), 'w2a-cfg-'))
  dirs.push(d)
  writeFileSync(join(d, 'config.json'), JSON.stringify(obj), 'utf8')
  return loadConfig(d)
}

test('loadConfig：空串 data_dir 端到端保留；title 空回落 app_name', () => {
  const { config, warnings } = loadFromJson({ app_name: '我的应用', data_dir: '' })
  assert.equal(config.data_dir, '', '空串必须穿过完整解析链')
  assert.equal(config.title, '我的应用')
  assert.equal(warnings.length, 0)
})

test('loadConfig：数组 app_url + public_key 可用；android 字段只告警', () => {
  const { config, warnings } = loadFromJson({
    app_url: ['a.com', 'b.com'],
    public_key: 'pk',
    android_key_alias: 'k',
  })
  assert.deepEqual(config.app_url, ['a.com', 'b.com'])
  assert.ok(warnings.some((w) => w.includes('android_key_alias')))
})

test('loadConfig：文件不是对象 / 字符串布尔 / 未知字段，一律抛 ConfigError', () => {
  assert.throws(() => loadFromJson([1, 2]), ConfigError)
  assert.throws(() => loadFromJson(null), ConfigError)
  assert.throws(() => loadFromJson({ debug: 'false' }), ConfigError)
  assert.throws(() => loadFromJson({ window_title: 'x' }), ConfigError)
})
