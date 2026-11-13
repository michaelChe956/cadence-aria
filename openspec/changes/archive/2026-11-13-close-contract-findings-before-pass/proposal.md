# Proposal
## ⚠️ 处置声明（归档前落档 · 2026-11-13）

本 change 以 `--skip-specs` 归档：**其 spec delta（MODIFIED REQ-WSC-17、ADDED REQ-TOP-07）不合并进主规格**（行为未实施）。

| 要素 | 内容 |
|---|---|
| **事实** | 单候选 WorkItemPlan 评审的**唯一「有效必关闭」谓词** `requires_closure`（D1/D2，新建 `review/closure_policy.rs`）+ 七个消费面同口径（verdict 重开 / owner 再归一 / 必关闭清单 / policy 分类 / 非 advisory 指纹 / 返修轮作者呈现 / 影响闭合）+ D4 两级判定 |
| **现状** | **未实施**（0/29 工作包）。主 Plan 明确依赖 isolate 的版本化 scope 传递（`Plan:24`）；其 D9 增补 Plan（`WIG闭合资格反向投影增补`，5 Task / 23 Step）**硬依赖**本 change §1–§6 先行落地（`D9 Plan:18`）⇒ 二者均未开工 |
| **归谁** | 集成 owner |
| **建议落点** | **D9 的意图由 `repair-wig-fact-projections` 的 B1–B4 以窄口径承接**（落在 **REQ-WSC-12** 的**显式例外**：SC 会话有效投影下，「`severity=Suggestion` + category 明确非契约 + `class_hint=Repairable`」的 finding 不进必关闭清单、不驱返修）。**全口径七面闭合、许可两级分类、契约类 suggestion 重开**留待后续 change，届时**不得**把今天的窄口径投影当作已完成的全口径 |
| **边界** | **D1–D8 有意重开被误标为建议的真实契约缺口 ⇒ 可能增加人工门**（更严闭合的另一面）。⇒ **不得**把其收益写成「减少人工门」或「提高自动完成率」；今天的窄例外亦**不**放宽契约类、缺 category、must_fix/blocking、机械 Error 与原生 NeedsHuman |


## Why

单候选 WorkItemPlan 评审中，模型把一条真实的 `contract_gap` finding 标成 `severity=suggestion` + `class_hint=advisory`，平台随即把它判为 advisory、评审结论保持 `pass`，该 finding **从未进入必关闭清单**——实测基线里 reviewer 精确抓到了 author 上一轮重写引入的契约缺陷（`input_contracts` 条目丢失），平台却放行。

主规格 REQ-WSC-12/REQ-WSC-17 早已明文「`contract_gap` / `self_contradiction` MUST NOT 仅凭模型标 `advisory` 获得免阻断」且「必须进必关闭清单」，但平台当前**消费面口径不一致**：平台硬化只改 `disposition`，verdict 重开、必关闭清单、owner 再归一、policy 分类、返修轮作者呈现、返修影响闭合六面各用各的门槛（多数仍看 `severity ∈ {blocking, must_fix}`），非 advisory 指纹集合又派生自旧分类结论，结果是「模型自觉」才生效，明文 MUST 未兑现。

owner 已裁决（决策四）：本期把契约类 finding 的**有效必关闭**语义收敛为多面同口径的单一谓词，**保留模型原 severity 与内容身份**，不改 owner 的 finding key，也**不因此增加自动重写次数**。

## What Changes

- **新增唯一「有效必关闭（effective closure）」谓词**，作为契约类 finding 是否进入必关闭集合、是否重开 `pass`、是否参与非 advisory 指纹/返修预算、是否被 policy 判为可自动返修的**唯一权威口径**；verdict 判定、必关闭清单、owner 再归一、policy 分类、非 advisory 指纹、返修轮作者呈现、返修影响闭合**七个消费面**全部改为消费该谓词。
- **保留原 severity 与内容身份**：`severity=suggestion` 不被改写为 `must_fix`，`finding_key`（`sha256(severity + \u{1} + message)` 前 16 位小写 hex）逐字不变，owner 既有排除项的身份与幂等语义不受影响。
- **模型的 `severity=suggestion`、`class_hint=advisory`、`verdict=pass` 均不得使契约类 finding 自动放行**；`class_hint=advisory` 在有效必关闭语义下不再盖过平台判定。
- **两级判定（闭合资格 / 自动修复许可分离）**：`requires_closure`（闭合资格）为真但 `auto_repair_permitted`（自动修复许可）为假——缺可验证定位**或**缺服务端授权的既有合法机械修复许可，任一缺失——时落 `HumanRequired` 转既有人工处理通道，不得 `Advisory`、不得 `Repairable`；`category`、`severity`、`class_hint` 均不构成许可或定位证据。MUST NOT 因此增加自动 author 调用次数。
- **作用域受限且有取值表**：新增例外**只作用于 WorkItemPlan 的既有契约类 finding**（`contract_gap` / `self_contradiction`），「本 Work Item」＝ 该 finding 的 `target_work_item_id` 指向的 Work Item；`WorkItemPlanReviewScope::{Outline,Item,Batch}` 与非 SingleCandidate 的 legacy flow 逐项写明是否启用（见 design「WorkItemPlan 作用域取值表」）。作用域由可信 session / 解析入口传入，**不由模型标签授予**；平台 disposition 硬化已在通用 strict/lenient 解析器无条件生效，**只有 severity 豁免是 WorkItemPlan 专属**。
- **strict / lenient / historical 三条入口同一规则**：不只修 live 路径；historical 入口分重新解析型与纯 serde 反序列化型两类各自落地；历史展示路径重新归一，但**不自动启动 provider、不回滚已确认或已发布的历史产物**。
- **审计与排除**：原始模型文本保留用于审计，但 policy MUST NOT 重新解析 raw 后绕过 owner 排除，历史重建路径同样禁止。
- **owner 显式排除优先**：排除判定先于有效必关闭计算，且排除后不得再次平台硬化把排除项复活。
- **范围外 advisory 规则不变**：不放大普通 advisory；四条范围外/完备度意见仍不扣预算、不阻断。
- **不做**：不把所有 `suggestion` 升为 `must_fix`（会改 owner key）；不只放宽 verdict 函数（其余各面仍会漏）；不推迟到二期（已是明文 MUST 未兑现）；不削弱规格承认 `pass`（把真实缺口放行制度化）。

## Capabilities

### New Capabilities

- 无。

### Modified Capabilities

- `work-item-plan-single-candidate`: MODIFIED REQ-WSC-17（返修触发路径只认 actionable）——补入「未被 owner 显式排除的契约类 finding MUST 进入必关闭集合；模型 severity/class_hint/verdict 均不得使其自动放行；保留原 severity 与内容身份，以有效必关闭语义归一路由；两级判定下缺可验证定位或修复许可时转既有人工处理且不得自动调用 author；owner 排除优先，范围外 advisory 规则不变」，并补齐七面同口径、两级判定落点、raw pass 未重开路径、作用域取值与入口一致的可验收场景。
- `work-item-typed-outcome-policy`: ADDED REQ-TOP-07（契约类 finding 的有效必关闭投影与 policy 分类口径）——规定有效必关闭谓词在 typed outcome policy 中的投影：`class_hint=advisory` 与 raw `pass` fallback（raw verdict 指未经有效必关闭重开的原始判定）不得把有效必关闭的契约类 finding 判为 `Advisory`；两级判定落点为 `Repairable`（沿既有许可）/ `HumanRequired` / 维持既有行为；许可证据不得凭 `category`、`severity` 或 `class_hint` 构成；policy 不得重新解析 raw 绕过 owner 排除；**不提高 A/B 余额**、**不改变 Verification 新 finding 既有停止规则**。

## 非目标

- 不改任何 `suggestion` 的通用门槛，不把普通 advisory 放大成阻断项。
- 不改 finding 身份算法，不做身份迁移，不改 owner 排除的 key 语义。
- 不新增 wire 字段、不扩公共快照、不扩 UI 配置面。
- 不提高 `max_repairs` / `max_manual_repairs` 余额，不改返修预算归属。
- 不改 Verification scope 新 finding 的既有停止规则，不改复评只验指纹的既有边界。
- 不改 SG-02A 内容自修授权边界，不在本期扩大系统内容修改权。
- 不回滚、不重跑、不自动撤销已确认或已发布的历史产物。

## Impact

- **受影响 capability**：`work-item-plan-single-candidate`、`work-item-typed-outcome-policy`（均以 delta 形式，不改主 specs 文件）。
- **受影响代码面**：单候选评审的 verdict 解析与归一、必关闭清单渲染、owner 处置再归一、policy 分类与预算裁决、评审路由与重复指纹判定、返修轮作者呈现、返修影响闭合、历史评审展示归一。集中在 `src/web/workspace_ws_types/review.rs`、`src/product/workspace_engine/parsers.rs`、`src/product/workspace_engine/review/`（含 `disposition_override.rs`、`routing.rs`、`routing_scope.rs`、`routing_platform_rescue.rs`、`structured_output.rs`、新建 `closure_policy.rs`）、`src/product/workspace_engine/prompts/`（`history_compaction.rs`、`review.rs`）、`src/product/work_item_plan_policy/classify.rs`、`src/product/workspace_engine/session_state_parts/historical_review.inc.rs`、`src/product/work_item_split_engine/prompts/sc_revision.rs`、`src/product/workspace_engine/plan_outline/revision.rs`。精确文件、命令、测试与提交步骤由 `writing-plans` 写入 `cadence/plans/`，不在本 change 内规定。
- **数据契约**：无新增持久化字段；durable 已处置事实清单与 finding key 格式不变。
- **共享面**：与 `isolate-plan-criterion-namespace`（`work_item_plan_policy/classify.rs`，isolate → close，本 change 消费其版本化 scope）、`mechanicalize-plan-compile-repair`（`review/routing*.rs` 与新建 `review/closure_policy.rs`，mechanicalize → close）、`fix-author-rewrite-quota-pool`（`review/routing.rs`、`review/routing_platform_rescue.rs` 预算域，quota-pool → close）存在文件级重叠；次序按全局串行表**双边**登记，对方目录登记由集成 owner 统一补齐。串行归属见 `design.md` 的「共享面与排序」。
- **无对外 API / 协议破坏性变更**：判定语义收紧属行为修正，不改 wire 形状。
