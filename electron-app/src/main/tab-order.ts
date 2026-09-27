/**
 * 多页签的**纯顺序规则**（零 Electron 依赖）。
 *
 * 抽成独立模块的原因：`tabs.ts` 顶层有 `import { WebContentsView } from 'electron'`，
 * node:test 直接加载它会在 ESM 解析阶段失败（node 下 require('electron') 返回路径字符串，
 * 没有命名导出）。而「关掉一个页签后激活谁」是不依赖任何 GUI 对象的纯规则，
 * 理应能被毫秒级单测直接钉住，故放这里由 `tabs.ts` 导入。
 */

/**
 * 关闭一个页签后，激活态应落到哪个幸存页签。
 *
 * 与 Tauri `webview-app/src/tabs.rs::close_tab_inner` 同一条规则：
 * **优先右邻，右邻不存在（删掉的是末位）则取左邻**。
 *
 * ⚠️ 入参必须是【删除前】的完整 label 列表与被删页签在其中的下标。
 * 历史 bug：在删除之后的列表里 `findIndex(被删 label)` 恒为 -1，
 * `Math.min(-1 + 1, len - 1)` 恒为 0 —— 关中间 / 关末尾都错误地跳回第一个幸存页签；
 * 只用 2 个页签的探针恰好放过（左右邻是同一个）。
 *
 * @returns 下一个应激活的 label；列表为空 / 下标非法 / 关掉的是唯一页签时返回 null
 *          （最后一种由调用方走「关闭宿主」语义，本函数不做决定）。
 */
export function nextActiveLabelAfterClose(
  labelsBefore: readonly string[],
  removedIndex: number,
): string | null {
  if (removedIndex < 0 || removedIndex >= labelsBefore.length || labelsBefore.length <= 1) {
    return null
  }
  // 右邻下标 = removedIndex + 1（删除后它正好移到 removedIndex 位置）；
  // 删掉的是末位时没有右邻，退回左邻 removedIndex - 1。
  if (removedIndex + 1 < labelsBefore.length) return labelsBefore[removedIndex + 1]
  return labelsBefore[removedIndex - 1]
}
