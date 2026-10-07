#!/usr/bin/env python3
"""静态检查 GitHub Actions 工作流：YAML 合法性 + 内联脚本语法 + `.github/scripts/*.sh` 语法。

## 为什么需要

`.github/workflows/*.yml` 里塞了大量**内联脚本**（`python - <<'PY'`、`python3 -c '...'`、
大段 bash），另有若干**被 workflow 调用的 shell 脚本**（`.github/scripts/*.sh`，如
`install-android-sdk.sh`）。它们有三类只在 CI 上才炸、本地完全看不出来的错：

1. YAML 块标量与 heredoc 缩进没对齐 ⇒ 整个 workflow 无法解析，Actions 页只报一个语焉不详的错；
2. heredoc 里的 Python 语法错 ⇒ 打包跑到一半才失败（而且要等 checkout / 装工具链之后）；
3. `python3 -c '...'` 的**单引号包裹**里出现了 `'` ⇒ 单引号提前闭合，bash 把后面的
   Python 代码当成命令名去执行，报 `xxx: command not found` —— 排查方向会被带偏到 PATH 上。

本脚本把这三类都变成「提交前就能跑出结果」的检查。挂进 `ci.yml` 的 config-contract 作业。

用法：
    python3 .github/scripts/check-workflows.py
"""

from __future__ import annotations

import io
import os
import pathlib
import re
import subprocess
import sys
import tempfile

try:
    import yaml
except ImportError:  # pragma: no cover
    print("::error::需要 PyYAML（pip install pyyaml / CI 里的 python 自带）")
    sys.exit(1)

ROOT = pathlib.Path(__file__).resolve().parents[2]
WORKFLOWS = sorted((ROOT / ".github" / "workflows").glob("*.y*ml"))

HEREDOC_OPEN = re.compile(r"python3?\s+-\s*<<\s*['\"]?(\w+)['\"]?")
PY_C_OPEN = re.compile(r"python3?\s+-c\s+'")


def check_python_heredocs(name: str, run: str, errors: list[str]) -> None:
    lines = run.splitlines()
    i = 0
    while i < len(lines):
        m = HEREDOC_OPEN.search(lines[i])
        if not m:
            i += 1
            continue
        tag = m.group(1)
        start = i + 1
        end = start
        while end < len(lines) and lines[end].strip() != tag:
            end += 1
        if end >= len(lines):
            errors.append(f"{name}: heredoc `<<{tag}`（第 {i + 1} 行）没有闭合标记")
            return
        body = "\n".join(lines[start:end])
        try:
            compile(body, f"{name}:heredoc@{i + 1}", "exec")
        except SyntaxError as e:
            errors.append(f"{name}: heredoc（run 内第 {i + 1} 行起）Python 语法错误："
                          f"{e.msg}（第 {e.lineno} 行）")
        i = end + 1


def check_python_dash_c(name: str, run: str, errors: list[str]) -> None:
    """`python3 -c '...'`：整体被**单引号**包住，所以内容里不能出现任何 `'`。

    两种写法都要看：单行内联（`python3 -c 'code'`）与多行块（`python3 -c '` 换行 code 换行 `'`）。
    """
    lines = run.splitlines()
    i = 0
    while i < len(lines):
        if not PY_C_OPEN.search(lines[i]):
            i += 1
            continue
        # 单行内联：开引号之后还有内容，且行尾有闭引号 —— 只取引号之间的部分。
        # ⚠️ 行尾可能带续行符 `\`（把 python 的输出重定向到下一行），先剥掉再判。
        tail = lines[i].split("-c", 1)[1].strip().rstrip("\\").strip()
        if tail != "'" and tail.endswith("'") and len(tail) > 1:
            body = tail[1:-1]
            try:
                compile(body, f"{name}:dashc-inline@{i + 1}", "exec")
            except SyntaxError as e:
                errors.append(f"{name}: 内联 `python -c`（第 {i + 1} 行）Python 语法错误：{e.msg}")
            i += 1
            continue

        start = i + 1
        end = start
        while end < len(lines) and lines[end].strip() != "'":
            end += 1
        if end >= len(lines):
            errors.append(f"{name}: `python -c '`（第 {i + 1} 行）没有闭合的 `'` 行")
            return
        body = "\n".join(lines[start:end])
        if "'" in body:
            bad = next(ln for ln in lines[start:end] if "'" in ln)
            errors.append(f"{name}: `python -c '...'` 的内容里出现了单引号（会把 shell 字符串"
                          f"提前闭合）：{bad.strip()[:80]}")
        try:
            compile(body, f"{name}:dashc@{i + 1}", "exec")
        except SyntaxError as e:
            errors.append(f"{name}: `python -c`（run 内第 {i + 1} 行起）Python 语法错误："
                          f"{e.msg}（第 {e.lineno} 行）")
        i = end + 1


def _bash_is_usable(bash: str) -> bool:
    """用 `bash -c "exit 0"` 确认它**真的是 bash**。

    ⚠️ 必须实跑验证，不能只看文件存在 —— 见 `find_bash` 里的坑 1。
    """
    try:
        proc = subprocess.run([bash, "-c", "exit 0"], capture_output=True, timeout=30)
        return proc.returncode == 0
    except (OSError, subprocess.SubprocessError):
        return False


def find_bash() -> str | None:
    """找一个**真的能解析脚本**的 bash。

    ⚠️ 三个坑，顺序与验证方式都是为了绕开它们：

    1. `shutil.which("bash")` 在 Windows 上**可能命中 `C:\\Windows\\System32\\bash.exe`**
       —— 那是 **WSL 启动器**，不是 bash。它把 `-n <path>` 里的 `<path>` 当发行版名去解析，
       **退出码 -1**，并把一段 UTF-16 乱码写进 **stdout（不是 stderr）**。
       后果：本脚本对**每一个** `shell: bash` 步骤都报「bash -n 失败」且 detail 为空，
       看起来像整个仓库的 workflow 同时坏了 —— 实际是环境问题（2026-09-22 实测踩到）。
       故：**明确排除 system32**，且每个候选都要实跑验证。
    2. agent 沙箱里的 `bash` 是 cygwin shim，**不认 Windows 反斜杠路径**
       —— 传 `C:\\Users\\...\\x.sh` 它会报「文件不存在」，看起来像脚本本身有语法错。
       故 `check_bash_syntax` 统一把路径转成 POSIX 形式。
    3. 故候选顺序＝「显式 Git 安装位置优先 → 扫 PATH → which」，逐个实跑验证，
       第一个通过验证的才返回；全不通就返回 None（跳过该项检查，而不是制造假失败）。
    """
    import shutil

    cands: list[str] = []

    def add(p: str | None) -> None:
        if not p:
            return
        s = str(p)
        # 坑 1：WSL 启动器 —— 直接排除，连验证都不用做
        if "system32" in s.replace("/", "\\").lower():
            return
        if s not in cands:
            cands.append(s)

    # 显式 Git for Windows 安装位置（最可能装着「真正的 bash」）
    for p in (r"C:\Program Files\Git\bin\bash.exe",
              r"C:\Program Files\Git\usr\bin\bash.exe",
              r"C:\Program Files (x86)\Git\bin\bash.exe"):
        add(p)
    # 扫 PATH（which 只给第一个，可能是坑 1）
    for d in os.environ.get("PATH", "").split(os.pathsep):
        if d.strip():
            add(str(pathlib.Path(d.strip().strip('"')) / "bash.exe"))
    # 兜底：把 which 的结果也试一遍（可能不在 PATH 的常规写法里）
    add(shutil.which("bash"))

    for cand in cands:
        if pathlib.Path(cand).exists() and _bash_is_usable(cand):
            return cand
    return None


def check_bash_syntax(name: str, run: str, errors: list[str]) -> None:
    """把一段 bash 整体过一遍 `bash -n`（只解析、不执行）。

    调用点有两处：显式 `shell: bash` 的 workflow 步骤，以及 `.github/scripts/*.sh`
    （它们被 workflow 直接 `bash xxx.sh` 调用，语法错同样只在 CI 上炸）。
    """
    bash = find_bash()
    if not bash:
        return
    with tempfile.NamedTemporaryFile("w", suffix=".sh", delete=False,
                                     encoding="utf-8", newline="\n") as f:
        f.write(run)
        path = f.name
    try:
        env = {k: v for k, v in os.environ.items() if k not in ("BASH_ENV", "ENV", "NODE_OPTIONS")}
        # cygwin bash 不认 `C:\...`，必须转成 `C:/...`
        proc = subprocess.run([bash, "-n", pathlib.Path(path).as_posix()],
                              capture_output=True, encoding="utf-8", errors="replace",
                              env=env, timeout=60)
        if proc.returncode != 0:
            # stderr 优先；但**有些 bash（如 WSL 启动器）把报错写进 stdout**，
            # 只取 stderr 会得到空 detail、把真因藏掉 —— 故合并两路。
            raw = "\n".join(x for x in ((proc.stderr or ""), (proc.stdout or "")) if x.strip())
            detail = [ln for ln in raw.strip().splitlines() if ln.strip()]
            errors.append(f"{name}: bash -n 失败（rc={proc.returncode}）：{' | '.join(detail[-3:])[:400]}")
    except (OSError, subprocess.SubprocessError):
        pass
    finally:
        pathlib.Path(path).unlink(missing_ok=True)


def main() -> int:
    errors: list[str] = []
    checked = 0
    for path in WORKFLOWS:
        rel = path.relative_to(ROOT).as_posix()
        try:
            doc = yaml.safe_load(path.read_text(encoding="utf-8"))
        except yaml.YAMLError as e:
            errors.append(f"{rel}: YAML 解析失败：{e}")
            continue
        for job_name, job in (doc.get("jobs") or {}).items():
            for step in job.get("steps") or []:
                run = step.get("run")
                if not isinstance(run, str):
                    continue
                checked += 1
                label = f"{rel}::{job_name}::{step.get('name', '?')}"
                check_python_heredocs(label, run, errors)
                check_python_dash_c(label, run, errors)
                if step.get("shell") == "bash":
                    check_bash_syntax(label, run, errors)

    # 被 workflow 调用的共享 shell 脚本：不在 run 里，但同样是「只在 CI 上才炸」的脚本
    # （如 install-android-sdk.sh），一并过 bash -n。
    scripts = sorted((ROOT / ".github" / "scripts").glob("*.sh"))
    for path in scripts:
        rel = path.relative_to(ROOT).as_posix()
        checked += 1
        check_bash_syntax(rel, path.read_text(encoding="utf-8"), errors)

    if errors:
        print("工作流检查发现问题：")
        for e in errors:
            print(f"::error::{e}")
        return 1
    print(f"工作流检查通过 ✓（{len(WORKFLOWS)} 个 workflow / {checked} 个脚本片段，"
          f"其中 {len(scripts)} 个 .github/scripts/*.sh）")
    return 0


if __name__ == "__main__":
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except Exception:  # noqa: BLE001
            pass
    sys.exit(main())
