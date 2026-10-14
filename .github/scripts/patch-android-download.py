#!/usr/bin/env python3
"""Android 下载接管补丁：把网页下载交给系统 DownloadManager（含长按保存图片）。

## 背景（为什么需要）

- Windows 壳在 `webview_hooks.rs` 里挂了 WebView2 的 `DownloadStarting` 接管
  （另存为对话框 + 下载进度）；
- **wry 0.57 的 Android 实现完全没有下载通道**：`RustWebViewClient.kt` 未挂
  `DownloadListener`，Rust 层 `download_started_handler` 在 android/mod.rs 的
  attributes 解构里被 `..` 丢弃。结果是：附件下载链接、长按图片「保存图片」在
  App 内全部**静默无反应**。

本补丁在 `MainActivity.onWebViewCreate`（WryActivity 的 webview 创建回调）里
挂 `setDownloadListener`：

  · 用系统 `DownloadManager` 下载到公共 Download 目录，通知栏自带进度 / 完成提示；
  · 透传 Cookie（登录态站点）与 User-Agent；文件名取 `URLUtil.guessFileName`
    （正确解析 `Content-Disposition`）；重名由 DownloadManager 自动追加序号；
  · API ≤ 28（minSdk=24）：写公共目录需要 `WRITE_EXTERNAL_STORAGE` 运行时权限，
    Manifest 里以 `maxSdkVersion=28` 声明，首次下载时申请；
  · API 33+：下载进度依赖通知，声明并申请 `POST_NOTIFICATIONS`；
    用户拒绝不阻断下载本身（文件照常落盘，只是没有进度通知）；
  · `blob:` / `data:` 链接系统下载器拿不到内容 ⇒ toast 明示，而不是静默。

同时改两处生成工程文件：
  1. `MainActivity.kt`：监听 + 下载实现（本脚本注入）；
  2. `AndroidManifest.xml`：补两条 `<uses-permission>`。

与 `patch-android-back.py` 的关系：两个脚本都在 `onWebViewCreate` 上做文章 ——
返回键补丁**新增**该 override；本脚本只往「已存在的 override 函数体」里插一行调用，
若 override 尚不存在（单独运行 / 模板变化）也能自己补出完整 override。CI 调用顺序：
insets → back → download → schemes。

gen/android 每次构建现生成，两个 workflow（build-custom.yml / release.yml）
都必须在 init 之后调用本脚本。

用法：
    python3 .github/scripts/patch-android-download.py            # 打补丁（工程根自动探测）
    python3 .github/scripts/patch-android-download.py --selftest # 离线自测
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys

from android_gen import (  # noqa: E402
    android_manifest,
    find_main_activity,
    gen_root_candidates_str,
    resolve_gen_root,
)

# 幂等标记：已打过补丁的文件不再重复注入。
MARKER = "web2app-android-download"
MANIFEST_MARKER = "web2app-download-permissions"

# 需要补的 import（缺失才加）。
NEEDED_IMPORTS = (
    "import android.Manifest",
    "import android.app.DownloadManager",
    "import android.content.Context",
    "import android.content.pm.PackageManager",
    "import android.net.Uri",
    "import android.os.Build",
    "import android.os.Environment",
    "import android.webkit.CookieManager",
    "import android.webkit.URLUtil",
    "import android.webkit.WebView",
    "import android.widget.Toast",
)

IMPORT_LINE = re.compile(r"^import\s+.*$", re.M)
PACKAGE_LINE = re.compile(r"^package\s+.*$", re.M)
CLASS_LINE = re.compile(r"^([ \t]*)(?:public\s+)?class\s+MainActivity\b[^\n]*\{", re.M)
# 已存在的 onWebViewCreate override（返回键补丁或模板自带）。捕获：整段签名到 '{'、参数名、行首缩进。
ON_WEBVIEW_CREATE = re.compile(
    r"^([ \t]*)override\s+fun\s+onWebViewCreate\s*\(\s*(\w+)\s*:\s*WebView\s*\)\s*\{",
    re.M,
)
SUPER_ON_WEBVIEW_CREATE = re.compile(r"super\.onWebViewCreate\s*\([^)]*\)")

# 需要下载能力时补进 Manifest 的权限（带独立标记，便于幂等与人工识别）。
PERMISSION_BLOCK = (
    "    <!-- {marker}：下载接管所需。\n"
    "         WRITE_EXTERNAL_STORAGE 仅 API ≤ 28 写公共 Download 目录需要；\n"
    "         POST_NOTIFICATIONS 供 API 33+ 显示系统下载进度通知。 -->\n"
    '    <uses-permission android:name="android.permission.WRITE_EXTERNAL_STORAGE" '
    'android:maxSdkVersion="28" />\n'
    '    <uses-permission android:name="android.permission.POST_NOTIFICATIONS" />\n'
).format(marker=MANIFEST_MARKER)


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


def _class_closing_brace(src: str, class_match: re.Match[str]) -> int:
    """从 class 声明行的 `{` 起做花括号配对，返回类体闭合 `}` 的下标。

    生成工程的 MainActivity 很小且不含嵌套字符串花括号，朴素扫描足够；
    仍跳过 // 行注释、/* 块注释与字符串 / 字符字面量里的括号，避免误判。
    """
    depth = 0
    i = class_match.end() - 1  # 指向声明行末尾的 '{'
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


# 监听挂接（方法体里唯一的一行调用；单独成行，便于「已存在 override」场景注入）。
def _call_line(param: str, indent: str) -> str:
    return f"{indent}// {MARKER}：网页下载 / 长按存图统一交给系统 DownloadManager。\n" \
           f"{indent}web2appSetupDownloads({param})\n"


# override 缺失时补的完整函数（2 空格为类成员缩进，与 Tauri 模板 / 返回键补丁对齐）。
def _full_override() -> str:
    return (
        f"  // {MARKER}：onWebViewCreate 里挂下载监听。\n"
        "  override fun onWebViewCreate(webView: WebView) {\n"
        "    super.onWebViewCreate(webView)\n"
        "    web2appSetupDownloads(webView)\n"
        "  }\n"
    )


# 下载实现（类成员；请求码直接写字面量，避免与模板将来的 companion object 冲突）。
METHODS_BLOCK = """  // {marker}：下载接管实现（见 patch-android-download.py）。
  private fun web2appSetupDownloads(webView: WebView) {{
    webView.setDownloadListener {{ url, userAgent, contentDisposition, mimeType, _ ->
      web2appEnqueueDownload(webView, url, userAgent, contentDisposition, mimeType)
    }}
  }}

  private fun web2appEnqueueDownload(
    webView: WebView,
    url: String?,
    userAgent: String?,
    contentDisposition: String?,
    mimeType: String?
  ) {{
    if (url.isNullOrEmpty() || (!url.startsWith("http://") && !url.startsWith("https://"))) {{
      // DownloadManager 只认 http(s)：blob:/data: 链接的内容在页面内存里，系统下载器取不到。
      Toast.makeText(this, "该链接不支持系统下载（可尝试长按保存）", Toast.LENGTH_LONG).show()
      return
    }}
    // API ≤ 28：写公共 Download 目录是危险权限。Manifest 已以 maxSdkVersion=28 声明，
    // 在真正发起下载时才申请；被拒绝就不排队（toast 已说明），避免静默失败。
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q &&
      checkSelfPermission(Manifest.permission.WRITE_EXTERNAL_STORAGE) != PackageManager.PERMISSION_GRANTED
    ) {{
      requestPermissions(arrayOf(Manifest.permission.WRITE_EXTERNAL_STORAGE), 7202)
      Toast.makeText(this, "请先授予存储权限，再点一次下载", Toast.LENGTH_LONG).show()
      return
    }}
    // API 33+：下载进度 / 完成提示靠系统通知。没有通知权限不影响文件落盘，只申请一次。
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
      checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
    ) {{
      requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 7203)
    }}
    try {{
      val fileName = URLUtil.guessFileName(url, contentDisposition, mimeType)
      val request = DownloadManager.Request(Uri.parse(url))
      // 登录态站点的下载链接依赖会话 Cookie，必须透传。
      CookieManager.getInstance().getCookie(url)?.takeIf {{ it.isNotEmpty() }}?.let {{
        request.addRequestHeader("Cookie", it)
      }}
      if (!userAgent.isNullOrEmpty()) request.addRequestHeader("User-Agent", userAgent)
      if (!mimeType.isNullOrEmpty()) request.setMimeType(mimeType)
      // 落公共 Download 目录：API 29+ 由系统 DownloadManager（持系统权限）写入，应用无需存储权限；
      // 重名文件由系统自动追加序号。
      request.setDestinationInExternalPublicDir(Environment.DIRECTORY_DOWNLOADS, fileName)
      request.setNotificationVisibility(DownloadManager.Request.VISIBILITY_VISIBLE_NOTIFY_COMPLETED)
      request.allowScanningByMediaScanner()
      val manager = getSystemService(Context.DOWNLOAD_SERVICE) as DownloadManager
      manager.enqueue(request)
      Toast.makeText(this, "已开始下载：$fileName", Toast.LENGTH_SHORT).show()
    }} catch (e: Exception) {{
      Toast.makeText(this, "下载失败：${{e.message ?: "未知错误"}}", Toast.LENGTH_SHORT).show()
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
        # 已有 onWebViewCreate（返回键补丁新增 / 模板自带）：把调用插到函数体开头。
        # 在 super.onWebViewCreate(...) 之后插最稳；模板没有 super 调用时退化为紧跟 '{'。
        body_start = existing.end()
        sup = SUPER_ON_WEBVIEW_CREATE.search(src, body_start)
        if sup and sup.start() < _class_closing_brace(src, class_m):
            line_end = src.find("\n", sup.end())
            insert_at = body_start if line_end < 0 else line_end
            # 函数声明行缩进（类成员 2 空格）+ 一级（2 空格）= 函数体 4 空格。
            indent = existing.group(1) + "  "
        else:
            insert_at = body_start
            indent = existing.group(1) + "  "
        src = src[:insert_at] + "\n" + _call_line(existing.group(2), indent).rstrip("\n") + src[insert_at:]
    else:
        # 没有 override：在类声明行后补完整 override（实现方法统一在类尾注入）。
        insert_at = class_m.end()
        src = src[:insert_at] + "\n\n" + _full_override().rstrip("\n") + src[insert_at:]

    # 实现方法插到类体闭合 } 之前。
    class_m = CLASS_LINE.search(src)
    close = _class_closing_brace(src, class_m)
    return src[:close] + "\n" + METHODS_BLOCK.rstrip("\n") + "\n" + src[close:]


def patch_manifest_src(src: str) -> str:
    """AndroidManifest.xml：幂等补下载所需权限。"""
    if MANIFEST_MARKER in src:
        return src
    idx = src.rfind("</manifest>")
    if idx < 0:
        raise ValueError("AndroidManifest.xml 中未找到 </manifest>，模板可能已变化")
    return src[:idx] + PERMISSION_BLOCK + src[idx:]


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

# 模拟返回键补丁**先跑过**之后的形态（含 onWebViewCreate override）。
POST_BACK = (
    "package com.web2app.app\n\n"
    "import android.webkit.WebView\n"
    "import androidx.activity.OnBackPressedCallback\n\n"
    "class MainActivity : TauriActivity() {\n"
    "  // web2app-android-back-guard\n"
    "  override fun onWebViewCreate(webView: WebView) {\n"
    "    super.onWebViewCreate(webView)\n"
    "    onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {\n"
    "      override fun handleOnBackPressed() { finish() }\n"
    "    })\n"
    "  }\n"
    "}\n"
)

MANIFEST_PRISTINE = (
    '<?xml version="1.0" encoding="utf-8"?>\n'
    "<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\">\n"
    '    <application android:label="Web2App" />\n'
    "</manifest>\n"
)


def selftest() -> int:
    # 1) 原始模板 → 要素齐全
    patched = patch_activity_src(PRISTINE)
    for needle in (
        MARKER,
        "webView.setDownloadListener",
        "web2appEnqueueDownload(webView, url, userAgent, contentDisposition, mimeType)",
        "DownloadManager.Request(Uri.parse(url))",
        "setDestinationInExternalPublicDir(Environment.DIRECTORY_DOWNLOADS, fileName)",
        "WRITE_EXTERNAL_STORAGE",
        "POST_NOTIFICATIONS",
        "requestPermissions(arrayOf(Manifest.permission.WRITE_EXTERNAL_STORAGE), 7202)",
        "CookieManager.getInstance().getCookie(url)",
        "URLUtil.guessFileName(url, contentDisposition, mimeType)",
        "override fun onWebViewCreate(webView: WebView)",
        "super.onWebViewCreate(webView)",
    ):
        assert needle in patched, f"补丁缺少要素：{needle}"
    for imp in NEEDED_IMPORTS:
        assert imp in patched, f"缺少 import：{imp}"
    # 原有 onCreate 必须原样保留
    assert "enableEdgeToEdge()" in patched and "super.onCreate(savedInstanceState)" in patched
    # 实现方法在类闭合 } 之前、且类只闭合一次结构完好（花括号配对能走到尾）
    assert patched.rstrip().endswith("}")
    assert patched.count("class MainActivity") == 1

    # 2) 幂等
    assert patch_activity_src(patched) == patched, "MainActivity 补丁不是幂等的"

    # 3) 返回键补丁先跑过：只插调用，不得新增第二个 override
    post = patch_activity_src(POST_BACK)
    assert post.count("override fun onWebViewCreate") == 1, "不得重复定义 onWebViewCreate"
    assert "web2appSetupDownloads(webView)" in post
    # 调用必须在 super 之后
    assert post.index("super.onWebViewCreate(webView)") < post.index("web2appSetupDownloads(webView)")
    # 返回键守卫逻辑不得被破坏
    assert "web2app-android-back-guard" in post and "handleOnBackPressed()" in post
    # WebView import 已存在，不应重复
    assert post.count("import android.webkit.WebView\n") == 1
    # 再打一遍幂等
    assert patch_activity_src(post) == post

    # 4) 无 class 行 → 报错；无 package/import → 报错
    for bad in ("package com.x\n\n// nothing\n",):
        try:
            patch_activity_src(bad)
            raise AssertionError("畸形源码应报错")
        except ValueError:
            pass

    # 5) Manifest：注入两条权限且幂等；位置在 </manifest> 前；application 保留
    m = patch_manifest_src(MANIFEST_PRISTINE)
    assert MANIFEST_MARKER in m
    assert 'android:name="android.permission.WRITE_EXTERNAL_STORAGE"' in m
    assert 'android:maxSdkVersion="28"' in m
    assert 'android:name="android.permission.POST_NOTIFICATIONS"' in m
    assert m.index("POST_NOTIFICATIONS") < m.index("</manifest>")
    assert "<application" in m
    assert patch_manifest_src(m) == m, "Manifest 补丁不是幂等的"
    try:
        patch_manifest_src("<manifest />\n")
        raise AssertionError("缺 </manifest> 应报错")
    except ValueError:
        pass

    # 6) 文件级集成：临时 gen/android 工程，经 find_main_activity / android_manifest 走一遍
    import tempfile

    with tempfile.TemporaryDirectory() as tmp:
        gen_root = pathlib.Path(tmp) / "gen" / "android"
        kt = gen_root / "app/src/main/java/com/web2app/app/MainActivity.kt"
        kt.parent.mkdir(parents=True)
        kt.write_text(POST_BACK, encoding="utf-8")
        manifest = android_manifest(gen_root)
        manifest.parent.mkdir(parents=True, exist_ok=True)
        manifest.write_text(MANIFEST_PRISTINE, encoding="utf-8")

        path = find_main_activity(gen_root)
        path.write_text(patch_activity_src(path.read_text(encoding="utf-8")), encoding="utf-8")
        out = path.read_text(encoding="utf-8")
        assert MARKER in out and out.count("override fun onWebViewCreate") == 1

        mp = android_manifest(gen_root)
        mp.write_text(patch_manifest_src(mp.read_text(encoding="utf-8")), encoding="utf-8")
        assert MANIFEST_MARKER in mp.read_text(encoding="utf-8")

    print("patch-android-download.py 自测通过 ✓（6 组场景）")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="给 Android 注入系统下载接管")
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
        print(f"{activity} 已包含下载接管，跳过")
    else:
        activity.write_text(patched, encoding="utf-8", newline="\n")
        print(f"已给 {activity} 注入下载接管（DownloadManager + 存储/通知权限）")

    manifest = android_manifest(gen_root)
    msrc = manifest.read_text(encoding="utf-8")
    mout = patch_manifest_src(msrc)
    if mout == msrc:
        print(f"{manifest} 已包含下载权限声明，跳过")
    else:
        manifest.write_text(mout, encoding="utf-8", newline="\n")
        print(f"已给 {manifest} 补下载权限（WRITE_EXTERNAL_STORAGE maxSdk=28 / POST_NOTIFICATIONS）")
    return 0


if __name__ == "__main__":
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except Exception:  # noqa: BLE001
            pass
    sys.exit(main())
