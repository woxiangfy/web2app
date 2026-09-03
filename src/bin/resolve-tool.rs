//! resolve-tool —— 动态解析的【服务端】命令行工具。
//!
//! 用于在打包之外，本地/CI 上用 Ed25519 私钥生成：
//!   1. 密钥对（Ed25519 私钥 PEM + 公钥裸 base64）
//!   2. TXT 密文（私钥签名 + AES-256-GCM 加密，写入 DNS TXT）
//!
//! 与客户端 `src/resolve.rs` 的解密逻辑严格对称；与 `tools/encrypt_url.py` 等价，
//! 提供纯 Rust 版本，方便在 CI（无需 Python cryptography 依赖）里使用。
//!
//! 用法：
//!   resolve-tool genkey [--out DIR]             生成密钥对，写 private_key.pem + public_key.b64
//!   resolve-tool encrypt <private_key.pem> <url> [--ttl 秒]
//!      用私钥加密一个 URL，stdout 输出 base64 密文（单行）
//!   resolve-tool encrypt --batch <private_key.pem> [--ttl 秒]
//!      从 stdin 逐行读 URL，批量加密
//!
//! 说明：
//!   --ttl 有效期秒数，默认 172800（48h）；0 表示永不失效。

use std::process::ExitCode;

fn print_usage() {
    eprintln!(
        "\
resolve-tool —— 动态解析服务端工具（Ed25519 签名 + AES-256-GCM）

用法：
  resolve-tool genkey [--out DIR]
      生成 Ed25519 密钥对，写 DIR/private_key.pem 与 DIR/public_key.b64（默认 .）

  resolve-tool encrypt <private_key.pem> <url> [--ttl 秒]
      用私钥加密单个 URL，stdout 输出 base64 密文（单行，写入 DNS TXT）

  resolve-tool encrypt --batch <private_key.pem> [--ttl 秒]
      从 stdin 逐行读 URL，批量加密

参数：
  --ttl  有效期秒数（默认 172800=48h；0=永不失效）
"
    );
}

fn parse_ttl(args: &[String]) -> Result<u64, String> {
    // 找 --ttl，返回其后的整数值；不存在则默认 48h。
    for (i, a) in args.iter().enumerate() {
        if a == "--ttl" {
            let v = args
                .get(i + 1)
                .ok_or_else(|| "--ttl 需要一个秒数（整数）".to_string())?;
            return v
                .parse::<u64>()
                .map_err(|_| format!("--ttl 必须是整数秒数，收到：{v}"));
        }
    }
    Ok(48 * 3600)
}

/// 从 PEM（PKCS#8 纯私钥格式）解析 Ed25519 私钥。
///
/// 兼容两种来源：本工具 `genkey` 生成的纯私钥（48 字节 DER）、以及 Python
/// `cryptography` 导出的 `Ed25519PrivateKey`（同样 48 字节纯私钥 DER）。
/// 也兼容 ed25519-dalek 的 `to_pkcs8_pem` 输出的 RFC 8410 格式（83 字节，带内嵌公钥）。
fn load_signing_key(path: &str) -> Result<ed25519_dalek::SigningKey, String> {
    let pem = std::fs::read_to_string(path).map_err(|e| format!("读取私钥失败：{e}"))?;
    let pem = pem.trim();

    // 提取 PEM body 的 base64。
    let b64: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect();
    let der = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64.trim())
        .map_err(|e| format!("私钥 PEM base64 解码失败：{e}"))?;

    // Ed25519 私钥 seed（32 字节）位于 DER 末尾：
    //   - 纯私钥格式（48 字节）：`... 04 20 <seed>`，seed 是最后 32 字节。
    //   - RFC 8410 格式（83 字节）：`... 04 20 <seed> a1 23 03 21 00 <pub>`，seed 也是
    //     从内层 OCTET STRING 开始，但公钥跟在后面。统一做法：定位 `04 20`（内层
    //     OCTET STRING 长度 32）后紧跟的 32 字节即 seed。
    if der.len() < 34 {
        return Err("私钥 DER 长度异常".to_string());
    }
    let mut seed_pos = None;
    for i in 0..der.len().saturating_sub(1) {
        if der[i] == 0x04 && der[i + 1] == 0x20 {
            seed_pos = Some(i + 2);
            break;
        }
    }
    let seed_pos = seed_pos.ok_or_else(|| "私钥 DER 中未找到 Ed25519 seed".to_string())?;
    let seed = der
        .get(seed_pos..seed_pos + 32)
        .ok_or_else(|| "私钥 seed 长度不足 32 字节".to_string())?;
    let bytes: [u8; 32] = seed.try_into().map_err(|_| "seed 长度异常".to_string())?;
    Ok(ed25519_dalek::SigningKey::from_bytes(&bytes))
}

fn cmd_genkey(out_dir: &str) -> Result<(), String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("创建目录 {out_dir} 失败：{e}"))?;
    let (priv_pem, pub_b64) = web2app_lib::resolve::generate_keypair()?;

    let priv_path = std::path::Path::new(out_dir).join("private_key.pem");
    let pub_path = std::path::Path::new(out_dir).join("public_key.b64");
    std::fs::write(&priv_path, priv_pem.as_bytes())
        .map_err(|e| format!("写私钥失败：{e}"))?;
    std::fs::write(&pub_path, pub_b64.as_bytes())
        .map_err(|e| format!("写公钥失败：{e}"))?;

    println!("私钥（服务端持有，切勿公开）: {}", priv_path.display());
    println!("公钥裸 base64（客户端 PUBLIC_KEY）: {}", pub_path.display());
    println!();
    println!("公钥内容：");
    println!("{pub_b64}");
    Ok(())
}

fn cmd_encrypt(args: &[String]) -> Result<(), String> {
    // args 形如：[encrypt, <private_key.pem>, <url>, (--ttl N)]
    //          或 [encrypt, --batch, <private_key.pem>, (--ttl N)]
    let ttl = parse_ttl(args)?;

    let mut positional: Vec<&String> = Vec::new();
    let mut batch = false;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--ttl" {
            i += 2; // 跳过 --ttl 及其值
            continue;
        } else if a == "--batch" {
            batch = true;
            i += 1;
            continue;
        } else {
            positional.push(a);
            i += 1;
        }
    }

    let key_path = positional
        .first()
        .ok_or_else(|| "缺少私钥路径参数".to_string())?
        .as_str();
    let signing_key = load_signing_key(key_path)?;

    let urls: Vec<String> = if batch {
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
        positional
            .iter()
            .skip(1)
            .map(|s| s.as_str().to_string())
            .collect()
    };

    if urls.is_empty() {
        return Err("未提供要加密的 URL".to_string());
    }

    for url in &urls {
        let cipher = web2app_lib::resolve::sign_and_encrypt(&signing_key, url, ttl, None)?;
        println!("# {url}  (ttl={ttl}s)");
        println!("{cipher}");
        println!();
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
        "genkey" => {
            // 解析 --out
            let mut out = String::from(".");
            let mut i = 1;
            while i < args.len() {
                if args[i] == "--out" {
                    match args.get(i + 1) {
                        Some(v) => {
                            out = v.clone();
                            i += 2;
                        }
                        None => {
                            print_usage();
                            return ExitCode::from(2);
                        }
                    }
                } else {
                    out = args[i].clone();
                    i += 1;
                }
            }
            cmd_genkey(&out)
        }
        "encrypt" => cmd_encrypt(&args[1..]),
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
