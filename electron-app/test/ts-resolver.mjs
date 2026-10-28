/**
 * node --test 直跑 TS 源码的跨文件导入垫片 —— 由 `pnpm run unit` 经 `--import` 预载，
 * 在**每个测试进程解析入口之前**生效（钩子必须先于被测模块的导入图解析，故不能写成
 * 测试文件里的普通 import）。
 *
 * 背景（2026-10-24，批次⑤ P2-5 起多文件需要）：src/ 内部相对导入统一**无扩展名**
 * （`'./util'` / `'./config'`），tsc 编译成 CJS 后靠 require 的扩展名搜索工作，
 * 但 Node 的 ESM 解析不认 —— 于是任何「被测模块又 import 兄弟模块」的测试都会死在
 * ERR_MODULE_NOT_FOUND。这里注册一个 resolve 钩子，把「父模块是 .ts」的无扩展名
 * 相对导入补成 `.ts`。
 *
 * 只影响测试进程，不动 src 的导入风格与构建配置（`allowImportingTsExtensions`
 * 仅限 noEmit 的 tsconfig.test.json，主 tsconfig 禁止）。
 */
import { existsSync } from 'node:fs'
import { registerHooks } from 'node:module'
import { extname } from 'node:path'
import { fileURLToPath } from 'node:url'

registerHooks({
  resolve(specifier, context, nextResolve) {
    const isExtlessRel =
      (specifier.startsWith('./') || specifier.startsWith('../')) && extname(specifier) === ''
    if (isExtlessRel && context.parentURL?.endsWith('.ts')) {
      const cand = new URL(`${specifier}.ts`, context.parentURL)
      if (existsSync(fileURLToPath(cand))) return nextResolve(`${specifier}.ts`, context)
    }
    return nextResolve(specifier, context)
  },
})
