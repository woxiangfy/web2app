/**
 * IPC sender 闸门 —— **一条**判据、四种调用点共用（2026-11-07，架构审查 H-3）。
 *
 * 背景：窗口命令 / 页签命令 / 设置命令 / 悬浮球命令各自写过一份「sender 是否可信」，
 * 其中 `app-ipc.ts::assertPrivileged` 在**单窗口模式**直接 `return` —— 等于按「是不是
 * 多页签」定义安全边界，而不是按「sender 是不是我们自己的页面」。一旦未来误把带 invoke
 * 的 preload 挂到内容视图或子框架，远程页面就能关窗 / 开设置，且无日志、无拒绝。
 *
 * 现在统一按「sender.id 必须等于宿主 webContents.id」判定，两种模式都校验；
 * 不匹配一律**先记日志再抛错**（绝不静默失效 —— 本项目最忌的失败形态）。
 *
 * 本文件零 Electron 依赖，`pnpm run unit` 可直接测。
 */
export interface SenderGate {
  /** 用于日志 / 报错的命令族名，如「窗口命令」「设置命令」 */
  label: string
}

/**
 * 校验 sender 是否属于期望的 webContents。
 *
 * @param senderId   实际发送方 `webContents.id`（或 `event.sender.id`）
 * @param expectedId 期望的宿主 `webContents.id`；宿主尚未创建 / 已销毁时为 `null`（一律拒绝）
 */
export function assertTrustedSender(senderId: number, expectedId: number | null, gate: SenderGate): void {
  if (expectedId !== null && senderId === expectedId) return
  const expected = expectedId === null ? '无（宿主未就绪或已销毁）' : String(expectedId)
  console.warn(`[web2app] 拒绝${gate.label}（sender.id=${senderId}，期望=${expected}）`)
  throw new Error(`拒绝：${gate.label}只能由宿主页面发起`)
}
