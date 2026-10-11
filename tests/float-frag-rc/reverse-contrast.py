#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
v1.77b 新守卫的**反向对照**（负向验证）。

纪律（项目约定）：新增守卫必须反向对照 —— 把被测不变量改坏，确认守卫如期 FAILED，
再按字节还原并比对 sha256。否则守卫可能是「自我满足」的假绿。

⚠️⚠️ 本脚本第一版有两个**会让它谎报成功**的缺陷（2026-10-11 自查抓到）：
  ① 用 `cargo test --all <叶子名> -- --exact`：真实路径带模块前缀
     （`contract_tests::float_window_contract::<名字>`）⇒ `--exact` 一个测试都匹配不到，
     输出是 `0 passed; 169 filtered out`；「没跑」被当成了「通过」。
  ② `finally` 里的 `return` 会**覆盖** try 里的 `return` ⇒ 返回的永远是「还原是否成功」，
     于是四项全部显示 ✅（其实只证明了还原成功，什么都没验证）。
  修法：判据改成解析 `test result:` 那一行（必须出现 `1 passed` 才算真的跑了）；
  结果用变量收集，`return` 放在 `finally` 之后。

四条对照：
  A. 球径 56px 改回 46px            → float_page_ball_fills_the_live_pillar 必须 FAILED
  B. hover 改回 scale(1.1)          → 同上必须 FAILED
  C. collapsed 改回 opacity: .92    → 同上必须 FAILED
  D. delete_taskbar_tab 去掉 HrInit → float_every_show_path_goes_through_show_ball 必须 FAILED

用法：python reverse-contrast.py        （全跑）
      python reverse-contrast.py A B    （只跑指定项）
"""
import hashlib
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
HTML = ROOT / "webview-app/src/floaticon.html"
RS = ROOT / "webview-app/src/floatwin.rs"

TESTS = {
    "A": (HTML, "float_page_ball_fills_the_live_pillar"),
    "B": (HTML, "float_page_ball_fills_the_live_pillar"),
    "C": (HTML, "float_page_ball_fills_the_live_pillar"),
    "D": (RS, "float_every_show_path_goes_through_show_ball"),
}

# (目标文件, 旧片段, 新片段) —— 全部**逐字节**替换（.encode('utf-8') 后按字节匹配，
# 绝不用文本模式改：本机 core.autocrlf 与「统一 LF」的约定冲突，行尾随时可能变）。
MUTATIONS = {
    "A": (HTML, "    width: 56px;\n    height: 56px;", "    width: 46px;\n    height: 46px;"),
    "B": (HTML, "  html #w2f-ball:hover {\n    filter: brightness(1.12);",
               "  html #w2f-ball:hover {\n    transform: translate(-50%, -50%) scale(1.1);"),
    "C": (HTML, "  html.w2f-collapsed #w2f-ball {\n    filter: brightness(.92);",
               "  html.w2f-collapsed #w2f-ball {\n    opacity: .92;"),
    # D：整块删掉 HrInit 调用（保留能编译的最小改动 —— 若换成别的调用，守卫可能是被编译器
    #    而不是被断言抓住，那就不是一次有效的负向对照）。
    "D": (RS,
          "                if let Err(e) = list.HrInit() {\n"
          "                    dlog!(\"[float] ITaskbarList::HrInit 失败（DeleteTab 将无效，任务栏按钮可能残留）：{e}\");\n"
          "                    return;\n"
          "                }\n",
          ""),
}

RESULT_RE = re.compile(r"test result: (\w+)\. (\d+) passed; (\d+) failed")
# ⚠️ 第二版又踩的坑：cargo 在**测试失败**时也会打印一行 `error: test failed, to rerun ...`，
#    若用 `^error:` 判「编译失败」，四条对照会被一律误标成编译失败 —— 看起来像抓到了，
#    其实没证明是**断言**抓到的。真正的编译失败特征是 `error[E0xxx]` 或 `could not compile`。
COMPILE_ERR_RE = re.compile(r"^error\[E\d+\]|^error: could not compile", re.M)


def sha(p: pathlib.Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()


def run_one(key: str) -> bool:
    """返回 True 表示「守卫如期抓住了改坏」。"""
    _, test = TESTS[key]
    mut_path, old_s, new_s = MUTATIONS[key]
    old, new = old_s.encode("utf-8"), new_s.encode("utf-8")
    before = sha(mut_path)
    data = mut_path.read_bytes()
    if old not in data:
        print(f"[{key}] SETUP-FAIL: 在 {mut_path.name} 里找不到待改坏的片段（守卫或被改点已漂移）")
        return False

    mut_path.write_bytes(data.replace(old, new, 1))
    caught = False
    try:
        r = subprocess.run(
            ["cargo", "test", "--all", test],
            cwd=str(ROOT / "webview-app"),
            capture_output=True, text=True, encoding="utf-8", errors="replace",
        )
        out = r.stdout + r.stderr
        # 只认「真的跑了这个测试」的那些结果行：必须 passed+failed>0
        ran = [m for m in RESULT_RE.finditer(out) if int(m.group(2)) + int(m.group(3)) > 0]
        compile_err = bool(COMPILE_ERR_RE.search(out))
        if not ran and not compile_err:
            print(f"[{key}] ❌ 测试根本没跑起来（filter 没匹配到）—— 这条对照无效！")
            print(out[-1500:])
        elif compile_err:
            # 编译不过也算「被抓住」，但**不是**断言抓的 ⇒ 对守卫而言是一次弱对照，如实标注。
            caught = True
            print(f"[{key}] 改坏 {mut_path.name} → 编译失败（弱对照：是编译器拦下的，不是断言）")
        else:
            statuses = {(m.group(1), m.group(2), m.group(3)) for m in ran}
            nfail = sum(int(s[2]) for s in statuses)
            npass = sum(int(s[1]) for s in statuses)
            caught = nfail > 0 and npass == 0
            print(
                f"[{key}] 改坏 {mut_path.name} → 实跑 passed={npass} failed={nfail}"
                f" ⇒ {'PASS（守卫如期抓住）' if caught else '❌ 假绿：守卫没抓住！'}"
            )
            if not caught:
                print(out[-2500:])
    finally:
        mut_path.write_bytes(data)
        after = sha(mut_path)
        if after != before:
            print(f"[{key}] ❌ 还原 sha256 不一致！{before} -> {after}")
            caught = False
        else:
            print(f"[{key}] 还原 sha256 一致（{before[:16]}…）")
    return caught


def main() -> int:
    keys = sys.argv[1:] or list(MUTATIONS)
    results = {k: run_one(k) for k in keys}
    print("\n=== 反向对照汇总 ===")
    for k in keys:
        print(f"  {k}: {'✅ 如期 FAILED' if results[k] else '❌ 未抓住 / 无效对照'}")
    return 1 if any(not results[k] for k in keys) else 0


if __name__ == "__main__":
    raise SystemExit(main())
