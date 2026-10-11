#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""把 v1.77b 的索引条目追加进三处 AGENTS.md（按行号定位 + 断言，避免锚点撞车）。"""
import pathlib

ROOT = pathlib.Path(r"C:\dev\rust\web-app")

ROOT_CLAUSE = (
    "；**v1.77b「背景异样」的第三个源头——球比窗口圆形区域小、中间那圈透明环带透出了窗口后面的东西**"
    "（使用者第二次报同一现象、重打包后依旧）：①截图像素取证——残片**内**边界 = 球的边缘、**外**边界与球同心 "
    "⇒ 以球心为圆心、内径 = 球、外径 ≈ 活柱边长的**圆环**（只在左上 / 右侧可见，是球自己的深色 `box-shadow` "
    "把其余部分压到判据阈值以下）；②对使用者现场实例（`osfipin.mark` pid 9864）做**只读窗口区域探针**"
    "（`EnumWindows`+`GetWindowRgn`+`GetRgnBox`+`PtInRegion` 网格，`tests/float-frag-rc/region-probe.py`）："
    "`GetRgnBox=(0,0)..(55,56)`、每行宽度两端递减、y=28 处最宽 55 ⇒ **区域确是一个内接于 56×56 活柱的圆**，"
    "而球只有 46 ⇒ 那圈 **5 逻辑像素宽的环带**一直是透的：Tauri/WebView2 底层非真透明 ⇒ 透成浅色残片；"
    "Electron 真透明 ⇒ 只透出桌面 —— 这就是「仅限 Tauri」。修法只动双壳共用的 `floaticon.html`：球 `46px → **56px**`"
    "（铺满区域圆，环带在几何上不存在）、hover / grabbing 的 `scale(1.1/1.04) → **filter: brightness(1.12/1.06)**`"
    "（球已铺满，`scale>1` 会被区域裁掉 = 没有反馈，放大出的那圈又是环带）、collapsed 的 `opacity:.92 → "
    "**filter: brightness(.92)**`（`opacity` 让整颗球半透明 = 自毁这层保护）、图标 `30 → 36px`、过渡去掉 `transform`。"
    "新增守卫 `float_page_ball_fills_the_live_pillar`（球宽高必须等于**从 `ICON_SIZE_LOGICAL` 常量里抠出的值**，"
    "不许两边各写一个 56；hover 块必须 `filter: brightness(1.12)` 且不得 `scale(`；collapsed 块必须 "
    "`filter: brightness(` 且**不得** `opacity`）+ 新辅助 `css_block()`（把断言限定在那一条 CSS 规则内）/ "
    "`icon_size_logical_int()`，Electron `float-contract` 同源一条；顺手修掉**第二个静默失败**："
    "`delete_taskbar_tab` 漏了 `ITaskbarList::HrInit()`（契约要求任何方法前先初始化，未初始化时 `DeleteTab` "
    "返 `E_FAIL` 且什么都不做）且用 `let _ =` 吞掉 HRESULT ⇒ 补 `HrInit()` + 失败留痕 + 守卫补强。"
    "门禁：`fmt --check` 0 diff、`clippy --all-targets -- -D warnings` 退出码 0、`test --all` 全过"
    "（19 + 22 + 32 + **166**，`contract_tests` 46）、Electron `unit` **96/96**；反向对照 4/4 实跑 "
    "`passed=0 failed=1` 且按字节还原 sha256 吻合。⚠️⚠️ 该反向对照脚本**第一版谎报成功**："
    "`--exact` 配叶子名 ⇒ `0 passed; 169 filtered out`，「没跑」被当成「通过」；`finally` 里的 `return` "
    "又覆盖了 try 的返回值；修正判据后第三次又把 cargo 的 `error: test failed` 误判成「编译失败」"
    "⇒ **反向对照脚本自己也要先自证「真的跑了那个测试」**。另记两条环境坑：本机 Electron GUI 起不来"
    "（`app.whenReady()` 后进程即死、退出码 127）⇒ **渲染型 A/B 在本机不可行**；本机 PowerShell 按 ANSI "
    "读 .ps1 会乱码 ⇒ 这类探针改用 Python + ctypes。详见 `docs/changes/2026-10-11.md`"
)

WV_39_CLAUSE = (
    "；**v1.77b：球必须铺满活柱**（球径 `46px → **56px**` = `ICON_SIZE_LOGICAL`）—— 窗口区域是"
    "「内接于活柱的圆」（现场只读取证 `GetRgnBox=(0,0)..(55,56)`、`PtInRegion` 网格两端递减），球比它小 "
    "⇒ 中间那圈透明环带把窗口后面的东西透出来（Tauri/WebView2 非真透明 ⇒ 浅色残片；Electron 真透明 ⇒ 无感），"
    "这是「图标背景异样」的第三个源头（前两个：v1.73 球元素 `title`、v1.77 任务栏按钮）；连带 hover / grabbing 的 "
    "`scale()` 改 `filter: brightness()`、collapsed 的 `opacity` 改 `filter: brightness(.92)`、图标 `30 → 36px`，"
    "守卫 `float_page_ball_fills_the_live_pillar`"
)

WV_84_CLAUSE = (
    " **v1.77b**：球径必须 = 活柱边长（`46px → 56px`）—— 窗口圆形区域与球之间那圈环带会透出窗口后面的东西"
    "（Tauri 侧成浅色残片）；悬停 / 拖动反馈从 `transform: scale()` 改 `filter: brightness()`，半隐藏从 "
    "`opacity` 改 `filter: brightness(.92)`（`opacity` 让整颗球半透明 = 自毁这层保护）；"
    "另补上 `ITaskbarList::HrInit()`（v1.77 漏了它 ⇒ `DeleteTab` 静默无效）。"
)

EL_395_CLAUSE = (
    "；**v1.77b**：本壳与 Tauri 共用同一份 `floaticon.html`，故同源守卫一并钉死「球径 == `DEFAULT_FLOAT_W`"
    "（= 区域圆直径）」「球块不得 `scale(`」「collapsed 块不得 `opacity`」—— Tauri 侧把球从 46 改成 56 之后，"
    "本壳由 `circleShapeRects(56)` 裁出的圆与球严格重合，环带在两侧都不再存在"
)


def patch(rel, lineno, mode, text, anchor=None):
    p = ROOT / rel
    src = p.read_text(encoding="utf-8")
    lines = src.splitlines(keepends=True)
    i = lineno - 1
    old = lines[i]
    if mode == "append":
        assert old.endswith("\n"), f"{rel}:{lineno} 不是完整行"
        lines[i] = old[:-1] + text + "\n"
    elif mode == "before":
        assert anchor in old, f"{rel}:{lineno} 找不到锚点 {anchor!r}"
        lines[i] = old.replace(anchor, text + anchor, 1)
    elif mode == "before_last":
        # ⚠️ 表格行里 ` |` 会出现多次（单元分隔符），replace 首个会插到**第一格**后面 ⇒ 必须用 rfind。
        at = old.rfind(anchor)
        assert at >= 0, f"{rel}:{lineno} 找不到锚点 {anchor!r}"
        assert old.count(anchor) >= 2, f"{rel}:{lineno} 锚点只出现一次，用 before 即可"
        lines[i] = old[:at] + text + old[at:]
    else:
        raise ValueError(mode)
    p.write_text("".join(lines), encoding="utf-8")
    print(f"patched {rel}:{lineno} ({mode}) -> {len(lines[i])} chars")


patch("AGENTS.md", 108, "append", ROOT_CLAUSE)
patch("webview-app/AGENTS.md", 39, "before", WV_39_CLAUSE, "，守卫 ⑥b / ⑥c | Windows |")
patch("webview-app/AGENTS.md", 84, "append", WV_84_CLAUSE)
patch("electron-app/AGENTS.md", 395, "before_last", EL_395_CLAUSE, " |")
print("done")
