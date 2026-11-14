# 动态解析：本机手动生成 TXT 密文与打包

> 适用：`app_url` 配成**域名数组**，应用启动时查 DNS TXT → Ed25519 验签 + AES-256-GCM 解密 → 得到真实 URL。  
> 本文是 **Windows 本机从零到可运行包**的完整操作手册：生成密钥材料 → 加密 URL → 发布 TXT → 写配置 → 本机出包。  
> 只用仓库自带的批处理、Rust 命令行工具与 Node 打包脚本；不需要 CI，不需要额外安装任何库。  
> 字段语义见 [packaging-config.md](packaging-config.md)；CI 侧的等效流程见 `.github/workflows/resolve-keys.yml`。

## 0. 准备

| 需要                | 说明                                   |
| ----------------- | ------------------------------------ |
| Rust 工具链（`cargo`） | 编译 `resolve-tool`（独立 crate，不编 Tauri） |
| Node.js ≥ 18      | 生成随机 Secret、跑本机打包脚本                  |
| 一个可改 DNS TXT 的域名  | 下称**跳板域名**，如 `node1.example.com`     |
| 仓库根 = 工作目录        | 下文命令除注明外都在仓库根执行                      |

三份工具：

| 文件                                | 作用                                           |
| --------------------------------- | -------------------------------------------- |
| `scripts/gen-resolve-keys.bat`    | **双击直接运行**，一次性生成并打印三段密钥材料，**不落盘**            |
| `webview-app/crates/resolve-tool` | 纯 Rust CLI：`encrypt` 出 TXT 密文、`decrypt` 本地自检 |
| `scripts/pack-local.mjs`          | 本机打包，读一份配置 JSON 出可运行的包                       |

> ⚠️ **密钥材料是一整套身份**：`RESOLVE_SECRET`、私钥、公钥、TXT 密文、打好的包互相关联。  
> 重新运行 bat 会生成全新的 Secret + 密钥对 ⇒ 之前的所有密文、公钥、包**全部作废**，必须整条链重做。

## 1. 双击生成密钥材料

双击 `scripts/gen-resolve-keys.bat`（或在 cmd 里运行它）。窗口依次打印三段值：

```
======================= RESOLVE_SECRET =======================
RANDOM_SECRET_BASE64_HERE                            ← ① Secret（实际为 44 字符随机串）
==============================================================

# private_key.pem —— Ed25519 私钥（服务端留存，切勿公开）
-----BEGIN PRIVATE KEY-----
PEM-BODY-HERE                                        ← ② 私钥（三行：BEGIN / body / END）
-----END PRIVATE KEY-----
# public_key —— Ed25519 公钥裸 base64（客户端配置 "public_key"）
PUBLIC_KEY_BASE64_HERE                               ← ③ 公钥（实际为 44 字符）
```

- **立刻把三段值抄到自己的安全笔记**：脚本不写任何文件、不读旧值，关掉窗口就永久丢失。
- `#` 开头是说明行，**不要**抄进私钥；私钥只取 `BEGIN` / 中间那行 / `END` 共三行。
- 窗口若报 `cargo was not found` / `Node.js was not found` ⇒ 装好 Rust 工具链 / Node 18+ 再重跑。

## 2. 用私钥生成 TXT 密文

在仓库根打开 cmd，进入 Rust 工程目录后调 `encrypt`：

```bat
cd webview-app
cargo run -p resolve-tool -- encrypt "-----BEGIN PRIVATE KEY-----PEM-BODY-HERE-----END PRIVATE KEY-----" "https://www.example.com/app/" --ttl-days 0 --secret "PASTE_SECRET_HERE" --count 5
```

把示例里三个占位处换成第 1 步抄下的值：

| 参数             | 填法                                                                                                            |
| -------------- | ------------------------------------------------------------------------------------------------------------- |
| 第 1 个参数        | 私钥 PEM **三行拼成一行**、整串用双引号包住（BEGIN + body + END，不带 `#` 行）。工具见 `-----BEGIN` 开头即按内联 PEM 解析，无需先存文件；也可直接传 PEM 文件路径。 |
| 第 2 个参数        | 要加密的**真实 URL**；可连续写多个，或用 `--batch` 从 stdin 逐行读。                                                               |
| `--secret`     | **必须与第 1 步的 `RESOLVE_SECRET` 逐字一致**；客户端把同一个值编进二进制，两端不一致必然解密失败。                                                |
| `--ttl-days 0` | 永不失效（推荐）。给正整数 = 生成后 N 天内有效（单位天，上限 65535），到期客户端拒绝。                                                             |
| `--count N`    | 同一 URL 出 N 份不同密文（随机 Nonce，解出来是同一 URL），可分散到多个域名 / 多条 TXT。                                                      |

输出形如（`#` 是注释行、末尾有空行）：

```
# https://www.example.com/app/  (ttl_days=0)
k2zRmLjpkYY/R8vlT8f/D6Pz647j7r2icyZ8uplqIYhS...
```

**只取密文那一行**（单行 base64，约 160~180 字符）。单条 TXT 记录的单串上限是 255 字节，放得下。

> 变体：若要把私钥存成文件长期复用，不用 bat，改跑  
> `cargo run -p resolve-tool -- genkey --out ..\tests\json\keys`，  
> 得到 `private_key.pem` / `public_key.b64`；此时**公钥改用 `public_key.b64` 的内容**，  
> `encrypt` 的第一个参数换该文件路径，Secret 仍由 bat 给出并用 `--secret` 传入。

## 3. 本机解密自检（先做，再上 DNS）

仍在 `webview-app` 目录，把公钥、密文、Secret 换成自己的值：

```bat
cargo run -p resolve-tool -- decrypt "PASTE_PUBLIC_KEY_HERE" "PASTE_CIPHER_HERE" --secret "PASTE_SECRET_HERE"
```

看到下面这样的输出才算通过：

```
解密验证通过（AES-GCM 认证 + Ed25519 验签）
URL      : https://www.example.com/app/
签发日期 : 2026-11-14 UTC（第 317 天）
有效期   : 永不失效（ttl_days=0）
```

失败就先查三样：Secret 是否抄全、公钥与私钥是否同一对、密文是否带空格/换行/漏字符。**别带着一条本地都解不开的密文去改 DNS。**

## 4. 发布 DNS TXT 记录

1. 在域名控制台给跳板域名（如 `node1.example.com`）新增一条 **TXT 记录**，值 = 第 2 步的密文那一行。
   ⚠️ **记录名里别在中间塞下划线**：`txt_w2a_abc.example.com` 这类名字在 2026-11-14 之前会被客户端
   静默判成非法（客户端只看到「服务器连接失败」，而 `nslookup` / 浏览器一切正常，极易误判为网络问题）。
   现在客户端已放宽（`Name::from_str_relaxed`），但仍建议用 `txt-w2a-abc.example.com` 或
   `_txt-w2a-abc.example.com` —— 见 `docs/changes/2026-11-14.md`。
2. 值必须是**单行纯密文**：不带引号、不带 `#` 注释、不带空格换行。控制台显示时加的引号是 DNS 展示格式，不属于记录值；若控制台把长值拆成多段，客户端会自动拼接各段，可用。
3. 多个跳板域名可各挂一条（用 `--count` 出的多份密文分散），运行时**按 `app_url` 数组顺序**逐个尝试，任一成功即用；全部失败报「服务器连接失败」。
4. 等 DNS 生效后本机核对：

```bat
nslookup -type=TXT node1.example.com
```

应能看到与第 2 步一致的密文。

## 5. 写打包配置

在 `tests/json/` 下新建 `resolve-local.json`（该目录已被 `.gitignore` 忽略，不会入库）：

```json
{
  "app_url": ["node1.example.com", "node2.example.com"],
  "public_key": "第 1 步的 public_key",
  "resolve_secret": "第 1 步的 RESOLVE_SECRET",
  "app_name": "MyApp",
  "title": "我的应用",
  "debug": true
}
```

三处必须同源，写错不报错、只在运行时表现为「解不开」：

- `public_key` 与加密用的**私钥成对**；
- `resolve_secret` 与 `encrypt --secret` **逐字相同**；
- `app_url` 必须是**真数组**（写成字符串就走直接地址模式，根本不查 TXT）；数组模式缺 `public_key` 会在打包期直接报错。

`debug: true` 便于首次验证看 `<数据目录>\logs\startup.log`，验完可删掉重打。

## 6. 本机打包并运行

回到仓库根：

```bat
node scripts/pack-local.mjs --config tests/json/resolve-local.json --target electron-all --dry-run
node scripts/pack-local.mjs --config tests/json/resolve-local.json --target electron-all
```

产物落 `tests/pack/electron/`，双击 exe 应打开真实 URL。要 Tauri 壳就把 `--target` 换成 `tauri-portable` / `tauri-msi`（需先 `cargo install tauri-cli --version "^2"`），产物落 `tests/pack/tauri/`。

> ⚠️ `resolve_secret` 是**编译期**注入（Tauri 编进主程序、Electron 编进 sidecar `w2a-proxy.exe`）：  
> **改 Secret 或公钥必须重新打包**；只换 DNS TXT 里的密文**不需要**重新打包 —— 这正是动态解析的用途。

## 7. 失败速查

| 现象                    | 原因 / 处理                                                                                    |
| --------------------- | ------------------------------------------------------------------------------------------ |
| 第 3 步本地 `decrypt` 就失败 | Secret 与 `--secret` 不一致（最常见）；公钥私钥不成对；密文带空格/换行/抄漏                                           |
| `nslookup` 看不到记录      | DNS 未生效或记录加错域名 / 记录类型；等 TTL 刷新后再试                                                          |
| 客户端报「服务器连接失败」         | 先看 `nslookup` 与 `startup.log`；再核对 `app_url` 是否为数组、`public_key` / `resolve_secret` 是否与生成时同源 |
| 客户端报「服务器连接失败」但 `nslookup` 正常 | **看记录名的下划线位置**：`txt_w2a_abc.example.com` 这种「下划线在 label 中间」的名字，2026-11-14 之前的客户端会在构造查询名时就拒绝（hickory 走 IDNA/STD3），请求根本没发出去 ⇒ 用户只看到「服务器连接失败」。已修（`Name::from_str_relaxed`），旧包需重新打包 |
| 解析成功但页面打不开（错误页 / ERR_NAME_NOT_RESOLVED） | **密文里的 URL 本身不可用**：主机 NXDOMAIN 或没有 A 记录（`nslookup <主机> 1.1.1.1` 一并核）；此时客户端解析是全绿的，问题在签发数据 ⇒ 用正确地址重新 `resolve-tool encrypt` 并更新 TXT |
| DNS 有记录但客户端拒绝         | TXT 值带了引号 / `#` 注释 / 换行；`--ttl-days` 已过期；密文是旧工具签发的格式（客户端只认当前格式，按本手册重签）                     |
| 只改了 DNS / 配置，忘了重打包    | 换了 Secret 或公钥 ⇒ 客户端里编死的值没变，必须重新打包                                                          |
| 重跑过 bat               | 身份全变 ⇒ 公钥、Secret、TXT 密文、包整套重做                                                              |

## 8. 规则与安全底线

- **格式**（排错够用即可）：`Payload = URL ‖ 0x00 ‖ 签发日(2B 大端) ‖ 有效期天数(2B 大端)`；  
  `TXT = base64( Nonce(12B) ‖ AES-256-GCM( HKDF(Secret, Nonce), Payload ‖ Ed25519 签名(64B) ) )`。  
  固定开销 97 字节 + URL 长度；解密端只认这一种尾部 4 字节的结构，权威实现在  
  `webview-app/crates/resolve-core/src/lib.rs`。
- **时间**：签发日 = 距 UTC 2026-01-01 的天数；`ttl_d = 0` 永不失效，`> 0` 时客户端校验 `今日 - d <= ttl_d`。
- **安全边界**：私钥签名保证 DNS 即使被控制也无法伪造出能验签的密文（最多造成解析失败）；  
  AES 层只防随手爬取，Secret 编在客户端里，**不防反编译**。私钥与 Secret 都不要提交仓库。
- **每次加密结果都不同**（随机 Nonce），但解密结果一致；不要拿「两次密文不一样」当故障。
