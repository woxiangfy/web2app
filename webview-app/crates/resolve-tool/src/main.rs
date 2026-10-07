//! resolve-tool —— 动态解析的【服务端】命令行工具。
//!
//! 用于在打包之外，本地/CI 上用 Ed25519 私钥生成：
//!   1. 密钥对（Ed25519 私钥 PEM + 公钥裸 base64）
//!   2. TXT 密文（私钥签名 + AES-256-GCM 加密，写入 DNS TXT）
//!
//! 与客户端 `resolve-core` 的解密逻辑严格对称；与 `tools/encrypt_url.py` 等价，
//! 提供纯 Rust 版本，方便在 CI（无需 Python cryptography 依赖）里使用。
//!
//! 本 crate 只依赖 `resolve-core`（纯 Rust），**不依赖 Tauri**，因此可在 ubuntu-latest
//! 等没有 glib-2.0 开发库的 Linux runner 上直接 `cargo run` 编译，无需装系统库。
//!
//! Secret（AES 密钥派生源头）：encrypt/all 可用 `--secret <值>` 逐次调用显式指定，同一份
//! 工具二进制不必为不同 Secret 分别编译；不传时回落到 `resolve-core` 编译期环境变量
//! `RESOLVE_SECRET`（`option_env!`，cargo 经 dep-info 追踪该 env，改值会自动重编），
//! 编译期也没设则用内置默认串 `web2app-fixed-secret-2026`。客户端解密只用构建期注入的
//! Secret，加密侧 `--secret` 的值必须与客户端构建值一致，否则 HKDF 派生密钥不同、解密
//! 必然失败。Python 等价工具 `tools/encrypt_url.py` 提供同名参数，优先级同为
//! 参数 > 环境变量 RESOLVE_SECRET > 内置默认。
//!
//! 用法：
//!   resolve-tool genkey [--out DIR]             生成密钥对，写 private_key.pem + public_key.b64
//!   resolve-tool genkey --stdout                生成密钥对但不落盘，私钥 PEM / 公钥 base64 直接打到 stdout
//!   resolve-tool encrypt <private-key> <url> [--ttl-days N] [--count N] [--secret 值]
//!      用私钥加密一个 URL，stdout 输出 base64 密文；--count N 表示同一 URL 生成 N 份密文。
//!      <private-key> 既可是私钥 PEM 文件路径，也可直接传 PEM 字符串本体（见下方「内联 PEM」）
//!   resolve-tool encrypt --batch <private-key> [--ttl-days N] [--count N] [--secret 值]
//!      从 stdin 逐行读 URL，每个 URL 生成 N 份密文（<private-key> 同上：路径或内联 PEM）
//!   resolve-tool all <url> [--out DIR] [--ttl-days N] [--count N] [--secret 值]
//!      一键：生成密钥对 + 立即加密一个 URL（生成 N 份密文），同时输出密钥对文件与密文
//!   resolve-tool decrypt <public-key> [ciphertext] [--secret 值]
//!      解密验证一条 TXT 密文：AES-GCM 认证 + Ed25519 验签通过后，打印真实 URL、
//!      签发日期（密文内含 2B 天数，基准日 2026-01-01）与 TTL 天数；ciphertext 省略时从 stdin 读入。
//!      <public-key> 同私钥规则：公钥文件路径（.b64/.pem）或直接内联公钥值（裸 base64/SPKI PEM）。
//!
//! 内联 PEM：encrypt 第一个参数 trim 后以 `-----BEGIN` 开头即视为 PEM 本体直接解析（不必先存成
//! 文件），可多行粘贴、也可折叠成单行（解析器对所有空白健壮）；但只能含 PEM 本体，不要带
//! `genkey --stdout` 输出里的 `#` 注释行。注意私钥走命令行对同机其他进程可见（进程参数），
//! 介意请继续用文件路径方式。
//!
//! 说明：
//!   --ttl-days 有效期天数，默认 0（永不失效）；需要防重放/防过期时显式给正整数（最小 1 天）。
//!   --count 每个 URL 生成的密文份数（默认 1）。每份用不同的随机 Nonce，密文不同，
//!           但解密后是同一个 URL，可分发到多个域名 / 多条 DNS TXT 记录。
//!   --secret 显式指定 AES 密钥派生 Secret（encrypt/all/decrypt 均支持；不传则用编译期
//!           RESOLVE_SECRET / 内置默认）；必须与客户端构建期注入值一致。走进程命令行对
//!           同机其他进程可见，介意可用编译期环境变量方式。

use std::process::ExitCode;

/// 默认有效期：0 = 永不失效。
/// 记录的防重放/防过期完全由调用方显式 `--ttl-days 正整数` 开启（v4 单位为天）。
const DEFAULT_TTL_DAYS: u64 = 0;

fn print_usage() {
    eprintln!(
        "\
resolve-tool —— 动态解析服务端工具（Ed25519 签名 + AES-256-GCM）

用法：
  resolve-tool genkey [--out DIR]
      生成 Ed25519 密钥对，写 DIR/private_key.pem 与 DIR/public_key.b64（默认 .）

  resolve-tool genkey --stdout
      生成 Ed25519 密钥对但【不写任何文件】：私钥 PEM 与公钥 base64 直接打到 stdout
      （# 开头为说明行；供只在控制台展示密钥材料、不落盘的入口使用）。

  resolve-tool encrypt <private-key> <url> [--ttl-days N] [--count N] [--secret 值]
      用私钥加密一个 URL，stdout 输出 base64 密文（单行，写入 DNS TXT）。
      <private-key> 既可是私钥 PEM 文件路径，也可直接粘贴 PEM 本体（以 -----BEGIN 开头；
      可折叠成单行、不要带 # 注释行）——后者不必先落盘，但私钥会出现在进程命令行里。
      --count N 让同一 URL 生成 N 份不同密文（不同随机 Nonce），可分发到多个域名。

  resolve-tool encrypt --batch <private-key> [--ttl-days N] [--count N] [--secret 值]
      从 stdin 逐行读 URL，每个 URL 生成 N 份密文（<private-key> 同上：路径或内联 PEM）

  resolve-tool all <url> [--out DIR] [--ttl-days N] [--count N] [--secret 值]
      一键：生成密钥对 + 立即加密一个 URL（N 份密文），同时输出密钥对文件与密文

  resolve-tool decrypt <public-key> [ciphertext] [--secret 值]
      解密验证一条 TXT 密文（AES-GCM 认证 + Ed25519 验签），打印真实 URL、签发日期
      与 TTL 天数。<public-key> 可为公钥文件路径（.b64/.pem）或直接内联公钥值（裸 base64
      或以 -----BEGIN 开头的 SPKI PEM）；ciphertext 省略（或为 -）时从 stdin 读入。
      过期记录（ttl_days>0 且已超时）仍展示解密内容，但退出码为 1（客户端同样会拒绝）。

参数：
  --ttl-days 有效期天数（默认 0=永不失效；给正整数则生成后 N 天内有效，到期客户端拒绝）
  --count    每个 URL 生成的密文份数（默认 1）
  --out      输出目录（genkey / all 用，默认 .）
  --secret   显式指定 AES 派生 Secret（不传则用编译期 RESOLVE_SECRET，再缺省用内置默认）；
             必须与客户端构建期注入的 Secret 完全一致，否则客户端解密必然失败
"
    );
}

/// 一次解析出的命令行参数：所有可选 flag 的取值 + 剩余位置参数。
struct ParsedArgs {
    /// 有效期天数（0 = 永不失效）。
    ttl_days: u64,
    count: u64,
    out: String,
    batch: bool,
    /// genkey 专用：只把密钥材料打到 stdout，不写任何文件。
    stdout: bool,
    /// encrypt/all 的显式 Secret；None 时回落 resolve-core 编译期 Secret。
    secret: Option<String>,
    positional: Vec<String>,
}

/// 统一解析 `--ttl-days / --count / --out / --secret / --batch / --stdout` 及剩余位置参数（不引 clap，保持零重依赖）。
///
/// 之前 `cmd_encrypt` / `cmd_all` / `genkey` 各自手写一份 while 循环解析，重复且易漏新 flag；
/// 这里归并为一处，三个子命令共用同一套解析规则，便于后续扩展新 flag。
fn parse_args(args: &[String]) -> Result<ParsedArgs, String> {
    let mut ttl_days: Option<u64> = None;
    let mut count: Option<u64> = None;
    let mut out: Option<String> = None;
    let mut secret: Option<String> = None;
    let mut batch = false;
    let mut stdout = false;
    let mut positional: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--ttl-days" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--ttl-days 需要一个天数（整数）".to_string())?;
                ttl_days = Some(
                    v.parse::<u64>()
                        .map_err(|_| format!("--ttl-days 必须是整数天数，收到：{v}"))?,
                );
                i += 2;
            }
            "--count" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--count 需要一个正整数".to_string())?;
                let n = v
                    .parse::<u64>()
                    .map_err(|_| format!("--count 必须是正整数，收到：{v}"))?;
                if n == 0 {
                    return Err("--count 必须 >= 1".to_string());
                }
                count = Some(n);
                i += 2;
            }
            "--out" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--out 需要一个目录路径".to_string())?;
                out = Some(v.clone());
                i += 2;
            }
            "--secret" => {
                let v = args.get(i + 1).ok_or_else(|| {
                    "--secret 需要一个值（与客户端构建期一致的 Secret）".to_string()
                })?;
                if v.is_empty() {
                    return Err("--secret 不允许空字符串（如确需默认值，请省略该参数）".to_string());
                }
                secret = Some(v.clone());
                i += 2;
            }
            "--batch" => {
                batch = true;
                i += 1;
            }
            "--stdout" => {
                stdout = true;
                i += 1;
            }
            _ => {
                positional.push(a.to_string());
                i += 1;
            }
        }
    }

    Ok(ParsedArgs {
        ttl_days: ttl_days.unwrap_or(DEFAULT_TTL_DAYS),
        count: count.unwrap_or(1),
        out: out.unwrap_or_else(|| ".".to_string()),
        batch,
        stdout,
        secret,
        positional,
    })
}

/// 从 PEM（PKCS#8 纯私钥格式）解析 Ed25519 私钥。
///
/// 兼容两种来源：本工具 `genkey` 生成的纯私钥（48 字节 DER）、以及 Python
/// `cryptography` 导出的 `Ed25519PrivateKey`（同样 48 字节纯私钥 DER）。
/// 也兼容 ed25519-dalek 的 `to_pkcs8_pem` 输出的 RFC 8410 格式（83 字节，带内嵌公钥）。
///
/// PEM → SigningKey 的解析已下沉到 `resolve-core::parse_signing_key_pem`，与客户端测试
/// 共用同一套健壮实现（对空白完全健壮，即使私钥被折叠成单行也能正确提取）。
fn load_signing_key(path: &str) -> Result<ed25519_dalek::SigningKey, String> {
    let pem = std::fs::read_to_string(path).map_err(|e| format!("读取私钥文件失败：{e}"))?;
    resolve_core::parse_signing_key_pem(&pem)
}

/// 解析 encrypt 的私钥入参：PEM 文件路径【或】内联 PEM 字符串，二选一自动识别。
///
/// 判定规则与 `resolve_core::parse_public_key` 的「SPKI PEM 文本 vs 裸 base64」双态识别同思路：
/// 入参 trim 后以 `-----BEGIN` 开头 ⇒ 当作内联 PEM 本体直接解析（不必先存成文件，配合
/// `genkey --stdout` 的不落盘流程）；否则按文件路径读取（历史行为，完全不变）。
/// 合法文件路径不会以 `-----BEGIN` 打头；真有此类古怪文件名时用 `./-----BEGIN...` 相对路径即可
/// 落到文件分支。内联 PEM 只接受 PEM 本体——`genkey --stdout` 打印的 `#` 注释行不能一起贴进来
/// （解析器只剔除空白，注释字符会导致 base64 解码失败）。
fn resolve_signing_key(spec: &str) -> Result<ed25519_dalek::SigningKey, String> {
    let spec = spec.trim();
    if spec.starts_with("-----BEGIN") {
        resolve_core::parse_signing_key_pem(spec)
    } else {
        load_signing_key(spec)
    }
}

/// 单次加密的统一分派：命令行给了 `--secret` 就走显式 Secret 版本，
/// 否则回落 `resolve-core` 编译期 Secret（`RESOLVE_SECRET` / 内置默认）。
fn encrypt_one(
    signing_key: &ed25519_dalek::SigningKey,
    url: &str,
    ttl_days: u64,
    sign_day: Option<u64>,
    secret: Option<&str>,
) -> Result<String, String> {
    match secret {
        Some(s) => resolve_core::sign_and_encrypt_with_secret(
            signing_key,
            url,
            ttl_days,
            sign_day,
            s.as_bytes(),
        ),
        None => resolve_core::sign_and_encrypt(signing_key, url, ttl_days, sign_day),
    }
}

/// 生成密钥对并写入 out_dir，返回（私钥路径，公钥路径，公钥 base64）。
fn generate_and_write_keypair(out_dir: &str) -> Result<(String, String, String), String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("创建目录 {out_dir} 失败：{e}"))?;
    let (priv_pem, pub_b64) = resolve_core::generate_keypair()?;

    let priv_path = std::path::Path::new(out_dir).join("private_key.pem");
    let pub_path = std::path::Path::new(out_dir).join("public_key.b64");
    std::fs::write(&priv_path, priv_pem.as_bytes()).map_err(|e| format!("写私钥失败：{e}"))?;
    std::fs::write(&pub_path, pub_b64.as_bytes()).map_err(|e| format!("写公钥失败：{e}"))?;
    Ok((
        priv_path.display().to_string(),
        pub_path.display().to_string(),
        pub_b64,
    ))
}

fn cmd_genkey(out_dir: &str) -> Result<(), String> {
    let (priv_path, pub_path, pub_b64) = generate_and_write_keypair(out_dir)?;

    println!("私钥（服务端持有，切勿公开）: {priv_path}");
    println!("公钥裸 base64（客户端 PUBLIC_KEY）: {pub_path}");
    println!();
    println!("公钥内容：");
    println!("{pub_b64}");
    Ok(())
}

/// 生成密钥对但【不落盘】：私钥 PEM 与公钥 base64 只经 stdout 输出，不创建任何文件。
///
/// 与 `cmd_genkey` 共用 `resolve_core::generate_keypair`，密钥格式完全一致，区别仅在输出形态。
/// 供 `scripts/gen-resolve-keys.bat` 这类「控制台一次性展示、由使用者自行抄走」的入口使用：
/// 密钥材料只存在于内存与终端，关掉窗口即消失（再次运行会生成一对全新密钥）。
///
/// 输出约定：`#` 开头是给人看的说明行（与 `encrypt` 的密文注释同风格），随后依次是
/// PEM 私钥块（`resolve_core` 的 PEM 编码自带结尾换行）与单行公钥裸 base64。
fn cmd_genkey_stdout() -> Result<(), String> {
    let (priv_pem, pub_b64) = resolve_core::generate_keypair()?;

    println!("# private_key.pem —— Ed25519 私钥（服务端留存，切勿公开）");
    print!("{priv_pem}");
    println!("# public_key —— Ed25519 公钥裸 base64（客户端配置 \"public_key\"）");
    println!("{pub_b64}");
    Ok(())
}

/// 一键：生成密钥对 + 立即用新私钥加密一个 URL（N 份密文）。
/// args 形如：[all, <url>, (--out DIR), (--ttl-days N), (--count N)]
fn cmd_all(args: &[String]) -> Result<(), String> {
    let a = parse_args(args)?;

    if a.positional.is_empty() {
        return Err("未提供要加密的 URL".to_string());
    }

    // 生成密钥对并写文件。
    let (priv_path, pub_path, pub_b64) = generate_and_write_keypair(&a.out)?;

    // 从刚写的私钥文件重新加载 SigningKey（复用同一套解析逻辑）。
    let signing_key = load_signing_key(&priv_path)?;

    // 逐个 URL、每个 URL 生成 count 份密文。
    let mut ciphers: Vec<String> = Vec::new();
    for url in &a.positional {
        for _ in 0..a.count {
            let cipher = encrypt_one(&signing_key, url, a.ttl_days, None, a.secret.as_deref())?;
            ciphers.push(cipher);
        }
    }

    println!("========== 密钥对 ==========");
    println!("私钥: {priv_path}");
    println!("公钥: {pub_path}");
    println!();
    println!("公钥内容（填 build-custom.yml 的 public_key）：");
    println!("{pub_b64}");
    println!();
    println!(
        "========== TXT 密文（共 {} 条，均可用于 DNS TXT） ==========",
        ciphers.len()
    );
    let mut idx = 0;
    for url in &a.positional {
        for _ in 0..a.count {
            let cipher = &ciphers[idx];
            println!(
                "# {url}  (ttl_days={}, 第 {} 份)",
                a.ttl_days,
                idx % a.count as usize + 1
            );
            println!("{cipher}");
            println!();
            idx += 1;
        }
    }
    Ok(())
}

fn cmd_encrypt(args: &[String]) -> Result<(), String> {
    // args 形如：[encrypt, <private-key>, <url>, (--ttl-days N), (--count N)]
    //          或 [encrypt, --batch, <private-key>, (--ttl-days N), (--count N)]
    // <private-key> 既可是 PEM 文件路径，也可是以 -----BEGIN 开头的内联 PEM 字符串。
    let a = parse_args(args)?;

    let key_spec = a.positional.first().ok_or_else(|| {
        "缺少私钥参数（PEM 文件路径，或以 -----BEGIN 开头的内联 PEM）".to_string()
    })?;
    let signing_key = resolve_signing_key(key_spec)?;

    let urls: Vec<String> = if a.batch {
        // 从 stdin 逐行读 URL。
        let mut input = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)
            .map_err(|e| format!("读取 stdin 失败：{e}"))?;
        input
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    } else {
        a.positional
            .iter()
            .skip(1)
            .map(|s| s.as_str().to_string())
            .collect()
    };

    if urls.is_empty() {
        return Err("未提供要加密的 URL".to_string());
    }

    for url in &urls {
        for n in 0..a.count {
            let cipher = encrypt_one(&signing_key, url, a.ttl_days, None, a.secret.as_deref())?;
            if a.count > 1 {
                println!("# {url}  (ttl_days={}, 第 {} 份)", a.ttl_days, n + 1);
            } else {
                println!("# {url}  (ttl_days={})", a.ttl_days);
            }
            println!("{cipher}");
            println!();
        }
    }
    Ok(())
}

/// 解析 decrypt 的公钥入参：公钥文件路径【或】内联公钥值（裸 base64 / SPKI PEM）。
///
/// 判定顺序与私钥侧 [`resolve_signing_key`] 对称、并多兜一层裸 base64：
///   1. trim 后以 `-----BEGIN` 开头 ⇒ 内联 SPKI PEM，直接交 `parse_public_key`；
///   2. 指向一个存在的文件 ⇒ 读文件内容（.b64 裸 base64 或 .pem 均可）再解析；
///   3. 其余一律当作内联裸 base64 直接解析（不存在的路径写错时也会得到明确的解码错误）。
fn resolve_public_key(spec: &str) -> Result<resolve_core::VerifyingKey, String> {
    let spec = spec.trim();
    if spec.starts_with("-----BEGIN") {
        return resolve_core::parse_public_key(spec);
    }
    if std::path::Path::new(spec).is_file() {
        let content =
            std::fs::read_to_string(spec).map_err(|e| format!("读取公钥文件失败：{e}"))?;
        return resolve_core::parse_public_key(&content);
    }
    resolve_core::parse_public_key(spec)
}

/// 把 UNIX 天数（距 1970-01-01）换算成公历 (年, 月, 日)。
///
/// 不引 chrono 等第三方依赖：用 Howard Hinnant《chrono-Compatible Low-Level Date
/// Algorithms》里 days/civil 互转的整数算法（公历，支持 1970 年前后）。
fn civil_from_unix_day(days: i64) -> (i64, u32, u32) {
    // 1970-01-01 ⇒ 公历第 719468 天；以 400 年（146097 天）为一个 era。
    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let day_of_era = (z - era * 146097) as u64; // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365; // [0, 399]
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100); // [0, 365]
    let month_prime = (5 * day_of_year + 2) / 153; // [0, 11]，3 月起算
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1; // [1, 31]
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    }; // [1, 12]
    let year = if month <= 2 { year + 1 } else { year };
    (year, month as u32, day as u32)
}

/// 把签发日（距基准日 2026-01-01 的天数）格式化为 UTC 日期 `YYYY-MM-DD`。
fn format_day_date(day: u16) -> String {
    let unix_day = resolve_core::BASE_UNIX_DAY + day as i64;
    let (year, month, d) = civil_from_unix_day(unix_day);
    format!("{year:04}-{month:02}-{d:02}")
}

/// 解密验证一条 TXT 密文：两道密码学验证通过后，展示 URL、签发日期、TTL 与时效状态。
/// args 形如：[decrypt, <public-key>, (ciphertext), (--secret 值)]
fn cmd_decrypt(args: &[String]) -> Result<(), String> {
    let a = parse_args(args)?;

    let key_spec = a.positional.first().ok_or_else(|| {
        "缺少公钥参数（公钥文件路径，或裸 base64 / 以 -----BEGIN 开头的 SPKI PEM）".to_string()
    })?;
    let public_key = resolve_public_key(key_spec)?;

    // 密文：第 2 个位置参数（支持用 - 显式占位）；省略则从 stdin 读入。
    let inline_cipher = a.positional.get(1).filter(|s| s.as_str() != "-").cloned();
    let cipher = match inline_cipher {
        Some(c) => c,
        None => {
            let mut input = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)
                .map_err(|e| format!("读取 stdin 失败：{e}"))?;
            input
        }
    };
    let cipher = cipher.trim();
    if cipher.is_empty() {
        return Err("未提供要解密的 TXT 密文".to_string());
    }

    // 结构化解密：GCM 认证 + Ed25519 验签失败会直接返回 Err；不因过期而吞掉解密内容。
    let record = match a.secret.as_deref() {
        Some(s) => resolve_core::decrypt_record_with_secret(&public_key, cipher, s.as_bytes()),
        None => resolve_core::decrypt_record(&public_key, cipher),
    }?;

    println!("解密验证通过（AES-GCM 认证 + Ed25519 验签）");
    println!("URL      : {}", record.url);
    println!(
        "签发日期 : {} UTC（第 {} 天）",
        format_day_date(record.sign_day),
        record.sign_day
    );
    if record.ttl_days == 0 {
        println!("有效期   : 永不失效（ttl_days=0）");
    } else {
        println!("有效期   : {} 天", record.ttl_days);
    }

    // 时效状态单独判定：过期时内容已全部展示，再以 Err 让进程退出码 = 1，
    // 与客户端运行时「过期即拒绝」的处理保持一致。
    record.check_ttl().map_err(|_| {
        let today = resolve_core::current_unix_day()
            .map(|d| (d - resolve_core::BASE_UNIX_DAY).max(0))
            .unwrap_or(0);
        format!(
            "记录已过期（距今 {} 天），客户端将拒绝该记录",
            today.abs_diff(record.sign_day as i64)
        )
    })?;
    if record.ttl_days > 0 {
        if let Ok(today) = resolve_core::current_unix_day() {
            let sign_unix = resolve_core::BASE_UNIX_DAY + record.sign_day as i64;
            let remaining = sign_unix + record.ttl_days as i64 - today;
            println!("时效状态 : 有效（剩余 {} 天）", remaining.max(0));
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        print_usage();
        return ExitCode::from(2);
    }

    let result = match args[0].as_str() {
        "genkey" => parse_args(&args[1..]).and_then(|a| {
            if a.stdout {
                // 不落盘模式：--out / 裸目录参数在此模式下无意义，直接忽略，绝不创建文件。
                cmd_genkey_stdout()
            } else {
                // 兼容历史用法：genkey 既支持 `--out DIR`，也支持 `genkey DIR`（裸位置参数当作目录）。
                let out = a.positional.last().cloned().unwrap_or(a.out);
                cmd_genkey(&out)
            }
        }),
        "encrypt" => cmd_encrypt(&args[1..]),
        "all" => cmd_all(&args[1..]),
        "decrypt" => cmd_decrypt(&args[1..]),
        _ => {
            print_usage();
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("错误：{e}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    /// 把内容写到系统临时目录下的唯一文件，返回路径（测试结束前由调用方清理）。
    fn write_temp_pem(content: &str, tag: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "resolve-tool-test-{}-{tag}.pem",
            std::process::id()
        ));
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn load_signing_key_pure_private_pem() {
        let (priv_pem, _pub_b64) = resolve_core::generate_keypair().unwrap();
        let path = write_temp_pem(&priv_pem, "pure");
        let key = load_signing_key(path.to_str().unwrap()).expect("纯私钥 PEM 应能解析");
        assert_eq!(key.to_bytes().len(), 32);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_signing_key_folded_single_line() {
        // 头尾标记与 base64 被折叠成一行（多行传输损坏的常见形态）也应能解析。
        let (priv_pem, _pub_b64) = resolve_core::generate_keypair().unwrap();
        let folded = priv_pem.replace('\n', "");
        let path = write_temp_pem(&folded, "folded");
        let key = load_signing_key(path.to_str().unwrap()).expect("折叠单行 PEM 应能解析");
        assert_eq!(key.to_bytes().len(), 32);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_signing_key_rfc8410_with_embedded_pubkey() {
        // RFC 8410 格式（83 字节内容，seed 后内嵌公钥）：解析应跳过内嵌公钥、取到 seed。
        let (_, pub_b64) = resolve_core::generate_keypair().unwrap();
        let pub_raw =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &pub_b64).unwrap();
        let seed = [7u8; 32];
        let mut der = Vec::new();
        der.extend_from_slice(&[0x30, 0x53]); // SEQUENCE len=83
        der.extend_from_slice(&[0x02, 0x01, 0x00]); // INTEGER version
        der.extend_from_slice(&[0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70]); // Ed25519 OID
        der.extend_from_slice(&[0x04, 0x22]); // OCTET STRING len=34
        der.extend_from_slice(&[0x04, 0x20]); // 内层 OCTET STRING len=32
        der.extend_from_slice(&seed);
        der.extend_from_slice(&[0xa1, 0x23, 0x03, 0x21, 0x00]); // [1] BIT STRING 32 字节
        der.extend_from_slice(&pub_raw);

        let b64 = base64::engine::general_purpose::STANDARD.encode(&der);
        let pem = format!("-----BEGIN PRIVATE KEY-----\n{b64}\n-----END PRIVATE KEY-----\n");
        let path = write_temp_pem(&pem, "rfc8410");
        let key = load_signing_key(path.to_str().unwrap()).expect("RFC8410 应能解析");
        assert_eq!(key.to_bytes(), seed);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_signing_key_invalid_returns_err() {
        let path = write_temp_pem("这不是 PEM", "garbage");
        assert!(load_signing_key(path.to_str().unwrap()).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn resolve_signing_key_inline_pem_multiline() {
        let (priv_pem, _) = resolve_core::generate_keypair().unwrap();
        // 以 -----BEGIN 开头即走内联分支，得到的 seed 必须与 PEM 完全对应。
        let key = resolve_signing_key(&priv_pem).expect("多行内联 PEM 应能解析");
        let again = resolve_core::parse_signing_key_pem(&priv_pem).unwrap();
        assert_eq!(key.to_bytes(), again.to_bytes());
    }

    #[test]
    fn resolve_signing_key_inline_pem_folded_and_padded() {
        let (priv_pem, _) = resolve_core::generate_keypair().unwrap();
        // 折叠成单行（命令行粘贴的实际形态）+ 首尾多余空白，都应被 trim/空白剔除兜住。
        let folded = format!("  \n{}\n ", priv_pem.replace('\n', ""));
        let key = resolve_signing_key(&folded).expect("折叠单行 + 外围空白的内联 PEM 应能解析");
        assert_eq!(key.to_bytes().len(), 32);
    }

    #[test]
    fn resolve_signing_key_file_path_unchanged() {
        // 不以 -----BEGIN 开头的入参仍按文件路径处理（历史行为不回归）。
        let (priv_pem, _) = resolve_core::generate_keypair().unwrap();
        let path = write_temp_pem(&priv_pem, "dispatch");
        let key = resolve_signing_key(path.to_str().unwrap()).expect("文件路径应能解析");
        assert_eq!(key.to_bytes().len(), 32);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn resolve_signing_key_inline_garbage_returns_err() {
        // 内联形态但内容损坏：报解析错误，而不是去读一个同名文件。
        let bogus = "-----BEGIN PRIVATE KEY-----\nnot-base64!!\n-----END PRIVATE KEY-----\n";
        assert!(
            resolve_signing_key(bogus).is_err(),
            "损坏的内联 PEM 必须返回 Err"
        );
        assert!(
            resolve_signing_key("  \n-----BEGIN").is_err(),
            "半截内联 PEM 必须返回 Err"
        );
    }

    #[test]
    fn encrypt_roundtrip_with_inline_pem() {
        // 全链路：内联 PEM 解析出的私钥加密 → 对应公钥解密必须还原同一 URL。
        let (priv_pem, pub_b64) = resolve_core::generate_keypair().unwrap();
        let signing_key = resolve_signing_key(&priv_pem.replace('\n', "")).unwrap();
        let url = "https://user:pass@example.com/app/";
        let cipher = resolve_core::sign_and_encrypt(&signing_key, url, 0, None).unwrap();
        let pub_key = resolve_core::parse_public_key(&pub_b64).unwrap();
        assert_eq!(resolve_core::decrypt_txt(&pub_key, &cipher).unwrap(), url);
    }

    #[test]
    fn parse_args_defaults() {
        let a = parse_args(&[]).unwrap();
        assert_eq!(a.ttl_days, 0, "默认 ttl_days=0（永不失效）");
        assert_eq!(a.count, 1, "默认 1 份");
        assert_eq!(a.out, ".", "默认输出目录 .");
        assert!(!a.batch);
        assert!(!a.stdout, "默认落盘，不开 stdout 模式");
        assert!(a.secret.is_none(), "默认不带 --secret");
        assert!(a.positional.is_empty());
    }

    #[test]
    fn parse_args_secret_flag() {
        let a = parse_args(&[
            "--secret".to_string(),
            "s3cr3t".to_string(),
            "key.pem".to_string(),
            "url".to_string(),
        ])
        .unwrap();
        assert_eq!(a.secret.as_deref(), Some("s3cr3t"));
        assert_eq!(a.positional, vec!["key.pem", "url"]);

        // 允许像 UTF-8 中文 / 空白这类真实粘贴值（只禁止空串）。
        let a = parse_args(&["--secret".to_string(), "口令 123".to_string()]).unwrap();
        assert_eq!(a.secret.as_deref(), Some("口令 123"));

        assert!(
            parse_args(&["--secret".to_string()]).is_err(),
            "--secret 缺少值应报错"
        );
        assert!(
            parse_args(&["--secret".to_string(), String::new()]).is_err(),
            "--secret 空串应报错（要默认值就省略参数）"
        );
    }

    #[test]
    fn encrypt_one_dispatches_secret() {
        // 工具层分派：Some(secret) 出的密文只有同一 secret 能解；None 走编译期默认。
        let (priv_pem, pub_b64) = resolve_core::generate_keypair().unwrap();
        let signing_key = resolve_core::parse_signing_key_pem(&priv_pem).unwrap();
        let pub_key = resolve_core::parse_public_key(&pub_b64).unwrap();
        let url = "https://dispatch.example.com/";

        let custom = "per-call-secret";
        let c_custom = encrypt_one(&signing_key, url, 0, None, Some(custom)).unwrap();
        assert_eq!(
            resolve_core::decrypt_txt_with_secret(&pub_key, &c_custom, custom.as_bytes()).unwrap(),
            url
        );
        assert!(
            resolve_core::decrypt_txt(&pub_key, &c_custom).is_err(),
            "默认 Secret 解不开 --secret 密文"
        );

        let c_default = encrypt_one(&signing_key, url, 0, None, None).unwrap();
        assert_eq!(
            resolve_core::decrypt_txt(&pub_key, &c_default).unwrap(),
            url
        );
        assert!(
            resolve_core::decrypt_txt_with_secret(&pub_key, &c_default, custom.as_bytes()).is_err()
        );
    }

    #[test]
    fn parse_args_stdout_flag() {
        let a = parse_args(&["--stdout".to_string()]).unwrap();
        assert!(a.stdout, "--stdout 应被识别");
        // 即使同时给了 --out，标志位本身也应正常解析（落不落盘由 genkey 分派决定）。
        let a = parse_args(&[
            "--stdout".to_string(),
            "--out".to_string(),
            "keys".to_string(),
        ])
        .unwrap();
        assert!(a.stdout);
        assert_eq!(a.out, "keys");
    }

    #[test]
    fn parse_args_all_flags_and_positional() {
        let args: Vec<String> = [
            "--ttl-days",
            "60",
            "--count",
            "3",
            "--out",
            "/tmp",
            "--batch",
            "key.pem",
            "url1",
            "url2",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let a = parse_args(&args).unwrap();
        assert_eq!(a.ttl_days, 60);
        assert_eq!(a.count, 3);
        assert_eq!(a.out, "/tmp");
        assert!(a.batch);
        assert!(!a.stdout);
        assert_eq!(a.positional, vec!["key.pem", "url1", "url2"]);
    }

    #[test]
    fn cmd_genkey_stdout_ok_and_touches_no_files() {
        // --stdout 模式只应成功产出密钥材料，不得在 cwd（crate 源码目录）落下任何文件。
        cmd_genkey_stdout().expect("--stdout 生成密钥对应成功");
        let cwd = std::env::current_dir().unwrap();
        assert!(
            !cwd.join("private_key.pem").exists(),
            "--stdout 不得写 private_key.pem"
        );
        assert!(
            !cwd.join("public_key.b64").exists(),
            "--stdout 不得写 public_key.b64"
        );
    }

    #[test]
    fn parse_args_errors() {
        assert!(
            parse_args(&["--ttl-days".to_string()]).is_err(),
            "--ttl-days 缺少数值应报错"
        );
        assert!(
            parse_args(&["--ttl-days".to_string(), "abc".to_string()]).is_err(),
            "--ttl-days 非整数应报错"
        );
        assert!(
            parse_args(&["--count".to_string(), "0".to_string()]).is_err(),
            "--count 0 不允许"
        );
        assert!(
            parse_args(&["--count".to_string(), "x".to_string()]).is_err(),
            "--count 非整数应报错"
        );
        assert!(
            parse_args(&["--out".to_string()]).is_err(),
            "--out 缺少数值应报错"
        );
    }

    #[test]
    fn resolve_public_key_inline_bare_b64() {
        // 内联裸 base64：直接解析出 32 字节公钥，与 parse_public_key 结果一致。
        let (_, pub_b64) = resolve_core::generate_keypair().unwrap();
        let key = resolve_public_key(&pub_b64).expect("内联裸 base64 公钥应能解析");
        assert_eq!(key, resolve_core::parse_public_key(&pub_b64).unwrap());
    }

    #[test]
    fn resolve_public_key_inline_spki_pem() {
        // 内联 SPKI PEM：拼上 Ed25519 固定 SPKI 前缀后应走 PEM 分支解析。
        let (_, pub_b64) = resolve_core::generate_keypair().unwrap();
        let pub_raw =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &pub_b64).unwrap();
        let der = [
            &[
                0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
            ],
            pub_raw.as_slice(),
        ]
        .concat();
        let b64 = base64::engine::general_purpose::STANDARD.encode(der);
        let pem = format!("-----BEGIN PUBLIC KEY-----\n{b64}\n-----END PUBLIC KEY-----\n");
        let key = resolve_public_key(&pem).expect("内联 SPKI PEM 应能解析");
        assert_eq!(key, resolve_core::parse_public_key(&pem).unwrap());
    }

    #[test]
    fn resolve_public_key_file() {
        // 文件路径：.b64 文件存在时读文件解析；写错的路径按内联 base64 解析应报错。
        let (_, pub_b64) = resolve_core::generate_keypair().unwrap();
        let path = write_temp_pem(&pub_b64, "pubfile");
        let key = resolve_public_key(path.to_str().unwrap()).expect("公钥文件应能解析");
        assert_eq!(key, resolve_core::parse_public_key(&pub_b64).unwrap());
        let _ = std::fs::remove_file(&path);

        assert!(
            resolve_public_key("nonexistent-key-file.b64").is_err(),
            "不存在的路径应得到解析错误"
        );
    }

    #[test]
    fn civil_from_unix_day_known_vectors() {
        // 固定向量：epoch 零点、闰年日 2024-02-29、基准日与今日。
        assert_eq!(civil_from_unix_day(0), (1970, 1, 1));
        // 2024-02-29 ⇒ UNIX 日 19782（1709208000/86400），验证 2 月年末归算与闰年。
        assert_eq!(civil_from_unix_day(19782), (2024, 2, 29));
        // 2026-01-01 ⇒ 20454；2026-10-07 ⇒ 20733。
        assert_eq!(civil_from_unix_day(20454), (2026, 1, 1));
        assert_eq!(civil_from_unix_day(20733), (2026, 10, 7));
    }

    #[test]
    fn format_day_date_known_vectors() {
        // v4 签发日 → UTC 日期：第 0 天即基准日；第 279 天为 2026-10-07。
        assert_eq!(format_day_date(0), "2026-01-01");
        assert_eq!(format_day_date(279), "2026-10-07");
    }

    #[test]
    fn cmd_decrypt_success_and_expired() {
        // 命令级闭环：encrypt 产出的密文直接喂给 cmd_decrypt（位置参数）。
        let (priv_pem, pub_b64) = resolve_core::generate_keypair().unwrap();
        let signing_key = resolve_core::parse_signing_key_pem(&priv_pem).unwrap();
        let url = "https://verify.example.com/";

        // ttl_days=0：解密验证成功，退出语义为 Ok。
        let cipher = resolve_core::sign_and_encrypt(&signing_key, url, 0, None).unwrap();
        cmd_decrypt(&[pub_b64.clone(), cipher]).expect("ttl_days=0 密文应验证成功");

        // 7 天窗口、当日签发未过期：同样 Ok。
        let cipher = resolve_core::sign_and_encrypt(&signing_key, url, 7, None).unwrap();
        cmd_decrypt(&[pub_b64.clone(), cipher]).expect("未过期密文应验证成功");

        // 第 0 天（2026-01-01）签发、窗口 1 天的过期密文：解密内容可展示，但命令必须返回 Err。
        let cipher = resolve_core::sign_and_encrypt(&signing_key, url, 1, Some(0)).unwrap();
        assert!(
            cmd_decrypt(&[pub_b64, cipher]).is_err(),
            "过期密文必须返回 Err"
        );
    }
}
