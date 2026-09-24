#!/usr/bin/env python3
"""本地打包驱动：读一份配置 JSON，在本机跑出与 CI 同语义的产物。

## 为什么需要它

CI 的三条流水线（`build-custom.yml` Tauri/Android、`electron-build-custom.yml`）都靠
**同一个解析器** `.github/scripts/parse-packaging-config.py` 把 JSON 变成「编译期环境变量」
或「规范化后的 config.json」。本脚本做的事就是**把同一个解析器接到本机**，
让本地产物与 CI 产物吃的是同一份契约，而不是各写一遍。

两壳的配置消费方式**根本不同**，这决定了本地要做什么：

| 壳 | 配置何时被读 | 本地要做的 |
| :-- | :--- | :--- |
| Tauri（MSI / 单文件 zip） | **编译期**：`option_env!` 读环境变量，值被写成二进制常量 | `cargo tauri build` **之前**把环境变量设好 |
| Electron（zip / portable） | **打包期**：读 `electron-app/config.json` → `out/config.json` → 进 `app.asar` | 先落一份**规范化**的 `config.json`，再 `pnpm run dist` |

⚠️ Android **本机做不了**（`ring` 的 build script 需要 `aarch64-linux-android-clang`，即 NDK），
本脚本会明确拒绝而不是给你一个半成品。Android 只能走 CI。

## ⚠️⚠️ 三个「本地特有」的坑（都实测过，所以本脚本刻意绕开）

1. **不要 `source` 解析器写出的 env 文件。** 在 Windows 上 `--env-out` 落盘的是 **CRLF**
   （解析器里 `open(path, "a", encoding="utf-8")` 会把 `\\n` 按 `os.linesep` 翻译成 `\\r\\n`）⇒
   `source` 之后每个值尾部都粘一个 `\\r`，`APP_URL` 直接变成带 `\\r` 的地址。
   另外 `PROXY_BYPASS=localhost;127.0.0.1;<-loopback>` 含 `;`（bash 会当成命令分隔符拆断），
   `public_key` 是多行 PEM 时写的是 **GitHub heredoc**（`KEY<<EOF … EOF`，bash 不认）。
   ⇒ 本脚本**不落也不读 env 文件**，直接把 `os.environ` 传给子进程。

2. **`--env-out` 不传时解析器会静默跳过**（`main()` 里是 `if args.env_out:`）——
   本地忘了传就「看起来解析成功、但一个环境变量都没设」。走本脚本不会有这个问题。

3. **`--env-out` 是 append 模式**（`open(path, "a")`）⇒ 反复跑会累积、新值盖不住旧值。
   本脚本不产生中间文件，天然没有这个问题。

## ⚠️⚠️ tracked 文件会被改写，必须备份 + 还原

本地打包必须动两个**被版本控制**的文件（CI 是无状态工作区，改了无所谓；本地会脏工作区）：

- `webview-app/tauri.conf.json` —— 把 `app_name` 写进 `productName`（安装包/快捷方式/安装目录名），
  并固定 `mainBinaryName = webviewapp`（exe 文件名稳定）；
- `webview-app/icons/*` —— `icon_url` 配置时 `cargo tauri icon` 会**重新生成整套图标**。

另外 `electron-app/config.json` 虽然**未被版本控制**，但通常是开发者本地调试用的那一份
（如 baidu 配置）—— 直接覆盖等于静默毁掉本地状态，所以**同样纳入备份还原**。
（CI 不还原是因为工作区一次性；本地不行。）

以上三类一律 **先备份、`finally` 里还原**（并打印还原后的 sha256），除非显式 `--keep-config`。

> ⚠️ **`finally` 的前提是进程活着走到那里。** 实测踩过：构建中途被人为中断
> （kill / Ctrl-C / 关终端）⇒ 还原不执行 ⇒ `electron-app/config.json` 就留在**生成态**，
> 本地调试配置看起来「丢了」。备份其实还在（`mkdtemp` 的目录不会被自动清），
> 所以脚本一开始就把**备份目录路径打印出来** —— 中断后照着它手工拷回即可。
> 想彻底避免：别在下一次打包前中断（或先用 `--keep-config` 明确表示「我知道会留在生成态」）。

## 用法

    # 看将要执行的命令，不动任何文件
    python scripts/pack-local.py --config test1.json --target electron-all --dry-run

    # Electron 绿色版（zip + portable exe）
    python scripts/pack-local.py --config test1.json --target electron-all

    # Tauri Windows 安装版（需先 cargo install tauri-cli --version "^2"）
    python scripts/pack-local.py --config test1.json --target tauri-msi

    # 也可以直接给 JSON 字符串，或一个返回 JSON 的 URL
    python scripts/pack-local.py --json '{"app_url":"https://example.com"}' --target electron-zip
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PARSER = ROOT / ".github" / "scripts" / "parse-packaging-config.py"
WEBVIEW = ROOT / "webview-app"
ELECTRON = ROOT / "electron-app"

TARGETS = (
    "electron-zip",
    "electron-portable",
    "electron-all",
    "tauri-msi",
    "tauri-portable",
    "tauri-all",
)


# ---------------------------------------------------------------------------
# 载入共享解析器（唯一真源）
# ---------------------------------------------------------------------------
def load_parser():
    """按路径加载 `.github/scripts/parse-packaging-config.py`。

    文件名带连字符，不能 `import parse_packaging_config`，故用 importlib 显式取。
    顺带复用它导出的 `DEFAULTS` / `SKIP_ENV` —— 契约只能有一处定义，本脚本不另抄一份。
    """
    if not PARSER.exists():
        sys.exit(f"✗ 找不到共享解析器：{PARSER}")
    spec = importlib.util.spec_from_file_location("w2a_packaging_config", PARSER)
    if spec is None or spec.loader is None:
        sys.exit(f"✗ 无法加载共享解析器：{PARSER}")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def safe_name(raw: str) -> str:
    """与 CI 逐字一致的「安装名」清洗：去 Windows 文件名非法字符、压空白、去首尾空格与句点。"""
    s = re.sub(r'[\\/:*?"<>|]+', "_", raw) if raw else ""
    s = re.sub(r"\s+", " ", s).strip(" .") if s else ""
    return s or "Web2App"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


# ---------------------------------------------------------------------------
# 备份 / 还原（对 tracked 文件）
# ---------------------------------------------------------------------------
class Restorer:
    """记录「原文件 → 备份」，退出时无条件还原。

    ⚠️ 用 `finally` 而不是「成功才还原」：打包中途失败时更要还原，
    否则工作区留着一份被改过的 `tauri.conf.json`，下次 git diff 会莫名其妙。
    """

    def __init__(self) -> None:
        self._items: list[tuple[Path, Path]] = []  # (原路径, 备份路径)
        self._dirs: list[tuple[Path, Path]] = []  # 目录整份备份
        self._tmp = Path(tempfile.mkdtemp(prefix="w2a-pack-backup-"))
        # ⚠️⚠️ 把备份目录**打印出来**，因为 `finally` 还原有一个致命前提：**进程得活着走到那里**。
        #    实测踩过：构建跑到一半被人为中断（kill / Ctrl-C / 终端关闭）⇒ `finally` 不执行
        #    ⇒ 被改写的 `electron-app/config.json` 就**留在生成态**，原来的本地调试配置丢失。
        #    备份本身是好的（`mkdtemp` 的目录在异常路径上不会被删），只是没人知道它在哪。
        #    这一行就是那个「知道它在哪」的入口。
        print(f"（原文件备份目录：{self._tmp}）")

    def file(self, path: Path) -> None:
        if not path.exists():
            return
        bak = self._tmp / f"{len(self._items)}-{path.name}"
        shutil.copy2(path, bak)
        self._items.append((path, bak))

    def dir(self, path: Path) -> None:
        if not path.exists():
            return
        bak = self._tmp / f"dir-{len(self._dirs)}-{path.name}"
        shutil.copytree(path, bak)
        self._dirs.append((path, bak))

    def restore(self, keep: bool) -> None:
        if keep:
            print(f"⚠️ --keep-config：刻意保留改动，备份留在 {self._tmp}")
            return
        for path, bak in self._items:
            shutil.copy2(bak, path)
            print(f"已还原 {path.relative_to(ROOT)}（sha256 {sha256(path)[:12]}）")
        for path, bak in self._dirs:
            if path.exists():
                shutil.rmtree(path)
            shutil.copytree(bak, path)
            print(f"已还原目录 {path.relative_to(ROOT)}/")
        shutil.rmtree(self._tmp, ignore_errors=True)


# ---------------------------------------------------------------------------
# 子进程执行
# ---------------------------------------------------------------------------
def run(cmd: list[str], cwd: Path, env: dict[str, str], dry: bool) -> None:
    printable = " ".join(f'"{c}"' if " " in c else c for c in cmd)
    print(f"\n$ (cd {cwd.relative_to(ROOT)}{'/' if str(cwd.relative_to(ROOT)) != '.' else ''}) {printable}")
    if dry:
        return
    proc = subprocess.run(cmd, cwd=str(cwd), env=env)
    if proc.returncode != 0:
        sys.exit(f"✗ 命令失败（退出码 {proc.returncode}）：{printable}")


def have(cmd: list[str]) -> bool:
    try:
        return subprocess.run(cmd, capture_output=True).returncode == 0
    except (OSError, FileNotFoundError):
        return False


# ---------------------------------------------------------------------------
# 主体
# ---------------------------------------------------------------------------
def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(
        description="本地打包：按配置 JSON 在本机产出与 CI 同语义的产物",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    src = ap.add_mutually_exclusive_group(required=True)
    src.add_argument("--config", help="配置 JSON 文件路径（如 test1.json）")
    src.add_argument("--json", help="配置 JSON 字符串")
    src.add_argument("--url", help="返回配置 JSON 的 URL")
    ap.add_argument("--target", required=True, choices=TARGETS, help="要打哪个产物")
    ap.add_argument("--icon", action="store_true",
                    help="处理 icon_url（下载 + 生成全套图标）。⚠️ 会改写 tracked 的 icons/，"
                         "脚本会备份还原——但 CI 靠的是「工作区一次性」，本地唯一真源图标是仓库默认图标")
    ap.add_argument("--keep-config", action="store_true",
                    help="打包后**不还原**被改写的 tracked 文件（便于反复迭代，记得自己 git checkout）")
    ap.add_argument("--out", metavar="DIR",
                    help="Electron 产物输出目录（默认 electron-app/release；**相对路径以 electron-app/ 为基准**）。"
                         "⚠️ 默认目录已存在且**无法清空**时（文件被占用 / EBUSY / 沙箱禁删）"
                         "electron-builder 会在 emptyDir 阶段失败，此时指一个**新目录**即可绕过，"
                         "如 `--out tests/pack-verify`（落在 electron-app/tests/，已被 gitignore）")
    ap.add_argument("--dry-run", action="store_true", help="只打印将执行的命令，不动任何文件")
    args = ap.parse_args(argv)

    mod = load_parser()

    # ---- 1) 解析配置（复用唯一真源，不落 env 文件）----
    # 三种输入统一成「先把原始文本取出来」再解析一次，避免在各分支里重复读——
    # ⚠️ 别在后续步骤里再调一次 read_raw(args.json, args.url)：`--config` 分支下两个入参都是
    #    None，read_raw 会抛 ConfigError("必须提供 config_json 或 config_url")。
    try:
        if args.json or args.url:
            raw = mod.read_raw(args.json, args.url)
        else:
            raw = Path(args.config).read_text(encoding="utf-8")
        data = mod.parse_json(raw)
        merged = mod.build_merged(data)
    except mod.ConfigError as e:
        return int(str(e)) if str(e).isdigit() else _fail(str(e))
    except OSError as e:
        return _fail(f"读不到配置文件：{e}")

    env = os.environ.copy()
    for key, value in merged.items():
        # 签名类字段（android_*）与 Android 无关的产物不需要，也刻意不进环境变量
        # ——与解析器 write_env() 的 SKIP_ENV 同源，不另抄一份清单。
        if key in getattr(mod, "SKIP_ENV", set()):
            continue
        env[key.upper()] = value

    name = safe_name(merged.get("app_name", ""))
    print("=" * 68)
    print(f"本地打包：target = {args.target}｜安装名 = {name}")
    print("=" * 68)
    for key in mod.DEFAULTS:
        if key in getattr(mod, "SKIP_ENV", set()):
            continue
        print(f"  {key.upper():<22} {mod.display(key, merged[key])}")

    rr = Restorer()
    try:
        if args.target.startswith("electron"):
            do_electron(args, mod, data, merged, env, name, rr)
        else:
            do_tauri(args, mod, merged, env, name, rr)
    finally:
        rr.restore(args.keep_config)

    print("\n✓ 完成" + ("（dry-run，未动任何文件）" if args.dry_run else ""))
    return 0


def _fail(msg: str) -> int:
    print(f"✗ 配置不合法：{msg}", file=sys.stderr)
    print("  字段表与多端行为见 docs/packaging-config.md", file=sys.stderr)
    return 1


# ---------------------------------------------------------------------------
# Electron：打包期读 config.json
# ---------------------------------------------------------------------------
def do_electron(args, mod, data, merged, env, name, rr) -> None:
    if not have(["pnpm", "--version"]):
        sys.exit("✗ 未找到 pnpm。Electron 侧本地打包需要 pnpm（见 electron-app/package.json）")

    dry = args.dry_run

    # 1) 落一份**规范化后**的 config.json —— 这是本地手工改配置最容易做错的一步：
    #    布尔写成字符串 "false"、app_url 数组写成字符串，都是「能出包但行为不对」。
    #    走解析器则与 CI 逐字一致（真 bool / 真数组）。
    #
    # ⚠️ 它虽然**未被版本控制**，但往往是开发者本地调试用的那份（如 baidu 配置）——
    #    直接覆盖会把本地状态静默搞掉。所以照样备份 + 还原（`--keep-config` 可保留新的）。
    #    CI 那边不还原是因为工作区一次性；本地不行。
    cfg_path = ELECTRON / "config.json"
    if dry:
        print(f"\n[1/4] 将写出规范化配置 → {cfg_path.relative_to(ROOT)}（原文件备份，打包后还原）")
    else:
        rr.file(cfg_path)
        mod.write_config_json(str(cfg_path), merged, data)
        print(f"\n[1/4] 已写出规范化配置 → {cfg_path.relative_to(ROOT)}（原文件已备份，打包后还原）")

    # 2) 图标（可选）
    extra: list[str] = []
    if args.icon and merged.get("icon_url"):
        url = merged["icon_url"]
        # ⚠️ 必须先建 build/：它被 .gitignore 忽略、fresh checkout 里不存在，
        #    而 curl 写不进去时报的是 `curl: (23) client returned ERROR on write`
        #    —— 看着像网络问题，换 URL 毫无用处（2026-09-22 实测）。
        build_dir = ELECTRON / "build"
        dest = build_dir / "icon.png"
        if dry:
            print(f"\n[2/4] 将下载图标 {url} → {dest.relative_to(ROOT)}")
        else:
            build_dir.mkdir(parents=True, exist_ok=True)
            run(["curl", "-fsSL", "--retry", "3", url, "-o", str(dest)], ELECTRON, env, False)
            magic = dest.read_bytes()[:8].hex()
            if magic != "89504e470d0a1a0a":
                sys.exit(f"✗ icon_url 下载的不是 PNG（文件头={magic}）")
            # 托盘图标取 resources/icons/icon.png（由 extraResources 从 webview-app/icons/
            # 拷进包），只设 -c.win.icon 对托盘无效 ⇒ 必须同步覆盖那一份。它是 tracked，
            # 所以先备份（Restorer 在 main 的 finally 里还原）。
            tray = WEBVIEW / "icons" / "icon.png"
            rr.file(tray)
            shutil.copy2(dest, tray)
            print(f"托盘图标已同步 → {tray.relative_to(ROOT)}（打包后还原）")
        extra = ["-c.win.icon=build/icon.png"]
    else:
        why = "配置里 icon_url 为空" if not merged.get("icon_url") else "未加 --icon（本地默认不覆盖 tracked 的 icons/）"
        print(f"\n[2/4] 跳过图标：{why}，沿用仓库默认图标")

    # 3) sidecar + 编译 + 铺资源
    #    ⚠️⚠️ 顺序关键：`w2a-proxy.exe` 必须**在 RESOLVE_SECRET 存在的同一环境里**编译。
    #    resolve-core 的 Secret 是 `option_env!("RESOLVE_SECRET")` 的编译期常量，
    #    先编后设 ⇒ sidecar 内置公开默认 Secret，用户配的 resolve_secret **静默失效**。
    #    这里 env 已经带上 RESOLVE_SECRET，所以直接串起来即可（cargo 按 dep-info 重编译）。
    print("\n[3/4] 编译 sidecar（继承 RESOLVE_SECRET）→ 编译 TS → 铺打包资源")
    run(["pnpm", "run", "sidecar"], ELECTRON, env, dry)
    run(["pnpm", "run", "build"], ELECTRON, env, dry)
    run(["pnpm", "run", "stage"], ELECTRON, env, dry)

    # 4) 出包
    forms: list[str] = []
    if args.target in ("electron-zip", "electron-all"):
        forms.append("zip")
    if args.target in ("electron-portable", "electron-all"):
        forms.append("portable")
    print(f"\n[4/4] electron-builder：{' + '.join(forms)}")

    # ⚠️ 受限网络下 electron-builder 会**自己去下载** Electron 运行时（223MB）并卡到超时：
    #       ⨯ Timeout awaiting 'request' for 600000ms  failedTask=build
    #       RequestError: Timeout awaiting 'request' for 600000ms
    #    这是环境问题、不是配置或代码问题——`node_modules/electron/dist` 里其实已经有一份。
    #    显式指给 electronDist 即可完全跳过下载（本机实测：加上之后不再出现该条 request 超时）。
    #    ⚠️ 必须写 `--config.electronDist=`（长写法）；`-c.electronDist=` 在部分版本上会被
    #       当成「配置文件路径」而报 ENOENT。
    extra_builder: list[str] = []
    local_dist = ELECTRON / "node_modules" / "electron" / "dist"
    if local_dist.exists():
        extra_builder.append("--config.electronDist=node_modules/electron/dist")
        print(f"    使用本地 Electron 运行时（跳过下载）：{local_dist.relative_to(ROOT)}")

    # 输出目录：默认 release/。⚠️ electron-builder 在打包前会**清空** appOutDir，
    # 若该目录里已有产物且被占用（或沙箱禁止删除）就会在 emptyDir 阶段炸：
    #     ⨯ [safe-delete] 操作失败: ...\release\win-unpacked\resources:
    #       Error during a `trash` operation: Unknown { description: "Some operations were aborted" }
    # 这时指一个**新目录**即可绕过（本机实测有效）。
    # ⚠️ 相对路径以 **electron-app/** 为基准（electron-builder 就在那下面跑），
    #    所以 `--out tests/xxx` 落进 `electron-app/tests/xxx` —— 该位置被
    #    `electron-app/.gitignore` 的 `tests/*/` 忽略，不会污染 git status。
    #    若按调用方 cwd（仓库根）解析，产物会落在仓库根的 tests/ 下而**不被忽略**。
    if args.out:
        p = Path(args.out)
        out_dir = p.resolve() if p.is_absolute() else (ELECTRON / p).resolve()
        extra_builder.append(f"--config.directories.output={out_dir}")
    else:
        out_dir = ELECTRON / "release"

    cmd = [
        "pnpm", "exec", "electron-builder",
        "--win", *forms,
        f"-c.productName={name}",
        *extra_builder,
        *extra,
    ]
    run(cmd, ELECTRON, env, dry)

    # 护栏：配置没进 asar 时 electron-builder 照样出包、双击照样打开，
    # 只是静默退回内置默认地址（example.com），构建日志全绿 ⇒ 必须显式校验。
    # ⚠️ check-asar-config.cjs 只在**没给参数**时去 `release/` 下自动找，
    # 用了 --out 就必须把 asar 路径显式传给它，否则它会报「未找到 app.asar」。
    check = ["node", "scripts/check-asar-config.cjs"]
    if args.out:
        check.append(str(out_dir / "win-unpacked" / "resources" / "app.asar"))
    run(check, ELECTRON, env, dry)

    print(f"\n产物目录：{out_dir}")


# ---------------------------------------------------------------------------
# Tauri：编译期读环境变量
# ---------------------------------------------------------------------------
def do_tauri(args, mod, merged, env, name, rr) -> None:
    if not have(["cargo", "--version"]):
        sys.exit("✗ 未找到 cargo")
    if not have(["cargo", "tauri", "--version"]):
        sys.exit(
            "✗ 未安装 tauri-cli。本地 Tauri 打包需要它：\n"
            '    cargo install tauri-cli --version "^2"\n'
            "  （CI 的 windows job 也是这一步；首次安装要编几分钟）"
        )

    dry = args.dry_run
    conf = WEBVIEW / "tauri.conf.json"

    # 1) 安装名 → productName（与 CI 同一个清洗规则）
    if dry:
        print(f"\n[1/3] 将改写 {conf.relative_to(ROOT)}：productName = {name}，mainBinaryName = web2app")
    else:
        rr.file(conf)  # tracked，必须备份
        data = json.loads(conf.read_text(encoding="utf-8"))
        data["productName"] = name
        # 固定主程序文件名 ⇒ target/release/web2app.exe 路径稳定，单文件版那步可以直接引用
        data["mainBinaryName"] = "web2app"
        conf.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print(f"\n[1/3] 已改写 {conf.relative_to(ROOT)}：productName = {name}（打包后还原）")

    # 2) 图标（可选）—— `cargo tauri icon` 会重新生成整套 icons/，而它是 tracked
    if args.icon and merged.get("icon_url"):
        url = merged["icon_url"]
        png = ROOT / "icon-custom.png"
        if dry:
            print(f"\n[2/3] 将下载图标 {url} 并执行 cargo tauri icon")
        else:
            rr.dir(WEBVIEW / "icons")
            run(["curl", "-fsSL", "--retry", "3", url, "-o", str(png)], ROOT, env, False)
            if png.read_bytes()[:8].hex() != "89504e470d0a1a0a":
                sys.exit("✗ icon_url 下载的不是 PNG")
            run(["cargo", "tauri", "icon", str(png)], WEBVIEW, env, False)
            png.unlink(missing_ok=True)
            print("icons/ 已重新生成（打包后整目录还原）")
    else:
        print("\n[2/3] 跳过图标，沿用仓库默认图标")

    # 3) 编译 + 打包
    #    ⚠️ 环境变量必须在 cargo 之前就位 —— Tauri 的配置是 `option_env!` 编译期常量，
    #    改了环境变量必须**重新编译**（cargo 通过 dep-info 的 env-dep 感知，无需 clean）。
    want_msi = args.target in ("tauri-msi", "tauri-all")
    want_zip = args.target in ("tauri-portable", "tauri-all")

    if want_msi:
        print("\n[3/3] cargo tauri build（产 MSI，同时也产出 web2app.exe）")
        run(["cargo", "tauri", "build"], WEBVIEW, env, dry)
    elif want_zip:
        print("\n[3/3] cargo tauri build --no-bundle（只编译 exe，不产 MSI）")
        run(["cargo", "tauri", "build", "--no-bundle"], WEBVIEW, env, dry)

    if want_zip:
        exe = WEBVIEW / "target" / "release" / "web2app.exe"
        out = WEBVIEW / "target" / "release" / f"{name}.exe"
        if dry:
            print(f"将复制 {exe.relative_to(ROOT)} → {out.relative_to(ROOT)}（单文件绿色版）")
        else:
            if not exe.exists():
                sys.exit(f"✗ 未找到 {exe}，主程序没编译出来")
            shutil.copy2(exe, out)
            print(f"单文件版 → {out.relative_to(ROOT)}（改名后即为 {name}.exe 免安装版）")

    print(f"\n产物目录：{WEBVIEW.relative_to(ROOT)}/target/release/bundle/")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
