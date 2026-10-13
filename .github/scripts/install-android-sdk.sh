#!/usr/bin/env bash
# 安装 Android 构建所需的 SDK 平台 / 构建工具 —— build-custom.yml 与 release.yml 的 android 作业共用。
#
# 用法（**必须在 webview-app/ 目录下运行**；两个调用点的
# `defaults.run.working-directory: webview-app` 已保证这一点，脚本自身也会校验）：
#     bash ../.github/scripts/install-android-sdk.sh
#
# 职责：
#   1) 仓库里没有 gen/android 时先 `cargo tauri android init`（跳过交互式询问与 target 安装）；
#   2) 读出模板实际写的 compileSdk，从 sdkmanager 仓库里【真实存在的包名】中挑对应平台安装，
#      再按「模板钉住的版本 → 同主版本 → 最高可用」挑 build-tools。
#
# ⚠️ 为什么不能简单写 `sdkmanager "platforms;android-$SDK"`（2026-10-07 修复线上失败）：
#   ① Android 16 起 SDK 平台引入**次版本号**：仓库里可能只有 `platforms;android-37.0/-37.1/-37.2`，
#      **没有** `platforms;android-37`；而 tauri-cli 模板已把 compileSdk 前移到 37。旧写法直接
#      exit 1（新版 sdkmanager 对「找不到的包」不再只警告，会整步失败）：
#          Warning: Failed to find package 'platforms;android-37'
#   ② 新平台还先落在 **canary 通道**：2026-10-07 实测 stable(`repository2-1.xml`) 的平台只到
#      android-36，36.1 / 37.x 只在 canary(`repository2-3.xml`)。所以「列得到」还不够，
#      安装命令也必须带 `--channel=3`，否则仍然「找不到包」。
#
# 本地行为验证（别只看代码）：`bash tests/ci-sdk-step/run-tests.sh`
# —— 用假 sdkmanager + 从真实仓库清单生成的 stable/canary 数据干跑 36 条断言（含两处反向对照）。
#
# 环境变量 `ANDROID_SDK_STRICT`（可选）：非空时，「模板要的平台在仓库里不存在 ⇒ 回落最高可用」
# 这条**尽力而为**的降级会变成硬失败（`::error::` + 退出 1）。默认空 = 保留降级。
set -euo pipefail

# 严格模式开关（默认关 = 保留「尽力而为」的降级，见下方回落分支的说明）：
#   空/未设 → 模板要的平台不存在时，回落最高可用平台并改写 gradle（现状）；
#   非空   → 同一情形直接 `::error::` + 退出 1，让「模板已前移而环境没跟上」在安装步就暴露，
#            而不是等到 gradle 配置期才以更难读的报错出现。
ANDROID_SDK_STRICT="${ANDROID_SDK_STRICT:-}"

if [ ! -f tauri.conf.json ]; then
  echo "::error::install-android-sdk.sh 必须在 webview-app/ 目录下运行（当前目录：$PWD）"
  exit 2
fi

if [ ! -d "gen/android" ]; then
  cargo tauri android init --ci --skip-targets-install
fi

# ⚠️ 末尾 `|| true` 不能省：`ls` 只要有任一参数不存在，退出码就是 2，叠加下面的 pipefail + set -e
# 会让整步【静默】退出（零输出、exit 2）——本地干跑已实测复现并做了反向对照。
APP_GRADLE=$(ls gen/android/app/build.gradle.kts gen/android/app/build.gradle 2>/dev/null | head -n1 || true)
SDK=36
if [ -n "$APP_GRADLE" ]; then
  v=$(grep -oE 'compileSdk\s*[=:]\s*[0-9]+' "$APP_GRADLE" | grep -oE '[0-9]+' | head -n1 || true)
  [ -n "$v" ] && SDK=$v
fi
echo "模板 compileSdk=$SDK"

# 接受许可（安装任何 SDK 包都需要）。⚠️ `yes |` 在 sdkmanager 先退出时会收到 SIGPIPE，
# 叠加 pipefail 会把整步判成失败，因此必须 `|| true` 吞掉（已做反向对照验证）。
yes 2>/dev/null | sdkmanager --licenses >/dev/null 2>&1 || true

# 枚举仓库里【真实存在】的包名，落到文件再 grep（不用管道，避开 pipefail 的 SIGPIPE 陷阱）。
# ⚠️ 必须 `sort -u`：清单里同一个包名会出现两次（Path 列 + Location 列），不去重会让变量变成
# 多行、甚至把带换行的字符串当包名传给 sdkmanager（本地干跑实测踩到）。
list_pkgs() {
  sdkmanager --list ${1:-} 2>/dev/null \
    | grep -oE '(platforms;android-|build-tools;)[0-9][0-9.]*(-rc[0-9]+)?' | sort -u || true
}
PKG_LIST="${RUNNER_TEMP:-/tmp}/sdk-available-pkgs.txt"
list_pkgs > "$PKG_LIST"

# 默认(stable)清单里没有模板要的版本时，合并 canary 清单（并把 --channel=3 带给安装命令）
CHANNEL=""
if ! grep -qE "^platforms;android-$SDK(\.|$)" "$PKG_LIST"; then
  echo "默认(stable)通道清单里没有 android-$SDK，合并 canary 通道（--channel=3）"
  CHANNEL="--channel=3"
  list_pkgs --channel=3 >> "$PKG_LIST"
  sort -u "$PKG_LIST" -o "$PKG_LIST" || true
fi
echo "── 仓库可用 platforms（尾部 8 个）──"
grep -E '^platforms;' "$PKG_LIST" | sort -V | tail -n 8 || true
echo "── 仓库可用 build-tools（尾部 4 个）──"
grep -E '^build-tools;' "$PKG_LIST" | sort -V | tail -n 4 || true

# 选平台包：优先无小数的 android-N，其次最小的小数版本 N.x（N.x 的磁盘目录名带小数）
# `head -n1` 是兜底：万一清单去重失效，也保证这里只返回单行。
pick_platform() {
  local n=$1 exact minor
  exact=$(grep -Fx "platforms;android-$n" "$PKG_LIST" | head -n1 || true)
  if [ -n "$exact" ]; then printf '%s\n' "$exact"; return 0; fi
  minor=$(grep -E "^platforms;android-$n\.[0-9]+$" "$PKG_LIST" | sort -V | head -n1 || true)
  [ -n "$minor" ] && printf '%s\n' "$minor"
  return 0
}

if ! grep -qE '^platforms;android-[0-9]' "$PKG_LIST"; then
  # --list 解析为空（输出格式漂移等）：退回旧行为，直接试无小数的包名
  echo "::warning::未能从 sdkmanager --list 解析出平台包名，直接尝试 platforms;android-$SDK"
  PLATFORM="platforms;android-$SDK"
else
  PLATFORM=$(pick_platform "$SDK")
  if [ -z "$PLATFORM" ]; then
    # 仓库里没有模板要的版本 → 退到最高可用平台，并把 gradle 的 compileSdk/targetSdk 改写过去
    MAX_N=$(grep -E '^platforms;android-[0-9]+(\.[0-9]+)?$' "$PKG_LIST" | sed -nE 's|^platforms;android-([0-9]+).*|\1|p' | sort -n | tail -n1 || true)
    if [ -z "$MAX_N" ]; then
      echo "::error::仓库中找不到任何 platforms;android-* 包，无法继续"
      exit 1
    fi
    echo "::warning::仓库没有 android-$SDK，回落 android-$MAX_N（gradle 里同步改写 compileSdk/targetSdk）"
    # ⚠️ 这是**尽力而为**的降级：gradle 的 compileSdk 会被降到低于 Tauri 依赖预期的版本，
    #    很可能仍在 gradle 配置/编译期失败（AGP 的 sdkDownload 有机会自动补齐，故不直接判死）。
    #    要让这种情形硬失败，调用点设 ANDROID_SDK_STRICT=1。
    if [ -n "$ANDROID_SDK_STRICT" ]; then
      echo "::error::ANDROID_SDK_STRICT=1：模板要 android-$SDK，但仓库最高只有 android-$MAX_N，拒绝降级"
      exit 1
    fi
    PLATFORM=$(pick_platform "$MAX_N")
    SDK=$MAX_N
    if [ -n "$APP_GRADLE" ]; then
      sed -i -E "s/(compileSdk[[:space:]]*[=:][[:space:]]*)[0-9]+/\1$MAX_N/" "$APP_GRADLE"
      sed -i -E "s/(targetSdk[[:space:]]*[=:][[:space:]]*)[0-9]+/\1$MAX_N/" "$APP_GRADLE"
      grep -nE 'compileSdk|targetSdk' "$APP_GRADLE" || true
    fi
  fi
fi
echo "选定平台包：$PLATFORM"
sdkmanager $CHANNEL "$PLATFORM" >/dev/null

# 只装到 android-N.x 时补一个无小数的目录名（部分 AGP 只按 platforms/android-N 查目录）
N=$(printf '%s\n' "$PLATFORM" | sed -E 's|^platforms;android-([0-9]+).*|\1|')
if [ ! -d "$ANDROID_HOME/platforms/android-$N" ]; then
  SRC=$(ls -d "$ANDROID_HOME"/platforms/android-$N.* 2>/dev/null | sort -V | head -n1 || true)
  if [ -n "$SRC" ]; then
    ln -sfn "$SRC" "$ANDROID_HOME/platforms/android-$N"
    echo "已补软链 platforms/android-$N → $(basename "$SRC")"
  else
    echo "::warning::未找到 platforms/android-$N*，AGP 可能报 Failed to find Platform SDK"
  fi
fi
ls -d "$ANDROID_HOME"/platforms/android-* 2>/dev/null || true

# build-tools：模板一般已不指定版本 → 取与平台同主版本，其次最高可用；
# 装不上也不阻断（AGP 会按默认版本自行补齐/下载）。
BT_VER=""
if [ -n "$APP_GRADLE" ]; then
  BT_VER=$(grep -oE 'buildToolsVersion\s*[=:]\s*"[0-9.]+"' "$APP_GRADLE" | grep -oE '[0-9.]+' | head -n1 || true)
fi
if [ -z "$BT_VER" ]; then
  BT_VER=$(grep -Fx "build-tools;$N.0.0" "$PKG_LIST" | head -n1 | sed 's|^build-tools;||' || true)
fi
if [ -z "$BT_VER" ]; then
  BT_VER=$(grep -E '^build-tools;[0-9.]+$' "$PKG_LIST" | sed 's|^build-tools;||' | sort -V | tail -n1 || true)
fi
if [ -n "$BT_VER" ]; then
  echo "安装 build-tools;$BT_VER"
  sdkmanager $CHANNEL "build-tools;$BT_VER" >/dev/null || echo "::warning::build-tools;$BT_VER 安装失败，交 AGP 默认/自下载处理"
else
  echo "仓库无可用 build-tools 包，交 AGP 默认/自下载处理"
fi
