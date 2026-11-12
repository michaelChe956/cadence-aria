# Proposal：只装部分 provider 时 LC 仓聚合初始化必须走得通，且失败必须可行动

## Why

用户只装了两家 provider（Pi、Kimi Code）时，LC 仓聚合初始化在第一步 `pre_check` 失败，
界面只给出两句无信息的话：「provider 的第一步（pre_check）没有完成」与
`root recipe turn produced no workspace change (provider=pi, step=pre_check, zero_change=true)`。

平台侧的事实是：

- **provider 选择门没有问题**——候选顺序固定 `[Pi, ClaudeCode, Codex]`
  （`provider_selection.rs:44-48`），未安装的 Claude Code / Codex 已被判 `cli_unavailable`
  并跳过（`provider_selection.rs:302-315`），不阻断执行；「只装一家也能跑」的能力
  已在 `2026-11-11-allow-single-provider-aggregate-initialization` 落地。
- **失败发生在 provider turn 内部**：Pi 的 `pre_check` turn 结束时整个聚合根零字节变化，
  被 REQ-BOOT-17 的零变更判据判为失败（`root_recipe_receipt_store.inc.rs:122-143`）。
  审计白名单覆盖 `.claude` / `.agents` / `.pi` / `.kimi-code` / `openspec` / `cadence`
  等 Pi 应落盘面（`root_recipe_receipt_types.inc.rs:18-35`）⇒ 不是审计漏判，是这次 turn 真的没留下变更。
- **失败面把根因丢了**：provider turn 的真实输出（Cadence-skills 脚本的 JSON 报告、
  退出码、失败原文）在失败路径上被整段丢弃（`coordinator_provider_turn.inc.rs:222-300`
  收集了输出，只有成功路径写进 step 产物），durable 诊断里只剩
  `root_recipe_turn_zero_change` 一句 ⇒ 用户无从判断是「命令没被识别」「命令跑了但失败」
  还是「命令按设计幂等、无变更」。
- **聚合索引缺失是连带结果**：索引构建只在 `execute` 返回 `Ok` 后才 spawn
  （`handlers.inc.rs:222`），所以「聚合索引不可用 / evidence_index_unavailable」
  不是独立故障。

## What Changes

取证结论（2026-11-12，podman 容器内只装 Pi 的真实 E2E，见 `cadence/plans/` 对应计划文档）：

- provider 选择门**不是**问题：容器里同样选出 Pi，Claude Code / Codex 标 `cli_unavailable` 跳过，
  与用户界面完全一致。
- `pre_check` 步骤**真跑了**：Pi 正确展开 skill、正确把 `用大陆镜像` 翻译成 `--mirror cn`，
  并执行了 `pre-check.sh run --mirror cn --no-interrupt --upgrade`。
- 脚本返回 `"overall": "failed"`：某个 phase 失败（容器现场是 base-tools phase 的 `uvx`
  装不上——脚本用 `pip install uv` 装 uv，容器无 pip）⇒ `--no-interrupt` 立即中止
  ⇒ **聚合根零落盘** ⇒ 平台判零变更失败。
- 平台把脚本那份 JSON 报告**整段丢弃** ⇒ 用户只看到 `zero_change=true`，看不到「哪个 phase
  失败、失败原文是什么」。

因此本 change 的具体条目：

- **失败诊断必须带上 provider 的真实输出**：provider turn 失败时，把 turn 输出（尾部优先、
  截断、脱敏）写进 durable 错误记录的诊断字段；零变更失败不再只有一句 `zero_change=true`。
- **超长 turn 输出必须保留收尾**：输出预算超限时保留头尾（脚本报告在结尾）。
- （待定，取决于二次取证）**零变更判据是否要区分「没做」与「做了但无变更」**：Cadence-skills
  四条命令的契约就是幂等（重跑报告 `skipped` / `all-skipped`、产物无变化），若二次取证证实
  「已初始化根上重新初始化」必然命中零变更，则需要把「命令确实执行且报告成功 + 工作区无变更」
  与「命令根本没被执行」分成两种判定与两种失败面。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `non-interrupt-repository-bootstrap`: 零变更失败判据（REQ-BOOT-17）与 provider turn
  失败诊断的事实内容——零变更判据要能区分「命令未执行」与「命令已执行且按契约幂等无变更」，
  并要求失败诊断保留 provider 的真实输出。

## Impact

- 后端：`src/product/logical_codebase/`（零变更判据、provider turn 输出收集）、
  `src/web/handlers/aggregate_initialization/`（审计窗口收口与诊断产出）、
  `src/product/repository_store/types.rs`（若命令参数面需要调整）。
- 前端：失败卡片的诊断展示（诊断文本变长，保持折叠）。
- 契约：`openspec/specs/non-interrupt-repository-bootstrap/spec.md`（REQ-BOOT-17）需 delta。
- 测试：podman 容器内「只装 Pi」的 E2E（容器资产不入库，仅本地脚本）。
