#!/usr/bin/env python3
"""Android 工程（`cargo tauri android init` 生成的 `gen/android`）路径解析的**单一真源**。

⚠️ 为什么需要它 —— 工程根在不同调用上下文里位置不同：

  · **CI**：两个 workflow 的 `defaults.run.working-directory` 都是 `webview-app`，
    脚本以 `../.github/scripts/xxx.py` 调用 ⇒ cwd = `<repo>/webview-app` ⇒ 工程根 = `gen/android`；
  · **本机在仓库根手跑** ⇒ 工程根 = `webview-app/gen/android`。

历史缺陷（2026-10-13 修复）：`patch-android-back.py` 的 `--gen-root` 默认值写死成后者，
CI 里按 cwd 解析成 `<repo>/webview-app/webview-app/gen/android` ⇒ `FileNotFoundError`
⇒ android job 在「接管返回键」这一步中断，后续**构建 APK / 签名 / 上传全部被跳过**；
而该 job 是 `continue-on-error: true`，所以「APK 根本没产出」看不出根因。

此后所有安卓补丁脚本一律经本模块解析工程根，不再各自写死。
"""

from __future__ import annotations

import os
import pathlib
import re
import sys

# 候选顺序 = 探测优先级：`gen/android`（CI 形态，cwd=webview-app）优先于
# `webview-app/gen/android`（仓库根手跑形态）。
GEN_ROOT_CANDIDATES = ("gen/android", "webview-app/gen/android")

_JAVA_REL = ("app", "src", "main", "java")


# ---------------------------------------------------------------------------
# 锚点漂移：降级 or 失败（「升级不需要改脚本」的机制层）
# ---------------------------------------------------------------------------
# 这套补丁链是**下游项目的参照实现**：上游升级（tauri-cli 模板 / wry 的 Kotlin 胶水）
# 不该逼下游改 Python。所以失败分两类：
#
#   · 基础设施错误（找不到 gen/android、读不到 tauri.conf.json、JSON 解析失败）——
#     那不是「上游变了」而是「环境不对」⇒ 照旧抛异常让构建红；
#   · 锚点漂移（平台签名 / 模板结构对不上）⇒ 抛 AnchorDrift，由 run_guarded() 决定结局：
#     默认**降级**（`::warning::` 后继续构建，最坏是某个可选能力在包里缺失），
#     设 `W2A_ANDROID_PATCH_STRICT=1` 则仍按失败处理（想让 CI 当门禁时用）。
#
# 为什么默认不失败：上游一次重排/格式化就让下游构建红，等于强迫所有人改脚本 ——
# 正是这套机制要消灭的东西。锚点本身选的是**平台合约**（`WebViewClient` /
# `WebChromeClient` 的 override 签名），上游改实现体不会动签名，所以漂移是低频事件。
STRICT_ENV = "W2A_ANDROID_PATCH_STRICT"


class AnchorDrift(ValueError):
    """补丁锚点与当前上游模板对不上（**不是**环境错误）。

    继承 ValueError：历史脚本的自测都用 `except ValueError` 断言漂移，语义不变。
    """


def _strict() -> bool:
    raw = os.environ.get(STRICT_ENV, "").strip().lower()
    return raw not in ("", "0", "false", "no")


def degrade_or_fail(script: str, err: BaseException) -> int:
    """锚点漂移的处置，返回退出码（严格模式 1 / 默认 0）。"""
    if _strict():
        print(f"::error::{script} 锚点漂移且已开启 {STRICT_ENV}：{err}", file=sys.stderr)
        return 1
    # 注意：`::warning::` 只作用于**单行**，后文的说明行不能带前缀（否则 GitHub 会把
    # 第二行当成新的 annotation 并丢弃）。
    print(f"::warning::{script} 锚点漂移，本补丁降级跳过（构建继续）：{err}")
    print("    · 上游若已自带该能力 ⇒ 无需处理，可直接停用本脚本；")
    print("    · 否则该能力在本包里缺失（日志里搜本补丁名即可确认）。")
    print(f"    · 想让它变门禁：设 {STRICT_ENV}=1。")
    return 0


def run_guarded(script: str, fn) -> int:
    """执行补丁脚本的 main()：只有漂移会被降级，其余异常照旧上抛（构建红）。"""
    try:
        return fn()
    except AnchorDrift as e:
        return degrade_or_fail(script, e)


def clean_name(name: str) -> str:
    """「安装名」清洗（productName / APK 文件名共用同一套规则）。

    去掉 Windows 文件名非法字符 → 压缩连续空白 → 去首尾空格与句点 → 空则回落 "Web2App"。
    单一真源：`prepare-android-conf.py`（productName）与 `set-android-apk-name.py`（APK 文件名）
    共用，避免两处规则漂移。
    """
    safe = re.sub(r'[\\/:*?"<>|]+', "_", name)
    safe = re.sub(r"\s+", " ", safe).strip(" .")
    return safe or "Web2App"


def gen_root_candidates_str() -> str:
    """给 `--help` 与错误信息用的候选串。"""
    return " / ".join(GEN_ROOT_CANDIDATES)


def resolve_gen_root(explicit: str | None = None) -> pathlib.Path:
    """确定 Android 工程根：显式给定则直接用；否则按候选顺序探测第一个真实存在的。

    一个都不存在时返回首选候选 —— 让上层报出可读错误，而不是静默按错路径找。
    """
    if explicit:
        return pathlib.Path(explicit)
    for cand in GEN_ROOT_CANDIDATES:
        if (pathlib.Path(cand).joinpath(*_JAVA_REL)).is_dir():
            return pathlib.Path(cand)
    return pathlib.Path(GEN_ROOT_CANDIDATES[0])


def java_root(gen_root: pathlib.Path) -> pathlib.Path:
    """安卓工程里 Java/Kotlin 源码根（模板里是 `app/src/main/java`）。"""
    return gen_root.joinpath(*_JAVA_REL)


def find_main_activity(gen_root: pathlib.Path) -> pathlib.Path:
    """定位 MainActivity.kt / MainActivity.java。"""
    jroot = java_root(gen_root)
    cands = sorted(p for p in jroot.rglob("MainActivity.*") if p.suffix in (".kt", ".java")) \
        if jroot.exists() else []
    if not cands:
        if not jroot.is_dir():
            # 环境错误：gen/android 还没 init ⇒ 必须红（不是上游变了）
            raise FileNotFoundError(
                f"未找到 {jroot}/**/MainActivity.{{kt,java}}"
                f"（请确认已先执行 cargo tauri android init；已尝试工程根："
                f"{gen_root_candidates_str()}）"
            )
        # 工程在、入口类没了 = 上游模板改了组织方式 ⇒ 漂移（可降级）
        raise AnchorDrift(
            f"{jroot} 下没有 MainActivity.{{kt,java}} —— tauri 模板可能改了入口类名/位置"
        )
    return cands[0]


def find_kotlin_file(gen_root: pathlib.Path, filename: str) -> pathlib.Path:
    """按文件名定位 wry 生成到 java 根下的 Kotlin 文件（如 `RustWebViewClient.kt`）。

    `cargo tauri android init` 会把 wry 的 kotlin 模板（包名占位符替换后）复制进
    `app/src/main/java/<包名路径>/`，与 MainActivity 同目录但包名来自 identifier，
    调用方不应硬编码包名路径 —— 统一 rglob。
    失败分两种（见本文件顶部「锚点漂移」）：工程还没 init ⇒ `FileNotFoundError`（红）；
    工程在但文件被改名/挪走 ⇒ `AnchorDrift`（补丁降级跳过，升级不逼下游改脚本）。
    """
    jroot = java_root(gen_root)
    cands = sorted(jroot.rglob(filename)) if jroot.exists() else []
    if not cands:
        if not jroot.is_dir():
            # 环境错误：还没 init ⇒ 红
            raise FileNotFoundError(
                f"未找到 {jroot}/**/{filename}（请确认已先执行 cargo tauri android init）"
            )
        # 工程在、这个胶水文件没了 = wry 改布局/改名 ⇒ 漂移（脚本走降级，不逼下游改 Python）
        raise AnchorDrift(
            f"{jroot} 下没有 {filename} —— wry 模板可能改了文件名/位置（若上游已用别的实现"
            f"提供该能力，对应补丁可直接退役）"
        )
    return cands[0]


def app_gradle(gen_root: pathlib.Path) -> pathlib.Path | None:
    """模板的 app 级 gradle 文件（优先 `.kts`；Tauri 2.12 起为 Kotlin DSL）。"""
    app = gen_root / "app"
    for name in ("build.gradle.kts", "build.gradle"):
        p = app / name
        if p.exists():
            return p
    return None


def strings_xml(gen_root: pathlib.Path) -> pathlib.Path:
    """安卓应用名资源文件。"""
    return gen_root / "app" / "src" / "main" / "res" / "values" / "strings.xml"


def android_manifest(gen_root: pathlib.Path) -> pathlib.Path:
    """AndroidManifest.xml。"""
    return gen_root / "app" / "src" / "main" / "AndroidManifest.xml"


def res_dir(gen_root: pathlib.Path) -> pathlib.Path:
    """资源根 `app/src/main/res`（`values/`、`values-night/`、`xml/` 的父目录）。"""
    return gen_root / "app" / "src" / "main" / "res"


# ---------------------------------------------------------------------------
# 谁走「漂移降级」这条通道
# ---------------------------------------------------------------------------
# 判据：这个补丁打不上时，App 还能不能跑？
#   能（某项能力缺失/视觉退化）⇒ 降级，列在这里；
#   不能（Manifest 权限、明文策略）⇒ 不列，让构建红（宁可红也不出废包）。
GUARDED_SCRIPTS = (
    # prepare-wry-kotlin 打不上 ⇒ 三个胶水补丁各自降级（build.rs 后面仍会生成）⇒ 能跑
    "prepare-wry-kotlin.py",
    "patch-android-insets.py",
    "patch-android-back.py",
    "patch-android-download.py",
    "patch-android-extras.py",
    "patch-android-schemes.py",
    "patch-android-fullscreen.py",
    "patch-android-splash-theme.py",
)


# ---------------------------------------------------------------------------
# 自测：漂移策略本身（`python3 android_gen.py --selftest`）
# ---------------------------------------------------------------------------
def _raise_drift():
    raise AnchorDrift("锚点没了")


def selftest() -> int:
    import contextlib
    import io

    # 1) AnchorDrift 必须仍是 ValueError：历史自测都用 `except ValueError` 断言漂移。
    assert issubclass(AnchorDrift, ValueError)

    saved = os.environ.pop(STRICT_ENV, None)
    try:
        # 2) 默认宽松：退出码 0，且首行是**单行** `::warning::`（多行注解 GitHub 只认第一行）。
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = degrade_or_fail("x.py", AnchorDrift("锚点没了"))
        out = buf.getvalue()
        assert rc == 0, "默认必须降级（构建继续），否则上游一升级就逼下游改脚本"
        assert out.splitlines()[0].startswith("::warning::"), "首行必须是 CI 注解"
        assert "x.py" in out and "锚点没了" in out
        assert out.count("::warning::") == 1, "只有首行能带注解前缀，其余行会被丢弃"

        # 3) 严格模式：退出码 1 + ::error::
        os.environ[STRICT_ENV] = "1"
        err = io.StringIO()
        with contextlib.redirect_stderr(err), contextlib.redirect_stdout(io.StringIO()):
            rc = degrade_or_fail("x.py", AnchorDrift("锚点没了"))
        assert rc == 1 and "::error::" in err.getvalue(), "严格模式必须红"

        # 4) "0"/"false"/"" 视为关闭（workflow 里 `X: 0` 是常见写法）
        for off in ("0", "false", "FALSE", ""):
            os.environ[STRICT_ENV] = off
            with contextlib.redirect_stdout(io.StringIO()):
                assert degrade_or_fail("x.py", AnchorDrift("d")) == 0, f"{off!r} 应视为关闭"
    finally:
        os.environ.pop(STRICT_ENV, None)
        if saved is not None:
            os.environ[STRICT_ENV] = saved

    # 5) run_guarded：只吞漂移；其它异常照旧上抛（环境错误必须红）
    assert run_guarded("x.py", lambda: 0) == 0
    with contextlib.redirect_stdout(io.StringIO()):
        assert run_guarded("x.py", _raise_drift) == 0
    try:
        run_guarded("x.py", lambda: 1 / 0)
        raise AssertionError("非漂移异常必须上抛，否则真实故障会被静默吞掉")
    except ZeroDivisionError:
        pass

    # 6) 名单里的脚本必须真的接上（新增/重写脚本时漏接 ⇒ 这里红灯）
    here = pathlib.Path(__file__).parent
    for name in GUARDED_SCRIPTS:
        text = (here / name).read_text(encoding="utf-8")
        assert "AnchorDrift" in text, f"{name} 未抛 AnchorDrift（漂移会直接让构建红）"
        assert f'run_guarded("{name}", main)' in text, f"{name} 入口未走 run_guarded"

    print(f"android_gen.py 自测通过 ✓（漂移策略 6 组场景 / 名单 {len(GUARDED_SCRIPTS)} 个脚本）")
    return 0


if __name__ == "__main__":
    if "--selftest" in sys.argv:
        for stream in (sys.stdout, sys.stderr):
            try:
                stream.reconfigure(encoding="utf-8", errors="replace")
            except Exception:  # noqa: BLE001
                pass
        sys.exit(selftest())
    print("这是被其它补丁脚本 import 的模块；只支持 --selftest", file=sys.stderr)
    sys.exit(2)
