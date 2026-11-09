# Design

## Context

见 `proposal.md` —— Why。本节只记录当前实现状态与约束。

### 当前状态（已核实）

平台侧存在**四套互不相同的门槛**，决定一条 finding 是否阻断、是否进入必关闭清单、是否消耗返修预算：

| 消费面 | 现状（代码） | 口径 |
|---|---|---|
| 平台 disposition 硬化 | `src/web/workspace_ws_types/review.rs:187-197` `with_platform_disposition_rules()` | `contract_gap`/`self_contradiction` + `disposition=advisory` → 改写为 `actionable` |
| verdict 重开 `pass` | `src/product/workspace_engine/parsers.rs:427-455` `effective_work_item_plan_review_verdict()` | 只看 `is_actionable() && severity ∈ {blocking, must_fix}` |
| 必关闭清单渲染 | `src/product/workspace_engine/prompts/history_compaction.rs:85-122` `render_open_required_findings()` | 先要求 `review_gate == RequiresRevision`，再筛 `is_actionable() && is_required(severity)` |
| owner 再归一 | `src/product/workspace_engine/review/disposition_override.rs:43-77` `normalize_review_verdict_for_dispositions()` | 先把 owner 排除项置 `advisory`，再 `renormalize_verdict_after_disposition_change()` |
| policy 分类 | `src/product/work_item_plan_policy/classify.rs:124-185` `classify_finding()` | `!is_actionable()` → `Advisory`；否则 `class_hint` 优先；否则 `fallback_class(raw_verdict, finding)`（`Pass` → `Advisory`） |
| 非 advisory 指纹集合 | `src/product/workspace_engine/review/routing_scope.rs:81-91` `non_advisory_finding_fingerprints()` | 以 `classify_finding` 的 `class != Advisory` 为准（口径与 policy 一致，但源头是 class 而非有效必关闭） |

### 现场事实（实测基线）

单候选 WorkItemPlan 评审 1 轮，reviewer 输出 `verdict=pass`，5 条 finding 中 `#1` 是 `category=contract_gap` / `disposition=actionable` / `class_hint=advisory` / `severity=suggestion` / `contract_field=None`，其原文精确描述了上一轮 author 重写引入的 `input_contracts` 条目丢失缺陷。其余 4 条为范围外 / 完备度意见（`advisory`）。durable `run_history` 显示 `repairs_used=0`、`manual_repairs_used=0`、`seen_fingerprints=[]`、`verification_review_count=0` —— 该 finding **从未进入必关闭清单，也从未消耗任何预算**。

### 关键约束

- REQ-WSC-13 钉住 finding 身份 = `sha256(severity + \u{1} + message)` 前 16 位小写 hex；owner 排除以**现算 key** 匹配。改 severity 即改身份，owner 既有排除会失配。
- owner 排除事实存于会话 durable 已处置事实清单，是**独立事实源**，必须先于平台派生判定生效。
- 有效必关闭的例外**只适用于 WorkItemPlan 的既有契约类 finding**；通用 Story/Design 评审走的是 `parse_review_value()` 通用入口，其旧门槛不得扩大。
- 平台侧**不允许**在没有可验证定位与服务端修复许可的情况下新造自动修复许可（与 SG-02A 内容自修边界一致）。
- 本 change **不得**提高 A/B 池余额，**不得**改变 Verification 新 finding 的既有停止规则（REQ-TOP-04 `VerificationNewFindings`）。

## Goals / Non-Goals

**Goals:**

- 建立**唯一** effective-closure 谓词，作为契约类 finding「是否必须关闭」的权威口径，并让 verdict 判定 / 必关闭清单 / owner 再归一 / policy 分类**四面同口径**。
- 谓词的**作用域**显式且受限：只作用于 WorkItemPlan 的既有契约类 finding；作用域由**可信 session / 解析入口**传入，**不由模型标签授予**。
- strict / lenient / historical 三条入口共用同一规则，不只修 live 路径。
- 保留原 severity 与内容身份；缺可验证定位 / 修复许可时**转既有人工处理**，不增加自动重写次数。

**Non-Goals:**

- 不改 finding 身份算法、不做身份迁移。
- 不提高 A/B 池余额，不改返修预算归属，不改 Verification 新 finding 停止规则。
- 不放大普通 advisory；不改所有 `suggestion` 的通用门槛。
- 不新增持久化字段、不扩 wire 形状、不扩公共配置面。
- 不回滚 / 不重跑 / 不自动撤销已确认或已发布的历史产物。

## Decisions

### D1 · 唯一 effective-closure 谓词，保留原 severity 与内容身份

**决定：** 引入一个服务端语义的**有效必关闭**判定，作为唯一的「是否必须关闭」权威口径：

````text
requires_closure(finding, scope) =
    effective_disposition(finding) == Actionable
    AND (
        severity ∈ {blocking, must_fix}
        OR (scope ∈ {WorkItemPlan 契约类作用域}
            AND category ∈ {contract_gap, self_contradiction})
    )
````

其中 `effective_disposition` = 平台硬化（`with_platform_disposition_rules`）后的 disposition。

**关键性质：severity 是「充分条件」而非「必要条件」。** 契约类 finding 即使 `severity=suggestion` 也满足有效必关闭；平台**不改写** `severity` 字段本身，也不改写 `message`，因此 finding key（`sha256(severity + \u{1} + message)`）逐字不变。

**顺序固定（三段，不可颠倒）：**

````text
1. 解析平台硬化   —— category=contract_gap/self_contradiction 且 disposition=advisory → actionable
2. durable owner 排除 —— 命中的 key → 视作 advisory（owner 事实优先）
3. 有效必关闭 / 路由计算 —— 在 (1)(2) 之后的 disposition 上计算
````

第 2 步之后**不得**再做平台硬化，否则会把 owner 已排除的项复活。这是当前 `disposition_override.rs` 已经做对的顺序，必须保留。

**替代方案与否决理由：**

| 备选 | 否决理由 |
|---|---|
| 只把 `suggestion` 升为 `must_fix` | 改写 severity 即改 finding key → owner 既有排除全部失配，且仍漏 policy 的 `class_hint` 优先级与 raw `Pass` 兜底 |
| 只放宽 verdict 函数（`parsers.rs`） | 必关闭清单（`history_compaction.rs`）、owner 再归一（`disposition_override.rs`）、policy 分类（`classify.rs`）仍各用旧门槛，漏三处 |
| 推迟到二期 | 已是 REQ-WSC-12/17 的明文 MUST 未兑现，补输入不能保证模型不再标 `suggestion` |
| 在规格上承认 `pass` 可放行契约缺口 | 把真实缺口放行制度化 |

### D2 · 作用域由可信入口传入，不由模型标签授予

**决定：** 谓词的作用域（是否启用契约类例外）是**服务端事实**，由可信 session / 解析入口确定，**不得**由 finding 的任何字段（`category`、`class_hint`、`severity`、`verdict`、`disposition`）或模型输出内容决定。

- 启用例外：WorkItemPlan 工作区类型下、既有契约类 finding（`contract_gap` / `self_contradiction`）。
- 不启用例外：通用 Story/Design 等其他评审类型 —— 其旧门槛（`severity ∈ {blocking, must_fix}` + `actionable`）**逐字不变**。

**理由：** 若作用域可由模型标签授予，模型就能通过输出一个 `category` 值自我豁免或自我加压，判定将退化为「模型自觉」。作用域必须落在**可信调用点**（`parse_work_item_plan_review_value*` 系列入口与 WorkItemPlan session 上下文），与「可信 session / 解析入口传入」一致。

**注意：** `category` 参与的是**例外内部的类别判定**（是否属于契约类），不是**例外本身的授予**。两者必须分离：谁决定启用例外由服务端决定；例外适用于哪些类别由既有契约类枚举决定。

### D3 · 四面同口径：一个谓词，四个消费面

**决定：** 四个消费面全部改为消费 D1 的谓词或其投影，不允许任何一面自行重算门槛：

| 面 | 现状门槛 | 改后 |
|---|---|---|
| verdict 判定（`parsers.rs`） | `actionable && severity ∈ {blocking,must_fix}` | 改用 `requires_closure` |
| 必关闭清单（`prompts/history_compaction.rs`） | `gate == RequiresRevision` 且 `actionable && is_required(severity)` | 清单成员改用 `requires_closure`；标题不再声称仅 blocking/must_fix；`NeedsHuman` 中未解决的契约项 MUST NOT 因 gate 不是 `RequiresRevision` 而消失 |
| owner 再归一（`review/disposition_override.rs`） | 排除 → advisory → 重新归一 verdict/gate | 重新归一时改用 `requires_closure`；排除项与 key、幂等、纯读恢复语义不变；非本作用域不偷改 |
| policy 分类（`work_item_plan_policy/classify.rs`） | `!actionable → Advisory`，否则 `class_hint` 优先，否则 `fallback_class` | 有效必关闭的契约类 finding MUST NOT 被 `class_hint=advisory` 或 raw `Pass` 兜底判为 `Advisory` |

**policy 面的具体投影：** `classify_finding` 里，契约类例外命中时，分类**不再**由 `class_hint` 决定，也**不再**由 raw `Pass` 兜底为 `Advisory`。但「标签不足即自动 Repairable」也是错的 —— 见 D4。

**投影 vs 谓词的关系：** policy 不直接复刻谓词公式，而是**消费解析侧已算出的有效必关闭事实**（平台硬化的 disposition + 契约类例外判定）。单一事实源在解析侧；policy 只做投影，避免两处公式漂移。

**为什么不改 REQ-TOP-02（中央策略裁决与预算边界）：** REQ-TOP-02 的「裁决优先级固定」顺序为「转换预算耗尽 → 重复指纹 → 返修需求 → human_required → valid」。本 change 新增的「缺定位转人工」落在既有 `human_required` 分支内，**不改变该顺序**，也不改变 `max_repairs` / `max_manual_repairs` 的语义；REQ-TOP-02 的既有场景逐字仍然成立。因此以 **ADDED REQ-TOP-07** 承载新约束，而不是 MODIFIED REQ-TOP-02 —— 避免在未发生语义变化时重写既有条款。

**为什么 REQ-TOP-01 不改：** REQ-TOP-01 的既有场景「分类使用原始 verdict」约束的是 **verdict 维度的来源**（分类用 raw_verdict 而非归一后的 gate），本 change 不改这一点；改的是**分类结论的优先级**（契约类例外优先于 `class_hint` 与 raw verdict 兜底）。两者正交，故以 ADDED 场景显式声明「verdict 维度仍用 raw_verdict」以消除歧义，而非重写 REQ-TOP-01 全文。

### D4 · 缺可验证定位 / 修复许可时转既有人工处理

**决定：** 有效必关闭**只决定「必须关闭」，不决定「由谁关闭」**。

- 有可验证定位（`contract_field` 可用）**且**存在既有合法机械修复证据 → 沿既有许可走可自动返修路径（沿用既有 `Repairable` 判定与预算记账）。
- 无可验证定位、或无既有合法修复许可 → 落 `HumanRequired`，进入既有人工处理通道。

**MUST NOT** 仅凭 `category` 或 `severity` 新造 L1 自动修复许可。这条与 SG-02A 的「内容变更只由原授权 author 执行、平台不得凭类别自造修复权」一致。

**代价（明示）：** 契约类 finding 若缺定位，将更早落到人工门，可能**增加人工门数量**。这是有意的失败安全方向：宁可多一次人工确认，不可漏一次契约缺口。**自动重写次数不增加** —— 本 change 不新增任何自动 author 调用路径。

### D5 · strict / lenient / historical 三入口同一规则

**决定：** 三个入口共用同一谓词与同一作用域传递方式：

- **strict 入口**（live 严格解析）：平台硬化 → owner 排除 → 有效必关闭。
- **lenient 入口**（宽松解析）：同上，同一谓词。
- **historical 入口**（历史展示重建）：同上，同一谓词；**重新归一**，但 **MUST NOT 自动启动 provider**，**MUST NOT 回滚已确认 / 已发布的历史产物**。

**理由：** 只修 live 会让「历史 rebuild」与「live」对同一 finding 给出不同结论，owner 看到的门卡与系统内部判定不一致；这正是 REQ-WSC-13 要求 live 与 rebuild 算出同一个 key 的同类问题。

**历史边界：** 历史展示路径重建的是**展示归一**，不是**重新执行**。已确认 / 已发布的历史产物保持原状；历史 finding 的归一变化只影响展示，不触发任何 provider 启动、不撤销既有确认。

### D6 · 范围外 advisory 与其他 suggestion 保持原行为

**决定：** 例外的类别枚举**只有** `contract_gap` 与 `self_contradiction`。其余 `suggestion`（`completeness`、`scope_conflict`、`verification_unattributable`、`other` 等）继续走原门槛：不进必关闭清单、不计非 advisory 指纹、不消耗返修预算、不阻断。

**理由：** 现场 5 条中只有 1 条是契约类；其余 4 条是范围外 / 完备度意见，放大它们会制造大量无意义人工门，是「顺带放大普通 advisory」——本 change 明确不做。

### D7 · 必关闭清单与 NeedsHuman 门卡的呈现口径

**决定：** 必关闭清单的标题与说明 MUST NOT 再声称「仅 blocking/must_fix」；保留原 severity 展示值，并在说明中点出「平台契约闭合义务」这一独立于 severity 的义务来源。

`NeedsHuman` 结论下未解决的契约项 MUST NOT 因 `review_gate != RequiresRevision` 而从清单消失 —— 当前 `render_open_required_findings` 的 `review_gate` 前置判断是漏点之一。

## 共享面与排序

本 change 与以下**活动 change** 存在文件级重叠。**这些 change 目录本 change 一律不改**，仅在此记录归属与串行顺序。

| 共享文件 | 对方 change | 本 change 的动作 | 串行归属 |
|---|---|---|---|
| `src/product/workspace_engine/review/routing.rs` | `fix-author-rewrite-quota-pool`（A/B 池共池：平台自动重写改扣 B 池、`repairs_used_delta` 生成） | 只改**判定口径**（是否有效必关闭），**不改**任何预算扣减与 reservation 逻辑 | **对方先**：本 change 在其预算域改动落地后再叠加判定口径，避免同一次修改同时动「余额」与「谁算 repairable」两处语义 |
| `src/product/workspace_engine/review/routing_platform_rescue.rs` | `fix-author-rewrite-quota-pool`（`:124` A 池守卫改读 B 余额） | 本 change 需让「平台救回」路径使用同一谓词（否则被救回的 finding 仍可能被免阻断） | **对方先**，本 change 随后 |
| `src/product/workspace_engine/review/routing_scope.rs` | `mechanicalize-plan-compile-repair`（`:815-849` 现场 #3 重复 finding 归因：相邻两轮指纹集合完全相等判定） | 本 change 改 `non_advisory_fingerprints` 的**成员资格**（哪些 finding 进集合）；对方改**集合比较方式** | **对方先**（其现场归因结论需先稳定），本 change 随后只改成员资格 |
| `src/product/work_item_plan_policy/classify.rs` + `work_item_plan_policy/tests_classify.rs` | `fix-author-rewrite-quota-pool`（`routing.rs` 调 `classify_review`；测试面共享 `tests_classify.rs`） | 本 change 改 `classify_finding` 的契约类例外分支 | 与预算域改动**可并行**，但**同一文件**须串行：建议本 change 的 `classify.rs` 改动排在对方预算域改动**之后**，避免双方同时编辑同一文件 |
| `src/product/workspace_engine/parsers.rs`、`review/disposition_override.rs`、`prompts/history_compaction.rs`、`prompts/review.rs`、`review/structured_output.rs`、`session_state_parts/historical_review.inc.rs` | 无活动 change 占用 | 本 change 独占 | — |

**总排序：** ① `fix-author-rewrite-quota-pool`（预算域）→ ② `mechanicalize-plan-compile-repair`（指纹集合归因）→ ③ 本 change（有效必关闭判定）。

**为何必须串行：** 本 change 收紧「什么算 blocking」，对方改动收紧「blocking 由哪个池付钱」。同时落地会让「判定口径变更」与「预算域变更」纠缠在同一 diff 中，无法判定哪条语义变化导致了预算行为变化 —— 而验收必须能分别归因。本 change 的验收明确包含「其余四条仍不扣预算」与「不增加自动重写次数」，前提是预算域本身在稳定基线上。

**与归档 change 的边界：** `mechanicalize-plan-compile-repair` 与 `fix-author-rewrite-quota-pool` 目录本 change **不改**；本 change 的 `design.md` / `tasks.md` 只引用它们的工作包编号作为前置条件。

## Risks / Trade-offs

- **[人工门数量可能增加]** 契约类 finding 若缺可验证定位，将更早落人工门 → 用 D4 的「转既有人工处理」而非「自动放行」控制；owner 可用既有排除项显式排除；**自动重写次数不增加**。
- **[模型误标 category 可能放大]** 模型把普通问题误标为 `contract_gap`/`self_contradiction` → 类别枚举**不扩大**（只有两类）；owner 排除优先且 key 不变（排除语义仍可用）；D4 保证不因此自动调用 author，最多转人工。
- **[四面漂移回归]** 未来有人在某一面另设门槛 → 谓词是**唯一**实现点，四面只消费；`design.md` 与验收矩阵固定「四面同口径」为可验收结论（同一 finding 四面结论一致）。
- **[owner 排除项被复活]** 平台硬化与排除的顺序颠倒 → 顺序固定为「硬化 → 排除 → 计算」，排除后不再硬化；验收含「owner 排除优先且排除项不得复活」。
- **[历史产物被误动]** 历史重建路径引入新判定可能触发 provider 或撤销已确认产物 → D5 明确历史入口只做展示归一，不启动 provider、不回滚已确认/已发布产物。
- **[通用评审门槛被误扩大]** 契约类例外误扩散到 Story/Design → D2 的作用域由可信入口传入，不由模型标签授予；验收含「例外作用域不扩到通用评审」。
- **[指纹集合成员变化引发重复判定漂移]** `non_advisory_fingerprints` 成员资格变化会影响相邻两轮集合相等判定 → 与 `mechanicalize-plan-compile-repair` 串行（见「共享面与排序」）；验收须复跑既有重复指纹与「同一指纹复评停止」回归。

## Migration Plan

- **无数据迁移**：不新增持久化字段，finding key 与 durable 已处置事实清单格式不变。
- **部署**：判定口径变更随代码发布即生效；无 feature flag 需求（收紧密判定，不涉及新外部依赖）。
- **回退策略（受限）**：回退**只能**回到 fail-safe 人工处置（即保留「有效必关闭 ⇒ 转人工」而放弃「可自动返修」），**或**整体回退未启用的新版本。**不得**静默恢复「契约缺口自动 pass」——那等于撤销 REQ-WSC-12/17 的明文 MUST。
- **历史兼容**：已确认 / 已发布的历史产物不回滚；历史展示可重新归一，但 MUST NOT 自动启动 provider。
