#!/usr/bin/env python3
"""Android 返回键规则补丁：给 MainActivity.kt 注入「不允许退回到内置页」的返回处理。

## 背景（为什么需要）

Tauri 在 Android 上的返回键由框架内 AppPlugin 的 OnBackPressedCallback 处理：
只要 `WebView.canGoBack()` 为真就 `goBack()`，否则才走系统默认（结束当前页）。
而本应用启动时先加载内置 splash 启动页（`http://splash.localhost/`，由 wry 的
custom_protocol_workaround 改写而成），随后再导航到目标网址 —— splash 因此成为
历史中的上一条 ⇒ 在首个目标页按返回键，会被带回到内置启动页。

本补丁在 `onWebViewCreate`（WryActivity 的 open 钩子，webview 创建时回调）里
再注册一个 OnBackPressedCallback：它注册更晚、在 dispatcher 中优先级更高，
按 `WebView.copyBackForwardList()` 的相邻条目 URL 判定 —— 落点是真实网页才
`goBack()`；没有可退的真实页面时直接 `finish()`（与「无历史时系统默认」同
手感：回桌面 / 后台），绝不停在内置页。

gen/android 每次构建现生成（见 build-custom.yml / release.yml 的
`cargo t android init` 步骤），所以两个 workflow 都必须在 init 之后调用本脚本。

用法：
    python3 .github/scripts/patch-android-back.py            # 打补丁（默认 webview-app/gen/android）
    python3 .github/scripts/patch-android-back.py --selftest # 离线自测
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys
import tempfile

# 幂等标记：已打过补丁的文件不再重复注入。
MARKER = "web2app-android-back-guard"

# 需要补的 import（缺失才加）。
NEEDED_IMPORTS = (
    "import android.webkit.WebView",
    "import androidx.activity.OnBackPressedCallback",
)

# 注入到类体内的 Kotlin 代码（2 空格缩进，对齐 Tauri 模板）。
# 前缀 class_indent 由调用点补上（模板里 class 不缩进，前缀为空）。
KOTLIN_BLOCK = """  // {marker}：接管系统返回键。
  // Tauri 自带的 AppPlugin 只认 WebView.canGoBack()，而启动时内置 splash 启动页
  //（http://splash.localhost/）就在历史里 ⇒ 首个目标页按返回会退回到内置页。
  // 本回调注册更晚（onWebViewCreate）、在 OnBackPressedDispatcher 中优先级更高，
  // 按 copyBackForwardList 的相邻条目 URL 判定：落点是真实网页才 goBack；
  // 否则 finish()（与「无历史时系统默认」同手感：回桌面/后台），绝不停在内置页。
  override fun onWebViewCreate(webView: WebView) {{
    super.onWebViewCreate(webView)
    onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {{
      override fun handleOnBackPressed() {{
        val list = webView.copyBackForwardList()
        val idx = list.currentIndex
        if (idx > 0) {{
          val prevUrl = list.getItemAtIndex(idx - 1)?.url ?: ""
          if (!isWeb2appBuiltInPage(prevUrl)) {{
            webView.goBack()
            return
          }}
        }}
        // 没有可退的真实页面：不触发后退，直接 finish。
        // 不能「关闭本回调后重分发」：AppPlugin 的回调仍启用，会把 splash 当可后退页。
        finish()
      }}
    }})
  }}

  private fun isWeb2appBuiltInPage(url: String): Boolean {{
    return url.startsWith("splash:") ||
      url.startsWith("http://splash.localhost") ||
      url.startsWith("https://splash.localhost")
  }}
""".format(marker=MARKER)

# MainActivity 类声明行（容忍 public 修饰与行尾其它字符），注入点紧跟其后。
# ⚠️ 缩进捕获只用 [ \t]*：\s* 贪婪且 \s 含换行，在空行位置会把上一行末尾的 \n
# 吃进缩进组，导致注入的每一行都多出一个空行（2026-10-09 调试取证）。
CLASS_LINE = re.compile(r"^([ \t]*)(?:public\s+)?class\s+MainActivity\b[^\n]*\{", re.M)
IMPORT_LINE = re.compile(r"^import\s+.*$", re.M)


def patch_src(src: str) -> str:
    """对 MainActivity.kt 源码打补丁；已打过则原样返回。"""
    if MARKER in src:
        return src

    m = CLASS_LINE.search(src)
    if not m:
        raise ValueError("未找到 `class MainActivity ... {{` 声明行，模板可能已变化，请人工确认")

    # 1) 补 import：插到最后一条 import 之后；没有 import 就插在 package 行之后。
    missing = [imp for imp in NEEDED_IMPORTS if imp not in src]
    if missing:
        imports = list(IMPORT_LINE.finditer(src))
        if imports:
            pos = imports[-1].end()
            inject = "\n" + "\n".join(missing)
        else:
            pkg = re.search(r"^package\s+.*$", src, re.M)
            if not pkg:
                raise ValueError("源码中既无 import 也无 package 声明，无法确定注入位置")
            pos = pkg.end()
            inject = "\n\n" + "\n".join(missing)
        src = src[:pos] + inject + src[pos:]
        # 上面的切片可能使类行的匹配位置失效，重新定位。
        m = CLASS_LINE.search(src)

    # 2) 注入类体代码：放在类声明行的下一行，整体按 class 缩进对齐。
    class_indent = m.group(1)
    block = "\n".join(
        (class_indent + line) if line.strip() else line
        for line in KOTLIN_BLOCK.rstrip("\n").splitlines()
    )
    insert_at = m.end()
    # 类声明与守卫之间、守卫与原 onCreate 之间各留一个空行（tail 以 \n 起首）。
    return src[:insert_at] + "\n\n" + block + "\n" + src[insert_at:]


def find_main_activity(gen_root: pathlib.Path) -> pathlib.Path:
    """在 gen/android 工程下定位 MainActivity.kt。"""
    java_root = gen_root / "app" / "src" / "main" / "java"
    candidates = sorted(java_root.rglob("MainActivity.kt")) if java_root.exists() else []
    if not candidates:
        raise FileNotFoundError(
            f"未找到 {java_root}/**/MainActivity.kt（请确认已先执行 cargo tauri android init）"
        )
    return candidates[0]


def selftest() -> int:
    """离线自测：用合成源码验证补丁的正确性与幂等性。"""
    pristine = (
        "package com.web2app.app\n\n"
        "import android.os.Bundle\n"
        "import androidx.activity.enableEdgeToEdge\n\n"
        "class MainActivity : TauriActivity() {\n"
        "  override fun onCreate(savedInstanceState: Bundle?) {\n"
        "    enableEdgeToEdge()\n"
        "    super.onCreate(savedInstanceState)\n"
        "  }\n"
        "}\n"
    )

    # 1) 原始模板 → 注入成功且要素齐全
    patched = patch_src(pristine)
    for needle in (
        MARKER,
        "override fun onWebViewCreate(webView: WebView)",
        "onBackPressedDispatcher.addCallback(this",
        "webView.copyBackForwardList()",
        "list.currentIndex",
        "isWeb2appBuiltInPage(prevUrl)",
        "finish()",
        NEEDED_IMPORTS[0],
        NEEDED_IMPORTS[1],
    ):
        assert needle in patched, f"补丁缺少要素：{needle}"
    # 原有 onCreate 必须原样保留
    assert "enableEdgeToEdge()" in patched and "super.onCreate(savedInstanceState)" in patched

    # 2) 幂等：再打一次结果逐字不变
    assert patch_src(patched) == patched, "补丁不是幂等的"

    # 3) 注入位置：紧跟类声明行之后（在 onCreate 之前）
    cls_pos = patched.index("class MainActivity")
    guard_pos = patched.index(MARKER)
    oncreate_pos = patched.index("override fun onCreate")
    assert cls_pos < guard_pos < oncreate_pos, "守卫代码没有紧跟类声明行"

    # 4) 无 import / package 形态（防御）：仍能注入
    no_imports = "class MainActivity : TauriActivity() {\n}\n"
    try:
        patch_src(no_imports)
        raised = False
    except ValueError:
        raised = True
    assert raised, "既无 import 也无 package 时应报错"

    # 5) 找不到类行 → 报错
    try:
        patch_src("package com.x\n\n// nothing here\n")
        raised = False
    except ValueError:
        raised = True
    assert raised, "找不到 MainActivity 类行时应报错"

    # 6) 文件级补丁：写入后内容即补丁结果，且 find_main_activity 能定位
    with tempfile.TemporaryDirectory() as tmp:
        gen_root = pathlib.Path(tmp) / "gen" / "android"
        kt = gen_root / "app/src/main/java/com/web2app/app/MainActivity.kt"
        kt.parent.mkdir(parents=True)
        kt.write_text(pristine, encoding="utf-8")
        path = find_main_activity(gen_root)
        path.write_text(patch_src(path.read_text(encoding="utf-8")), encoding="utf-8")
        assert MARKER in path.read_text(encoding="utf-8")
        try:
            find_main_activity(pathlib.Path(tmp) / "missing")
            missing_raised = False
        except FileNotFoundError:
            missing_raised = True
        assert missing_raised, "工程缺失时应报错"

    print("patch-android-back.py 自测通过 ✓（6 组场景）")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="给 Android MainActivity 注入返回键守卫")
    parser.add_argument(
        "--gen-root",
        default="webview-app/gen/android",
        help="Android 工程根（默认 webview-app/gen/android）",
    )
    parser.add_argument("--selftest", action="store_true", help="运行离线自测")
    args = parser.parse_args()

    if args.selftest:
        return selftest()

    gen_root = pathlib.Path(args.gen_root)
    path = find_main_activity(gen_root)
    src = path.read_text(encoding="utf-8")
    patched = patch_src(src)
    if patched == src:
        print(f"{path} 已包含返回键守卫，跳过")
    else:
        path.write_text(patched, encoding="utf-8", newline="\n")
        print(f"已给 {path} 注入返回键守卫（内置页不可后退，无真实历史则 finish）")
    return 0


if __name__ == "__main__":
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except Exception:  # noqa: BLE001
            pass
    sys.exit(main())
