#!/usr/bin/env bash
# 本地干跑：CI 步骤「Initialize Android project (gen/android) + 安装匹配平台」的脚本（不进 CI）。
# 用法：bash tests/ci-sdk-step/run-tests.sh [step 脚本路径]
# 测试数据取自真实仓库清单（data/ 由 dl.google.com 的 repository2-1.xml = stable /
# repository2-3.xml = canary 生成）：
#   stable 的平台只到 android-36；android-36.1 / 37.0 / 37.1 / 37.2 只在 canary。
#
# ⚠️ 用例目录一律写到**系统临时区**（跑一次约 72 个文件），**不写进仓库树** —— 之前落在
#    tests/ci-sdk-step/tmp/ 里，9 次跑就攒了 455 个文件。要固定留证据时：
#      CI_SDK_STEP_TMP=/some/dir bash tests/ci-sdk-step/run-tests.sh
set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
STABLE="$HERE/data/repo-pkgs-stable.txt"
CANARY="$HERE/data/repo-pkgs-canary.txt"
EXACT37="$HERE/data/repo-pkgs-exact37.txt"
STEP_BODY="${1:-$HERE/../../.github/scripts/install-android-sdk.sh}"
TMP_BASE="${CI_SDK_STEP_TMP:-${TMPDIR:-/tmp}/ci-sdk-step}"
RUN_DIR="$TMP_BASE/run-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$RUN_DIR"
echo "用例目录（系统临时区，可整个删：$TMP_BASE）：$RUN_DIR"
PASS=0; FAIL=0
ok() { echo "  [PASS] $1"; PASS=$((PASS + 1)); }
no() { echo "  [FAIL] $1（期望 [$3]，实际 [$2]）"; FAIL=$((FAIL + 1)); }
chk() { if [ "$2" = "$3" ]; then ok "$1"; else no "$1" "$2" "$3"; fi; }
chk_has() { if printf '%s' "$2" | grep -q -- "$3"; then ok "$1"; else no "$1" "$2" "含 [$3]"; fi; }
chk_not() { if printf '%s' "$2" | grep -q -- "$3"; then no "$1" "$2" "不含 [$3]"; else ok "$1"; fi; }
count() { grep -c -e "$2" "$1" 2>/dev/null || true; }

# $1=用例名 $2=模板 compileSdk（空=不写 gradle）$3=stable 清单 $4=canary 清单 $5=额外 gradle 行
case_run() {
  local name="$1" sdk="$2" stable="$3" canary="$4" extra="${5:-}"
  local d="$RUN_DIR/$name"
  mkdir -p "$d/gen/android/app" "$d/sdk" "$d/bin"
  : > "$d/tauri.conf.json"   # 脚本会校验「必须在 webview-app/ 下运行」
  if [ -n "$sdk" ]; then
    {
      printf 'android {\n'
      printf '    compileSdk = %s\n' "$sdk"
      printf '    defaultConfig {\n'
      printf '        targetSdk = %s\n' "$sdk"
      [ -n "$extra" ] && printf '%s\n' "$extra"
      printf '    }\n}\n'
    } > "$d/gen/android/app/build.gradle.kts"
  fi
  cp "$HERE/fake-bin/sdkmanager" "$d/bin/sdkmanager"
  cp "${STEP_SRC:-$STEP_BODY}" "$d/step.sh"
  : > "$d/fake.log"
  (
    cd "$d" || exit 99
    ANDROID_HOME="$d/sdk" RUNNER_TEMP="$d" FAKE_PKGS="$stable" FAKE_PKGS_CANARY="$canary" \
      FAKE_LOG="$d/fake.log" PATH="$d/bin:$PATH" bash step.sh > out.log 2>&1
  )
  echo "$?" > "$d/rc"
  D="$d"
}
log() { cat "$D/fake.log"; }
out() { cat "$D/out.log"; }

echo "== 用例 1：模板 compileSdk=37（stable 只到 36，37.x 只在 canary）—— 本次 CI 报错的场景"
case_run c1-37 37 "$STABLE" "$CANARY"
chk "退出码为 0" "$(cat "$D/rc")" "0"
chk_has "先查了默认(stable)清单" "$(log)" "sdkmanager --list"
chk_has "发现缺 37 后补查 canary" "$(log)" "sdkmanager --list --channel=3"
chk_has "安装 platforms;android-37.0 时带上 --channel=3" "$(log)" "sdkmanager --channel=3 platforms;android-37.0"
chk "没有直接尝试不存在的 platforms;android-37" "$(count "$D/fake.log" '^sdkmanager platforms;android-37$')" "0"
chk_has "安装了 build-tools;37.0.0" "$(log)" "build-tools;37.0.0"
chk_has "补了无小数目录名" "$(out)" "已补软链 platforms/android-37"
if [ -e "$D/sdk/platforms/android-37" ]; then ok "platforms/android-37 已存在（内容同 android-37.0）"; else no "platforms/android-37 已存在" "不存在" "存在"; fi

echo "== 用例 2：模板 compileSdk=36（stable 里就有）→ 不应触碰 canary"
case_run c2-36 36 "$STABLE" "$CANARY"
chk "退出码为 0" "$(cat "$D/rc")" "0"
chk_has "安装了 platforms;android-36" "$(log)" "platforms;android-36"
chk_has "安装了 build-tools;36.0.0" "$(log)" "build-tools;36.0.0"
chk "没有多查 canary 通道" "$(count "$D/fake.log" '--channel=3')" "0"
chk "未多此一举建目录软链" "$(count "$D/out.log" '已补软链')" "0"

echo "== 用例 3：模板 compileSdk=39（两个通道都没有）→ 回落最高可用 + 改写 gradle"
case_run c3-39 39 "$STABLE" "$CANARY"
chk "退出码为 0" "$(cat "$D/rc")" "0"
chk_has "打了回落 warning" "$(out)" "::warning::仓库没有 android-39，回落 android-37"
chk_has "安装了 platforms;android-37.0" "$(log)" "platforms;android-37.0"
chk "gradle compileSdk 被改写为 37" "$(count "$D/gen/android/app/build.gradle.kts" 'compileSdk = 37$')" "1"
chk "gradle targetSdk 被改写为 37" "$(count "$D/gen/android/app/build.gradle.kts" 'targetSdk = 37$')" "1"

echo "== 用例 4：没有 gen/android gradle（解析不出 compileSdk）→ 默认 36"
case_run c4-nogradle "" "$STABLE" "$CANARY"
chk "退出码为 0" "$(cat "$D/rc")" "0"
chk_has "按默认 36 安装" "$(log)" "platforms;android-36"

echo "== 用例 5：仓库里存在 无小数 platforms;android-37 → 优先选它"
case_run c5-exact37 37 "$EXACT37" "$CANARY"
chk "退出码为 0" "$(cat "$D/rc")" "0"
chk_has "安装了 platforms;android-37（无小数）" "$(log)" "platforms;android-37"
chk "没有退而求其次装 37.0" "$(count "$D/fake.log" 'android-37\.0')" "0"

echo "== 用例 6：模板显式钉住 buildToolsVersion → 优先用模板值"
case_run c6-bt 37 "$STABLE" "$CANARY" 'buildToolsVersion = "35.0.0"'
chk "退出码为 0" "$(cat "$D/rc")" "0"
chk_has "安装了 build-tools;35.0.0" "$(log)" "build-tools;35.0.0"

echo "== 用例 7：不在 webview-app/ 目录下运行（没有 tauri.conf.json）→ 立刻报错退出"
GUARD="$RUN_DIR/c9-guard"
mkdir -p "$GUARD"
cp "$STEP_BODY" "$GUARD/step.sh"
( cd "$GUARD" && bash step.sh > out.log 2>&1 )
echo "$?" > "$GUARD/rc"
chk "退出码为 2（提前失败，不会误跑 sdkmanager）" "$(cat "$GUARD/rc")" "2"
chk_has "给出明确错误信息" "$(cat "$GUARD/out.log")" "必须在 webview-app/ 目录下运行"

echo "== 反向对照 A：去掉 licenses 那行的 \`|| true\` → 应非 0 退出"
NEG="$RUN_DIR/step-neg-licenses.sh"
sed 's#^yes 2>/dev/null | sdkmanager --licenses >/dev/null 2>&1 || true$#yes 2>/dev/null | sdkmanager --licenses >/dev/null 2>\&1#' "$STEP_BODY" > "$NEG"
if grep -q 'licenses >/dev/null 2>&1$' "$NEG" && [ "$(count "$NEG" 'licenses.*|| true')" = "0" ]; then
  ok "反向样本已生成（licenses 行去掉 || true）"
else
  no "反向样本生成" "未生效" "licenses 行已去兜底"
fi
STEP_SRC="$NEG"; case_run c7-neg-licenses 37 "$STABLE" "$CANARY"; STEP_SRC=""
chk_has "确实跑过 licenses 之后才失败" "$(out)" "模板 compileSdk=37"
if [ "$(cat "$D/rc")" != "0" ]; then ok "去掉 \`|| true\` 后确实非 0 退出（pipefail + SIGPIPE 真的会咬人）"; else no "应当非 0 退出" "0" "非 0"; fi

echo "== 反向对照 B：去掉 APP_GRADLE 那行的 \`|| true\` → 应静默 exit 2"
NEG2="$RUN_DIR/step-neg-ls.sh"
sed 's#| head -n1 || true)$#| head -n1)#' "$STEP_BODY" > "$NEG2"
if [ "$(count "$NEG2" 'build.gradle 2>/dev/null | head -n1)')" = "1" ]; then
  ok "反向样本已生成（APP_GRADLE 行去掉 || true）"
else
  no "反向样本生成" "未生效" "APP_GRADLE 行已去兜底"
fi
STEP_SRC="$NEG2"; case_run c8-neg-ls 37 "$STABLE" "$CANARY"; STEP_SRC=""
chk "确实以 exit 2 静默失败（零输出）" "$(cat "$D/rc")" "2"
chk "失败时 out.log 为空（正是这个坑难查的原因）" "$(wc -c < "$D/out.log" | tr -d ' ')" "0"

echo
echo "===== 结果：PASS=$PASS FAIL=$FAIL（用例目录 $RUN_DIR）====="
[ "$FAIL" = "0" ]
