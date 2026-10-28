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
