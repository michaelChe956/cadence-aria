# Proposal

## Why

owner 2026-10-30 真机全流程现场：多仓（LC 聚合根）plan 会话中 WI-004 的 `write_policy.exclusive_scopes` 声明 `scripts/dev.sh`，该路径位于聚合根本地文件区（根目录不是 git 仓库，仅 busi/gateway/frontend 各自入库）——任何成员仓的执行期写入都到不了这个路径。该缺口目前只能靠 reviewer 人工发现（needs_human 结论 owner 读不懂），机械校验层完全缺位；coder 执行期也只在 final_confirm 写范围门代偿性失败，返修成本最高。

## What Changes

- 单候选机械校验（`work_item_plan_compiler/validate.rs`）新增「写范围 × 被授权仓归属」核对：多仓会话下逐 WI 判断 `exclusive_scopes` 的固定路径锚点是否落在该 WI 被授权成员仓（IR `target_repository_id`）的 checkout 目录内。
- 违规产出稳定错误码 `write_scope_outside_bound_repository` 的 Error finding（preflight 族，经既有机械 ReviewVerdict 回灌修订，不硬失败 author 轮）：聚合根非成员区与其他成员 checkout 两类归属事实在消息中分别点名；跨成员写按既有授权语义（每 WI 恰落一被授权成员仓）判越界。
- 只判归属不判存在：被授权 checkout 内的新建路径（基线缺失）不拦；无固定路径锚点的 scope（`*` 等）与空 exclusive 列表不触发。
- `PlanCandidateValidationContext` 扩「成员 checkout 前缀表」（物理成员仓 ID → checkout 相对聚合根目录）；前缀推导与 gates 的 `scope_member_prefix` 同源（manifest `provider_context_root` + 成员 Main checkout `canonical_path`）；路径域锚点提取复用平台既有 scope 匹配权威（`cross_cutting/worktree`，含顿号方言与路径归一）。
- fail-closed 口径：前缀表缺席（单仓/Manual）＝不触发（零回归）；表装载失败＝装载点显式错误（既有惯例，不以 None 吞错）；表在场而被授权仓不在表内＝Error finding 说明理由。
- 三条生产校验路径（author 权威落盘 / 门内人工修订 / 运行期预校验）同源接入。

## Capabilities

### Modified Capabilities

- `work-item-plan-single-candidate`：ADDED REQ-WSC-10——写范围 × 被授权仓归属机械核对（含非成员区/跨成员/新建不误伤/零回归/映射缺失 fail-closed/preflight 修复动作六个场景）。

## Impact

- `src/cross_cutting/worktree.rs`：新增 `scope_fixed_path_anchors`（scope 固定锚点提取，与 `ScopePattern`/顿号方言同源）。
- `src/product/work_item_plan_compiler/`：`types.rs`（上下文字段 + 两个共享常量 + preflight 码表）、新模块 `write_scope_bound.rs`、`validate.rs` 接线、测试。
- `src/product/workspace_engine/draft_batch/compile_support.rs`：新增 `plan_member_checkout_prefixes` 装载器。
- `src/product/workspace_engine/single_candidate.rs`、`conversational_gate_parts/revision_turn.inc.rs`、`src/web/workspace_ws_handler/run/single_candidate.rs`：三处上下文构造接入；`plan_preflight.rs` required_action 映射。
- 不改 `work_item_contract/validation.rs`（既有意图合同语义零变化）；不碰 `workspace_engine/prompts/review.rs` 与 `web/src/**`（并行任务 A 的写面）。
