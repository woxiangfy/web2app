/**
 * 清除浏览数据（缓存 / Cookie / 本地存储 …）—— 设置页「清除全部」按钮的后端。
 *
 * 与 Tauri 版（`webview-app/src/web_data.rs`）**语义对齐**：一次全清，没有分项选项。
 *
 * ⚠️ 只清 HTTP 缓存是不够的。本应用还有两处「清了却看不出来」的状态：
 *   · Basic 鉴权凭据缓存在 Chromium 的 **HTTP 鉴权缓存**里 —— 不清的话表现就是
 *     「清了缓存、重新打开还是自动登录」；
 *   · 域名解析结果有独立的 host resolver 缓存。
 * 所以这里是「三清 + 重载」：`clearData`（官方明示比 `clearStorageData` 更彻底，覆盖
 * cookie / 缓存 / 各类存储 / Service Worker / 下载记录 …）、`clearAuthCache`、
 * `clearHostResolverCache`，最后把内容页重新加载。
 *
 * ⚠️ 探针下走**桩**分支：探针跑在用户自己的机器上，把人家真实的登录态与缓存清掉是不可接受的
 *    副作用。桩只保证「设置页 → 垫片 → IPC → 处理器 → 返回」这条链是通的。
 *    （托盘 / 全局快捷键在探针下不做桩：m3 组断言的就是 applySettings 的真实副作用。）
 */
import { session } from 'electron'

export interface ClearWebDataDeps {
  /** 是否探针模式（true 时只验证链路连通，不动真实数据） */
  probe: boolean
  /** 重新加载全部内容页，返回重载的页面数 */
  reloadContentPages: () => number
}

/** 全清浏览数据；返回给设置页直接展示的一句话。 */
export async function clearWebData(deps: ClearWebDataDeps): Promise<string> {
  if (deps.probe) {
    return '（探针模式）已跳过真实清除，仅验证命令连通'
  }

  const ses = session.defaultSession
  // ⚠️ 顺序有意义：先把数据清干净，**最后**才重载页面。反过来会让刚重载的页面
  //    立刻又写回一份缓存与 Cookie，「清了等于没清」。
  // ⚠️ session 是共享的：主窗口与所有内容视图（WebContentsView）都没设 partition，
  //    都挂在 defaultSession 上（见 index.ts 的 will-download 注释），所以清一次即全覆盖。
  await ses.clearData()
  await ses.clearAuthCache()
  await ses.clearHostResolverCache()

  const reloaded = deps.reloadContentPages()
  return `已清除缓存、Cookie 等全部浏览数据；已重新加载 ${reloaded} 个页面`
}
