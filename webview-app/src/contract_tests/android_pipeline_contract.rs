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

/// ① 两个 Android 打包 workflow 与 CI 必须共用同一份 NDK 安装脚本（版本单一真源，见 ④）。
///
/// ⚠️ 「共享脚本有没有被调用」不再靠**手写名单** —— 名单会变成第二个真源（新增脚本不自动
///    纳入、删脚本还要回来改测试）。那份检查现在由 ⑧ 的目录派生版承担。
#[test]
fn workflows_share_the_ndk_installer() {
    for (name, yml) in [
        ("build-custom.yml", BUILD_CUSTOM_YML),
        ("release.yml", RELEASE_YML),
    ] {
        assert!(
            yml.contains("install-android-ndk.sh"),
            "{name} 必须调用共享脚本 install-android-ndk.sh"
        );
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

/// 读取 `.github/scripts/` 下的全部 `.py`（文件名 + 源码），按文件名排序保证报错可复现。
///
/// 用 `CARGO_MANIFEST_DIR` 定位，避免依赖 `cargo test` 的 cwd。
fn python_scripts() -> Vec<(String, String)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.github/scripts");
    let mut out: Vec<(String, String)> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("读不到 {}：{e}", dir.display()))
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|x| x.to_string_lossy() == "py")
                .unwrap_or(false)
        })
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let src = std::fs::read_to_string(e.path()).unwrap_or_default();
            (name, src)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// 从 workflow 正文里抽「**真的**调用了哪个脚本、在第几行」。
///
/// 只认 `python3 … <name>.py` 形态：整行注释先被 `code_only` 剥掉，含 `::error::` 的
/// 提示文案行与 `python3 -c` / `-m` 一律不算 —— 上一版正是被 `echo "::error::…"` 里的
/// 脚本名满足过一次（假绿）。`$GITHUB_WORKSPACE/` 前缀会先剥掉；其余 `$` 变量拼接的
/// 路径静态推不出来，直接跳过（本仓库的 Android 脚本调用都不走这种形态）。
fn real_calls(yml: &str) -> Vec<(String, usize)> {
    let code = code_only(yml);
    let mut out = Vec::new();
    for (i, line) in code.lines().enumerate() {
        if !line.contains("python3") || line.contains("::error::") {
            continue;
        }
        for tok in line.split_whitespace() {
            let t = tok
                .trim_matches(|c| c == '"' || c == '\'' || c == '`')
                .replace("$GITHUB_WORKSPACE/", "")
                .replace("${GITHUB_WORKSPACE}/", "");
            if !t.ends_with(".py") || t.contains('$') {
                continue;
            }
            if let Some(base) = t.rsplit('/').next() {
                out.push((base.to_string(), i));
            }
        }
    }
    out
}

/// ⑧ **补丁链的调用关系一律由目录派生**（2026-11-07 第二轮修正）。
///
/// 上一版把 9 条链 + 3 个旁路脚本**手写**成两个常量数组 —— 那等于「测试又抄了一份打包名单」：
/// 新增脚本不会自动纳入、删脚本还得回来改测试，名单本身成了第二个真源。而现在唯一的权威是
/// **打包 workflow 的步骤本身**，测试只做派生：
///
/// ① 从 `.github/scripts/` **目录**发现管线脚本（机械判据见下，不用任何人写名单）；
/// ② 每个被发现者都必须被**两份**打包 workflow 真实调用；
/// ③ 反向：workflow 里不得调用目录里**不存在**的脚本（改名/删文件忘了同步会红）；
/// ④ 两份 workflow 的调用顺序必须**互相一致**（跨 workflow 自洽，而不是照抄文档里的顺序）；
/// ⑤ 管线脚本必须自带 `--selftest`，否则 `ci.yml` 的目录发现循环会**静默跳过**它；
/// ⑥ `ci.yml` 的自测循环必须是「按目录发现 + 非空断言」形态（空集不许假绿）。
#[test]
fn android_pipeline_is_derived_from_scripts_dir_and_wired_consistently() {
    let scripts = python_scripts();
    assert!(
        !scripts.is_empty(),
        "读不到 .github/scripts/*.py —— 路径或读取逻辑有问题"
    );

    // 机械判据（不写名单）：
    //   a) 源码里引用 `android_gen` —— 凡是改 `gen/android` 的脚本都共用那份工程根解析；
    //   b) 文件名以 `patch-android-` 开头 —— 兜住「不改 Kotlin 只改 JS/资源」的补丁脚本。
    // `android_gen.py` 自身是那个被 import 的模块，不算管线脚本。
    let pipeline: Vec<String> = scripts
        .iter()
        .filter(|(n, src)| {
            n.as_str() != "android_gen.py"
                && (src.contains("android_gen") || n.starts_with("patch-android-"))
        })
        .map(|(n, _)| n.clone())
        .collect();
    assert!(
        !pipeline.is_empty(),
        "管线脚本发现结果为空 ⇒ 判据写错会让整条守卫空集假绿"
    );

    // ② 每个管线脚本都必须被两份打包 workflow 真实调用。
    for (name, yml) in [
        ("build-custom.yml", BUILD_CUSTOM_YML),
        ("release.yml", RELEASE_YML),
    ] {
        let calls = real_calls(yml);
        for script in &pipeline {
            assert!(
                calls.iter().any(|(s, _)| s == script),
                "{name} 没有真实调用 {script}（只被 echo/注释提到不算）—— 删掉它不会让任何测试\
                 变红，但 Android 侧对应能力会静默消失，只有真机才能发现"
            );
        }
    }

    // ③ 反向：workflow 不得调用目录里不存在的脚本。
    let known: Vec<&str> = scripts.iter().map(|(n, _)| n.as_str()).collect();
    for (name, yml) in [
        ("build-custom.yml", BUILD_CUSTOM_YML),
        ("release.yml", RELEASE_YML),
        ("ci.yml", CI_YML),
    ] {
        for (script, line) in real_calls(yml) {
            assert!(
                known.contains(&script.as_str()),
                "{name}:{line} 调用了不存在的脚本 {script} —— 改名或删文件后忘了同步调用点"
            );
        }
    }

    // ④ 两份 workflow 的管线脚本调用顺序必须一致（派生不变量：顺序错位会让 Kotlin 模板 /
    //    `override` 的叠加关系静默错乱，而任何单侧命令都跑不出问题）。
    let order = |yml: &str| -> Vec<String> {
        let mut seen: Vec<String> = Vec::new();
        for (s, _) in real_calls(yml) {
            if pipeline.contains(&s) && !seen.contains(&s) {
                seen.push(s);
            }
        }
        seen
    };
    assert_eq!(
        order(BUILD_CUSTOM_YML),
        order(RELEASE_YML),
        "两份打包 workflow 的补丁链调用顺序不一致 —— 以 workflow 步骤为唯一权威，两侧必须同序"
    );

    // ⑤ 管线脚本必须有自测入口，否则 ⑥ 的目录发现循环会静默跳过它。
    for (n, src) in &scripts {
        if pipeline.contains(n) {
            assert!(
                src.contains("--selftest"),
                "{n} 没有 --selftest —— ci.yml 的目录发现循环会跳过它，该脚本的自测在 CI 永不执行"
            );
        }
    }

    // ⑥ ci.yml 的自测循环必须由目录发现驱动（不得再手写脚本名清单），且发现结果非空。
    let ci = code_only(CI_YML);
    assert!(
        ci.contains("for f in .github/scripts/*.py"),
        "ci.yml 的自测循环必须按目录发现，不得再手写脚本名清单"
    );
    assert!(
        ci.contains("grep -q -- '--selftest'"),
        "ci.yml 的循环必须按能力过滤：纯模块（android_gen.py）与 CI 工具（check-workflows.py）没有 --selftest"
    );
    assert!(
        ci.contains("found=0") && ci.contains("::error::"),
        "ci.yml 的循环必须断言「至少发现一个脚本」，否则规则写错时空集也全绿"
    );
}

/// ⑨ #13 启动底色：主题补丁里的两个色值必须与 `src/splash.html` 的 `background` 逐字一致。
///
/// 为什么要跨文件比：这一项的全部正确性就是「窗口底色 == 启动页底色」。两边各存一份色值、
/// 谁也管不到谁 ⇒ 只改一侧就是启动瞬间一次颜色跳变（暗色下尤其刺眼），而任何单侧断言都
/// 发现不了。⚠️ 只取**常量赋值**与 HTML 的 `background:` 取值，注释里的历史说明不参与判定
///（`mobile_bar_contract` 上踩过「针被注释满足」的坑）。
#[test]
fn splash_theme_colors_match_splash_html() {
    const PY: &str = include_str!("../../../.github/scripts/patch-android-splash-theme.py");
    const HTML: &str = include_str!("../splash.html");

    fn py_color(src: &str, name: &str) -> String {
        let key = format!("{name} = \"#");
        let i = src
            .find(&key)
            .unwrap_or_else(|| panic!("脚本缺常量赋值 {name} = \"#…\""));
        let hex: String = src[i + key.len()..].chars().take(6).collect();
        assert!(
            hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()),
            "{name} 取值异常：{hex}"
        );
        format!("#{hex}")
    }
    let light = py_color(PY, "SPLASH_LIGHT");
    let dark = py_color(PY, "SPLASH_DARK");
    assert_ne!(light, dark, "日夜两色相同 ⇒ 等于没做");

    // splash.html 里所有 `background:` 的取值。
    let mut html_colors: Vec<String> = Vec::new();
    let mut rest = HTML;
    while let Some(i) = rest.find("background:") {
        let tail = rest[i + "background:".len()..].trim_start();
        if let Some(h) = tail.strip_prefix('#') {
            let hex: String = h.chars().take(6).collect();
            if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
                html_colors.push(format!("#{hex}"));
            }
        }
        rest = &rest[i + "background:".len()..];
    }
    for (label, want) in [("亮色", &light), ("暗色", &dark)] {
        assert!(
            html_colors.contains(want),
            "splash.html 没有{label} {want} —— 脚本与页面已漂移：{html_colors:?}"
        );
    }

    // 暗色只能出现在 `prefers-color-scheme: dark` 之后；写在全局就是「亮色用户也看到暗底」。
    let media = HTML
        .find("prefers-color-scheme: dark")
        .expect("splash.html 缺暗色媒体查询");
    let dark_use = HTML.find(&dark).expect("splash.html 缺暗色字面量");
    assert!(dark_use > media, "暗色必须落在 dark 媒体查询内");

    // 主题引用与两份资源落点（名字写错等于整条补丁静默不生效）。
    for needle in [
        "android:windowBackground",
        "@color/web2app_splash",
        "values-night",
        "--selftest",
    ] {
        assert!(
            PY.contains(needle),
            "patch-android-splash-theme.py 必须包含：{needle}"
        );
    }
}

/// 抽样一行里所有「大写常量形态」的字符串字面量（`"ANDROID_PERMISSIONS"` → 该键）。
///
/// 只认 `"` 包裹、全部由 `A-Z0-9_` 组成、首字符是字母、长度 ≥ 3 的片段：补丁脚本读环境变量
/// 一律是 `os.environ.get("X")` / `os.environ["X"]`，这样扫足够；换个写法导致扫不到时，
/// 下面的「产品输入必须都被用到」断言会亮红灯，不会静默放过。
fn upper_quoted_keys(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    for seg in line.split('"').skip(1).step_by(2) {
        let ok = seg.len() >= 3
            && seg
                .chars()
                .next()
                .map(|c| c.is_ascii_alphabetic())
                .unwrap_or(false)
            && seg
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
        if ok {
            out.push(seg.to_string());
        }
    }
    out
}

/// ⑨ **Android 打包脚本的「产品输入面」冻结**（2026-11-19，见 `docs/android-packaging-freeze.md`）。
///
/// 为什么钉这个：这套打包链是下游项目的参照实现，**每多一个随打包配置变化的输入，下游就多
/// 一处必须跟着改**。所以输入面只允许四个产品字段：`app_name` / `title` / `version` /
/// `android_permissions`；其余只允许基础设施变量。
///
/// 反例（本条要拦的）：为某个功能往补丁脚本里加 `os.environ.get("THEME_COLOR")` ——
/// 那一刻打包逻辑就开始按「业务」演化了，下游也得跟着加。碰到这种情况先回到
/// 冻结文档 §5 的四种情形里对号；不是 L0/L1/L2 的，一律走自身代码。
#[test]
fn android_packaging_product_inputs_are_frozen() {
    const PRODUCT: &[&str] = &["APP_NAME", "TITLE", "VERSION", "ANDROID_PERMISSIONS"];
    const INFRA: &[&str] = &[
        "CONFIG_JSON",
        "CONFIG_URL",
        "GITHUB_ENV",
        "GITHUB_WORKSPACE",
        "RUNNER_TEMP",
        "WRY_SRC_DIR",
        "W2A_SKIP_GRADLE_PATCH",
        // 漂移策略开关（默认关 = 降级跳过；开 = 当门禁）。属于基础设施变量，
        // 不是产品输入：它只决定「补丁打不上时怎么办」，不改任何打进包里的内容。
        "W2A_ANDROID_PATCH_STRICT",
        "USERPROFILE",
        "PATH",
    ];
    // 配置解析器是输入面本身的入口（读 CONFIG_JSON / CONFIG_URL 是它的职责）；
    // check-workflows.py 是 CI 自身的控制流检查，不改生成工程。
    const EXEMPT: &[&str] = &["parse-packaging-config.py", "check-workflows.py"];

    let mut violations: Vec<String> = Vec::new();
    let mut seen_product: Vec<&str> = Vec::new();

    for (name, src) in python_scripts() {
        if EXEMPT.contains(&name.as_str()) {
            continue;
        }
        for (idx, line) in src.lines().enumerate() {
            // Python 的行注释是 `#`（`mod.rs::strip_line_comments` 切的是 JS 的 `//`，不能复用）。
            // 先剥注释再看是否读环境变量：注释里写 `os.environ.get("XXX")` 不该触发守卫。
            let code = line.split('#').next().unwrap_or("");
            if !code.contains("environ") {
                continue;
            }
            for key in upper_quoted_keys(code) {
                if let Some(p) = PRODUCT.iter().find(|p| **p == key) {
                    if !seen_product.contains(p) {
                        seen_product.push(p);
                    }
                } else if !INFRA.contains(&key.as_str()) {
                    violations.push(format!("{name}:{} 读取了未登记的环境变量 {key}", idx + 1));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "Android 打包脚本出现未登记的输入（先读 docs/android-packaging-freeze.md §5，\
         再决定它是不是产品输入）：\n{}",
        violations.join("\n")
    );

    // 自校验：四个产品输入必须真的被扫到，否则说明扫描方式已与脚本写法脱节（假绿）。
    for key in PRODUCT {
        assert!(
            seen_product.contains(key),
            "白名单里的 {key} 一个都没扫到 —— 环境变量读取方式变了，本条守卫已失效"
        );
    }
}
