# Proposal: relax-legacy-rule-read-gate-for-generation

## Why

单仓（Legacy）模式下，所有生成类 prompt（Work Item Group outline、Work Item Draft、plan/revision 等）通过 `direct_cadence_routing_rules_reference` 注入 `cadence_rule_read_gate`：要求 Provider 在生成前用原生工具**完整读取**目标仓库 AGENTS.md 与 CLAUDE.md，且任一文件/工具不可用时**只报告阻塞、禁止输出任何 artifact/JSON**。

实测问题（naruto 测试项目 + 弱模型 provider：kimi+deepseek-v4-flash、claude code+glm-5.3）：

1. 目标项目规则文件合计 16KB+，每次结构化生成都被强制全文读取，挤占本已紧张的上下文与注意力预算，加剧 outline/draft 的结构化生成失败；
2. 失败关闭姿态（"不可用即阻塞、禁止输出 JSON"）本身会把结构化生成推向 context blocker，与生成可靠性目标冲突；
3. 路由/流程规则对"生成候选 artifact"这一动作本身没有语义贡献——规则约束的是编码行为（构建命令、格式、TDD 纪律），这些在 coding 阶段已被注入且保留。

## What Changes

- Legacy（单仓）上下文中，**生成类 prompt**（outline、draft、plan、revision、review 生成侧）的规则引用从"强制完整读取 + 失败关闭"降级为轻量提示：声明规则文件位置，按需查阅（不要求全读、不作为输出前置条件、不可用不阻塞生成）。
- Legacy 上下文中 **coding 阶段 prompt 保持现状**（编码需要真实规则：构建/测试命令、格式规范）。
- Logical（多仓聚合政策）上下文**完全不变**：政策权威加载与"未加载即阻塞"由 `project-rule-aware-prompts` 既有 Requirement 保护，本变更不触碰。

## Capabilities

### Modified Capabilities
- `project-rule-aware-prompts`: 新增单仓（Legacy）生成类 prompt 的规则引用要求——按需查阅、不以读取为输出前置、不因规则文件不可用阻塞生成。

## Impact

- 代码：`src/product/cadence_skills/routing_reference.rs`（`LEGACY_REFERENCE` 文案与按调用场景分流）、`src/product/work_item_split_engine/prompts.rs`、`src/product/workspace_engine/prompts.rs`、`src/product/workspace_engine/prompts/revision.rs`、`src/product/workspace_engine/draft_batch/runs.rs`、`src/web/workspace_context/prompts.rs`、`src/cross_cutting/provider_context_builder.rs` 中的注入点；`src/product/coding_workspace_engine/prompts.rs` 不变。
- 测试：routing_reference 单测、各 prompt 契约测试（parser_prompt 系列、routing_reference_contract）需同步更新。
- 非目标：不修改 Logical 多仓政策门禁；不修改 coding 阶段规则读取；不修改 Cadence-skills 侧模板（另有任务）；不重构 work item group 链路（后续独立变更）。
