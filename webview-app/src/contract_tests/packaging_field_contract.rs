/// 打包配置字段表（`scripts/packaging-fields.json`）的**数据驱动**契约守卫
/// （架构审查 2026-10-25 P1-b / 2026-09-29 遗留 P2-1）。
///
/// 存在理由：21 字段的字段表此前在 5+ 处各有一份副本（Python 解析器 `DEFAULTS` /
/// Node 复刻 `DEFAULTS` + `BOOL_KEYS` / electron `config.ts` / 两份 `config.example.json`
/// / docs），每加一个字段要改 6 处、还要新写一条逐字段手写守卫 —— 漏改任何一处的表现
/// 都是「配置里写了却静默不生效」。现在**机器可读唯一真源**是 `scripts/packaging-fields.json`
///（name / kind / default / 需出现的真源），两个解析器从它加载 `DEFAULTS` / `BOOL_KEYS`，
/// 本模块按这张表**遍历**断言每个字段在其声明的真源里都存在 —— 加字段只改字段表 +
/// 各真源实现，守卫自动覆盖，不再新增手写断言。
///
/// 覆盖（只管「存在性」；语义断言留在 mobile_bar_contract.rs 等各领域模块）：
///   · 字段表自身形态：name 唯一、kind 合法、default 类型与 kind 一致、sources 标记合法；
///   · Python / Node 两个解析器确实从字段表加载 DEFAULTS / BOOL_KEYS（否则表形同虚设）；
///   · 每个字段在其 `sources` 声明的每个真源里都出现（各真源按各自形态找：JSON 文件看
///     `"字段名"` 字段串，electron config.ts 看 `字段名:` / `'字段名'`）；
///   · 两份 config.example.json 的键集与字段表声明**双向一致**（多键 / 少键都红）。
///
/// ⚠️ `electron-app/src/main/config.ts` 与两份 `config.example.json` 都在禁改清单里，
///    这里只做静态存在性钉住 —— 它们漂移时由本守卫红，而不是被顺手改掉。
const FIELDS_JSON: &str = include_str!("../../../scripts/packaging-fields.json");
const PARSER_PY: &str = include_str!("../../../.github/scripts/parse-packaging-config.py");
const PARSER_MJS: &str = include_str!("../../../scripts/packlib/constants.mjs");
const ELECTRON_CONFIG: &str = include_str!("../../../electron-app/src/main/config.ts");
const EXAMPLE_TAURI_JSON: &str = include_str!("../../config.example.json");
const EXAMPLE_ELECTRON_JSON: &str = include_str!("../../../electron-app/config.example.json");

/// `sources` 的合法取值（字段表里出现别的标记 = 表写错了，当场红）。
const KNOWN_SOURCES: [&str; 5] = [
    "python_parser",
    "node_parser",
    "electron_ts",
    "example_tauri",
    "example_electron",
];

use std::collections::BTreeSet;

use serde_json::Value;

struct Field<'a> {
    name: &'a str,
    kind: &'a str,
    default: &'a Value,
    sources: Vec<&'a str>,
}

fn table() -> Value {
    serde_json::from_str(FIELDS_JSON)
        .unwrap_or_else(|e| panic!("scripts/packaging-fields.json 不是合法 JSON：{e}"))
}

fn parse_fields(v: &Value) -> Vec<Field<'_>> {
    let arr = v
        .get("fields")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("字段表必须有 fields 数组"));
    arr.iter()
        .map(|f| Field {
            name: f
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("每个字段必须有 name 字符串：{f}")),
            kind: f
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("每个字段必须有 kind 字符串：{f}")),
            default: f
                .get("default")
                .unwrap_or_else(|| panic!("每个字段必须有 default：{f}")),
            sources: f
                .get("sources")
                .and_then(Value::as_array)
                .unwrap_or_else(|| panic!("每个字段必须有 sources 数组：{f}"))
                .iter()
                .map(|s| {
                    s.as_str()
                        .unwrap_or_else(|| panic!("sources 的元素必须是字符串：{f}"))
                })
                .collect(),
        })
        .collect()
}

/// 只看「代码行」：剥掉 `//` 注释后再匹配，否则针会被注释满足（与 mobile_bar_contract 同法）。
fn has_code_line(src: &str, needle: &str) -> bool {
    src.lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .any(|l| l.contains(needle))
}

/// JSON 文件里的键集（样例 / 字段表通用）。
fn json_keys(src: &str, what: &str) -> BTreeSet<String> {
    let v: Value =
        serde_json::from_str(src).unwrap_or_else(|e| panic!("{what} 不是合法 JSON：{e}"));
    v.as_object()
        .unwrap_or_else(|| panic!("{what} 必须是 JSON 对象"))
        .keys()
        .cloned()
        .collect()
}

/// ① 字段表自身形态：name 唯一、kind 合法、default 类型与 kind 一致、sources 标记合法。
///    这层挡住「表自己写错」—— 否则后面按错表遍历，断言越多错得越自信。
#[test]
fn field_table_is_well_formed_and_defaults_match_kinds() {
    let v = table();
    let fields = parse_fields(&v);
    assert!(!fields.is_empty(), "字段表不得为空");
    let mut seen = BTreeSet::new();
    for f in &fields {
        assert!(seen.insert(f.name), "字段名重复：{}", f.name);
        let default_ok = match f.kind {
            "bool" => f.default.is_boolean(),
            "string" => f.default.is_string(),
            "list" => f.default.is_array(),
            "object" => f.default.is_object(),
            other => panic!(
                "字段 {} 的 kind 非法：{other}（只允许 bool / string / list / object）",
                f.name
            ),
        };
        assert!(
            default_ok,
            "字段 {} 的 default 类型与 kind={} 不一致：{}",
            f.name, f.kind, f.default
        );
        assert!(
            !f.sources.is_empty(),
            "字段 {} 必须声明至少一个真源",
            f.name
        );
        for s in &f.sources {
            assert!(
                KNOWN_SOURCES.contains(s),
                "字段 {} 声明了未知真源 {s}（合法值：{KNOWN_SOURCES:?}）",
                f.name
            );
        }
    }
}

/// ② 两个解析器必须**真的**从字段表加载 DEFAULTS / BOOL_KEYS —— 否则字段表只是
///    一份没人消费的文档，「存在性由表保证」立刻塌掉。
///
/// ⚠️ 路径锚定也是契约：Python 侧用 `Path(__file__).resolve().parents[2]`（= 仓库根）
///    推路径，Node 侧用 `new URL(..., import.meta.url)`（模块相对）—— 两者都必须
///    **与 cwd 无关**，GitHub Actions 的任意 working-directory 与本地任意目录都要能找到。
#[test]
fn both_parsers_load_the_shared_field_table() {
    for needle in [
        "parents[2]",
        "\"packaging-fields.json\"",
        "{f[\"name\"]: f[\"default\"] for f in _FIELDS}",
        "{f[\"name\"] for f in _FIELDS if f[\"kind\"] == \"bool\"}",
    ] {
        assert!(
            has_code_line(PARSER_PY, needle),
            "parse-packaging-config.py 必须从 scripts/packaging-fields.json 加载 DEFAULTS/BOOL_KEYS（找 {needle}）"
        );
    }
    for needle in [
        "import.meta.url",
        "'../packaging-fields.json'",
        "[f.name, f.default]",
        "f.kind === 'bool'",
    ] {
        assert!(
            has_code_line(PARSER_MJS, needle),
            "scripts/packlib/constants.mjs 必须从 scripts/packaging-fields.json 加载 DEFAULTS/BOOL_KEYS（找 {needle}）"
        );
    }
}

/// ③ 每个字段在其 `sources` 声明的每个真源里都存在 —— 原来逐字段手写的
///    `config_field_exists_in_every_source_of_truth` / `android_permissions_field_is_in_every_source_of_truth`
///    的「存在性」部分由此覆盖（新增字段自动进巡检，无需新写断言）。
#[test]
fn every_field_appears_in_each_declared_source() {
    let v = table();
    for f in parse_fields(&v) {
        // JSON 文件（字段表 / 两份样例）按「"字段名"」字段串找。
        let json_needle = format!("\"{}\"", f.name);
        for src in &f.sources {
            match *src {
                // Python / Node 解析器的字段契约**就是这张表**（DEFAULTS / BOOL_KEYS 由它
                // 加载，见 ②）⇒ 存在性判据 =「字段在表里」+「解析器吃这张表」，而不是
                // .py / .mjs 里的逐字段字面量副本（那正是本次重构删掉的东西）。
                "python_parser" => {
                    assert!(
                        FIELDS_JSON.contains(&json_needle)
                            && PARSER_PY.contains("packaging-fields.json"),
                        "字段 {} 必须由 parse-packaging-config.py 经字段表承载",
                        f.name
                    );
                }
                "node_parser" => {
                    assert!(
                        FIELDS_JSON.contains(&json_needle)
                            && PARSER_MJS.contains("packaging-fields.json"),
                        "字段 {} 必须由 scripts/packlib/constants.mjs 经字段表承载",
                        f.name
                    );
                }
                // electron config.ts：AppConfig 用 `字段名:` 形态；4 个 android 签名键只在
                // ANDROID_ONLY_KEYS 白名单里（`'字段名'` 形态）—— 两种形态都算认识该字段。
                "electron_ts" => {
                    assert!(
                        ELECTRON_CONFIG.contains(&format!("{}:", f.name))
                            || ELECTRON_CONFIG.contains(&format!("'{}'", f.name)),
                        "electron-app/src/main/config.ts 必须认识字段 {}（找 `{}:` 或 `'{}'`）",
                        f.name,
                        f.name,
                        f.name
                    );
                }
                "example_tauri" => {
                    assert!(
                        EXAMPLE_TAURI_JSON.contains(&json_needle),
                        "webview-app/config.example.json 必须含字段 {}（找 {json_needle}）",
                        f.name
                    );
                }
                "example_electron" => {
                    assert!(
                        EXAMPLE_ELECTRON_JSON.contains(&json_needle),
                        "electron-app/config.example.json 必须含字段 {}（找 {json_needle}）",
                        f.name
                    );
                }
                other => panic!("未知真源标记：{other}"),
            }
        }
    }
}

/// ④ 两份 config.example.json 的键集与字段表声明**双向一致**：少了 = 样例漏字段
///    （用户照抄就少配），多了 = 样例带了契约外的键（解析器会硬拒）。
///    electron 侧缺的 4 个 android 键（keystore base64 / 两个密码 / key alias）在字段表
///    里**不标** `example_electron`（含明文密码的签名材料不随 Windows 包分发）。
#[test]
fn example_jsons_contain_exactly_the_fields_declared_for_them() {
    let v = table();
    let fields = parse_fields(&v);
    for (src, label, text) in [
        (
            "example_tauri",
            "webview-app/config.example.json",
            EXAMPLE_TAURI_JSON,
        ),
        (
            "example_electron",
            "electron-app/config.example.json",
            EXAMPLE_ELECTRON_JSON,
        ),
    ] {
        let expected: BTreeSet<&str> = fields
            .iter()
            .filter(|f| f.sources.contains(&src))
            .map(|f| f.name)
            .collect();
        let owned = json_keys(text, label);
        let actual: BTreeSet<&str> = owned.iter().map(String::as_str).collect();
        let missing: Vec<&&str> = expected.difference(&actual).collect();
        let extra: Vec<&&str> = actual.difference(&expected).collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "{label} 的键集与字段表 sources={src} 声明不一致：缺 {missing:?}，多 {extra:?}"
        );
    }
}
