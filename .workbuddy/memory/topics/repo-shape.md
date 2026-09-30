# 仓库形态 / Git / CI（2026-09-22 自 MEMORY.md 瘦身迁入，内容未改）

> 本文件覆盖「仓库根 = `C:\dev\rust\web-app`」这一形态下的所有非直观事实。
> 相关工作前的**单一真源清单**见 `topics/tauri-contracts.md` 开头。

## 0. 仓库形态（2026-09-22 起**单仓库**，仓库根 = `C:\dev\rust\web-app`）

两壳同级 `webview-app/` + `electron-app/`；远端 `git@github.com:woxiangfy/web2app.git`（此前只有 `webview-app/` 是仓库）。

1. ⚠️⚠️ `.git` 由 `webview-app/` **直移**上移一级、**未重写历史** ⇒ 老文件被识别为「重命名」（40 项），**逐文件历史必须 `git log --follow`**（提交 `be5cfa9`）。
2. ⚠️⚠️ **跨壳相对路径禁改**（两壳同级是前提）：`electron-app/package.json` 的 `--manifest-path ../webview-app/Cargo.toml`、`stage-assets.cjs` 取 `../webview-app/icons/`、注入脚本/`settings.html`/`shell.html`/sidecar 产物全指 `webview-app/` ⇒ **禁止把 `electron-app` 挪进 `webview-app`**。
3. ⚠️⚠️ CI 在**仓库根** `.github/workflows/`：4 个含 cargo 的 workflow 统一 `defaults.run.working-directory`（`webview-app` / `electron-app`）。⚠️ `defaults.run` **只作用于 `run:`、不作用于 `uses:`** ⇒ `action-gh-release` 的 `files:`、`upload-artifact` 的 `path:` **必须手加目录前缀**。⚠️ `generate-signing-key.yml` 有 checkout 但**刻意不设**该默认值（只为读根目录契约脚本；设了每个 run 步骤都 "No such file or directory"）。
4. 根 `.gitignore` 忽略 `signing/`（明文 keystore/secrets）、`.workbuddy/`、`.trae/`、防御性 `target/`+`node_modules/`，以及 **锚定的 `/tests/`**（2026-09-23 加：根 README §3.2 约定 `tests/` 只放一次性产物；**必须锚定**，不锚定会连带忽略将来任何层级的 `tests/`，如 `src/tests/`）。自检：`git check-ignore -v signing/ks.b64`，暂存区 `^signing/` 计数必须 0。
5. 归档稿已迁 `.trae/documents/` → `docs/archive/`。⚠️ 本机 `core.autocrlf=true` ⇒ `git add` 警告 `LF will be replaced by CRLF`，工作区被 git touch 后变 CRLF（与 README §五.2「统一 LF」冲突）——**未处理，待决**。
6. 两壳版本号都 `0.1.0` 且无 tag ⇒ 首次发版前定前缀（`tauri-v*` / `electron-v*`）避免撞车。
