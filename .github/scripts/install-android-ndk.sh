#!/usr/bin/env bash
# 安装 Android NDK —— build-custom.yml / release.yml / ci.yml **三处共用**，版本单一真源。
#
# 用法（**必须在 webview-app/ 目录下运行**；三个调用点的
# `defaults.run.working-directory: webview-app` 已保证这一点）：
#     bash ../.github/scripts/install-android-ndk.sh
#
# 为什么抽出来：NDK 版本原先在三个 workflow 里**各写死一遍**（`27.0.12077973` × 3），
# 上游 Tauri 模板换 NDK 时必然漂移 —— 与 install-android-sdk.sh 同一类问题的同一套解法。
#
# 导出三个变量（覆盖各调用点的不同需要）：
#   ANDROID_NDK_HOME  —— 通用
#   NDK_HOME          —— tauri-cli / gradle 侧识别
#   ANDROID_NDK_ROOT  —— cargo-ndk（ci.yml 的 check-android）识别。⚠️ 不设它会与
#                        setup-android 预置的 NDK 不一致，cargo-ndk 报 "doesn't match" 警告。
set -euo pipefail

NDK_VERSION="${W2A_NDK_VERSION:-27.0.12077973}"

# ⚠️ 末尾 `|| true` 不能省：`yes |` 在 sdkmanager 先退出时会收到 SIGPIPE（141），叠加
# pipefail 会把整步判成失败 —— 与 install-android-sdk.sh 的 licenses 行同一个坑。
# 但「装不上」不能因此被吞掉 ⇒ 下面用**存在性校验**兜底，不存在即硬失败。
yes 2>/dev/null | sdkmanager "ndk;${NDK_VERSION}" >/dev/null 2>&1 || true

NDK_DIR="${ANDROID_HOME:?ANDROID_HOME 未设置（应先跑 actions/setup-android）}/ndk/${NDK_VERSION}"
if [ ! -d "$NDK_DIR" ]; then
  echo "::error::NDK 安装失败：$NDK_DIR 不存在（sdkmanager 未装上 ndk;${NDK_VERSION}）"
  exit 1
fi

if [ -n "${GITHUB_ENV:-}" ]; then
  {
    echo "ANDROID_NDK_HOME=$NDK_DIR"
    echo "NDK_HOME=$NDK_DIR"
    echo "ANDROID_NDK_ROOT=$NDK_DIR"
    echo "W2A_NDK_VERSION=$NDK_VERSION"
  } >> "$GITHUB_ENV"
fi
export ANDROID_NDK_HOME="$NDK_DIR" NDK_HOME="$NDK_DIR" ANDROID_NDK_ROOT="$NDK_DIR"
echo "NDK 就绪：$NDK_DIR"
