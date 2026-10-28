#!/usr/bin/env python3
"""把 workflow 里 Build Android APK 的 run 块抽出来，用假 cargo 喂三条分支。

本机没有 Android SDK/NDK，跑不了真构建；但「失败必须被看见」这条契约完全可以离线验证：
  A) cargo 失败（模拟 gradle / rustc 报错）→ 必须 exit 1，且主线日志里出现 ::error:: 与根因摘录
  B) cargo 成功但没产物 → 必须 exit 1，并打出产物目录内容
  C) cargo 成功且有产物 → 必须 exit 0，并列出产物
"""
import os
import pathlib
import shutil
import subprocess
import sys

import yaml

ROOT = pathlib.Path(__file__).resolve().parents[2]
WORK = pathlib.Path(os.environ.get("TMPDIR") or os.environ.get("TEMP") or "/tmp") / "bd-harness"


def extract(yml_path: pathlib.Path, step_name: str) -> str:
    doc = yaml.safe_load(yml_path.read_text(encoding="utf-8"))
    for job in doc["jobs"].values():
        for step in job.get("steps", []):
            if step.get("name") == step_name:
                return step["run"]
    raise SystemExit(f"{yml_path.name} 里找不到步骤：{step_name}")


FAKE_CARGO_FAIL = """#!/usr/bin/env bash
echo "   Compiling web2app v0.1.0"
echo "error[E0432]: unresolved import crate::nope" >&2
echo "FAILURE: Build failed with an exception." >&2
echo "* What went wrong:" >&2
echo "Execution failed for task ':app:compileUniversalReleaseKotlin'." >&2
exit 1
"""

FAKE_CARGO_OK = """#!/usr/bin/env bash
echo "   Compiling web2app v0.1.0"
exit 0
"""


def make_fake_cargo(mode: str) -> None:
    bindir = WORK / "bin"
    bindir.mkdir(parents=True, exist_ok=True)
    body = FAKE_CARGO_FAIL if mode == "fail" else FAKE_CARGO_OK
    p = bindir / "cargo"
    p.write_text(body, encoding="utf-8", newline="\n")
    p.chmod(0o755)


def run_case(label: str, script: str, mode: str, with_apk: bool) -> bool:
    if WORK.exists():
        shutil.rmtree(WORK)
    proj = WORK / "proj"
    (proj / "gen/android").mkdir(parents=True)
    runner = WORK / "runner"
    runner.mkdir(parents=True)
    if with_apk:
        apk_dir = proj / "gen/android/app/build/outputs/apk/universal/release"
        apk_dir.mkdir(parents=True)
        (apk_dir / "Demo-universal-release-unsigned.apk").write_bytes(b"PK\x03\x04")
    make_fake_cargo(mode)
    sh = WORK / "build.sh"
    sh.write_text(script, encoding="utf-8", newline="\n")

    env = dict(os.environ)
    env["PATH"] = f"{WORK / 'bin'}{os.pathsep}{env.get('PATH', '')}"
    env["RUNNER_TEMP"] = str(runner)
    # ⚠️ 故意用 GitHub Linux 默认模板的形态（bash -e，**不带 pipefail**）跑：
    #    脚本必须靠自己那行 `set -o pipefail` 站住，不依赖 runner 的 shell 模板。
    proc = subprocess.run(["bash", "-e", str(sh)], cwd=proj, env=env,
                          capture_output=True, text=True)
    out = proc.stdout + proc.stderr
    print(f"\n===== {label}（exit={proc.returncode}）=====")
    print("\n".join(out.splitlines()[:14]))
    return proc.returncode, out


def main() -> int:
    ymls = {
        "build-custom.yml": ROOT / ".github/workflows/build-custom.yml",
        "release.yml": ROOT / ".github/workflows/release.yml",
    }
    failures = []
    for name, path in ymls.items():
        script = extract(path, "Build Android APK")
        print(f"\n########## {name} ##########")

        code, out = run_case(f"{name} · A 构建失败", script, "fail", with_apk=False)
        if code == 0:
            failures.append(f"{name}: 构建失败却返回 0（失败仍被吞）")
        if "::error::" not in out:
            failures.append(f"{name}: 失败时没有 ::error:: 注解")
        if "unresolved import" not in out:
            failures.append(f"{name}: 失败时没摘出根因行（error[E0432]）")
        log = pathlib.Path(os.environ.get("TEMP", "/tmp")) / "bd-harness/runner/android-build.log"
        if not log.is_file() or "What went wrong" not in log.read_text(encoding="utf-8"):
            failures.append(f"{name}: 失败日志没落到 RUNNER_TEMP/android-build.log")

        code, out = run_case(f"{name} · B 成功但无产物", script, "ok", with_apk=False)
        if code == 0:
            failures.append(f"{name}: 无产物却返回 0")
        if "没产出 *-unsigned.apk" not in out:
            failures.append(f"{name}: 无产物时文案不对")

        code, out = run_case(f"{name} · C 成功且有产物", script, "ok", with_apk=True)
        if code != 0:
            failures.append(f"{name}: 有产物却返回 {code}")
        if "Demo-universal-release-unsigned.apk" not in out:
            failures.append(f"{name}: 成功时没列出产物")

    print()
    if failures:
        print("::error::harness 失败：")
        for f in failures:
            print("  - " + f)
        return 1
    print("Build 步骤行为验证全部通过 ✓（3 分支 × 2 workflow）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
