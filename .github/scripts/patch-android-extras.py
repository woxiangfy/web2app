#!/usr/bin/env python3
"""Android「更多」能力补丁：给 WebView 挂 @JavascriptInterface 桥。

## 背景（为什么需要）

移动端底栏「更多」面板（src/mobilebar.js，2026-10-14 起）有四个动作在**页面里做不到**：

  · 系统分享（Intent.ACTION_SEND + 选择器）—— WebView 不实现 navigator.share；
  · 在系统浏览器打开当前页 —— 只能靠原生 Intent(ACTION_VIEW)；
  · 清除全部浏览数据 —— wry 0.57 在 tauri Android 侧拿不到 WebView 句柄
    （tauri 的 PlatformWebview 在 Android 只暴露 jni_handle()，没有 clear 入口），
    且 wry 自己的 Kotlin `clearAllBrowsingData()` **漏清 Cookie 与 HTML5 存储**
    （只有 clearCache/clearHistory/clearFormData/deleteDatabase），不清 Cookie
    的「清除浏览数据」是假清除（登录态还在）；
  · 退出应用 —— finishAndRemoveTask + 杀进程。

本补丁改两个生成工程文件：

1. `MainActivity.kt`
   · 在既有的 `onWebViewCreate` 钩子里插一行 `web2appSetupExtras(webView)`
     （该 override 由 patch-android-back.py 新建、patch-android-download.py 复用；
     不存在时本脚本也能自补完整 override）；
   · 注入 setup 方法：`addJavascriptInterface(object { … }, "__web2appAndroid")`，
     桥方法 share / openBrowser / clearData / quitApp；
   · 文件尾注入顶层 `object Web2appBridge`（当页密钥持有者）。

2. wry 的 `RustWebViewClient.kt`（按文件名 rglob 定位，包名随 identifier 变化）
   · 在 `onPageFinished` 方法体首行轮换密钥并 evaluateJavascript 注入
     `window.__web2appAndroidKey`。evaluateJavascript 只作用于**主框架**，
     跨源 iframe 读不到顶层变量（跨源 window 访问直接抛错）⇒ 第三方广告 frame
     无法伪造调用清数据 / 退出；同源 frame 与主页面同属被封装站点，等同可信。
     所有桥方法第一件事都是核密钥，不匹配直接 return。

gen/android 每次构建现生成，两个 workflow 都必须在 init 之后、按
insets → back → download → extras → schemes 顺序调用本脚本。

用法：
    python3 .github/scripts/patch-android-extras.py            # 打补丁（工程根自动探测）
    python3 .github/scripts/patch-android-extras.py --selftest # 离线自测
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys

from android_gen import (  # noqa: E402
    find_kotlin_file,
    find_main_activity,
    gen_root_candidates_str,
    resolve_gen_root,
)

# 幂等标记（MainActivity 侧）。
MARKER = "web2app-android-extras"
# 幂等标记（RustWebViewClient 侧；与 MainActivity 标记分开，两处可独立识别）。
CLIENT_MARKER = "web2app-android-bridge-key"
# 暴露给页面的桥名 / 密钥变量名（也是 mobilebar.js 里逐字依赖的契约，改这里必须同步 JS）。
BRIDGE_NAME = "__web2appAndroid"
KEY_VAR = "__web2appAndroidKey"

NEEDED_IMPORTS = (
    "import android.annotation.SuppressLint",
    "import android.app.Activity",
    "import android.content.Intent",
    "import android.webkit.CookieManager",
    "import android.webkit.JavascriptInterface",
    "import android.webkit.WebStorage",
    "import android.webkit.WebView",
    "import android.widget.Toast",
    "import java.util.UUID",
)

IMPORT_LINE = re.compile(r"^import\s+.*$", re.M)
PACKAGE_LINE = re.compile(r"^package\s+.*$", re.M)
CLASS_LINE = re.compile(r"^([ \t]*)(?:public\s+)?class\s+MainActivity\b[^\n]*\{", re.M)
ON_WEBVIEW_CREATE = re.compile(
    r"^([ \t]*)override\s+fun\s+onWebViewCreate\s*\(\s*(\w+)\s*:\s*WebView\s*\)\s*\{",
    re.M,
)
SUPER_ON_WEBVIEW_CREATE = re.compile(r"super\.onWebViewCreate\s*\([^)]*\)")

# wry 0.57 模板里 onPageFinished 是老签名（无 WebResourceRequest）⇒ 只可能是主框架回调。
ON_PAGE_FINISHED = re.compile(
    r"^([ \t]*)override\s+fun\s+onPageFinished\s*\(\s*"
    r"(\w+)\s*:\s*WebView\s*,\s*(\w+)\s*:\s*String\??\s*\)\s*\{",
    re.M,
)


def _add_imports(src: str, needed: tuple[str, ...]) -> str:
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


def _class_closing_brace(src: str, class_match: re.Match[str]) -> int:
    """从 class 声明行的 `{` 起花括号配对，返回类体闭合 `}` 的下标（朴素扫描，跳过注释/字符串）。"""
    depth = 0
    i = class_match.end() - 1
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
    raise ValueError("MainActivity 类体花括号不配对，模板可能已变化，请人工确认")


def _call_line(param: str, indent: str) -> str:
    return (
        f"{indent}// {MARKER}：JS 桥（分享 / 浏览器打开 / 清数据 / 退出）。\n"
        f"{indent}web2appSetupExtras({param})\n"
    )


def _full_override() -> str:
    return (
        f"  // {MARKER}：onWebViewCreate 里挂 JS 桥。\n"
        "  override fun onWebViewCreate(webView: WebView) {\n"
        "    super.onWebViewCreate(webView)\n"
        "    web2appSetupExtras(webView)\n"
        "  }\n"
    )


# 桥实现（类成员；2 空格类成员缩进，与 back/download 补丁对齐）。
METHODS_BLOCK = """  // {marker}：JS 桥（见 patch-android-extras.py）。所有方法先核当页密钥，
  // 且全部从 binder 线程 post 回 UI 线程执行（@JavascriptInterface 不在主线程回调）。
  @SuppressLint("JavascriptInterface")
  private fun web2appSetupExtras(webView: WebView) {{
    webView.addJavascriptInterface(object {{
      // 系统分享面板：把当前页链接作为 text/plain 分享出去。
      @JavascriptInterface
      fun share(key: String?, url: String?, title: String?) {{
        if (!web2appBridgeOk(key) || url.isNullOrEmpty() ||
          (!url.startsWith("http://") && !url.startsWith("https://"))
        ) {{
          return
        }}
        webView.post {{
          try {{
            val send = Intent(Intent.ACTION_SEND)
            send.type = "text/plain"
            send.putExtra(Intent.EXTRA_TEXT, url)
            if (!title.isNullOrEmpty()) send.putExtra(Intent.EXTRA_SUBJECT, title)
            send.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            val chooser = Intent.createChooser(send, "分享到")
            chooser.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            webView.context.startActivity(chooser)
          }} catch (e: Exception) {{
            Toast.makeText(webView.context, "没有可用于分享的应用", Toast.LENGTH_SHORT).show()
          }}
        }}
      }}

      // 在系统浏览器（而不是 App 内 WebView）打开当前页。
      @JavascriptInterface
      fun openBrowser(key: String?, url: String?) {{
        if (!web2appBridgeOk(key) || url.isNullOrEmpty() ||
          (!url.startsWith("http://") && !url.startsWith("https://"))
        ) {{
          return
        }}
        webView.post {{
          try {{
            val intent = Intent(Intent.ACTION_VIEW, android.net.Uri.parse(url))
            intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            webView.context.startActivity(intent)
          }} catch (e: Exception) {{
            Toast.makeText(webView.context, "未安装浏览器，无法打开", Toast.LENGTH_SHORT).show()
          }}
        }}
      }}

      // 清除全部浏览数据：wry 0.57 自带的 clearAllBrowsingData 漏清 Cookie / HTML5 存储，
      // 这里按完整配方补全；Cookie 清完在回调里 reload，用户立刻看到登录态已失效。
      @JavascriptInterface
      fun clearData(key: String?) {{
        if (!web2appBridgeOk(key)) return
        webView.post {{
          try {{ webView.clearCache(true) }} catch (e: Exception) {{}}
          try {{ webView.clearHistory() }} catch (e: Exception) {{}}
          try {{ webView.clearFormData() }} catch (e: Exception) {{}}
          try {{
            webView.context.deleteDatabase("webviewCache.db")
            webView.context.deleteDatabase("webview.db")
          }} catch (e: Exception) {{}}
          try {{ WebStorage.getInstance().deleteAllData() }} catch (e: Exception) {{}}
          CookieManager.getInstance().removeAllCookies {{
            try {{ CookieManager.getInstance().flush() }} catch (e: Exception) {{}}
            webView.post {{
              try {{
                webView.evaluateJavascript("try{{location.reload();}}catch(e){{}}", null)
              }} catch (e: Exception) {{}}
            }}
          }}
        }}
      }}

      // 退出应用：先把任务从最近任务列表移除，再杀进程（否则空进程可能被系统短暂保留）。
      @JavascriptInterface
      fun quitApp(key: String?) {{
        if (!web2appBridgeOk(key)) return
        webView.post {{
          try {{
            (webView.context as? Activity)?.finishAndRemoveTask()
          }} catch (e: Exception) {{}}
          webView.postDelayed({{
            android.os.Process.killProcess(android.os.Process.myPid())
          }}, 300)
        }}
      }}

      private fun web2appBridgeOk(key: String?): Boolean =
        !key.isNullOrEmpty() && key == Web2appBridge.key
    }}, "{bridge}")
    // 给当前文档立刻补一次密钥（之后每次主框架 onPageFinished 都会轮换）。
    val keyNow = Web2appBridge.rotate()
    webView.evaluateJavascript("window.{key_var}='$keyNow';", null)
  }}
""".format(marker=MARKER, bridge=BRIDGE_NAME, key_var=KEY_VAR)

# 顶层 object（必须在 class 之外；插在类闭合 } 之后）。
HOLDER_BLOCK = """
// {marker}：JS 桥的当页密钥。主框架每次 onPageFinished 轮换一次
// （patch-android-extras.py 同时补丁 RustWebViewClient），跨源 iframe 拿不到 ⇒ 无法调桥。
object Web2appBridge {{
  @Volatile
  var key: String = ""

  fun rotate(): String {{
    val k = UUID.randomUUID().toString()
    key = k
    return k
  }}
}}
""".format(marker=MARKER)


def patch_activity_src(src: str) -> str:
    """对 MainActivity.kt 打补丁；已打过则原样返回。"""
    if MARKER in src:
        return src

    src = _add_imports(src, NEEDED_IMPORTS)

    class_m = CLASS_LINE.search(src)
    if not class_m:
        raise ValueError("未找到 `class MainActivity ... {` 声明行，模板可能已变化，请人工确认")

    existing = ON_WEBVIEW_CREATE.search(src)
    if existing:
        # 复用 back/download 已建的 override。CI 顺序固定 insets → back → download → extras：
        # extras 的调用必须排在 download 调用【之后】（同函数体里不能插到 super 与 download
        # 之间，否则顺序契约被破坏）。找不到 download 调用时退回 super 之后。
        body_start = existing.end()
        body_end = _class_closing_brace(src, class_m)
        param = existing.group(2)
        anchor = re.search(
            rf"web2appSetupDownloads\s*\(\s*{re.escape(param)}\s*\)",
            src[body_start:body_end],
        )
        if anchor:
            line_end = src.find("\n", body_start + anchor.end())
            insert_at = body_start + anchor.end() if line_end < 0 else line_end
        else:
            sup = SUPER_ON_WEBVIEW_CREATE.search(src, body_start)
            if sup and sup.start() < body_end:
                line_end = src.find("\n", sup.end())
                insert_at = body_start if line_end < 0 else line_end
            else:
                insert_at = body_start
        indent = existing.group(1) + "  "
        src = src[:insert_at] + "\n" + _call_line(existing.group(2), indent).rstrip("\n") + src[insert_at:]
    else:
        insert_at = class_m.end()
        src = src[:insert_at] + "\n\n" + _full_override().rstrip("\n") + src[insert_at:]

    # 桥方法插在类体闭合 } 之前；密钥 object 是顶层声明，插在类闭合 } 之后。
    class_m = CLASS_LINE.search(src)
    close = _class_closing_brace(src, class_m)
    src = src[:close] + "\n" + METHODS_BLOCK.rstrip("\n") + "\n" + src[close:]
    class_m = CLASS_LINE.search(src)
    close = _class_closing_brace(src, class_m)
    tail = src[close + 1 :]
    src = src[: close + 1] + "\n" + HOLDER_BLOCK.strip("\n") + "\n" + tail
    return src


def patch_client_src(src: str) -> str:
    """对 wry 的 RustWebViewClient.kt 打补丁：主框架 onPageFinished 轮换桥密钥。"""
    if CLIENT_MARKER in src:
        return src

    m = ON_PAGE_FINISHED.search(src)
    if not m:
        raise ValueError(
            "未找到 RustWebViewClient.onPageFinished(view: WebView, url: String)，"
            "wry 模板可能已变化，请人工确认"
        )
    indent = m.group(1) + "    "  # 方法声明 4 空格 + 函数体一级 4 空格 = 8 空格
    view_param = m.group(2)
    block = (
        f"\n{indent}// {CLIENT_MARKER}：主框架页面加载完成 → 轮换 JS 桥密钥。\n"
        f"{indent}// evaluateJavascript 只注入主框架；跨源 iframe 拿不到 {KEY_VAR}，调不到原生桥。\n"
        f'{indent}val web2appKey = Web2appBridge.rotate()\n'
        f'{indent}{view_param}.evaluateJavascript('
        f'"window.{KEY_VAR}=\'$web2appKey\';", null)\n'
    )
    return src[: m.end()] + block + src[m.end() :]


# ---------------------------------------------------------------------------
# 自测
# ---------------------------------------------------------------------------
PRISTINE = (
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

# 模拟 back + download 补丁都先跑过的形态（真实 CI 顺序里 extras 排第四）。
POST_DOWNLOAD = (
    "package com.web2app.app\n\n"
    "import android.webkit.WebView\n"
    "import android.webkit.CookieManager\n\n"
    "class MainActivity : TauriActivity() {\n"
    "  // web2app-android-back-guard\n"
    "  override fun onWebViewCreate(webView: WebView) {\n"
    "    super.onWebViewCreate(webView)\n"
    "    // web2app-android-download：网页下载 / 长按存图统一交给系统 DownloadManager。\n"
    "    web2appSetupDownloads(webView)\n"
    "  }\n"
    "}\n"
)

CLIENT_PRISTINE = (
    "package com.web2app.app\n\n"
    "import android.net.Uri\n"
    "import android.webkit.*\n\n"
    "class RustWebViewClient(webView: RustWebView, context: Context): WebViewClient() {\n"
    "    override fun onPageFinished(view: WebView, url: String) {\n"
    "        Rust.onPageLoaded((view as RustWebView).id, url)\n"
    "    }\n"
    "\n"
    "    override fun shouldOverrideUrlLoading(\n"
    "        view: WebView,\n"
    "        request: WebResourceRequest\n"
    "    ): Boolean {\n"
    "        return Rust.shouldOverride((view as RustWebView).id, request.url.toString())\n"
    "    }\n"
    "}\n"
)

# schemes 补丁先跑过的形态：守卫 + 辅助方法都在。
POST_SCHEMES = (
    "package com.web2app.app\n\n"
    "class RustWebViewClient(webView: RustWebView, context: Context): WebViewClient() {\n"
    "    private fun web2appHandleExternalScheme(view: WebView, uri: Uri): Boolean {\n"
    "        return true\n"
    "    }\n"
    "\n"
    "    override fun shouldOverrideUrlLoading(\n"
    "        view: WebView,\n"
    "        request: WebResourceRequest\n"
    "    ): Boolean {\n"
    "        // web2app-android-external-scheme：非网页 scheme 先交系统应用处理。\n"
    "        if (web2appHandleExternalScheme(view, request.url)) return true\n"
    "        return Rust.shouldOverride((view as RustWebView).id, request.url.toString())\n"
    "    }\n"
    "\n"
    "    override fun onPageFinished(view: WebView, url: String) {\n"
    "        Rust.onPageLoaded((view as RustWebView).id, url)\n"
    "    }\n"
    "}\n"
)


def selftest() -> int:
    # 1) 原始 MainActivity → 全要素
    patched = patch_activity_src(PRISTINE)
    for needle in (
        MARKER,
        "web2appSetupExtras(webView)",
        'addJavascriptInterface(object',
        f'"{BRIDGE_NAME}"',
        "@JavascriptInterface",
        "fun share(key: String?, url: String?, title: String?)",
        "fun openBrowser(key: String?, url: String?)",
        "fun clearData(key: String?)",
        "fun quitApp(key: String?)",
        "Intent.ACTION_SEND",
        "Intent.createChooser(send, \"分享到\")",
        "Intent.ACTION_VIEW",
        "WebStorage.getInstance().deleteAllData()",
        "CookieManager.getInstance().removeAllCookies",
        "finishAndRemoveTask()",
        "android.os.Process.killProcess(android.os.Process.myPid())",
        "object Web2appBridge",
        "fun rotate(): String",
        "UUID.randomUUID().toString()",
        f"window.{KEY_VAR}=",
        "web2appBridgeOk(key)",
        "override fun onWebViewCreate(webView: WebView)",
        "super.onWebViewCreate(webView)",
    ):
        assert needle in patched, f"补丁缺少要素：{needle}"
    for imp in NEEDED_IMPORTS:
        assert imp in patched, f"缺少 import：{imp}"
    # 顶层 object 必须在 class 之外：精确花括号配对找到类闭合位置再比较
    class_m = CLASS_LINE.search(patched)
    class_close_idx = _class_closing_brace(patched, class_m)
    assert patched.index("object Web2appBridge") > class_close_idx, (
        "Web2appBridge 必须是顶层 object（class 外）"
    )
    assert patched.count("class MainActivity") == 1
    # onCreate 原样保留
    assert "enableEdgeToEdge()" in patched and "super.onCreate(savedInstanceState)" in patched

    # 2) 幂等
    assert patch_activity_src(patched) == patched, "MainActivity 补丁不是幂等的"

    # 3) back+download 先跑过：只插一行调用，不新增第二个 override，且排在 download 之后
    post = patch_activity_src(POST_DOWNLOAD)
    assert post.count("override fun onWebViewCreate") == 1, "不得重复定义 onWebViewCreate"
    assert post.count("web2appSetupExtras(webView)") == 1
    idx_super = post.index("super.onWebViewCreate(webView)")
    idx_dl = post.index("web2appSetupDownloads(webView)")
    idx_ex = post.index("web2appSetupExtras(webView)")
    assert idx_super < idx_dl < idx_ex, "extras 调用必须在 super 与 download 调用之后"
    assert "web2app-android-back-guard" in post and "web2app-android-download" in post
    assert post.count("import android.webkit.WebView\n") == 1
    assert post.count("import android.webkit.CookieManager\n") == 1
    assert patch_activity_src(post) == post

    # 4) 畸形源码报错
    try:
        patch_activity_src("package com.x\n\n// nothing\n")
        raise AssertionError("畸形源码应报错")
    except ValueError:
        pass

    # 5) RustWebViewClient：注入位置在 Rust.onPageLoaded 之前；幂等；模板漂移报错
    c = patch_client_src(CLIENT_PRISTINE)
    assert CLIENT_MARKER in c
    assert "Web2appBridge.rotate()" in c
    assert f"window.{KEY_VAR}=" in c
    idx_rotate = c.index("Web2appBridge.rotate()")
    idx_loaded = c.index("Rust.onPageLoaded")
    assert idx_rotate < idx_loaded, "密钥轮换必须在 Rust.onPageLoaded 之前（方法体首行）"
    # shouldOverride 原样保留
    assert "return Rust.shouldOverride((view as RustWebView).id, request.url.toString())" in c
    assert patch_client_src(c) == c, "Client 补丁不是幂等的"

    # schemes 先跑过：两处补丁共存，schemes 守卫与辅助方法不被破坏
    cs = patch_client_src(POST_SCHEMES)
    assert "web2app-android-external-scheme" in cs
    assert "web2appHandleExternalScheme(view, request.url)" in cs
    assert cs.count("override fun onPageFinished") == 1
    assert cs.count("override fun shouldOverrideUrlLoading") == 1
    assert patch_client_src(cs) == cs

    try:
        patch_client_src("class RustWebViewClient {\n}\n")
        raise AssertionError("缺 onPageFinished 应报错")
    except ValueError:
        pass

    # 6) 文件级集成：临时 gen/android 工程，经 find_main_activity / find_kotlin_file 走一遍
    import tempfile

    with tempfile.TemporaryDirectory() as tmp:
        gen_root = pathlib.Path(tmp) / "gen" / "android"
        kt = gen_root / "app/src/main/java/com/web2app/app/MainActivity.kt"
        kt.parent.mkdir(parents=True)
        kt.write_text(POST_DOWNLOAD, encoding="utf-8")
        client = kt.parent / "RustWebViewClient.kt"
        client.write_text(CLIENT_PRISTINE, encoding="utf-8")
        # 与生产同一条定位路径（按文件名 rglob，包名目录随 identifier 变化）。
        assert find_kotlin_file(gen_root, "RustWebViewClient.kt") == client

        p = find_main_activity(gen_root)
        p.write_text(patch_activity_src(p.read_text(encoding="utf-8")), encoding="utf-8")
        out = p.read_text(encoding="utf-8")
        assert MARKER in out and out.count("override fun onWebViewCreate") == 1

        cp = find_kotlin_file(gen_root, "RustWebViewClient.kt")
        cp.write_text(patch_client_src(cp.read_text(encoding="utf-8")), encoding="utf-8")
        cout = cp.read_text(encoding="utf-8")
        assert CLIENT_MARKER in cout and "Web2appBridge.rotate()" in cout

        # 再打一遍全幂等
        assert patch_activity_src(out) == out
        assert patch_client_src(cout) == cout

    print("patch-android-extras.py 自测通过 ✓（6 组场景）")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="给 Android 注入「更多」面板所需的 JS 桥")
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

    activity = find_main_activity(gen_root)
    src = activity.read_text(encoding="utf-8")
    patched = patch_activity_src(src)
    if patched == src:
        print(f"{activity} 已包含 JS 桥补丁，跳过")
    else:
        activity.write_text(patched, encoding="utf-8", newline="\n")
        print(f"已给 {activity} 注入 JS 桥（{BRIDGE_NAME}：分享/浏览器/清数据/退出 + 密钥）")

    client = find_kotlin_file(gen_root, "RustWebViewClient.kt")
    csrc = client.read_text(encoding="utf-8")
    cout = patch_client_src(csrc)
    if cout == csrc:
        print(f"{client} 已包含桥密钥轮换补丁，跳过")
    else:
        client.write_text(cout, encoding="utf-8", newline="\n")
        print(f"已给 {client} 注入主框架 onPageFinished 密钥轮换（{KEY_VAR}）")
    return 0


if __name__ == "__main__":
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except Exception:  # noqa: BLE001
            pass
    sys.exit(main())
