#!/usr/bin/env python3
"""Android 外部 scheme 补丁：非 http(s) 链接交给系统应用处理。

## 背景（为什么需要）

wry 0.57 的 `RustWebViewClient.shouldOverrideUrlLoading` 把**所有**链接的决策都
交给 `Rust.shouldOverride`（壳层未配置导航拦截 ⇒ 一律放行），于是 `tel:`、
`mailto:`、`weixin://`、`alipays://`、`intent://`、`market://` 这类国内站点高频
使用的唤起链接会被直接塞进 WebView 加载，落到 `ERR_UNKNOWN_URL_SCHEME` 白页；
应用也没有任何「在系统浏览器打开」的出口。

本补丁在 wry 生成的 `RustWebViewClient.kt`（与 MainActivity 同目录，包名随
identifier 变化，故按文件名 rglob 定位）里做两处外科注入：

  1. `shouldOverrideUrlLoading` 函数体第一行：先过 `web2appHandleExternalScheme`；
  2. 新增同名私有方法：
       · http / https / about / data / blob / javascript / file / content / 空 scheme
         —— 放行给 WebView（splash.localhost、tauri 资源域都是 http(s)，不受影响）；
       · `intent://` —— `Intent.parseUri(URI_INTENT_SCHEME)` 解析后外跳，失败时尝试
         链接自带的 `browser_fallback_url`；
       · `market://` —— 无应用市场时回落 Play Store 网页；
       · 其余（tel / mailto / weixin / alipays / geo / mqq / ……）—— ACTION_VIEW 外跳；
       · 没有任何应用能处理：toast 提示并**吞掉**该次导航，不留下错误白页。

只动客户端类，不改 Rust / JNI：wry 升级后仍以「方法体首行注入」的方式打，
模板若调整方法签名会在补丁步显式报错（不允许静默跳过）。

gen/android 每次构建现生成，两个 workflow（build-custom.yml / release.yml）
都必须在 init 之后调用本脚本。

用法：
    python3 .github/scripts/patch-android-schemes.py            # 打补丁（工程根自动探测）
    python3 .github/scripts/patch-android-schemes.py --selftest # 离线自测
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

MARKER = "web2app-android-external-scheme"

NEEDED_IMPORTS = (
    "import android.content.Intent",
    "import android.widget.Toast",
)

IMPORT_LINE = re.compile(r"^import\s+.*$", re.M)
PACKAGE_LINE = re.compile(r"^package\s+.*$", re.M)

# wry 0.57 模板里该方法签名折成 4 行；分别捕获 view / request 的参数名，避免硬编码。
OVERRIDE_FUN = re.compile(
    r"^([ \t]*)override\s+fun\s+shouldOverrideUrlLoading\s*\(\s*"
    r"(\w+)\s*:\s*WebView\s*,\s*"
    r"(\w+)\s*:\s*WebResourceRequest\s*"
    r"\)\s*:\s*Boolean\s*\{",
    re.M | re.S,
)

GUARD_TEMPLATE = (
    "{indent}// {marker}：非网页 scheme 先交系统应用处理（tel/mailto/weixin/intent/…）。\n"
    "{indent}if (web2appHandleExternalScheme({view_param}, {req_param})) return true\n"
)

# 4 空格缩进对齐 wry kotlin 模板（类成员 4 空格）。
HELPER_METHOD = '''    /**
     * {marker}：非 http(s) scheme 交给系统应用处理。
     *
     * 返回 true = 本次导航已接管（WebView 不再加载，避免 ERR_UNKNOWN_URL_SCHEME 白页）；
     * false = 仍是网页内容，交回 wry 原逻辑（Rust.shouldOverride）。
     */
    private fun web2appHandleExternalScheme(view: WebView, uri: Uri): Boolean {{
        val scheme = uri.scheme?.lowercase() ?: return false
        when (scheme) {{
            // 网页与 WebView 内部内容：启动页 splash.localhost / tauri 资源域都是 http(s)。
            "http", "https", "about", "data", "blob", "javascript", "file", "content" -> return false
        }}
        val context = view.context
        val intent = if (scheme == "intent") {{
            // intent:// 语法（微信 / 支付宝 / 应用市场常用）：按 Android intent-URI 规范解析。
            try {{
                Intent.parseUri(uri.toString(), Intent.URI_INTENT_SCHEME)
            }} catch (e: Exception) {{
                Toast.makeText(context, "无法打开此链接", Toast.LENGTH_SHORT).show()
                return true
            }}
        }} else {{
            Intent(Intent.ACTION_VIEW, uri)
        }}
        if (intent == null) {{
            Toast.makeText(context, "无法打开此链接", Toast.LENGTH_SHORT).show()
            return true
        }}
        try {{
            intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            context.startActivity(intent)
        }} catch (e: Exception) {{
            // 目标应用未安装 / 被禁用：按链接自身声明的退路处理，仍无路可走才提示。
            val fallback = intent.getStringExtra("browser_fallback_url")
            val marketFallback = if (scheme == "market") {{
                val host = uri.host ?: "details"
                "https://play.google.com/store/apps/$host" +
                    (uri.query?.let {{ "?$it" }} ?: "")
            }} else null
            when {{
                !fallback.isNullOrEmpty() -> view.loadUrl(fallback)
                marketFallback != null -> view.loadUrl(marketFallback)
                else -> Toast.makeText(context, "未安装可打开此链接的应用", Toast.LENGTH_SHORT).show()
            }}
        }}
        return true
    }}
'''.format(marker=MARKER)


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


def patch_src(src: str) -> str:
    """对 RustWebViewClient.kt 打补丁；已打过则原样返回。"""
    if MARKER in src:
        return src

    m = OVERRIDE_FUN.search(src)
    if not m:
        raise ValueError(
            "未找到 wry 的 `override fun shouldOverrideUrlLoading(view: WebView, "
            "request: WebResourceRequest): Boolean`，模板可能已升级，请人工确认补丁注入点"
        )

    src = _add_imports(src, NEEDED_IMPORTS)
    m = OVERRIDE_FUN.search(src)
    indent, view_param, req_param = m.group(1), m.group(2), m.group(3)
    guard = GUARD_TEMPLATE.format(
        indent=indent + "    ",
        marker=MARKER,
        view_param=view_param,
        req_param=req_param,
    )

    # 1) 守卫插到方法体开头（签名 '{' 之后）。
    body_open = m.end()
    src = src[:body_open] + "\n" + guard.rstrip("\n") + src[body_open:]

    # 2) 辅助方法插在 override 函数**之前**（重新定位：上面的插入改变了下标）。
    m = OVERRIDE_FUN.search(src)
    start = m.start()
    # 吃掉函数行前的缩进 —— 辅助方法自己带 4 空格缩进，前后各留一个空行。
    line_start = src.rfind("\n", 0, start) + 1
    return (
        src[:line_start]
        + HELPER_METHOD.rstrip("\n")
        + "\n\n"
        + src[start:]
    )


# ---------------------------------------------------------------------------
# 自测
# ---------------------------------------------------------------------------
PRISTINE = (
    "// copyright header\n"
    "package com.web2app.app\n"
    "import android.net.Uri\n"
    "import android.webkit.*\n"
    "import android.content.Context\n"
    "class RustWebViewClient(webView: RustWebView, context: Context): WebViewClient() {\n"
    "    override fun onPageFinished(view: WebView, url: String) {\n"
    "        Rust.onPageLoaded((view as RustWebView).id, url)\n"
    "    }\n"
    "    override fun shouldOverrideUrlLoading(\n"
    "        view: WebView,\n"
    "        request: WebResourceRequest\n"
    "    ): Boolean {\n"
    "        return Rust.shouldOverride((view as RustWebView).id, request.url.toString())\n"
    "    }\n"
    "    override fun onPageStarted(view: WebView, url: String, favicon: Bitmap?) {\n"
    "        currentUrl = url\n"
    "    }\n"
    "}\n"
)


def selftest() -> int:
    # 1) 原始模板 → 要素齐全
    patched = patch_src(PRISTINE)
    for needle in (
        MARKER,
        "if (web2appHandleExternalScheme(view, request)) return true",
        "private fun web2appHandleExternalScheme(view: WebView, uri: Uri): Boolean",
        "Intent.parseUri(uri.toString(), Intent.URI_INTENT_SCHEME)",
        "Intent(Intent.ACTION_VIEW, uri)",
        "intent.getStringExtra(\"browser_fallback_url\")",
        "play.google.com/store/apps/",
        "未安装可打开此链接的应用",
        NEEDED_IMPORTS[0],
        NEEDED_IMPORTS[1],
        # wry 原逻辑必须保留
        "Rust.shouldOverride((view as RustWebView).id, request.url.toString())",
        "Rust.onPageLoaded((view as RustWebView).id, url)",
    ):
        assert needle in patched, f"补丁缺少要素：{needle}"
    # Uri 是模板原有 import，不得重复
    assert patched.count("import android.net.Uri\n") == 1

    # 2) 守卫必须在原 return 之前（先拦截、后回落）。
    #    注意用完整语句定位：辅助方法的 KDoc 注释里也出现过 Rust.shouldOverride 字样。
    assert patched.index("web2appHandleExternalScheme(view, request)") < patched.index(
        "return Rust.shouldOverride"
    ), "守卫必须在 wry 原决策之前"
    # 辅助方法在 override 函数之前
    assert patched.index("private fun web2appHandleExternalScheme") < patched.index(
        "override fun shouldOverrideUrlLoading"
    )

    # 3) 幂等
    assert patch_src(patched) == patched, "补丁不是幂等的"

    # 4) 模板升级删掉了该方法 → 显式报错
    try:
        patch_src("package com.web2app.app\nclass RustWebViewClient\n")
        raise AssertionError("找不到 shouldOverrideUrlLoading 时应报错")
    except ValueError:
        pass

    # 5) 单行签名变体（防御模板格式微调）也能注入
    single_line = PRISTINE.replace(
        "    override fun shouldOverrideUrlLoading(\n"
        "        view: WebView,\n"
        "        request: WebResourceRequest\n"
        "    ): Boolean {\n",
        "    override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {\n",
    )
    p2 = patch_src(single_line)
    assert "web2appHandleExternalScheme(view, request)" in p2

    # 6) 文件级集成：find_kotlin_file 定位 + 落盘 + 缺失报错
    import tempfile

    with tempfile.TemporaryDirectory() as tmp:
        gen_root = pathlib.Path(tmp) / "gen" / "android"
        kt_dir = gen_root / "app/src/main/java/com/web2app/app"
        kt_dir.mkdir(parents=True)
        kt = kt_dir / "RustWebViewClient.kt"
        kt.write_text(PRISTINE, encoding="utf-8")

        path = find_kotlin_file(gen_root, "RustWebViewClient.kt")
        path.write_text(patch_src(path.read_text(encoding="utf-8")), encoding="utf-8")
        out = path.read_text(encoding="utf-8")
        assert MARKER in out and patch_src(out) == out

        try:
            find_kotlin_file(pathlib.Path(tmp) / "nogen", "RustWebViewClient.kt")
            raise AssertionError("工程缺失时应报错")
        except FileNotFoundError:
            pass

    print("patch-android-schemes.py 自测通过 ✓（6 组场景）")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="给 wry RustWebViewClient 注入外部 scheme 外跳")
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
    path = find_kotlin_file(gen_root, "RustWebViewClient.kt")
    src = path.read_text(encoding="utf-8")
    patched = patch_src(src)
    if patched == src:
        print(f"{path} 已包含外部 scheme 接管，跳过")
    else:
        path.write_text(patched, encoding="utf-8", newline="\n")
        print(f"已给 {path} 注入外部 scheme 接管（tel/mailto/weixin/intent 等交系统应用打开）")
    return 0


if __name__ == "__main__":
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except Exception:  # noqa: BLE001
            pass
    sys.exit(main())
