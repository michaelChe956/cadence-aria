# Design

## Context

见 `proposal.md` —— Why。本节只记录当前实现状态与约束。

### 当前状态（已核实）

平台 disposition 硬化（`src/web/workspace_ws_types/review.rs:187-197` `with_platform_disposition_rules()`：`contract_gap`/`self_contradiction` + `disposition=advisory` → 改写为 `actionable`）是唯一已兑现的硬化面，作为下面各消费面的**前置基准**；其余**七个消费面**口径互不相同或派生自旧口径，共同决定一条 finding 是否阻断、是否进入必关闭清单、是否消耗返修预算、如何呈现给作者：

| 面 | 现状（代码） | 口径 |
|---|---|---|
| 平台 disposition 硬化（前置基准） | `src/web/workspace_ws_types/review.rs:187-197` `with_platform_disposition_rules()` | `contract_gap`/`self_contradiction` + `disposition=advisory` → 改写为 `actionable` |
| verdict 重开 `pass` | `src/product/workspace_engine/parsers.rs:427-455` `effective_work_item_plan_review_verdict()` | 只看 `is_actionable() && severity ∈ {blocking, must_fix}` |
| 必关闭清单渲染 | `src/product/workspace_engine/prompts/history_compaction.rs:85-122` `render_open_required_findings()` | 先要求 `review_gate == RequiresRevision`，再筛 `is_actionable() && is_required(severity)` |
| owner 再归一 | `src/product/workspace_engine/review/disposition_override.rs:43-77` `normalize_review_verdict_for_dispositions()` | 先把 owner 排除项置 `advisory`，再 `renormalize_verdict_after_disposition_change()` |
| policy 分类 | `src/product/work_item_plan_policy/classify.rs:124-185` `classify_finding()` | `!is_actionable()` → `Advisory`；否则 `class_hint` 优先；否则 `fallback_class(raw_verdict, finding)`（`Pass` → `Advisory`） |
| 非 advisory 指纹集合 | `src/product/workspace_engine/review/routing_scope.rs:84-94` `non_advisory_finding_fingerprints()` | 以 `classify_finding` 的 `class != Advisory` 为准（口径随 policy 分类，源头是 class 而非有效必关闭；原在状态表但未进 D3 消费面表，本版补入） |
| 返修轮作者呈现（补录） | `src/product/work_item_split_engine/prompts/sc_revision.rs:147` | 向作者声明「`severity=suggestion` 的条目为建议项；`must_fix`/`blocking` 条目为本轮必须修复项」——重开后仍为 `suggestion` 的契约缺口在返修轮被呈现为可选 |
| 返修影响闭合（补录） | `src/product/workspace_engine/plan_outline/revision.rs:627-638` `requires_impact_closure` | 仅认 `is_actionable() && severity ∈ {blocking, must_fix}`（另含 diagnostic 未修复分支），不认契约类例外 |

原「四套 / 四面」措辞按上表实际面数订正为**七面**（verdict 重开、必关闭清单、owner 再归一、policy 分类、非 advisory 指纹、返修轮作者呈现、返修影响闭合）；`sc_revision.rs:147` 与 `plan_outline/revision.rs:627-638` 是补录的遗漏两面。

### 现场事实（实测基线）

单候选 WorkItemPlan 评审 1 轮，reviewer 输出 `verdict=pass`，5 条 finding 中 `#1` 是 `category=contract_gap` / `disposition=actionable` / `class_hint=advisory` / `severity=suggestion` / `contract_field=None`，其原文精确描述了上一轮 author 重写引入的 `input_contracts` 条目丢失缺陷。其余 4 条为范围外 / 完备度意见（`advisory`）。durable `run_history` 显示 `repairs_used=0`、`manual_repairs_used=0`、`seen_fingerprints=[]`、`verification_review_count=0` —— 该 finding **从未进入必关闭清单，也从未消耗任何预算**。

### 关键约束

- REQ-WSC-13 钉住 finding 身份 = `sha256(severity + \u{1} + message)` 前 16 位小写 hex；owner 排除以**现算 key** 匹配。改 severity 即改身份，owner 既有排除会失配。
- owner 排除事实存于会话 durable 已处置事实清单，是**独立事实源**，必须先于平台派生判定生效。
- 有效必关闭的例外**只适用于 WorkItemPlan 的既有契约类 finding**；通用 Story/Design 评审走的是 `parse_review_value()` 通用入口，其旧门槛不得扩大。作用域取值逐项见 D8。
- 平台侧**不允许**在没有可验证定位与服务端修复许可的情况下新造自动修复许可（与 SG-02A 内容自修边界一致）。
- 本 change **不得**提高 A/B 池余额，**不得**改变 Verification 新 finding 的既有停止规则（REQ-TOP-04 `VerificationNewFindings`）。

## Goals / Non-Goals

**Goals:**

- 建立**唯一** effective-closure 谓词，作为契约类 finding「是否必须关闭」的权威口径，并让 verdict 判定 / 必关闭清单 / owner 再归一 / policy 分类 / 非 advisory 指纹 / 返修轮呈现 / 返修影响闭合**七面同口径**。
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

其中 `effective_disposition` = 平台硬化（`with_platform_disposition_rules`）后的 disposition；「WorkItemPlan 契约类作用域」的取值逐项见 D8。

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

### D2 · 谓词实现为纯函数，作用域由可信入口传入，不由模型标签授予

**决定：** 判定式实现为**纯函数** `requires_closure(finding, scope) -> bool`（无副作用），落点为 `src/product/workspace_engine/review/closure_policy.rs`（新建文件）——这是**唯一**谓词实现处，其他面（verdict 判定、清单渲染、owner 再归一、policy 分类、指纹、呈现）MUST NOT 各自复制判定式，只消费该函数或其投影；配套测试落 `review/closure_policy_tests.rs`。

**scope 是枚举、不是 wire 字段：** 由 `WorkspaceType` / `WorkItemPlanReviewScope` 等既有类型派生的枚举，随调用链传入。作用域（是否启用契约类例外）是**服务端事实**，由可信 session / 解析入口确定，**不得**由 finding 的任何字段（`category`、`class_hint`、`severity`、`verdict`、`disposition`）或模型输出内容决定。真实可信入口的证据：`review/structured_output.rs:251` 以 `workspace_type == WorkItemPlan` 分流到 `parse_work_item_plan_review_value*` 系列，该事实非模型可控。

**`classify_finding` 增 `scope` 参数，6 个调用点逐一适配并写明各自 scope 取值来源：**

| 调用点 | scope 来源 |
|---|---|
| `work_item_plan_policy/classify.rs:70`（`classify_review` 内部） | 由调用方随评审上下文传入 |
| `review/routing_scope.rs:88` | 所在路由的 session / 解析入口作用域 |
| `review/routing_platform_rescue.rs:71` | 同上（平台救回上下文） |
| `review/routing_platform_rescue.rs:267` | 同上 |
| `prompts/review_context.rs:557` | 评审上下文构建入口的作用域 |
| `review/routing.rs:468`（经 `classify_review`） | 路由入口的 session 作用域 |

- 启用例外：WorkItemPlan 工作区类型下、既有契约类 finding（`contract_gap` / `self_contradiction`），取值细则见 D8。
- 不启用例外：通用 Story/Design 等其他评审类型 —— 其旧门槛（`severity ∈ {blocking, must_fix}` + `actionable`）**逐字不变**。

**理由：** 若作用域可由模型标签授予，模型就能通过输出一个 `category` 值自我豁免或自我加压，判定将退化为「模型自觉」。作用域必须落在**可信调用点**，与「可信 session / 解析入口传入」一致。

**注意：** `category` 参与的是**例外内部的类别判定**（是否属于契约类），不是**例外本身的授予**。两者必须分离：谁决定启用例外由服务端决定；例外适用于哪些类别由既有契约类枚举决定。

### D3 · 七面同口径：一个谓词，七个消费面

**决定：** 七个消费面全部改为消费 D1 的谓词或其投影，不允许任何一面自行重算门槛：

| 面 | 现状门槛 | 改后 |
|---|---|---|
| verdict 判定（`parsers.rs`） | `actionable && severity ∈ {blocking,must_fix}` | 改用 `requires_closure` |
| 必关闭清单（`prompts/history_compaction.rs`） | `gate == RequiresRevision` 且 `actionable && is_required(severity)` | 清单成员改用 `requires_closure`；标题不再声称仅 blocking/must_fix；`NeedsHuman` 中未解决的契约项 MUST NOT 因 gate 不是 `RequiresRevision` 而消失 |
| owner 再归一（`review/disposition_override.rs`） | 排除 → advisory → 重新归一 verdict/gate | 重新归一时改用 `requires_closure`；排除项与 key、幂等、纯读恢复语义不变；非本作用域不偷改 |
| policy 分类（`work_item_plan_policy/classify.rs`） | `!actionable → Advisory`，否则 `class_hint` 优先，否则 `fallback_class` | 闭合资格为真的契约类 finding MUST NOT 被 `class_hint=advisory` 或 raw `Pass` 兜底判为 `Advisory`；两级判定见 D4 |
| 非 advisory 指纹集合（`review/routing_scope.rs:84-94`） | 以 `classify_finding` 的 `class != Advisory` 为准 | 成员资格随 policy 分类消费谓词后自动对齐，**不另设复制判定式** |
| 返修轮作者呈现（`work_item_split_engine/prompts/sc_revision.rs:147`） | 按 `severity` 字面声明「`suggestion` 条目为建议项」——重开后的契约缺口在返修轮仍显示可选 | 改按**有效必关闭**呈现：必关闭项 MUST 呈现为本轮必须修复，不再按 `severity=suggestion` 字面声明为建议项 |
| 返修影响闭合（`workspace_engine/plan_outline/revision.rs:627-638`） | `requires_impact_closure` 仅认 `actionable && severity ∈ {blocking,must_fix}` | 改认**有效必关闭** |

**policy 面的具体投影：** `classify_finding` 里，契约类例外命中（闭合资格为真）时，分类**不再**由 `class_hint` 决定，也**不再**由 raw `Pass` 兜底为 `Advisory`。但「标签不足即自动 Repairable」也是错的 —— 见 D4。

**投影 vs 谓词的关系：** policy 不直接复刻谓词公式，而是**消费解析侧已算出的有效必关闭事实**（平台硬化的 disposition + 契约类例外判定）。单一事实源在解析侧；policy 只做投影，避免两处公式漂移。

**审计与排除：** 原始模型文本保留用于审计，但 policy MUST NOT 重新解析 raw 后绕过 owner 排除；**历史重建路径同样禁止**（见 D5 与 REQ-TOP-07 场景「不得重新解析 raw 绕过 owner 排除」）。

**为什么不改 REQ-TOP-02（中央策略裁决与预算边界）：** REQ-TOP-02 的「裁决优先级固定」顺序为「转换预算耗尽 → 重复指纹 → 返修需求 → human_required → valid」。本 change 新增的「缺定位转人工」落在既有 `human_required` 分支内，**不改变该顺序**，也不改变 `max_repairs` / `max_manual_repairs` 的语义；REQ-TOP-02 的既有场景逐字仍然成立。因此以 **ADDED REQ-TOP-07** 承载新约束，而不是 MODIFIED REQ-TOP-02 —— 避免在未发生语义变化时重写既有条款。

**为什么 REQ-TOP-01 不改：** REQ-TOP-01 的既有场景「分类使用原始 verdict」约束的是 **verdict 维度的来源**（分类用 raw_verdict 而非归一后的 gate），本 change 不改这一点；改的是**分类结论的优先级**（契约类例外优先于 `class_hint` 与 raw verdict 兜底）。两者正交，故以 ADDED 场景显式声明「verdict 维度仍用 raw_verdict」以消除歧义，而非重写 REQ-TOP-01 全文。

### D4 · 两级判定：闭合资格与自动修复许可分离

**决定：** 有效必关闭**只决定「必须关闭」，不决定「由谁关闭」**。policy 分类的落点按两级判定：

````text
requires_closure      = effective_disposition == actionable
                     && category ∈ {contract_gap, self_contradiction}
                     && 未被 owner 显式排除          ← 纯谓词，无副作用

auto_repair_permitted = requires_closure
                     && 存在「服务端授权的既有合法机械修复许可」证据
                     && 该 finding 具备「可验证定位」证据
````

本节的 `requires_closure` 指**契约类闭合资格**——即 D1 谓词的契约类例外分支，并把「未被 owner 显式排除」写成显式合取项（与 D1 三段顺序「排除先于计算」一致）。severity 路径（`blocking`/`must_fix`）直接满足 D1 谓词、但 `category` 不属契约类的 finding 不进入本两级判定，维持既有分类行为。

**分类落点：**

| 判定 | 分类 |
|---|---|
| `requires_closure` 真 且 `auto_repair_permitted` 真 | `Repairable`（**沿既有许可**，不新造 L1） |
| `requires_closure` 真 且 `auto_repair_permitted` 假 | **`HumanRequired`**（不得 `Advisory`，也不得 `Repairable`） |
| `requires_closure` 假 | 维持既有行为（范围外 advisory 不变） |

**两个证据的落地口径（必须落到现码可判定项，不得造新概念）：**

- **可验证定位** ＝ 该 finding 能指到具体契约面：`contract_field` 非空，**或**携带可定位的 typed 事实引用（机械侧 typed IR / structure 诊断）。`category` / `severity` 本身**不构成**定位证据（裁决 §4.3「不凭 category 或 severity 新造 L1」）。
- **既有合法机械修复许可** ＝ 服务端既有 L1 机械修复授权链路的在场证据（`RepairReservation` / 机械修复指令链）。**不是** `class_hint`、**不是** `category`、**不是** `severity`。注意 `machine_repair` 字段在 Raw 适配入口恒 `None`、只由服务端 L1 配对产生（`parsers.rs:744-746` 注释），provider 伪造字段在 strict 白名单处已被拒——许可证据只能来自服务端链路。

**因此 `fallback_class` 必须改**（`work_item_plan_policy/classify.rs:174-183`）：现码对 `Revise + 契约类 + contract_field 非空` 直接给 `Repairable`，这条**仅凭 category + field** 的路径 MUST NOT 单独构成 `Repairable`；按新判定式，无服务端许可证据时落 `HumanRequired`。`class_hint=repairable` 亦 MUST NOT 单独构成自动调用许可（`classify.rs:134-140` 现让 hint 优先于 `fallback_class`；裁决 §4.3 routing 行「模型 hint 不能单独给自动调用许可」）。

**混合格统一：** `auto_repair_permitted` 的两个证据条件是**合取**；任一缺失即 `auto_repair_permitted` 为假 ⇒ `HumanRequired`。delta 场景与正文均按此表述——不再出现「无定位**且**无修复证据」与「缺定位**或**缺许可」两套混合格（「有定位无证据」「无定位有证据」两个混合象限随合取式自然归入 `HumanRequired`）。

**golden 影响（不得假装不变）：** `work_item_plan_policy/tests_classify.rs:24-114,248-270` 与 `review/disposition_override.rs:347-363` 中依赖「hint wins」或「Revise+field ⇒ Repairable」的既有断言，按裁决 §4.4 矩阵（category 两类 × suggestion/must_fix/blocking × disposition actionable/advisory/缺省 × hint 三类 + **缺省** × Pass/Revise/NeedsHuman）更新；**原真实 golden findings 不改写成新造输入**。

**owner 可读呈现口径（人工门卡）：** 落 `HumanRequired` 的契约类门卡沿用平台渲染的技术细节呈现机制向 owner 呈现闭合理由与缺定位/缺许可的判定依据；REQ-WSC-11/15 的 `owner_question` / `owner_options` 只在模型提供时强制（`parsers.rs:395-401` 的可选字段解析），平台渲染**不替代** owner 提问。

**代价（明示）：** 契约类 finding 若缺定位，将更早落到人工门，可能**增加人工门数量**。这是有意的失败安全方向：宁可多一次人工确认，不可漏一次契约缺口。**自动重写次数不增加** —— 本 change 不新增任何自动 author 调用路径。

### D5 · strict / lenient / historical 三入口同一规则

**决定：** 三个入口共用同一谓词与同一作用域传递方式：

- **strict 入口**（live 严格解析）：平台硬化 → owner 排除 → 有效必关闭。
- **lenient 入口**（宽松解析）：同上，同一谓词。
- **historical 入口**（历史展示重建）：同上，同一谓词；**重新归一**，但 **MUST NOT 自动启动 provider**，**MUST NOT 回滚已确认 / 已发布的历史产物**。

**raw verdict 的语义：** 本 change 所称 raw verdict 指**未经有效必关闭重开**的原始判定。`review/routing.rs:344,444` 取的是已被 `effective_work_item_plan_review_verdict` 重开后的序列化值（`finalize_review_verdict` 注释称「provider 原始 verdict」实为 post-parse 值），因此 live 重开路径上「闭合资格为真 + raw `pass` 到达 policy」的组合不可达。可达表述是：**当某路径上未发生重开时**——读持久化旧 verdict、`review/routing_scope.rs:88`、`review/routing_platform_rescue.rs:71`、`prompts/review_context.rs:557`——原始 `pass` 同样 MUST NOT 使未排除契约类自动放行。delta 场景已按此改写；**禁止**保留 live 路径不可达的场景而不说明。

**historical 入口的两类（各自写明）：**

1. **重新解析型**：`parse_historical_review_json` / `parse_historical_work_item_plan_review_json`（`review/routing.rs:1279,1286`、`workspace_engine/parsers/owner_guidance.rs:335`）——重新解析后经 D1 三段顺序与同一谓词归一。
2. **纯 serde 反序列化型**：`deserialize_historical_review_verdict`（`session_state_parts/historical_review.inc.rs:38`、`workspace_engine/lifecycle_recovery.rs:45`、`prompts/review_context.rs:546`、`review/routing_scope.rs:843`）——不重新解析模型文本；其消费面（分类 / 清单 / 呈现）按同一谓词对 finding 重算闭合事实，得出与 live 一致的展示归一。

两类入口 MUST 得出同一有效必关闭结论；均 MUST NOT 自动启动 provider、MUST NOT 回滚已确认 / 已发布产物。

**审计与排除：** 原始模型文本保留用于审计，但 policy MUST NOT 重新解析 raw 后绕过 owner 排除；**历史重建路径（含上述两类入口）同样禁止**。

**理由：** 只修 live 会让「历史 rebuild」与「live」对同一 finding 给出不同结论，owner 看到的门卡与系统内部判定不一致；这正是 REQ-WSC-13 要求 live 与 rebuild 算出同一个 key 的同类问题。

**历史边界：** 历史展示路径重建的是**展示归一**，不是**重新执行**。已确认 / 已发布的历史产物保持原状；历史 finding 的归一变化只影响展示，不触发任何 provider 启动、不撤销既有确认。

### D6 · 范围外 advisory 与其他 suggestion 保持原行为

**决定：** 例外的类别枚举**只有** `contract_gap` 与 `self_contradiction`。其余 `suggestion`（`completeness`、`scope_conflict`、`verification_unattributable`、`other` 等）继续走原门槛：不进必关闭清单、不计非 advisory 指纹、不消耗返修预算、不阻断。

**理由：** 现场 5 条中只有 1 条是契约类；其余 4 条是范围外 / 完备度意见，放大它们会制造大量无意义人工门，是「顺带放大普通 advisory」——本 change 明确不做。

### D7 · 必关闭清单与 NeedsHuman 门卡的呈现口径

**决定：** 必关闭清单的标题与说明 MUST NOT 再声称「仅 blocking/must_fix」；保留原 severity 展示值，并在说明中点出「平台契约闭合义务」这一独立于 severity 的义务来源。

`NeedsHuman` 结论下未解决的契约项 MUST NOT 因 `review_gate != RequiresRevision` 而从清单消失 —— 当前 `render_open_required_findings` 的 `review_gate` 前置判断是漏点之一。

### D8 · WorkItemPlan 作用域取值表

**决定：** 契约类例外（severity 豁免）的作用域按 `WorkItemPlanReviewScope`（`parsers.rs:304-308`）与流程类型逐项写明：

| 作用域取值 | 是否启用契约类例外 | 口径 |
|---|---|---|
| `WorkItemPlanReviewScope::Outline` | 启用（逐 finding 判定） | Outline 评审的定位主键是 `target_outline_id`；契约类例外只认「本 Work Item」finding（`target_work_item_id` 非空且指向 plan 内 Work Item）。无 Work Item target 的 outline 结构级 finding 不在例外内，维持既有 severity 门槛 |
| `WorkItemPlanReviewScope::Item` | 启用 | 评审对象即单个 Work Item；契约类 finding 按「本 Work Item」target 界定 |
| `WorkItemPlanReviewScope::Batch` | 启用 | 多 Work Item 评审；每条 finding 按各自 `target_work_item_id` 界定，互不外溢 |
| 非 SingleCandidate 的 legacy flow（通用 Story/Design 等非 WorkItemPlan 评审） | 不启用 | 既有门槛逐字不变（D2）；例外由可信入口传入的作用域授予，不由模型标签授予 |

**「本 Work Item」的定义：** ＝ 该 finding 的 `target_work_item_id` 指向的 Work Item（`target_work_item_id` 是 finding 的既有可选字段，`parsers.rs:710-712`）。

**与 `with_platform_disposition_rules` 现行行为的差异（必须声明，避免实施者收错面）：** 平台 disposition 硬化已在**通用** strict / lenient 解析器**无条件生效**（`parsers.rs:750` —— 通用 finding 解析末尾直接挂 `with_platform_disposition_rules()`；`lenient_findings.rs:136` 同口径），**只有 severity 豁免（本 change 的契约类例外）是 WorkItemPlan 专属**。实施者 MUST NOT 把平台硬化收窄到 WorkItemPlan，也 MUST NOT 把 severity 豁免扩散到通用评审。

## 共享面与排序

按全局串行表（裁决处置文件 §2.2，唯一权威）：本 change 为**序 6**，前置＝**序 5 `isolate-plan-criterion-namespace`**（本 change 消费其版本化 scope）与**序 1 `mechanicalize-plan-compile-repair`**（其 typed 判据与现场归因需先稳定）。**这些 change 目录本 change 一律不改**，仅在此登记共写文件与相对次序。

| 共享文件 | 对方 change | 相对次序 | 本 change 的动作 |
|---|---|---|---|
| `src/product/work_item_plan_policy/classify.rs`（测试面 `tests_classify.rs`） | `isolate-plan-criterion-namespace`（序 5：`classify.rs` 版本传递） | isolate → close | 本 change 消费 isolate 落地的版本化 scope，在其后叠加闭合谓词与 `scope` 参数（D2），**不改其版本语义**；`identity_for_finding` 的版本传递面不动 |
| `src/product/workspace_engine/review/routing.rs`、`routing_scope.rs`、`routing_platform_rescue.rs`、`closure_policy.rs`（新建） | `mechanicalize-plan-compile-repair`（序 1：typed 判据与现场归因） | mechanicalize → close | 本 change 只在其改动落地后叠加有效必关闭判定口径；`closure_policy.rs` 为唯一谓词实现处（D2） |
| `src/product/workspace_engine/review/routing.rs`、`routing_platform_rescue.rs` 的预算域 | `fix-author-rewrite-quota-pool`（序 2：A/B 池与 WSC-21 触发域） | quota-pool → close | 只改**判定口径**（是否有效必关闭），MUST NOT 改预算扣减与 reservation 逻辑，MUST NOT 在同一修改中同时动「A/B 余额」与「什么算 repairable」两处语义 |

**双边登记：** 与 mechanicalize / quota-pool / isolate 的次序按上表**双边**登记——对方目录的对应登记由集成 owner 统一补齐（列入 tasks 协调项）；本 change 不改对方目录。（「与 quota-pool 共享 `tests_classify.rs`」的无依据表述已删除：quota-pool 全目录无 `classify` 引用，`classify.rs` / `tests_classify.rs` 的共写方是 isolate。）

**`routing_platform_rescue.rs` 的边界：** 该文件只处理 `candidate_compile_failure:` 前缀的平台 finding，不涉模型契约类 finding；本 change 对其仅做 `classify_finding` 增参后的调用点适配（`:71`、`:267` 传 scope），**不**对其救回判定另设契约类谓词论证。

**不认领 bound / quota-pool 的 C：** 裁决 §1.1(3) —— C（机械修复实施侧）的 handler 复用 provider 函数不等于属于 A/B；裁决 §1.4 —— 不修改 `fix-author-rewrite-quota-pool/` 的 C 范围。本 change 的共享面**不包含** C 的任何面。

**总排序：** ① `mechanicalize-plan-compile-repair`（序 1）→ ② `fix-author-rewrite-quota-pool`（序 2）→ ③ `isolate-plan-criterion-namespace`（序 5）→ ④ 本 change（序 6）。

**为何必须串行：** 本 change 收紧「什么算 blocking」，对方改动收紧「blocking 由哪个池付钱」。同时落地会让「判定口径变更」与「预算域变更」纠缠在同一 diff 中，无法判定哪条语义变化导致了预算行为变化 —— 而验收必须能分别归因。本 change 的验收明确包含「其余四条仍不扣预算」与「不增加自动重写次数」，前提是预算域本身在稳定基线上。

**与归档 change 的边界：** `mechanicalize-plan-compile-repair` 与 `fix-author-rewrite-quota-pool` 目录本 change **不改**；本 change 的 `design.md` / `tasks.md` 只引用它们的工作包编号作为前置条件。

**其余文件：** `parsers.rs`、`disposition_override.rs`、`history_compaction.rs`、`prompts/review.rs`、`structured_output.rs`、`historical_review.inc.rs`、`sc_revision.rs`、`plan_outline/revision.rs` 等按 §2.2 全局表无其它活动 change 共写；谓词实现集中于 `review/closure_policy.rs`（唯一实现处），各消费面只消费、不复制。

## Risks / Trade-offs

- **[人工门数量可能增加]** 契约类 finding 若缺可验证定位，将更早落人工门 → 用 D4 的「转既有人工处理」而非「自动放行」控制；owner 可用既有排除项显式排除；**自动重写次数不增加**。
- **[模型误标 category 可能放大]** 模型把普通问题误标为 `contract_gap`/`self_contradiction` → 类别枚举**不扩大**（只有两类）；owner 排除优先且 key 不变（排除语义仍可用）；D4 保证不因此自动调用 author，最多转人工。
- **[七面漂移回归]** 未来有人在某一面另设门槛 → 谓词是**唯一**实现点（`closure_policy.rs`），七面只消费；`design.md` 与验收矩阵固定「七面同口径」为可验收结论（同一 finding 七面结论一致）。
- **[既有断言与 golden 更新]** `tests_classify.rs:24-114,248-270` 与 `disposition_override.rs:347-363` 中「hint wins」「Revise+field ⇒ Repairable」既有断言按裁决 §4.4 矩阵更新（含 `class_hint` **缺省**档）；**原真实 golden findings 不改写成新造输入**，避免用新造输入掩盖真实行为变化。
- **[owner 排除项被复活]** 平台硬化与排除的顺序颠倒，或 policy 重新解析 raw 绕过排除（含历史重建路径）→ 顺序固定为「硬化 → 排除 → 计算」，排除后不再硬化，policy 不重解析 raw；验收含「owner 排除优先且排除项不得复活」。
- **[历史产物被误动]** 历史重建路径引入新判定可能触发 provider 或撤销已确认产物 → D5 明确历史入口（重新解析型与反序列化型两类）只做展示归一，不启动 provider、不回滚已确认/已发布产物。
- **[通用评审门槛被误扩大]** 契约类例外误扩散到 Story/Design，或实施者误把无条件生效的平台硬化收窄 → D2/D8 的作用域取值表钉死：硬化无条件、豁免 WorkItemPlan 专属；验收含「例外作用域不扩到通用评审」。
- **[指纹集合成员变化引发重复判定漂移]** `non_advisory_fingerprints` 成员资格变化会影响相邻两轮集合相等判定 → 与 `mechanicalize-plan-compile-repair` 串行（见「共享面与排序」）；验收须复跑既有重复指纹与「同一指纹复评停止」回归。

## Migration Plan

- **无数据迁移**：不新增持久化字段，finding key 与 durable 已处置事实清单格式不变。
- **部署**：判定口径变更随代码发布即生效；无 feature flag 需求（收紧密判定，不涉及新外部依赖）。
- **回退策略（受限）**：回退**只能**回到 fail-safe 人工处置（即保留「有效必关闭 ⇒ 转人工」而放弃「可自动返修」），**或**整体回退未启用的新版本。**不得**静默恢复「契约缺口自动 pass」——那等于撤销 REQ-WSC-12/17 的明文 MUST。
- **历史兼容**：已确认 / 已发布的历史产物不回滚；历史展示可重新归一，但 MUST NOT 自动启动 provider。
