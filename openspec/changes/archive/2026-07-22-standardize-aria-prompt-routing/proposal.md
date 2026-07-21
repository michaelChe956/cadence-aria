## Why

Aria 的提示词分散在 Story/Design/Work Item Workspace、Coding Workspace 和 Runtime Unit 中。部分入口已经包含 OpenSpec 或 Superpowers 约束，其他入口缺少与 Cadence 原始路由规则一致的要求，且续跑或格式修复场景不能安全地重复触发路由。这会导致同一流程在不同角色和节点上对 Skill、审批 gate、OpenSpec 契约与完成证据的遵守不一致。

现在需要在不重建流程系统、不替代既有规则、不破坏结构化输出或恢复语义的前提下，让所有 Aria agent 节点直接使用并遵守 Cadence-skills 的 `agent-routing-kernel.md` 与 `openspec-superpowers-workflow.md`。

## What Changes

- 在现有 Aria prompt builder 中直接接入指定的 Cadence 路由规则，按节点真实阶段声明必调 Skill、OpenSpec/Plan 前置条件和人工 gate。
- 保留既有 `[openspec_contract]`、`[superpowers_contract]`、追踪关系、候选产物权限、结构化输出和 Provider 恢复契约；仅定向补强缺失或过于笼统的规则提示。
- 区分新任务、恢复任务与同会话格式修复：新建或恢复任务必须重新路由；仅修复 JSON、sentinel 或 artifact 格式的 follow-up 不得重复路由或要求新的首段回执。
- 覆盖 Story Spec、Design Spec、Work Item/WorkItemPlan、Coding、Tester、Code Review、组级 PR Review、返修、集成验证、最终审查与 Runtime Unit 节点。
- 为规则接入补充 prompt 回归测试，验证正确阶段被注入、既有契约仍然保留、格式修复不受影响，并禁止依赖 `cadence-workflow`。

## Capabilities

### New Capabilities

- `aria-prompt-rule-adherence`: Aria 所有 agent prompt 按实际节点阶段直接使用 Cadence OpenSpec 与 Superpowers 路由规则，并保留既有运行时契约的能力。

### Modified Capabilities

- 无。仓库当前没有已发布的 OpenSpec capability spec。

## Impact

- 受影响的主要代码：`src/product/workspace_engine`、`src/product/work_item_split_engine`、`src/product/coding_workspace_engine`、`src/product/coding_evaluation_context` 与 `src/runtime_units/prompt_template_registry.rs`。
- 不修改数据库、HTTP/WebSocket API、Provider 协议、OpenSpec artifact schema 或既有审批状态机。
- 影响 Provider 收到的提示词文本及其对应 Rust prompt 测试；目标项目仍以 Cadence-skills 的原始规则文件为唯一流程权威。
