#!/usr/bin/env python3
"""Android H5 <video> 全屏补丁：实现 wry 0.57 空掉的 onShow/onHideCustomView。

## 背景（为什么需要）

wry 0.57 的 `RustWebChromeClient.kt` 里两个全屏回调是**空实现**：

    override fun onShowCustomView(view: View, callback: CustomViewCallback) {
      callback.onCustomViewHidden()   // ← 立即通知「已退出全屏」= 全屏被立刻取消
      super.onShowCustomView(view, callback)
    }
    override fun onHideCustomView() { super.onHideCustomView() }

网页里 <video controls> 点全屏按钮、或 requestFullscreen() 触发时，Chromium 把视频
渲染层包成 custom view 交给这两个回调；wry 立刻 onCustomViewHidden() ⇒ 画面退回
内嵌播放器，**全屏按钮点了没反应**（视频站还可能卡在半全屏状态）。

本补丁把两个方法**整体替换**为标准全屏实现（只插首行不行——原方法体里的
onCustomViewHidden() 仍会立刻取消）：

  · onShowCustomView：wry 用 activity.setContentView(webView)，android.R.id.content
    的唯一子 view 就是承载页面的 WebView；把它 GONE，custom view 以 MATCH_PARENT
    加进 content；锁定横屏（SENSOR_LANDSCAPE）并进沉浸式（隐藏状态栏/导航栏）。
  · onHideCustomView：移除 custom view、WebView 恢复 VISIBLE、还原屏幕方向与系统栏，
    最后 callback.onCustomViewHidden() 通知 Chromium。
  · 沉浸式两套实现：API 30+ 走 WindowInsetsController（hide(systemBars) +
    SHOW_TRANSIENT_BARS_BY_SWIPE），低版本走 systemUiVisibility 标志（minSdk=24）。

补丁链固定顺序（见 docs/android-improvements.md §0）：
insets → back → download → extras → schemes → fullscreen → permissions → cleartext sed。
fullscreen 只改 RustWebChromeClient.kt，与只改 Manifest 的 permissions 补丁互不冲突。

gen/android 每次构建现生成，两个 workflow（build-custom.yml / release.yml）
都必须在 init 之后调用本脚本。

用法：
    python3 .github/scripts/patch-android-fullscreen.py            # 打补丁（工程根自动探测）
    python3 .github/scripts/patch-android-fullscreen.py --selftest # 离线自测
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys

from android_gen import (  # noqa: E402
    find_kotlin_file,
    gen_root_candidates_str,
    resolve_gen_root,
)

# 幂等标记：已打过补丁的文件不再重复注入。
MARKER = "web2app-android-fullscreen"

# 需要补的 import（缺失才加；android.webkit.* 已在模板里，CustomViewCallback 不用单列）。
NEEDED_IMPORTS = (
    "import android.content.pm.ActivityInfo",
    "import android.view.ViewGroup",
    "import android.view.WindowInsetsController",
    "import android.widget.FrameLayout",
)

IMPORT_LINE = re.compile(r"^import\s+.*$", re.M)
PACKAGE_LINE = re.compile(r"^package\s+.*$", re.M)

# 两个回调的签名（CustomViewCallback 类型允许带 WebChromeClient. 前缀，兼容模板微调）。
ON_SHOW = re.compile(
    r"^([ \t]*)override\s+fun\s+onShowCustomView\s*\(\s*"
    r"view\s*:\s*View\s*,\s*callback\s*:\s*(?:WebChromeClient\.)?CustomViewCallback\s*\)\s*\{",
    re.M,
)
ON_HIDE = re.compile(
    r"^([ \t]*)override\s+fun\s+onHideCustomView\s*\(\s*\)\s*\{",
    re.M,
)

# 替换进去的整块：属性 + 两个回调 + 沉浸式辅助（2 空格类成员缩进，与模板对齐）。
# 用占位符 {marker}（注意 Kotlin 代码里没有其它裸花括号给 str.format 误伤 —— 一律用 {{ }}）。
FULLSCREEN_BLOCK = """  // {marker}：H5 <video> / requestFullscreen 的全屏承载。
  // wry 0.57 原实现进来就 callback.onCustomViewHidden()（立即取消），全屏按钮形同虚设；
  // 这里按标准 WebChromeClient 做法：custom view 铺满 content、WebView 暂时藏起。
  private var web2appCustomView: View? = null
  private var web2appCustomCallback: WebChromeClient.CustomViewCallback? = null
  private var web2appSavedOrientation = ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED

  override fun onShowCustomView(view: View, callback: CustomViewCallback) {{
    // 页面连续两次 fullscreenchange 时旧视图可能还在：不堆叠，直接拒绝新的并退掉旧回调。
    if (web2appCustomView != null) {{
      callback.onCustomViewHidden()
      return
    }}
    val content = activity.findViewById<ViewGroup>(android.R.id.content)
    if (content == null) {{
      callback.onCustomViewHidden()
      return
    }}
    // wry 用 activity.setContentView(webView)：content 的第一个子 view 就是页面 WebView。
    val webView = if (content.childCount > 0) content.getChildAt(0) else null
    web2appCustomView = view
    web2appCustomCallback = callback
    web2appSavedOrientation = activity.requestedOrientation
    webView?.visibility = View.GONE
    content.addView(
      view,
      FrameLayout.LayoutParams(
        ViewGroup.LayoutParams.MATCH_PARENT,
        ViewGroup.LayoutParams.MATCH_PARENT
      )
    )
    activity.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
    web2appSetImmersive(true)
  }}

  override fun onHideCustomView() {{
    val customView = web2appCustomView
    if (customView == null) {{
      super.onHideCustomView()
      return
    }}
    val content = activity.findViewById<ViewGroup>(android.R.id.content)
    content?.removeView(customView)
    val webView = content?.let {{ if (it.childCount > 0) it.getChildAt(0) else null }}
    webView?.visibility = View.VISIBLE
    activity.requestedOrientation = web2appSavedOrientation
    web2appSetImmersive(false)
    web2appCustomView = null
    val callback = web2appCustomCallback
    web2appCustomCallback = null
    callback?.onCustomViewHidden()
    super.onHideCustomView()
  }}

  // {marker}：全屏期间隐藏系统栏；退出恢复。API 30+ 走 WindowInsetsController，
  // 低版本走 systemUiVisibility 标志位（minSdk=24，两套都要保留）。
  @Suppress("DEPRECATION")
  private fun web2appSetImmersive(enter: Boolean) {{
    val window = activity.window
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {{
      val controller = window.insetsController
      if (enter) {{
        controller?.hide(android.view.WindowInsets.Type.systemBars())
        controller?.systemBarsBehavior =
          WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
      }} else {{
        controller?.show(android.view.WindowInsets.Type.systemBars())
        controller?.systemBarsBehavior = WindowInsetsController.BEHAVIOR_DEFAULT
      }}
    }} else {{
      window.decorView.systemUiVisibility = if (enter) {{
        View.SYSTEM_UI_FLAG_LAYOUT_STABLE or
          View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION or
          View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN or
          View.SYSTEM_UI_FLAG_HIDE_NAVIGATION or
          View.SYSTEM_UI_FLAG_FULLSCREEN or
          View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
      }} else {{
        View.SYSTEM_UI_FLAG_VISIBLE
      }}
    }}
  }}""".format(marker=MARKER)


def _add_imports(src: str, needed: tuple[str, ...]) -> str:
    """缺失的 import 插到最后一条 import 之后；完全没有 import 则插在 package 行之后。"""
    missing = [imp for imp in needed if imp not in src]
    if not missing:
        return src
    imports = list(IMPORT_LINE.finditer(src))
    if imports:
        pos = imports[-1].end()
        return src[:pos] + "\n" + "\n".join(missing) + src[pos:]
    pkg = PACKAGE_LINE.search(src)
    if not pkg:
        raise ValueError("源码中既无 import 也无 package 声明，无法确定注入位置")
    return src[: pkg.end()] + "\n\n" + "\n".join(missing) + src[pkg.end() :]


def _block_end(src: str, open_brace: int) -> int:
    """从方法签名上的 '{' 起做花括号配对，返回方法体闭合 '}' 的下标（含）。

    跳过 // 行注释、/* 块注释与字符串 / 字符字面量；模板文件结构简单，朴素扫描足够。
    """
    depth = 0
    i = open_brace
    n = len(src)
    while i < n:
        c = src[i]
        nxt = src[i + 1] if i + 1 < n else ""
        if c == "/" and nxt == "/":
            j = src.find("\n", i)
            i = n if j < 0 else j
            continue
        if c == "/" and nxt == "*":
            j = src.find("*/", i + 2)
            i = n if j < 0 else j + 2
            continue
        if c in ('"', "'"):
            quote = c
            i += 1
            while i < n:
                if src[i] == "\\":
                    i += 2
                    continue
                if src[i] == quote:
                    break
                i += 1
        elif c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    raise ValueError("方法体花括号不配对，模板可能已变化，请人工确认")


def patch_chrome_client_src(src: str) -> str:
    """对 RustWebChromeClient.kt 打全屏补丁；已打过则原样返回；模板漂移显式报错。"""
    if MARKER in src:
        return src

    show = ON_SHOW.search(src)
    if not show:
        raise ValueError(
            "未找到 onShowCustomView(view: View, callback: CustomViewCallback) 签名，"
            "wry 模板可能已升级，请人工确认全屏回调形态后更新本补丁"
        )
    show_brace = src.index("{", show.end() - 1)
    show_end = _block_end(src, show_brace)
    show_body = src[show_brace + 1 : show_end]
    # 漂移保险：只有 wry 的「立即取消」桩实现才允许整段替换。上游若哪天实现了真全屏，
    # 这里直接报错让人重新评估，而不是静默把上游实现换成我们的。
    if "onCustomViewHidden()" not in show_body:
        raise ValueError(
            "onShowCustomView 已不是 wry 0.57 的「立即取消」空实现（体内无 onCustomViewHidden），"
            "模板可能已自带全屏逻辑，请人工确认后更新本补丁"
        )

    hide = ON_HIDE.search(src, show_end)
    if not hide:
        raise ValueError(
            "未找到 onHideCustomView() 签名，wry 模板可能已升级，请人工确认后更新本补丁"
        )
    hide_brace = src.index("{", hide.end() - 1)
    hide_end = _block_end(src, hide_brace)

    src = _add_imports(src, NEEDED_IMPORTS)
    # 加 import 会改变下标，两个匹配全部重算（替换区间文本不变，重搜最稳）。
    show = ON_SHOW.search(src)
    show_end = _block_end(src, src.index("{", show.end() - 1))
    hide = ON_HIDE.search(src, show_end)
    hide_end = _block_end(src, src.index("{", hide.end() - 1))
    return src[: show.start()] + FULLSCREEN_BLOCK + src[hide_end + 1 :]


# ---------------------------------------------------------------------------
# 自测
# ---------------------------------------------------------------------------
# 仿真实模板的文件头 + 两个桩方法 + 后续一个真实方法（验证替换不误伤后续代码）。
PRISTINE = (
    "// Copyright 2020-2023 Tauri Programme within The Commons Conservancy\n"
    "// SPDX-License-Identifier: Apache-2.0\n"
    "\n"
    "package com.web2app.app\n"
    "\n"
    "import android.app.Activity\n"
    "import android.os.Build\n"
    "import android.view.View\n"
    "import android.webkit.*\n"
    "import java.util.*\n"
    "\n"
    "class RustWebChromeClient(private val activity: WryActivity, private val webViewId: String)"
    " : WebChromeClient() {\n"
    "  private companion object {\n"
    "    const val PERMISSION_REQUEST_DEFAULT = 0\n"
    "  }\n"
    "\n"
    "  /**\n"
    "   * Render web content in `view`.\n"
    "   */\n"
    "  override fun onShowCustomView(view: View, callback: CustomViewCallback) {\n"
    "    callback.onCustomViewHidden()\n"
    "    super.onShowCustomView(view, callback)\n"
    "  }\n"
    "\n"
    "  override fun onHideCustomView() {\n"
    "    super.onHideCustomView()\n"
    "  }\n"
    "\n"
    "  override fun onPermissionRequest(request: PermissionRequest) {\n"
    "    request.deny()\n"
    "  }\n"
    "}\n"
)


def selftest() -> int:
    # 1) 原始模板：要素齐全
    patched = patch_chrome_client_src(PRISTINE)
    for needle in (
        MARKER,
        "private var web2appCustomView: View? = null",
        "override fun onShowCustomView(view: View, callback: CustomViewCallback)",
        "override fun onHideCustomView()",
        "activity.findViewById<ViewGroup>(android.R.id.content)",
        "content.addView(",
        "FrameLayout.LayoutParams(",
        "ViewGroup.LayoutParams.MATCH_PARENT",
        "content?.removeView(customView)",
        "View.GONE",
        "View.VISIBLE",
        "ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE",
        "web2appSavedOrientation",
        "android.view.WindowInsets.Type.systemBars()",
        "WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE",
        "WindowInsetsController.BEHAVIOR_DEFAULT",
        "View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY",
        "View.SYSTEM_UI_FLAG_VISIBLE",
        "Build.VERSION_CODES.R",
        "callback?.onCustomViewHidden()",
    ):
        assert needle in patched, f"补丁缺少要素：{needle}"
    for imp in NEEDED_IMPORTS:
        assert imp in patched, f"缺少 import：{imp}"
    # 后续真实方法与文件头必须保留
    assert "override fun onPermissionRequest(request: PermissionRequest)" in patched
    assert "class RustWebChromeClient" in patched
    assert patched.rstrip().endswith("}")
    # 两个回调各只定义一次
    assert patched.count("override fun onShowCustomView") == 1
    assert patched.count("override fun onHideCustomView") == 1

    # 2) 原「立即取消」语义必须消失：不允许再出现 紧跟 super 的 onCustomViewHidden
    stub = "callback.onCustomViewHidden()\n    super.onShowCustomView(view, callback)"
    assert stub not in patched, "原 onShowCustomView 立即取消桩必须被整体替换"
    # 新实现只在「拒绝重复全屏」与「退出收尾」两条路径调用回调，不是一进来就取消
    assert "web2appCustomView != null" in patched

    # 3) 幂等 + import 不重复
    again = patch_chrome_client_src(patched)
    assert again == patched, "全屏补丁不是幂等的"
    for imp in NEEDED_IMPORTS:
        assert again.count(imp + "\n") == 1, f"import 被重复注入：{imp}"

    # 4) 缺方法 → 报错
    no_show = PRISTINE.replace(
        "  override fun onShowCustomView(view: View, callback: CustomViewCallback) {\n"
        "    callback.onCustomViewHidden()\n"
        "    super.onShowCustomView(view, callback)\n"
        "  }\n",
        "",
    )
    for bad in (no_show, "package com.x\n\nclass C\n"):
        try:
            patch_chrome_client_src(bad)
            raise AssertionError("缺 onShowCustomView 应报错")
        except ValueError:
            pass
    no_hide = PRISTINE.replace(
        "  override fun onHideCustomView() {\n    super.onHideCustomView()\n  }\n", ""
    )
    try:
        patch_chrome_client_src(no_hide)
        raise AssertionError("缺 onHideCustomView 应报错")
    except ValueError:
        pass

    # 5) 模板漂移：上游已实现真全屏（体内无 onCustomViewHidden）→ 报错，不静默替换
    drifted = PRISTINE.replace(
        "    callback.onCustomViewHidden()\n"
        "    super.onShowCustomView(view, callback)\n",
        "    val content = activity.findViewById<ViewGroup>(android.R.id.content)\n"
        "    content.addView(view)\n",
    )
    try:
        patch_chrome_client_src(drifted)
        raise AssertionError("非桩实现应触发漂移报错")
    except ValueError as e:
        assert "立即取消" in str(e)

    # 6) 文件级集成：临时 gen/android 深包名目录，经 find_kotlin_file 走一遍
    import tempfile

    with tempfile.TemporaryDirectory() as tmp:
        gen_root = pathlib.Path(tmp) / "gen" / "android"
        kt_dir = gen_root / "app/src/main/java/com/example/foo"
        kt_dir.mkdir(parents=True)
        kt = kt_dir / "RustWebChromeClient.kt"
        kt.write_text(PRISTINE, encoding="utf-8")
        path = find_kotlin_file(gen_root, "RustWebChromeClient.kt")
        path.write_text(patch_chrome_client_src(path.read_text(encoding="utf-8")),
                        encoding="utf-8", newline="\n")
        out = path.read_text(encoding="utf-8")
        assert MARKER in out
        assert patch_chrome_client_src(out) == out, "文件级二次运行不幂等"

    print("patch-android-fullscreen.py 自测通过 ✓（6 组场景）")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="给 Android 注入 H5 视频全屏回调实现")
    parser.add_argument(
        "--gen-root",
        default=None,
        help="Android 工程根（缺省按候选自动探测：" + gen_root_candidates_str() + "）",
    )
    parser.add_argument("--selftest", action="store_true", help="运行离线自测")
    args = parser.parse_args()

    if args.selftest:
        return selftest()

    gen_root = resolve_gen_root(args.gen_root)
    target = find_kotlin_file(gen_root, "RustWebChromeClient.kt")
    src = target.read_text(encoding="utf-8")
    patched = patch_chrome_client_src(src)
    if patched == src:
        print(f"{target} 已包含全屏回调实现，跳过")
    else:
        target.write_text(patched, encoding="utf-8", newline="\n")
        print(f"已给 {target} 注入 H5 全屏实现（custom view 铺满 + 横屏 + 沉浸式）")
    return 0


if __name__ == "__main__":
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except Exception:  # noqa: BLE001
            pass
    sys.exit(main())
