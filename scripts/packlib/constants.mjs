import { readFileSync } from 'node:fs'

/**
 * 契约常量 —— 字段表的**机器可读唯一真源**是 `../packaging-fields.json`（name / kind /
 * default / 需出现的真源），与 .github/scripts/parse-packaging-config.py 共用同一份；
 * 顺序即打印顺序，也是写进 config.json 的键顺序（= 字段表里的排列顺序）。
 * 改字段 = 改那一份 JSON + 各真源实现；解析语义与 Python 侧逐字对应，改任何语义必须
 * 同步改那边 + docs/packaging-config.md，并跑 `node scripts/pack-local.mjs --parity`。
 */
const FIELDS = JSON.parse(readFileSync(new URL('../packaging-fields.json', import.meta.url), 'utf8'))
  .fields

/** 字段名 → 默认值（顺序 = 字段表顺序）。 */
export const DEFAULTS = Object.fromEntries(FIELDS.map((f) => [f.name, f.default]))

/** android_permissions 白名单（顺序即 Manifest 注入顺序；与 Python 侧元组逐字对齐）。 */
export const ANDROID_PERMISSION_CHOICES = ['camera', 'microphone', 'location']

/** 布尔字段（kind=「bool」）：只接受真 bool，字符串一律拒绝。 */
export const BOOL_KEYS = new Set(FIELDS.filter((f) => f.kind === 'bool').map((f) => f.name))
// 需要保持 JSON 结构（对象/数组/标量）的字段：序列化成单行 JSON 文本进 env。
export const JSON_KEYS = new Set(['extra'])
// 签名相关字段不进环境变量：keystore 体积大 / 密码敏感。
export const SIGNING_KEYS = new Set([
  'android_keystore_base64',
  'android_key_alias',
  'android_keystore_password',
  'android_key_password',
])
export const SKIP_ENV = SIGNING_KEYS

// 已移除的字段 → **可直接照做的改法说明**（只报「不认识这个键」的话，用户还得自己猜怎么改）。
export const REMOVED_KEYS = {
  window_title: '该字段**已移除**，请改用 title（语义相同；移除原因是同一概念曾有三个名字）',
  data_dir_default:
    '该字段**已移除**（旧的是布尔，新的是字符串），请改用 data_dir：\n' +
    '       旧值 false ⇒ "data_dir": "data"  （默认：程序所在目录下的 data\\，跟着程序走）\n' +
    '       旧值 true  ⇒ "data_dir": ""      （空串 = 系统默认程序目录，不受程序位置影响）',
}

// Windows 上永远非法的目录名字符。`:` 与 `\` `/` 是路径语法的一部分，不在此列。
export const DATA_DIR_BAD = new Set('<>"|?*'.split(''))

// electron-nsis：Windows 安装版（NSIS setup），与绿色版分开 —— 它必须带
// `-c.extraMetadata.distribution=nsis` 标记（见 driver-electron），与 zip/portable 不能同次构建。
export const TARGETS = [
  'electron-zip',
  'electron-portable',
  'electron-nsis',
  'electron-all',
  'tauri-msi',
  'tauri-portable',
  'tauri-all',
]
