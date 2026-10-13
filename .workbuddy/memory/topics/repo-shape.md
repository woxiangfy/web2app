# 仓库形态 / Git / CI（2026-09-22 自 MEMORY.md 瘦身迁入，内容未改）

> 本文件覆盖「单仓库（根目录下 `webview-app/` + `electron-app/` 同级）」这一形态下的所有非直观事实。
> ⚠️ 仓库根路径**已迁移**：文中历史叙述里的 `C:\dev\rust\web-app` 是 2026-09 时期的落点，
> **当前工作区为 `D:\web2app`**；具体路径随机器变化，判断形态只看「两壳同级 + `.github/` 在根」。
> 相关工作前的**单一真源清单**见 `topics/tauri-contracts.md` 开头。

## 0. 仓库形态（2026-09-22 起**单仓库**；根路径随机器而变，现为 `D:\web2app`）

两壳同级 `webview-app/` + `electron-app/`；远端 `git@github.com:woxiangfy/web2app.git`（此前只有 `webview-app/` 是仓库）。

1. ⚠️⚠️ `.git` 由 `webview-app/` **直移**上移一级、**未重写历史** ⇒ 老文件被识别为「重命名」（40 项），**逐文件历史必须 `git log --follow`**（提交 `be5cfa9`）。
2. ⚠️⚠️ **跨壳相对路径禁改**（两壳同级是前提）：`electron-app/package.json` 的 `--manifest-path ../webview-app/Cargo.toml`、`stage-assets.cjs` 取 `../webview-app/icons/`、注入脚本/`settings.html`/`shell.html`/sidecar 产物全指 `webview-app/` ⇒ **禁止把 `electron-app` 挪进 `webview-app`**。
3. ⚠️⚠️ CI 在**仓库根** `.github/workflows/`：4 个含 cargo 的 workflow 统一 `defaults.run.working-directory`（`webview-app` / `electron-app`）。⚠️ `defaults.run` **只作用于 `run:`、不作用于 `uses:`** ⇒ `action-gh-release` 的 `files:`、`upload-artifact` 的 `path:` **必须手加目录前缀**。⚠️ `generate-signing-key.yml` 有 checkout 但**刻意不设**该默认值（只为读根目录契约脚本；设了每个 run 步骤都 "No such file or directory"）。
4. 根 `.gitignore` 忽略 `signing/`（明文 keystore/secrets）与防御性 `target/`+`node_modules/`；⚠️ **`.workbuddy/`、`.trae/` 已改为纳入版本控制**（那两行在 `.gitignore` 里已注释掉，口径为「AI AGENT 记忆目录」）—— 别再拿它们当「被忽略目录」的例子。⚠️ 根 `tests/` **不是整目录忽略**，而是**逐条锚定**（`/tests/pack/`、`/tests/json/`、`/tests/pw-fixture/`、`/tests/pw-udd-*/`；每条**都带前导 `/` 锚定**，不锚定会连带忽略将来任何层级的 `tests/`，如 `src/tests/`）；`tests/ci-sdk-step/`、`tests/build-*.bat`、`tests/float-frag-rc/` 等已被跟踪（后者是**已知遗留待清理**，清除需破坏性的 `git rm -r --cached`）。自检：`git check-ignore -v signing/ks.b64`，暂存区 `^signing/` 计数必须 0。
5. 归档稿已迁 `.trae/documents/` → `docs/archive/`。⚠️ 本机 `core.autocrlf=true` ⇒ `git add` 警告 `LF will be replaced by CRLF`，工作区被 git touch 后变 CRLF（与根 `AGENTS.md` §3.4 第 2 条「文档统一 UTF-8（无 BOM）、LF 换行」冲突）——**未处理，待决**。⚠️ 该规则原在根 `README.md`（2026-10-07 更名为 `AGENTS.md`），旧引用写作「README §五.2」。
6. 两壳版本号都 `0.1.0` 且无 tag ⇒ 首次发版前定前缀（`tauri-v*` / `electron-v*`）避免撞车。
