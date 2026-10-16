# 打包配置 JSON 评审（2026-09-22）

> ⚠️ **历史快照（重构前）· 仅作溯源**：本文是**契约统一之前**的评审记录，请**不要**当现行规范阅读。
> 文中 P0/P1 问题（`resolve_secret` 无通道、`data_dir_default` 同名反义、字符串布尔、模板缺键、
> Android 包名冲突、未知字段静默忽略等）**均已解决或已按既有约束编排**；
> 现行唯一真源是 [`packaging-config.md`](./packaging-config.md)。
> 以下内容为「问题从哪来」的溯源记录，已删去与现行契约重复的逐项证据表与验收判据。

## 溯源结论（一句话）

字段设计合理，问题全在「契约有多份实现、且没有一致性门禁」——同一份 JSON 交给两个壳会得到不同行为，其中 3 处会造成静默失效或直接打不开。该根因已闭环。

## 当时识别并已闭环的关键问题（溯源）

- **P0-1 `resolve_secret` 在 Electron 绿色版不生效**：旧 `w2a-proxy` 无 `--secret` 通道、`config.ts` 读入后无使用点。已改为经 stdin 贯通两壳（Secret 走 `--secret-stdin`，与 URL 同规避命令行对同用户进程可见）。
- **P0-2 `data_dir_default` 同名反义**：同 `config-fields` 快照，已改枚举 `data_dir`。
- **P0-3 复用 CI 生成的 `packaging-config.json` 静默丢字段**：已统一为解析器单一真源（当时 18 键，现行键数以 `packaging-config.md` 为准），模板列全契约键、缺的留 `""` / `{}`；签名四项拆到独立文件，避免密钥随包分发。
- **P1 系列已落地**：未知字段硬失败 + 拼写建议（`difflib.get_close_matches`，未知字段 = 打包失败）；布尔白名单校验并归一（删 Electron 的 `'on'` 分支）；`app_url` scheme 校验（`^https?://` / 纯域名）；`icon_url` PNG IHDR 正方形且 ≥256×256 校验；新增 `data_dir_name`（解耦数据目录名与 `app_name`，发布后不可变更）、`android_package`、`allow_ephemeral_keystore`；一次性 keystore 的 warning 升级为 `::error::`；CI 增 `config-contract` 类门禁对齐多实现。

## 做得好的部分（保持）

`app_name` / `title` 语义分离；`app_url` 数组 + `public_key` 强校验与失败回落链；`extra` 注入不走 HTML 拼接（无 `</script>` 逃逸）；`packaging-config.single.json` 单行形态（专为粘贴输入框设计）。

> 完整的现行字段契约、类型、默认值、平台作用域与校验规则，一律以 [`packaging-config.md`](./packaging-config.md) 为准。
