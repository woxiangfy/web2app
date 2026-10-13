# web2app 配置字段总表（Step 1 交付物，2026-09-22）

> ⚠️ **历史快照（重构前）· 仅作溯源**：本文记录的是**契约统一之前**的字段现状与问题清单，请**不要**当现行规范阅读。
> 所列问题（`window_title` 冗余、`data_dir_default` 两壳同名反义、模板缺键、契约多实现漂移）**均已解决**；
> 现行唯一真源是 [`packaging-config.md`](./packaging-config.md)（解析器 `.github/scripts/parse-packaging-config.py`，现 **19** 键；`version` 字段是 2026-09 之后新增的，故早期快照写 18 键）。
> 以下内容为「问题从哪来」的溯源记录，已删去与现行契约重复的逐层字段明细表。

## 溯源结论（一句话）

字段设计本身合理（`app_name` / `title` 分离、`app_url` 双形态 + 公钥强校验、`extra` 注入干净）；
问题全在「契约有多份实现、无一致性门禁」。该根因已通过「解析器为唯一真源 + `packaging-config.md` 为唯一文档」闭环。

## 当时识别并已闭环的关键问题（溯源）

- **`window_title` 冗余**：旧仅 CI/Tauri 生效、Electron 完全忽略，取值链 `title → window_title → app_name`。解析器 `REMOVED_KEYS` 已移除该键，窗口标题改由 `title` 统一承载（见 `packaging-config.md` §已移除字段）。
- **`data_dir_default` 同名反义**：同一取值在两壳语义相反（`true` 在 Tauri 安装版可能直接打不开窗）。已改为**字符串** `data_dir`（取值 `"data"` = exe 同级 `data\`（默认）／`""` = 系统默认程序目录／其它相对路径／绝对路径，两壳同一套规则，见 `packaging-config.md` §4.3）。
- **模板缺键 / 多实现漂移**：当时 7 份实现键数各异（13–18），`signing/packaging-config.json` 复用会静默丢 `extra` / `resolve_secret` / `window_title`。已统一为解析器单一真源（当时 18 键，**现 19 键**），模板由它派生、缺键留 `""` / `{}`。
- **运行期只剩一个 JSON**：`window-state.json` 已失效，窗口几何并入 `settings.json` 的 `window` 段（旧文件不迁移、不删除，仅作读取兜底）。Electron 侧 `config.json` 改为**随 `app.asar` 进包**（不再是「改完重启即生效」的独立文件）。
- **空字段澄清**：`icon_url`（仅构建期）、4 个 `android_*`（仅打包期）本就不进编译期 env 层，属正常，非缺陷。

> 重构前的「6 层配置面」取证视角（A 打包 JSON / B 仓库变量 / C 编译期 env / D 运行期配置 / E 命令行 / F 构建配置）仅作理解背景；
> 现行字段含义、类型、默认值、校验规则一律以 [`packaging-config.md`](./packaging-config.md) 为准。
