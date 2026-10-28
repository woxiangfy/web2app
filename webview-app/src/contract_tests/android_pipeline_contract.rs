/// Android 打包流水线的「共享脚本」契约（2026-10-13）。
///
/// 背景：同一段逻辑曾在 `build-custom.yml` 与 `release.yml` 里**各存一份内联 python**
/// （applicationId 派生 / strings.xml 应用名 / 状态栏 insets / APK 文件名），只靠人肉保持同步 ——
/// 本仓库历史上同类分裂就漏过 EXTRA / ICON_URL。现四块全部抽进 `.github/scripts/` 下的共享脚本，
/// 两个 workflow 只留一行调用；NDK 版本也从三处硬编码收敛为 `install-android-ndk.sh` 单一真源。
/// 另有一处 P0：工程根解析的默认值曾写死成 `webview-app/gen/android`，而 CI 的 cwd 已是
/// `webview-app` ⇒ `FileNotFoundError` ⇒ android job 在补丁步中断、后续构建/签名/上传全被跳过
/// （job 是 `continue-on-error: true`，所以「APK 没产出」看不出根因）。
///
/// 这些守卫要拦的正是「内联实现又被抄回去」「两壳能力再次分叉」「工程根又被写死」。
const BUILD_CUSTOM_YML: &str = include_str!("../../../.github/workflows/build-custom.yml");
const RELEASE_YML: &str = include_str!("../../../.github/workflows/release.yml");
const CI_YML: &str = include_str!("../../../.github/workflows/ci.yml");
/// 工程根解析的单一真源（P0 回归守卫的落点）。
const ANDROID_GEN_PY: &str = include_str!("../../../.github/scripts/android_gen.py");
/// 返回键补丁（应复用 android_gen 的解析，而非自带一份候选表）。
const PATCH_PY: &str = include_str!("../../../.github/scripts/patch-android-back.py");
/// 预写 tauri.conf.json（内含包名派生实现）。
const PREPARE_PY: &str = include_str!("../../../.github/scripts/prepare-android-conf.py");
const LABELS_PY: &str = include_str!("../../../.github/scripts/apply-android-labels.py");
const INSETS_PY: &str = include_str!("../../../.github/scripts/patch-android-insets.py");
const APK_NAME_PY: &str = include_str!("../../../.github/scripts/set-android-apk-name.py");
/// NDK 安装脚本（版本单一真源）。
const NDK_SH: &str = include_str!("../../../.github/scripts/install-android-ndk.sh");
/// NDK 版本。⚠️ 用 concat! 拆段：避免「守卫的针」与「被测字面量」同在一处而被自己满足。
const NDK_VERSION: &str = concat!("27.0.", "12077973");

/// ① 两个 Android 打包 workflow 必须调用**全部**共享脚本（而不是内联实现）。
#[test]
fn workflows_call_shared_android_scripts() {
    for (name, yml) in [
        ("build-custom.yml", BUILD_CUSTOM_YML),
        ("release.yml", RELEASE_YML),
    ] {
        for script in [
            "prepare-android-conf.py",
            "apply-android-labels.py",
            "patch-android-insets.py",
            "set-android-apk-name.py",
            "patch-android-back.py",
            "install-android-ndk.sh",
        ] {
            assert!(yml.contains(script), "{name} 必须调用共享脚本 {script}");
        }
    }
    assert!(
        CI_YML.contains("install-android-ndk.sh"),
        "ci.yml 的 check-android 也必须用同一份 NDK 安装脚本"
    );
}

/// ② 反向对照：内联实现不得回流（这些特征串只应存在于共享脚本里）。
#[test]
fn workflows_no_longer_inline_android_logic() {
    for (name, yml) in [
        ("build-custom.yml", BUILD_CUSTOM_YML),
        ("release.yml", RELEASE_YML),
    ] {
        for sentinel in [
            "setOnApplyWindowInsetsListener",    // insets 内联实现的标志
            "archivesBaseName",                  // APK 文件名内联实现的标志
            "import hashlib, json, os, re, sys", // 包名派生内联实现的标志
            "def esc_res",                       // strings.xml 改写内联实现的标志
        ] {
            assert!(
                    !yml.contains(sentinel),
                    "{name} 仍含内联实现特征串 `{sentinel}` —— 应改为调用 .github/scripts/ 下的共享脚本"
                );
        }
    }
    // 特征串在共享脚本里必须存在，否则上面的「不存在」断言是空的（守卫自我满足）。
    assert!(
        INSETS_PY.contains("setOnApplyWindowInsetsListener"),
        "insets 实现应落在共享脚本"
    );
    assert!(
        APK_NAME_PY.contains("archivesName"),
        "APK 名校验应落在共享脚本"
    );
    assert!(
        PREPARE_PY.contains("derive_package"),
        "包名派生应落在共享脚本"
    );
    assert!(
        LABELS_PY.contains("def esc_res"),
        "strings.xml 改写应落在共享脚本"
    );
}

/// ③ P0 回归守卫：工程根解析的首选候选必须是 **cwd 相对的** `gen/android`。
#[test]
fn gen_root_resolution_prefers_cwd_relative_gen_android() {
    assert!(
        ANDROID_GEN_PY
            .contains("GEN_ROOT_CANDIDATES = (\"gen/android\", \"webview-app/gen/android\")"),
        "候选必须且只能按这个顺序：gen/android（CI 的 cwd 形态）优先 —— \
             在仓库根手跑时自动回落到 webview-app/gen/android"
    );
    // 补丁脚本必须复用 android_gen（不得再自带一份写死的工程根）。
    assert!(
        PATCH_PY.contains("from android_gen import"),
        "patch-android-back.py 必须复用 android_gen 的工程根解析"
    );
    // ⚠️ 只禁「把工程根写成默认值」这一原始形态 —— 自测里合法地引用了候选串，不能一禁了之。
    assert!(
        !PATCH_PY.contains("default=\"webview-app/gen/android\""),
        "补丁脚本里不得再把工程根写成 --gen-root 的默认值（P0 缺陷的原始形态）"
    );
    assert!(
        PATCH_PY.contains("default=None"),
        "patch-android-back.py 的 --gen-root 默认必须为 None，交给 android_gen 自动探测"
    );
}

/// ④ NDK 版本单一真源：只准出现在 install-android-ndk.sh，workflow 一律不得再硬编码。
#[test]
fn ndk_version_is_single_sourced() {
    assert!(
        NDK_SH.contains(NDK_VERSION),
        "NDK 版本必须定义在 install-android-ndk.sh"
    );
    for (name, yml) in [
        ("build-custom.yml", BUILD_CUSTOM_YML),
        ("release.yml", RELEASE_YML),
        ("ci.yml", CI_YML),
    ] {
        assert!(
            !yml.contains(NDK_VERSION),
            "{name} 不得再硬编码 NDK 版本 —— 应调用 install-android-ndk.sh（单一真源）"
        );
    }
}

/// ⑤ 能力等价：release.yml 也必须支持 version（历史上两个 job 都不写 ⇒ 版本恒为 0.1.0）。
#[test]
fn release_supports_version_like_manual_packaging() {
    assert!(
        RELEASE_YML.matches("VERSION:").count() >= 2,
        "release.yml 的 windows 与 android 两个 job 都要注入 VERSION"
    );
    assert!(
        RELEASE_YML.contains("github.event.release.tag_name"),
        "VERSION 应有 release tag 回落（tag 惯例 v1.2.3，脚本会剥前导 v）"
    );
    // 两个 android job 都要用同一份预写脚本（version 由它写入 tauri.conf.json）。
    for (name, yml) in [
        ("build-custom.yml", BUILD_CUSTOM_YML),
        ("release.yml", RELEASE_YML),
    ] {
        assert!(
            yml.contains("prepare-android-conf.py"),
            "{name} 必须用共享预写脚本"
        );
    }
}

/// 剥掉整行注释后再做文本匹配。
///
/// ⚠️ 本仓库的教训：**针会被注释满足**。此次修的 bug 的注释里就写着 `pipefail` / `tee`，
/// 直接 `contains` 会让「把 `set -o pipefail` 删掉、只在注释里留一句」这种退化照样通过。
fn code_only(yml: &str) -> String {
    yml.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// ⑥ **构建失败不许被 tee 吞掉**（2026-10-28 线上 P0）。
///
/// 事故链：Linux 上 `run:` 不写 `shell:` 时的默认模板是 `bash -e {0}`（**不含** pipefail）
/// ⇒ `cargo tauri android build 2>&1 | tee log` 的退出码取 tee（恒 0）⇒ gradle / rustc 的
/// 失败被吞、步骤显示绿色 ⇒ 下一步签名报「未找到 *-unsigned.apk，构建产物异常」+
/// `find: gen/android/app/build/outputs: No such file or directory`。
/// 真实错误只在 artifact `android-build-log` 里，主线日志一行 error 都没有 —— 排查方向全偏。
///
/// 所以这里钉三件事：① 显式 `shell: bash`；② 显式 `set -o pipefail`；③ 失败分支必须打
/// `::error::`（否则失败只是「红了一行」，看不出根因在哪）。
#[test]
fn android_build_step_must_not_swallow_failures() {
    for (name, yml) in [
        ("build-custom.yml", BUILD_CUSTOM_YML),
        ("release.yml", RELEASE_YML),
    ] {
        let code = code_only(yml);
        let lines: Vec<&str> = code.lines().collect();
        let idx = lines
            .iter()
            .position(|l| l.contains("cargo tauri android build"))
            .unwrap_or_else(|| panic!("{name} 必须调用 cargo tauri android build"));
        // 取该行前后各若干行：`shell:` / `run: |` / `set -o pipefail` 都在它之前，
        // 失败分支与产物检查在它之后。
        let from = idx.saturating_sub(30);
        let to = (idx + 40).min(lines.len());
        let block = lines[from..to].join("\n");

        assert!(
            block.contains("shell: bash"),
            "{name} 的 Build Android APK 步骤必须显式 `shell: bash` —— \
             Linux 上不写 shell 的默认模板 `bash -e {{0}}` 不含 pipefail"
        );
        assert!(
            block.contains("set -o pipefail"),
            "{name} 的 Build Android APK 步骤必须 `set -o pipefail`，否则 `| tee` 会把 \
             gradle/rustc 的失败吞成退出码 0（2026-10-28 P0：构建失败却显示绿色，\
             报错落到下一步「未找到 *-unsigned.apk」）"
        );
        assert!(
            block.contains("::error::"),
            "{name} 的构建失败分支必须打 `::error::` 注解（否则 CI 页只红一行、看不出根因）"
        );

        // ⑦ 下游那句「未找到 APK」必须指向真正的排查方向，不能只说「构建产物异常」。
        let msg = code
            .lines()
            .find(|l| l.contains("未找到 *-unsigned.apk"))
            .unwrap_or_else(|| panic!("{name} 应有「未找到 *-unsigned.apk」的兜底报错"));
        assert!(
            msg.contains("android-build-log") && msg.contains("Build Android APK"),
            "{name} 的「未找到 *-unsigned.apk」文案必须指向 Build 步骤与 android-build-log \
             —— 这行只是结果不是原因，历史上正是它把排查方向带偏"
        );
    }
}
