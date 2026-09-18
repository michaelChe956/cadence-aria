# Proposal: close-provider-validation

## Why

阶段 4 立项三方决议（`.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/stage4-tripartite-decision.md`，Q3/Q5，零分歧共识）将 provider 批切为本 change（change ①，落地序 ①→③→② 的首环）。四项事实依据：

1. **provider 面悬置（M4）**：kimi coding 验证轮从未实跑——`add-kimi-code-provider` change active 未归档，tasks 勾选严重失真：账面仅 5.2/5.3 勾选，实做面已覆盖 render/registry/config 与前端 catalog 大部（ter 实读核验：`work_item_projection/render/kimi_code.rs` 已存在、`coding_models/provider_config.rs` 无 Auto-only 强制、四家 `register_gated` 全量注册）。「代码已落+验证未跑」的悬置状态正是阶段 4 题内之义。
2. **claude headless AskUserQuestion 修复的账实二次失真**：既有提案（`fix-claude-code-ask-user-question-headless`，active）在立项输入包中被判「提案全就绪未实施」；本 change 起草时实读核验发现三提交已于 08-21 落地（`3164f6a7` 结果所有权归 control_request、`e367a84e` 权限模式映射 default、`20493b65` stdio 回调注册，测试名与提案 tasks 逐项对应，三提交均在 HEAD 祖先）。但该 change 仍 active、tasks 全未勾选、提案 4.1/4.2 整体验证（全量门禁+真实 CLI smoke）未留档——提案与代码事实需要一次吸收+验证收尾闭环。
3. **kimi terminal flaky 家族（RR-3①，DEF-7 桶）**：`output_is_capped_and_truncation_flagged_once`/`output_exactly_at_cap_is_not_truncated`/`concurrent_streams_share_budget_without_overdraw` 三测试为计时预算类（真实进程泵 1-2MB 输出+10-15s 墙钟预算），隔离过滤跑也偶发（阶段 3 收官 defer-ledger RR-3① 登记在案）。决议将根治列入本 change。
4. **两 active change 归档收尾**：add-kimi-code-provider（核账：实做大于账面，需补勾+证据）+add-pi-provider（遗留 2.2 aria-ask.ts 结构化提问扩展、2.3 版本范围检测待处置）；归档动作唯一 owner 待确认（决议风险登记）。

## What Changes

（四项，决议钉死，不加不减）

1. **claude headless 修复验证收尾（吸收既有提案）**：fix-claude-code-ask-user-question-headless 提案契约并入本 change（capability `claude-code-structured-interaction`：`--permission-prompt-tool=stdio` 始终注册+Auto/Supervised 双模式映射 `default`+AskUserQuestion 结果所有权归 control_request）；对已落地三提交补验——全量门禁留档+真实 claude CLI smoke（tool_use→control_request→control_response→tool_result 事件序列作为新鲜证据）；旧 change 文件夹按「被吸收」处置。
2. **kimi coding 验证轮**：provider 批第一个执行流——ACP 方言核对前置（现行 kimi 0.38.0 vs 冻结 0.34.0 fixtures）→真实 CLI 跑 coding 三角色（Coder/Code Reviewer/Internal Reviewer；与 claude headless 修复验证共享 campaign 基建）→修暴露问题→结论落盘。**结论只 gate kimi 自身状态流转（待验证→转正 or 受限登记），不作为退役/多仓或其他任何 change 的门禁**（REQ-WSC-07 判据原文仅 codex+pi）。
3. **kimi terminal flaky 根治（DEF-7 桶）**：三测试确定性根治——byte-exact cap 断言脱离真实进程计时预算（受控内存流确定性锁定）；真实进程管道行为收敛为一条 smoke；RR-3① 销账。cap 行为语义（上限值、恰为上限不截断、超限截断只标记一次、并发流共享预算不超额）不变，只改验证方式。
4. **两 active change 归档收尾**：add-kimi tasks 勾选与代码事实对齐（实做大于账面=补勾+证据；双向核账防「勾而未落」）；add-pi 遗留 2.2/2.3 显式 defer 登记（不实施，如实标注限制）；归档动作唯一 owner 确认后执行；delta 同步主 specs。

## 非目标（明确不做）

- 不做多仓 coding、旧协议退役、DEF-4 契约显式化（change ②③ 范围）；coding_run_campaign 未测区顺带核属 change ③ 门重测 WP，不在本 change。
- kimi 验证轮结论不作为其他任何 change 的门禁、输入前提或排序依据（Q3 口径贯穿本 change 全部产物与文案）。
- 不实施 pi 2.2（aria-ask.ts 结构化提问扩展）/2.3（版本范围检测）——显式 defer+如实登记；pi 结构化提问维持文本暂停信号现状。
- 不新增 provider；不改 ApprovalBridge 决策逻辑、text_fallback 二级兜底、task-run 四入口拒绝边界、Kimi ACP 协议行为与 `kimi-acp-client-services` 既有安全/沙箱/grammar 约束。
- 不改 claude/kimi 对外 wire 契约与产品行为（claude 面为已落地行为的补验；kimi 面为验证与测试确定性改造）。
- flaky 根治不采用「放宽断言/加大超时预算/标记 ignore」类症状压制手段。

## Capabilities

### New Capabilities

- `claude-code-structured-interaction`（吸收自 fix-claude-code-ask-user-question-headless 提案）：claude code headless 模式下结构化提问（AskUserQuestion）与权限回调（permission prompt tool）的注册、模式映射与结果所有权协议。
- `provider-validation-round`：provider 批验证轮与归档收尾契约——真实链验证、方言核对前置、结论 gate 边界（仅 kimi 自身状态流转）、kimi 状态流转二值（转正/受限登记）、归档核账纪律。

### Modified Capabilities

- `kimi-acp-client-services`：「终端生命周期与资源边界」requirement 增补确定性验证条款——cap 边界行为 SHALL 可在无真实计时预算依赖下被确定性锁定（DEF-7 桶根治的契约化）。

## Impact

- **代码**：`src/cross_cutting/kimi_code_provider/client_services/terminal.rs`（三测试族确定性改造+smoke 收敛）；kimi 验证轮暴露问题的修复点（届时按实跑定位）；claude 面预计零代码改动（已落地，仅补验——实跑暴露真实回归才修，修复不超出提案语义）。
- **测试**：terminal.rs 测试重写（确定性断言+单条真实进程 smoke）；kimi/claude 真实 CLI 验证证据留档（campaign 基建）。
- **openspec**：三个 change 文件夹处置——add-kimi-code-provider/add-pi-provider 归档、fix-claude-code-ask-user-question-headless 按被吸收处置；主 specs 同步（kimi-code-provider-integration/pi-provider-integration/claude-code-structured-interaction 入主，kimi-acp-client-services 增补条款）。
- **对外接口/wire**：零变化。无部署迁移、无数据迁移。
