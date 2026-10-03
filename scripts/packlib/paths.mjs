/**
 * 仓库路径常量。
 *
 * 本目录（scripts/packlib/）比入口（scripts/pack-local.mjs）深一层，
 * 故 ROOT 要连退两级 —— 这是拆模块时最容易踩的坑。
 */
import { dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..')
const WEBVIEW = join(ROOT, 'webview-app')
const ELECTRON = join(ROOT, 'electron-app')
const PARSER_PY = join(ROOT, '.github', 'scripts', 'parse-packaging-config.py')

const rel = (p) => relative(ROOT, p).replace(/\\/g, '/') || '.'

export { ROOT, WEBVIEW, ELECTRON, PARSER_PY, rel }
