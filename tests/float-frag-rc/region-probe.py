#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""只读取证：量用户现场悬浮球窗的**窗口区域**（SetWindowRgn 设的那个）。

要复核的命题（v1.77b 根因的另一半）
----------------------------------
「球（46 逻辑）比窗口区域圆（56 逻辑 = ICON_SIZE_LOGICAL）小 ⇒ 两者之间那圈
**透明环带**在 Tauri/WebView2 下透出浅色残片」。真机截图的几何已经量出残片的
内边界 = 球边缘、外边界与球同心；本脚本从**窗口自身的区域**独立复核：
  ① 区域包围盒是否 = 活柱边长²
  ② 区域是不是一个圆（用 PtInRegion 网格重建，看每行 x 起止是否贴合圆）

⚠️ 用 Python + ctypes 而不是 PowerShell：本机 Windows PowerShell 5.1 按 ANSI 读 .ps1，
中文注释会乱码；ctypes 没有这个问题。

全程只读（EnumWindows / GetWindowRect / GetWindowRgn / GetRgnBox / PtInRegion）。
输出写到同目录 region-probe.txt。
"""
import ctypes
import ctypes.wintypes as wt
import pathlib
import sys

u32 = ctypes.WinDLL("user32", use_last_error=True)
g32 = ctypes.WinDLL("gdi32", use_last_error=True)

GWL_EXSTYLE = -20


class RECT(ctypes.Structure):
    _fields_ = [("left", wt.LONG), ("top", wt.LONG), ("right", wt.LONG), ("bottom", wt.LONG)]


u32.EnumWindows.restype = wt.BOOL
u32.GetClassNameW.restype = ctypes.c_int
u32.GetWindowTextW.restype = ctypes.c_int
u32.GetWindowRgn.restype = ctypes.c_int
u32.GetWindowLongPtrW.restype = ctypes.c_ssize_t
g32.GetRgnBox.restype = ctypes.c_int
g32.PtInRegion.restype = wt.BOOL


def enum_windows():
    out = []
    cb = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)

    def _cb(h, _l):
        pid = wt.DWORD(0)
        u32.GetWindowThreadProcessId(h, ctypes.byref(pid))
        cls = ctypes.create_unicode_buffer(256)
        u32.GetClassNameW(h, cls, 256)
        ttl = ctypes.create_unicode_buffer(256)
        u32.GetWindowTextW(h, ttl, 256)
        r = RECT()
        u32.GetWindowRect(h, ctypes.byref(r))
        ex = u32.GetWindowLongPtrW(h, GWL_EXSTYLE)
        out.append({
            "hwnd": h, "pid": pid.value, "cls": cls.value, "title": ttl.value,
            "rect": (r.left, r.top, r.right - r.left, r.bottom - r.top),
            "ex": ex, "vis": bool(u32.IsWindowVisible(h)),
        })
        return True

    u32.EnumWindows(cb(_cb), 0)
    return out


def main() -> int:
    pid = int(sys.argv[1]) if len(sys.argv) > 1 else 0
    wins = enum_windows()
    if pid:
        wins = [w for w in wins if w["pid"] == pid]
    lines = []

    def w(s):
        lines.append(s)

    w("=== 顶层窗口（EnumWindows 顺序 = Z 序，从上到下）===")
    for x in wins:
        tag = []
        e = x["ex"]
        if e & 0x40000:
            tag.append("APPWINDOW")
        if e & 0x80:
            tag.append("TOOLWINDOW")
        if e & 0x8:
            tag.append("TOPMOST")
        if e & 0x8000000:
            tag.append("NOACTIVATE")
        l, t, ww, hh = x["rect"]
        w(f"hwnd={x['hwnd']:#x} pid={x['pid']} vis={x['vis']} rect={l},{t} {ww}x{hh} "
          f"ex={e:#x} [{'|'.join(tag)}] cls=[{x['cls']}] title=[{x['title']}]")

    cands = [x for x in wins if x["rect"][3] == 56 and x["rect"][2] >= 56]
    if not cands:
        w("")
        w("!! 没找到「高 56」的悬浮球窗（球窗可能没在跑，或缩放不是 100%）")
    for x in cands:
        l, t, ww, hh = x["rect"]
        w("")
        w(f"=== 区域取证 hwnd={x['hwnd']:#x} rect={l},{t} {ww}x{hh} ===")
        rgn = g32.CreateRectRgn(0, 0, 0, 0)
        kind = u32.GetWindowRgn(x["hwnd"], rgn)
        if kind <= 0:
            w(f"GetWindowRgn 返回 {kind}（<=0 = 该窗没有区域或调用失败）")
            g32.DeleteObject(rgn)
            continue
        rb = RECT()
        g32.GetRgnBox(rgn, ctypes.byref(rb))
        names = {1: "NULLREGION", 2: "SIMPLEREGION", 3: "COMPLEXREGION", 4: "ERROR"}
        w(f"GetWindowRgn kind={kind} ({names.get(kind, '?')})")
        w(f"GetRgnBox = ({rb.left},{rb.top})..({rb.right},{rb.bottom}) "
          f"=> 区域 {rb.right - rb.left}x{rb.bottom - rb.top}（**含端**：x 取到 right-1）")
        W = max(rb.right + 2, ww)
        H = max(rb.bottom + 2, hh)
        w("PtInRegion 网格（每 4 行一条）：每行 x 起止 + 宽度 —— 圆的话中间行最宽、两端递减")
        maxw = 0
        for y in range(0, H, 4):
            xs = [xx for xx in range(W) if g32.PtInRegion(rgn, xx, y)]
            if not xs:
                w(f"y={y:4d}  -")
                continue
            width = xs[-1] - xs[0] + 1
            maxw = max(maxw, width)
            w(f"y={y:4d}  x={xs[0]}..{xs[-1]}  宽={width}")
        w(f"最宽行宽 = {maxw}（圆 ⇒ ≈ 区域直径）")
        g32.DeleteObject(rgn)

    out = pathlib.Path(__file__).with_name("region-probe.txt")
    out.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("WROTE", out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
