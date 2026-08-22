# Proposal: harden-single-repo-design-weak-models

## Why

Story 链路的弱模型加固（harden-story-pipeline-weak-models）已完成并实测达标，全局协议层（sentinel nonce / severity 三档 / 滑动窗口 / kimi provider 能力）对 Design 自动生效；但 Design **业务侧从未实测**：全环境没有任何真实 design_spec 产物，reviewer 边界判定只有一句规则文本而无判例，用户自由反馈返修入口缺少全部结构契约，现有测试均为 grep 文案式断言。在「单仓全流程可用」的目标下，Design 段是 story 之后的第一块缺口。

## What Changes

- **Reviewer 边界判例 few-shot**：新增 3 条去 sentinel 封装的轻量对照判例（抽象追踪→最高 suggestion；可执行测试越界→must_fix；风险章节合法提及验证→pass），仅在 Design 的 `build_review_input` 单点注入；配套防照抄与防误伤回归断言。
- **Repair 纵深防御**：`is_repairable` 增加载荷内容指纹判据（示例 ID 组合命中即不可修复，覆盖 JsonNonceMismatch/MissingJsonNonce）；删除恒不可达的 NonceMismatch 死分支；repair prompt 增加 nonce 排除提示且回灌改用剥离 sentinel 后的 readable 文本。
- **用户反馈返修入口补全（仅 Design 分支）**：注入 parser schema、artifact 输出 fence 契约（含当前产物输入 fence 三反引号改四反引号）、Design skeleton、missing context notes；compact_history 暂缓（待 campaign usage 数据决定）；Story/WorkItem 分支字节不变。
- **结构契约回归矩阵**：Design candidate→finding 表驱动负例（单失败原因基准）；Design 多轮滑动窗口 fixture；skeleton 防照抄提示语从 REQ/AC 修正为 DEC/CMP/API + source id；新增单仓确认红线测试（pass 不自动 Completed、确认后才 Confirmed、无 aggregate contract）。
- **Design corpus/golden/campaign**：6 形态冻结语料各配冻结上游 Story Spec fixture；design golden normalizer（DEC/CMP-API 集、dec_req_links、source 覆盖、双章节 decision 抽取，不要求 ID 集合与 golden 完全一致）；强化版 manifest 校验器；baseline 先于 prompt 改造采集，revised campaign 以 12 样本/组合 × 3 provider 组合判定（full-chain 一次成功 gate + 边界分类独立最低门槛：抽象追踪假阳=0、测试越界假阴=0）。

## Capabilities

### New Capabilities

- `design-pipeline-weak-model-hardening`: 单仓 Design 段弱模型可用性——reviewer 边界判例、用户反馈修订入口契约、结构契约回归、corpus/golden/campaign 实测验收。

### Modified Capabilities

- `story-pipeline-weak-model-hardening`: 在既有「few-shot 示例（防照抄）」要求旁追加 reviewer 结构化输出 repair 层的防照抄约束（示例载荷不得经 envelope repair 复活；NonceMismatch 死分支清理）。该约束作用于全部 workspace reviewer（Story 同受益）。

## Impact

- 受影响代码：`src/product/workspace_engine/prompts/review.rs`、`prompts/author_revision.rs`、`prompts.rs`（skeleton 文案）、`src/product/workspace_engine/review/structured_output.rs`（is_repairable）、`src/product/workspace_engine/tests/*`（part_02/part_31/part_32 及新测试文件）、`src/cross_cutting/structured_output.rs`（仅死分支清理与注释级澄清，不改解析行为）。
- 新增验证产物：`cadence/reports/design-weak-model-campaign/`（corpus/golden/normalizer/manifest 校验器/报告）。
- 不改变 Story/WorkItem/WorkItemPlan 的任何 prompt 字节与运行时行为（显式负例锁定）；不改 aggregate 分支。

## Non-goals

1. aggregate Design 全部分支（request-bound nonce 贯穿返修、artifact/metadata 分离回写、raw/fenced fallback 移除、change_order exact-set 校验与确认 gate）——另立项。
2. 共享协议层/severity/滑窗/kimi provider 重写。
3. 关键词 deterministic pre-gate（测试越界判定留在 reviewer 侧，以测试锁定现状）。
4. heading 级别/稳定 ID grammar/fenced code 排除的 validator 收紧（牵涉通用 validator 与 work_item_split 复用及版本边界，另立项）。
5. reviewer 未关闭强 finding 双重重放去重（等 campaign usage 证据后另立共享优化）。
6. Story/WorkItem 用户反馈入口的结构契约补全（跨 workspace 统一另立项）。
7. `build_revision_delta_prompt` 补充 skeleton/历史（resume 设计使然）。

## 范围声明

本 change 仅覆盖 legacy 单仓 Design。验收结论只声称「单仓 Design 链路可用」，不声称全量 Design 链路（含 aggregate）可用。
