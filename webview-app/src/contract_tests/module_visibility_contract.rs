//! 跨模块可见性契约（2026-11-19，Android 打包 `error[E0603]` 回归守卫）。
//!
//! # 为什么需要
//!
//! 2026-11-19 的 Android 打包 red：`bootstrap.rs` 调 `crate::probe::probe_target` /
//! `spawn_reachability_probe`，而这两个函数在 `probe.rs` 里仍是私有的 `fn`。Windows 宿主
//! `cargo test` 全绿、`clippy -D warnings` 全绿、契约测试全绿 —— 因为**调用点与定义同时被
//! `#[cfg(not(windows))]` 摘掉了**，两边一起消失，编译器在桌面上根本看不到冲突；只有
//! Android 目标才会同时看到「调用在」+「定义私有」。
//!
//! 这是「架构审查 2026-11-19」把 `probe` 从 `bootstrap.rs` 尾部抽出为独立模块时留下的：
//! 抽模块把「同文件内私有调用」变成了「跨模块调用」，可见性却没跟着放宽。同类风险对
//! **所有平台专属模块**成立（`mobilebar`、`probe`、`floatwin` …），而它们恰恰是桌面 CI
//! 最看不见的那部分代码。
//!
//! # 判定口径（刻意窄，只拦真错误）
//!
//! ① 只看 `crate::<模块>::<项>` 这种**字面形态**（`use crate::a::b;` 也算，同一形态）；
//! ② `<模块>` 必须是**顶层模块**（`src/<名>.rs` 或 `src/<名>/mod.rs`）；
//! ③ `<项>` 必须能在该模块文件里找到**定义行**（`fn` / `struct` / `enum` / `trait` /
//!    `type` / `const` / `static`）；找不到就跳过 —— 那通常是 `pub use` 再导出或宏生成，
//!    继续判会变成假红；
//! ④ `mod <项>` 定义跳过（`crate::tabs::window` 是模块路径，不是项）；
//! ⑤ 引用方 == 被引用模块本身时跳过（同模块内私有调用完全合法）。
//!
//! 已知盲区（与 `strip_line_comments` 同源）：`/* */` 块注释与字符串字面量里的
//! `crate::a::b` 不剥，`use crate::a::*` 通配符不覆盖。对本仓库够用；真踩了再收紧。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 一个顶层模块里「项名 → (是否 pub, 是否 mod)」。
struct ModuleDefs {
    path: PathBuf,
    items: BTreeMap<String, (bool, bool)>,
}

/// 剥掉行注释，避免注释里的 `crate::a::b` 参与判定。
fn code_only(line: &str) -> &str {
    line.split("//").next().unwrap_or("")
}

/// 取「标识符」：`[A-Za-z0-9_]` 连续段。
fn ident_prefix(s: &str) -> &str {
    let end = s
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(s.len());
    &s[..end]
}

/// 解析一行里的「项定义」，返回 `(项名, 是否 pub, 是否 mod)`。
///
/// 只认行首形态（定义行永远从缩进后开始），不处理跨行签名 —— 可见性前缀总在同一行。
fn parse_def(raw: &str) -> Option<(String, bool, bool)> {
    let line = code_only(raw).trim_start();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    let mut rest = line;
    let mut is_pub = false;
    if let Some(r) = rest.strip_prefix("pub ") {
        is_pub = true;
        rest = r;
    } else if rest.starts_with("pub(") {
        // `pub(crate) ` / `pub(super) ` / `pub(in path) `
        let close = rest.find(')')?;
        is_pub = true;
        rest = rest[close + 1..].trim_start();
    }

    // 修饰符：定义行允许 `default` / `const` / `async` / `unsafe` / `extern "C"` 前置。
    // ⚠️ `const` 既是修饰符也是项关键字（`const X: u8`），只在后面跟 fn/async/unsafe 时当修饰符剥。
    loop {
        let stripped = ["default ", "async ", "unsafe "]
            .iter()
            .find_map(|p| rest.strip_prefix(p))
            .or_else(|| {
                rest.strip_prefix("const ").filter(|r| {
                    r.starts_with("fn ") || r.starts_with("async ") || r.starts_with("unsafe ")
                })
            })
            .or_else(|| rest.strip_prefix("extern \"C\" "));
        match stripped {
            Some(r) => rest = r,
            None => break,
        }
    }

    for (kw, is_mod) in [
        ("fn ", false),
        ("struct ", false),
        ("enum ", false),
        ("trait ", false),
        ("type ", false),
        ("union ", false),
        ("const ", false),
        ("static ", false),
        ("mod ", true),
    ] {
        if let Some(r) = rest.strip_prefix(kw) {
            // `static mut` / `const fn`（已被上面的修饰符循环吃掉）等残留前缀先跳过。
            let r = r.strip_prefix("mut ").unwrap_or(r);
            let name = ident_prefix(r);
            if name.is_empty() {
                return None;
            }
            return Some((name.to_string(), is_pub, is_mod));
        }
    }
    None
}

/// 递归收集 `src/**/*.rs`。
fn rust_sources(root: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(root)
        .unwrap_or_else(|e| panic!("读不到 {}：{e}", root.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            rust_sources(&p, out);
        } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
            out.push(p);
        }
    }
}

/// 顶层模块名 → 定义表。`src/foo.rs` 与 `src/foo/mod.rs` 都算 `foo`。
fn top_level_modules(src_root: &Path, files: &[PathBuf]) -> BTreeMap<String, ModuleDefs> {
    let mut map = BTreeMap::new();
    for f in files {
        let rel = f.strip_prefix(src_root).expect("路径必须位于 src 下");
        let name = if rel.file_name().map(|n| n == "mod.rs").unwrap_or(false) {
            // `src/foo/mod.rs` → foo；`src/mod.rs` 不存在，安全。
            match rel.parent().and_then(|p| p.file_name()) {
                Some(n) => n.to_string_lossy().into_owned(),
                None => continue,
            }
        } else if rel.components().count() == 1 {
            rel.file_stem().unwrap().to_string_lossy().into_owned()
        } else {
            // 子模块（`src/tabs/window.rs`）：其私有项被 `crate::tabs::x` 引用是不合法的，
            // 但那种情况会以「`pub use` 再导出」形态出现 ⇒ 判定为「定义未找到」而跳过。
            // 这里不做定义来源，避免 `src/window.rs` 与 `src/tabs/window.rs` 同名互撞。
            continue;
        };

        let text = std::fs::read_to_string(f).unwrap_or_default();
        let mut items = BTreeMap::new();
        for line in text.lines() {
            if let Some((n, is_pub, is_mod)) = parse_def(line) {
                // 同名多定义（如 `impl` 内的关联 fn 不会命中行首形态；真正的重载不存在）
                // ⇒ 任一处 pub 即视为可见（`cfg` 分支常各写一份）。
                let e = items.entry(n).or_insert((is_pub, is_mod));
                e.0 |= is_pub;
                e.1 &= is_mod;
            }
        }
        map.insert(
            name,
            ModuleDefs {
                path: f.clone(),
                items,
            },
        );
    }
    map
}

#[test]
fn cross_module_items_are_visible() {
    let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_sources(&src_root, &mut files);
    assert!(
        files.len() > 20,
        "只扫到 {} 个 .rs，路径可能错了",
        files.len()
    );

    let modules = top_level_modules(&src_root, &files);

    let mut violations: Vec<String> = Vec::new();
    // 自校验：至少要真的判过一些跨模块引用，否则说明扫描口径与代码写法脱节（假绿）。
    let mut checked = 0usize;

    for f in &files {
        let text = std::fs::read_to_string(f).unwrap_or_default();
        for (idx, raw) in text.lines().enumerate() {
            let line = code_only(raw);
            let mut from = 0usize;
            while let Some(pos) = line[from..].find("crate::") {
                let after = &line[from + pos + "crate::".len()..];
                let module = ident_prefix(after);
                let rest = &after[module.len()..];
                if let Some(rest) = rest.strip_prefix("::") {
                    let item = ident_prefix(rest);
                    if !module.is_empty() && !item.is_empty() {
                        if let Some(defs) = modules.get(module) {
                            // ⑤ 同模块内调用合法
                            if defs.path != *f {
                                if let Some((is_pub, is_mod)) = defs.items.get(item) {
                                    // ④ 模块路径不算项
                                    if !is_mod {
                                        checked += 1;
                                        if !is_pub {
                                            let rel = f.strip_prefix(&src_root).unwrap();
                                            let target = defs
                                                .path
                                                .strip_prefix(&src_root)
                                                .unwrap_or(&defs.path);
                                            violations.push(format!(
                                                "src/{}:{} 跨模块引用 crate::{}::{}，\
                                                 但它在 src/{} 里不是 pub —— \
                                                 Windows 上看不出问题（两侧可能都被 cfg 摘掉），\
                                                 Android 目标会 error[E0603]",
                                                rel.display(),
                                                idx + 1,
                                                module,
                                                item,
                                                target.display(),
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                from += pos + "crate::".len();
            }
        }
    }

    assert!(
        violations.is_empty(),
        "跨模块引用了私有项（Android 构建会 E0603，桌面 CI 看不见）：\n{}",
        violations.join("\n")
    );
    assert!(
        checked >= 10,
        "只判到 {checked} 处跨模块引用 —— 扫描口径可能已与代码写法脱节，本条守卫已失效"
    );
}
