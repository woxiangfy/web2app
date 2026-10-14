/**
 * 契约常量 —— 与 .github/scripts/parse-packaging-config.py **逐字对应**
 * 顺序即打印顺序，也是写进 config.json 的键顺序。改这里必须同步改那边 + docs/packaging-config.md
 */
export const DEFAULTS = {
  app_url: '',
  app_name: 'Web2App',
  title: '',
  // 打包产物版本（Tauri 写 tauri.conf.json.version；Electron 走 extraMetadata.version）。
  // 只接受三段纯数字 x.y.z（主/次 ≤ 255、补丁 ≤ 65535），见 parser.mjs::normalizeVersion。
  version: '0.0.1',
  extra: {},
  icon_url: '',
  proxy_enabled: false,
  proxy_http: '',
  proxy_https: '',
  // ⚠️ 两壳必须一致：webview-app 的 config.rs 编译期默认值、electron 的 DEFAULTS
  //    都用这同一个串。留空 = 不下发 bypass 参数（即空值不覆盖 Chromium 默认）。
  proxy_bypass: 'localhost;127.0.0.1;<-loopback>',
  // 空串在本字段里**是有含义的取值**（= 系统默认程序目录），不是「未设置」。
  data_dir: 'data',
  tabs_enabled: false,
  // 移动端底栏：仅移动端（Android/iOS）生效的底部导航栏（首页 / 后退 / 前进 / 列表）。
  // 桌面壳（Windows Tauri / Electron）注入了也一律忽略 —— Rust 侧常量按 #[cfg(mobile)] 编译。
  mobile_bar: false,
  public_key: '',
  resolve_secret: '',
  debug: false,
  android_keystore_base64: '',
  android_key_alias: '',
  android_keystore_password: '',
  android_key_password: '',
  // 仅 Android：按需向 AndroidManifest.xml 注入运行时权限声明。
  // 白名单三值 camera / microphone / location（见 parser.mjs::normalizeAndroidPermissions），
  // 默认 [] = 不声明任何可选权限。由 patch-android-permissions.py 消费 ANDROID_PERMISSIONS。
  android_permissions: [],
}

/** android_permissions 白名单（顺序即 Manifest 注入顺序；与 Python 侧元组逐字对齐）。 */
export const ANDROID_PERMISSION_CHOICES = ['camera', 'microphone', 'location']

export const BOOL_KEYS = new Set(['proxy_enabled', 'tabs_enabled', 'mobile_bar', 'debug'])
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
