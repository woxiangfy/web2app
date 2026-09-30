# 独立 Review 记录（2026-09-26）

- 审查者：fresh context 只读子代理（未参与实现）
- 范围：19 个改动文件 + 2 个新增文件（tab-order.ts / tabs.test.ts），对照 spec.md AC A1–A10
- 结论：**PASS / 10 分制 9 分**，无 P0/P1/P2 缺陷，无需修复项

## 逐项结论

| 审查项 | 结论 | 要点 |
| -- | -- | -- |
| a 邻接规则与 Tauri `close_tab_inner` 语义等价 | 2/2 | 空/单个/首中末/非法下标全边界数学恒等 |
| b P7b/P9/P8 标签生命周期自洽 | 2/2 | 4→3→2→1→P9→32→1 处处闭合；label2 关后无存活引用；不会误触 host.close |
| c 重试遮罩时序与 IPC 安全 | 2/2 | JSON.stringify 嵌文案、textContent 无注入；preload 白名单 + assertShell 双保险；远程页面触达不到 |
| d applySettings 回滚 | 2/2 | trayCreatedThisCall 置位点正确；幂等 destroy |
| e proxyUrl 校验不误杀 noProxy | 2/2 | Rust 生产端已限 http(s)；noProxy 导航用 resolvedUrl |
| f tabs-v4 node/electron 分叉 | 2/2 | 实测 NEGATIVE_CONTROL_OK；删 RUN_AS_NODE、参数/退出码透传正确 |
| g 死字段零消费方 | 2/2 | 全仓 grep 零命中 |
| h 文档事实 | 2/2 | 134/31/35/61/4/1、7 workflow、16/4 文件、52.9KB 全部复核一致 |
| spec 覆盖 / 无超范围 | 2/2 | T1–T12 全落地；P7b 多 1 条成对判据属合理增量且计数已同步 |
| 红→绿反证 | 2/2 | 纯函数层与探针层双重反向对照 |

## 审查者独立取证

- `node --test test/**/*.test.ts` → 35/35；tabs.test.ts 单跑 6/6
- 探针构成静态复算：117 check + 18 near − 1 互斥点 = 134；§11–14 = 31+7+19+4 = 61；多页签 31
- 文件系统实测：7 workflow、src/main 16、renderer 4、build-custom.yml 52.9 KB
- 采信委托方门禁：cargo 133、typecheck 0、探针 134/31 exit 0、smoke exit 0、selftest 通过

## 非阻断建议（已列入最终报告遗留）

1. onSpawn 的 loadURL `.catch` 仍吞「load 期失败」：遮罩重试只覆盖 spawn/解析期；
2. applySettings 销毁托盘后落盘失败不重建旧托盘（与新建侧不对称，存量问题）；
3. 失败遮罩 GUI 目视待真机（A2 本就列遗留，沙箱禁截屏）。
