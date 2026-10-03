/**
 * 错误类型与进程级输出 sink。
 *
 * parser / util / driver / selftest / parity 共用同一个模块级 sink 实例：
 * 解析期间会往 stdout 打 `::warning::`（CI 注解），`--selftest` / `--parity`
 * 用 captureOutput 临时接管它，而不是到处传一个 logger 参数。
 */

class ConfigError extends Error {}

/** 进程级致命错误（工具缺失、命令失败）—— 与配置错误区分开。 */
class FatalError extends Error {}

// 解析期间会往 stdout 打 `::warning::`（CI 注解）。`--selftest` / `--parity` 需要捕获它们，
// 这里用一个可替换的 sink，而不是到处传一个 logger 参数。
let sink = null
function emit(line) {
  if (sink) sink.push(line)
  else process.stdout.write(line + '\n')
}
/**
 * 捕获 `emit` 的输出，供 `--selftest` / `--parity` 使用。
 *
 * ⚠️ 返回 `{ lines, error }` 而不是「成功才返回」：解析器抛 `ConfigError` **之前**往往已经
 * 打过 warning（如 `data_dir 含非法字符`），比对时那些行也必须拿到。
 * 支持嵌套：selftest 整体静音的同时，内部还会逐 payload 再捕获一次。
 */
function captureOutput(fn) {
  const prev = sink
  sink = []
  try {
    fn()
    return { lines: sink, error: null }
  } catch (e) {
    return { lines: sink, error: e }
  } finally {
    sink = prev
  }
}

export { ConfigError, FatalError, emit, captureOutput }
