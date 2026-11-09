# Proposal

## Why

单候选 WorkItemPlan 评审中，模型把一条真实的 `contract_gap` finding 标成 `severity=suggestion` + `class_hint=advisory`，平台随即把它判为 advisory、评审结论保持 `pass`，该 finding **从未进入必关闭清单**——实测基线里 reviewer 精确抓到了 author 上一轮重写引入的契约缺陷（`input_contracts` 条目丢失），平台却放行。

主规格 REQ-WSC-12/REQ-WSC-17 早已明文「`contract_gap` / `self_contradiction` MUST NOT 仅凭模型标 `advisory` 获得免阻断」且「必须进必关闭清单」，但平台当前**四面口径不一致**：平台硬化只改 `disposition`，verdict 重开、必关闭清单、owner 再归一、policy 分类各用各的门槛（多数仍看 `severity ∈ {blocking, must_fix}`），结果是「模型自觉」才生效，明文 MUST 未兑现。

owner 已裁决（决策四）：本期把契约类 finding 的**有效必关闭**语义收敛为四面同口径的单一谓词，**保留模型原 severity 与内容身份**，不改 owner 的 finding key，也**不因此增加自动重写次数**。

## What Changes

- **新增唯一「有效必关闭（effective closure）」谓词**，作为契约类 finding 是否进入必关闭集合、是否重开 `pass`、是否参与非 advisory 指纹/返修预算、是否被 policy 判为可自动返修的**唯一权威口径**；verdict 判定、必关闭清单、owner 再归一、policy 分类四个消费面全部改为消费该谓词。
- **保留原 severity 与内容身份**：`severity=suggestion` 不被改写为 `must_fix`，`finding_key`（`sha256(severity + \u{1} + message)` 前 16 位小写 hex）逐字不变，owner 既有排除项的身份与幂等语义不受影响。
- **模型的 `severity=suggestion`、`class_hint=advisory`、`verdict=pass` 均不得使契约类 finding 自动放行**；`class_hint=advisory` 在有效必关闭语义下不再盖过平台判定。
- **作用域受限**：新增例外**只作用于 WorkItemPlan 的既有契约类 finding**（`contract_gap` / `self_contradiction`），通用 Story/Design 评审的旧门槛不扩大；作用域由可信 session / 解析入口传入，**不由模型标签授予**。
- **strict / lenient / historical 三条入口同一规则**：不只修 live 路径；历史展示路径重新归一，但**不自动启动 provider、不回滚已确认或已发布的历史产物**。
- **缺可验证定位或修复许可时转既有人工处理**：契约类 finding 没有可验证定位（无 `contract_field`）时不得被自动判为可返修，MUST NOT 因此增加自动 author 调用次数；转既有人工处理通道。
- **owner 显式排除优先**：排除判定先于有效必关闭计算，且排除后不得再次平台硬化把排除项复活。
- **范围外 advisory 规则不变**：不放大普通 advisory；四条范围外/完备度意见仍不扣预算、不阻断。
- **不做**：不把所有 `suggestion` 升为 `must_fix`（会改 owner key）；不只放宽 verdict 函数（其余三面仍会漏）；不推迟到二期（已是明文 MUST 未兑现）；不削弱规格承认 `pass`（把真实缺口放行制度化）。

## Capabilities

### New Capabilities

- 无。

### Modified Capabilities

- `work-item-plan-single-candidate`: MODIFIED REQ-WSC-17（返修触发路径只认 actionable）——补入「未被 owner 显式排除的契约类 finding MUST 进入必关闭集合；模型 severity/class_hint/verdict 均不得使其自动放行；保留原 severity 与内容身份，以有效必关闭语义归一路由；缺可验证定位/修复许可时转既有人工处理且不得自动调用 author；owner 排除优先，范围外 advisory 规则不变」，并补齐四面同口径与入口一致的可验收场景。
- `work-item-typed-outcome-policy`: ADDED REQ-TOP-07（契约类 finding 的有效必关闭投影与 policy 分类口径）——规定有效必关闭谓词在 typed outcome policy 中的投影：`class_hint=advisory` 与 raw `pass` fallback 不得把有效必关闭的契约类 finding 判为 Advisory；无可靠定位/服务端修复许可时落 `HumanRequired` 而非 `Repairable`；**不提高 A/B 余额**、**不改变 Verification 新 finding 既有停止规则**。

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
- **受影响代码面**：单候选评审的 verdict 解析与归一、必关闭清单渲染、owner 处置再归一、policy 分类与预算裁决、评审路由与重复指纹判定、历史评审展示归一。集中在 `src/web/workspace_ws_types/review.rs`、`src/product/workspace_engine/parsers.rs`、`src/product/workspace_engine/review/`（含 `disposition_override.rs`、`routing.rs`、`routing_scope.rs`、`routing_platform_rescue.rs`、`structured_output.rs`）、`src/product/workspace_engine/prompts/`（`history_compaction.rs`、`review.rs`）、`src/product/work_item_plan_policy/classify.rs`、`src/product/workspace_engine/session_state_parts/historical_review.inc.rs`。精确文件、命令、测试与提交步骤由 `writing-plans` 写入 `cadence/plans/`，不在本 change 内规定。
- **数据契约**：无新增持久化字段；durable 已处置事实清单与 finding key 格式不变。
- **共享面**：与 `mechanicalize-plan-compile-repair`（`review/routing_scope.rs` 现场归因段）、`fix-author-rewrite-quota-pool`（`review/routing.rs`、`review/routing_platform_rescue.rs` 预算域）存在文件级重叠，串行归属见 `design.md` 的「共享面与排序」。
- **无对外 API / 协议破坏性变更**：判定语义收紧属行为修正，不改 wire 形状。
