# Proposal

> 权威来源：
> - 主方案 `cadence/designs/2026-11-08_技术方案_WIG生成加速与收敛性加固_v1.0.md` §3.4（一致性裁决 X2）、§6.3（两个平台侧确定性缺陷）、§6.6（工作包 G-4 / G-5）、§8.0（待办登记）；
> - 处置裁决 `cadence/designs-reviews/2026-11-09_裁决_WIG四change审阅问题处置_v1.0.md` §2.2（共享面全局串行顺序）、§2.3（命名域与跨 change 归属的唯一性）、§7.2（X2 的 MODIFIED REQ-WSC-06 无 owner → 另立 change 承载）；
> - 实测证据 `cadence/analysis-docs/2026-11-08_WIG实测证据基线_v1.0.md` §2 / §3 / §5 / §6 / §17。
> 本 proposal 只记录 Why / 范围 / 非目标 / 受影响 capability；架构边界在 `design.md`，验收条款在 `specs/`，工作包在 `tasks.md`。

## Why

本 change 承接两个**尚无 change 承载**的遗留缺口。

**缺口一：author 输入面成本（G-4 / G-5）尚无 owner。** 主方案 §6.6 的 G-4 / G-5 在 §8.0 映射表里**没有对应 change**；这些缺陷都在**平台侧**、不在模型侧：

- G-4：聚合根顶层条目按名排序后截断到上限 12（`context.rs:233-234`），点条目占满名额，**三个业务成员仓全部落榜** —— 实测 `structure` 段仅 276 字符、5 个仓名出现 **0 次**，是模型 9 次探索的**直接诱因**。
- G-5：`budget_markdown_context`（`prompts.rs:395`）把上下文按固定比例压进同一预算，作者拿不到平台已确认的成员仓摘要与依赖图事实，只能自行 Read/Glob 补事实 —— 实测首轮 author 产出 markdown 前 **17 次串行工具往返**、探索期 **275.4 秒**。
- 返修轮更贵：`sc_revision.rs:27` 的返修 prompt **逐字节复用首轮完整 prompt**，每轮重付 **32KB 静态规范面**（run1 prompt 实测 **32,609 字符**；author 侧 `cache_read_tokens` 仅 **128**）。
- 另一侧同样有账：reviewer 侧 input **61,051 token**，而**作者的探索结论没有任何形式传递给 reviewer**。

**缺口二：MODIFIED REQ-WSC-06 无 owner。** 一致性裁决 X2（主方案 §3.4）已裁定：方案 E 的 reviewer 投影裁剪**与主规格 REQ-WSC-06 字面冲突**，必须带 MODIFIED delta，**不得静默裁**；「只留同仓冲突」必须显式收窄主规格 `:377` 的**无仓限定措辞**并给理由（跨仓 forbidden/exclusive 在 multi-repo 场景无意义，实测 **29 条写范围冲突中 27 条为跨仓假阳性**）。处置裁决 §7.2 定案：**承载方须另立 change，不得由 isolate 夹带**。

本 change 即承载方。

## What Changes

- **reviewer 投影最小内容重定义（X2）**：只读契约覆盖投影的呈现内容收窄为四类 —— 能力覆盖缺口（gaps-only）、handoff 消费闭环、**同仓**写范围冲突、Projection Validation Report；显式收窄主规格 `:377` 的无仓限定措辞并写明收窄理由，不以「信息仍可从 CCC 推导」代替收窄。
- **author 上下文分级供给（G-5）**：L1 成员仓摘要与 L2 依赖图事实由**平台直接注入 prompt**，不再依赖模型自行 Read/Glob 补事实；L3 维持既有预算约束。
- **成员仓摘要排序修正（G-4）**：`work_item_split_engine/context.rs:233-234` 改为成员仓优先，成员仓不得再被点文件/点目录条目挤掉；单仓摘要口径维持 REQ-PLN-17 的逐字不变义务。
- **返修 prompt 不复用首轮全文**：返修轮只带**本轮 delta + 与本轮相关的既有上下文**，MUST NOT 逐字节重发首轮完整 prompt；本轮 findings 回灌与逐条硬性修复指令保持注入，findings 的 `required_action` 所指向的 section 与其依赖上下文在返修轮仍须可得。
- **规格落点**：以上四项写成一个 **MODIFIED REQ-WSC-06**（`work-item-plan-single-candidate`）：主规格该 requirement 的**原有场景标题与正文逐字保留**，只追加场景 + 改写描述段。

## Non-Goals（本 change 明确不做）

- 不做 G-2（MCP 输出截断）/ G-3（Bash 输出截断）/ G-6（缓存 TTL 1h）—— 这三项是 **SC author 出口的 env 层**，由 `explicit-sc-author-effort` 承载该出口的 env 构造机制；本 change 只在 `design.md` **登记**相邻关系，**不夹带**。
- 不改 effort 档位（G-1）—— 裁决 §3 已定案为默认 `high`，由 `explicit-sc-author-effort` 承担。
- 不改 canonical 校验器的 fail-closed 语义、不删平台侧 source/IR/机械报告的绑定与同代校验 —— 投影裁剪只裁**呈现面**。
- 不改 legacy/story/design reviewer 路径的输入面。
- 不 bump 任何预算常量（author prompt 质量预算 23,500 B、`WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES` 等）；新增直供内容须在既有预算内成立。
- 不触碰 `isolate-plan-criterion-namespace` 的命名域与指纹代际面，不触碰 `close-contract-findings-before-pass` 的有效必关闭判定面。
- 本轮**只出规划产物**，不改 `src/**`。

## Capabilities

### New Capabilities

- 无。本 change 不引入新能力路径，全部落在既有 capability 的 delta 上。

### Modified Capabilities

- `work-item-plan-single-candidate`：**MODIFIED** `REQ-WSC-06`（prompt 分层职责）—— 承载 reviewer 投影最小内容重定义（X2）、author 上下文分级供给（G-5）、成员仓摘要排序修正（G-4）与返修 prompt 不复用首轮全文。

> 编号与归属说明：本 change 是 **REQ-WSC-06 的唯一 owner**（此前无 owner，见处置裁决 §2.3 与 §7.2）。本 change **只 MODIFIED REQ-WSC-06 一条 requirement**，与其它 active change 无同一 requirement 被双 MODIFIED。

## Impact

- **author 输入面**：`work_item_split_engine/context.rs`（成员仓摘要排序）、`work_item_split_engine/prompts.rs`（`budget_markdown_context` 签名与调用点）、`work_item_split_engine/prompts/sc_revision.rs`（返修 prompt 构造）。
- **reviewer 输入面**：`workspace_engine/prompts/review_parts/single_candidate_input.inc.rs`（投影与供给/需求段的呈现面）、`workspace_engine/prompts/review.rs`（单候选 reviewer prompt 装配）。
- **规格面**：`openspec/specs/work-item-plan-single-candidate/spec.md` 的 REQ-WSC-06（归档后生效）；`REQ-PLN-17` 的单仓逐字不变义务**保持不变**、不被本 change 修改。
- **共享文件与串行次序**：见 `design.md`「共享面与排序」—— 本 change 排在全局序 **7（最后）**；与 `isolate-plan-criterion-namespace` 共享 `work_item_split_engine/prompts.rs`（isolate `:185,:306` 的 WIC 命名纪律 ↔ 本 change `:395` 的上下文供给），次序 **isolate → 本 change**；与 `mechanicalize-plan-compile-repair` 共享 `sc_author_teaching.rs` 与 `prompt_contract*.rs` fixtures，次序 **mechanicalize → isolate → 本 change**；与 `close-contract-findings-before-pass` 共享 `prompts/review.rs` 与 `sc_revision.rs`（close `:147` 的呈现口径 ↔ 本 change `:27` 的 prompt 复用），次序 **close → 本 change**。
- **不在本 change 范围内**：G-2 / G-3 / G-6（SC author 出口 env 层）、G-1（effort 档位）、教学窗口与机械处置面（`bound-sc-generation-teaching` / `mechanicalize-plan-compile-repair`）、契约类 finding 免阻断闭环（`close-contract-findings-before-pass`）。
