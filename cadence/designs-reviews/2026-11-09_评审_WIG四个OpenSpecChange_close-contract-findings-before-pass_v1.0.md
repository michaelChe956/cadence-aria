# 评审 · WIG 四个 OpenSpec Change · close-contract-findings-before-pass · v1.0

**状态：`PASS_WITH_ISSUES`**

**一句话结论：** change 的方向与决策报告 §4.1~§4.4 逐条对齐，四条被否决方案均未开口，`openspec validate --strict` 通过；但 D4「定位 + 修复许可」判据与现码/golden 冲突、并使「自动重写次数不增加」的声明在有定位的场景下不成立（P1-01），另有 6 项 P2 需在进入 `writing-plans` 前修订，不影响方向。

> 路径根：`.worktrees/feat-wig-optimize-11-08/`。被审：`openspec/changes/close-contract-findings-before-pass/`（proposal 50 / design 186 / tasks 45 / WSC delta 73 / TOP delta 37）。
> 本报告只读审阅，未改任何被审文件、未改 `openspec/specs/**`、未改其他 change 目录、未 `git add`。
> 严重度口径：P1 = 规格/设计自相矛盾或与现码冲突，实施前必须修；P2 = 缺口或含糊，实施前应修；P3 = 措辞/可执行性/登记类小问题。

## 1. 问题清单

| 编号 | 严重度 | 位置 | 问题 | 证据 | 建议修法 |
|---|---|---|---|---|---|
| P1-01 | P1 | `design.md:122-127`（D4）；WSC delta `:11,:47`；TOP delta `:7,:26-27`；`tasks.md:22` | **「可验证定位 + 既有合法机械修复证据」判据三处互相矛盾，且与现码、golden 冲突。** ① 正文用「缺定位**或**缺许可 ⇒ 人工」，场景 WHEN 用「无定位**且**不存在修复证据」，两个混合格（有定位无证据 / 无定位有证据）无定义。② 「既有合法机械修复证据」在 `work_item_plan_policy/` 内**无对应代码概念**：`machine_repair` 仅存于 `contract_prerevision.rs`/`plan_preflight.rs`/`parsers.rs`（恒 None），`classify.rs`/`evaluate.rs` 不读它。③ 现码 `fallback_class` 对 `Revise + 契约类 + contract_field 非空` 直接给 `Repairable`（不看 machine_repair），`class_hint=repairable` 则无条件 `Repairable`；`tests_classify.rs:84-95`「contract fallback」、`:58-69`「hint wins」与 golden `rep2-f1/rep3-f1/rep4-f1-annotated`（contract_gap/self_contradiction + hint=repairable + 有 field、无 machine_repair，期望 Repairable）均依赖这两条路径。若按 D4 字面（须有 machine_repair 才可自动），这些既有用例全部翻转，与 task 4.2「既有真实 golden findings 不得改写」冲突；若按现码（有 field 即可），则基线 #1 之外「有 field + `class_hint=advisory` 的契约类 suggestion」由原 Advisory（零重写）变为 Repairable ⇒ `TriggerAggregateRepair`（`routing.rs:623-637`，扣 A 池），与 proposal `:9`、design Non-Goals/D4 `:127`「不增加自动重写次数、不新增任何自动 author 调用路径」直接矛盾。 | `classify.rs:132-139,169-186`；`evaluate.rs:349-377,409-414`；`tests_classify.rs:55-117`；`fixtures/golden_findings.json`（rep*-annotated）；`grep machine_repair` 在 `work_item_plan_policy/` 零命中 | 先由 owner 二选一并写入 D4 与两份 delta：(a) 沿用现码判据——「定位」＝`contract_field` 非空（其余照旧），删除「机械修复证据」措辞，改述为「不新增 category/severity/class_hint 授予的许可」；(b) 收紧为必须有 server-attested machine_repair，则 MODIFIED 现有 TOP 场景并显式迁移 golden。无论选哪个：把场景 WHEN 与正文统一成同一逻辑，补全混合格；并把「自动重写次数不增加」改成准确表述——**不新增调用路径，但扩大既有 `TriggerAggregateRepair` 的触发集合（仍受 `max_repairs=1` 且仅 Initial 相位约束）**，验收改为「基线 #1 零 author 调用；有定位的同类 finding 至多一次自动返修」 |
| P2-02 | P2 | `design.md:105-112`（D3 面 4、「投影 vs 谓词」）；`tasks.md:16` | **谓词事实如何送达 policy 未定义。** D3 称 policy「消费解析侧已算出的有效必关闭事实」，但 `classify_finding(raw_verdict, finding)` 无 scope 参数，`ReviewFinding` 是 wire/durable 类型（本 change 禁增字段）；`classify_finding` 有 6 个调用点，均无可信 scope。D2 又要求 scope 只由可信入口传入。 | `classify.rs:124-127`；调用点 `classify.rs:70`、`routing_scope.rs:88`、`routing_platform_rescue.rs:71,267`、`prompts/review_context.rs:557`、`routing.rs:468`（经 `classify_review`） | 在 design 增一条决定：谓词为纯函数 `(finding, scope) -> bool`（scope 为枚举，由 `WorkspaceType::WorkItemPlan` 等可信会话事实在 `routing.rs`/`review_context.rs` 等调用点构造），`classify_finding` 增 scope 参数，6 调用点逐个列入 tasks；明确不经由 `ReviewFinding` 新增字段 |
| P2-03 | P2 | TOP delta `:14-22`；`design.md:116` | **「raw verdict」语义含糊，TOP 场景 2/3 在 live 路径不可达。** `routing.rs:344,444` 对已被 `effective_work_item_plan_review_verdict` 重开的 `ReviewVerdict` 序列化后取 `verdict.verdict` 当 `raw_verdict`；故 live 路径上修好 verdict 面后，基线 `pass` 到达分类器时已是 `Revise`，「raw pass 兜底」场景只在**未重开**的路径可达（`review_context.rs:557` 读持久化旧 verdict、`routing_scope.rs:88`、`routing_platform_rescue.rs:71`）。`finalize_review_verdict` 注释称持久化「provider 原始 verdict」，实为 post-parse 值。 | `routing.rs:344,444`；`disposition_override.rs:172-187`；`parsers.rs:358-359` | 在 TOP delta 注明「raw_verdict」＝`ReviewVerdict.verdict` 持久化/传入值（可能已被 effective 重开），并把场景 2 限定到「verdict 未被重开的分类入口（历史/上一轮上下文/指纹闸门）」；tasks 增对这三个入口的专项用例 |
| P2-04 | P2 | `design.md:99-112`（D3 四面）；`tasks.md:13-17` | **消费面清单不完整，作者返修输入面会把已重开的契约项仍称「建议项」。** design「当前状态」表列 6 行、文字称「四套」、D3 只改 4 面。遗漏：① `sc_revision.rs:147` 向作者声明「severity=suggestion 条目为建议项；must_fix/blocking 为必须修复项」——重开后的契约缺口在返修轮对作者仍显示为可选；② `plan_outline/revision.rs:627-638` `requires_impact_closure` 仅认 blocking/must_fix，Outline 返修反馈不会附带闭合契约；③ `routing_scope.rs:84-94` 在状态表但不在 D3。 | `work_item_split_engine/prompts/sc_revision.rs:118-148`；`plan_outline/revision.rs:608-641`；`routing_scope.rs:84-94` | D3 表补行并在 tasks 增工作包「作者输入面同口径」（至少 SC 与 Outline 两处）；若 owner 决定不改作者面，须在 design 显式记为「已知不闭合」并说明依赖复评兜底 |
| P2-05 | P2 | WSC delta `:11-13,:57-58`；`design.md:90-97`（D2） | **作用域边界未定义。** 「WorkItemPlan」在代码中覆盖 `WorkItemPlanReviewScope::{Outline,Item,Batch}`（`parsers.rs:304-308`）及非 SingleCandidate 的 legacy flow，而 REQ-WSC-17 属 single-candidate capability；delta 又写「本 Work Item」，未说明与 `target_work_item_id` 的关系（现 `with_platform_disposition_rules` 对任意 `target_work_item_id` 的 contract_gap/self_contradiction advisory 一律转 actionable）。另需声明：平台 disposition 硬化已在通用 strict/lenient 解析器无条件生效（`parsers.rs:750`、`lenient_findings.rs:136`），**只有 severity 豁免**才是 WorkItemPlan 专属，避免实施者误把硬化也收窄。 | `review.rs:187-197`；`parsers.rs:300-308,750`；`structured_output.rs:251-270`（`workspace_type == WorkItemPlan` 为可信入口） | D2 增「scope 枚举取值表」：Outline/Item/Batch × SingleCandidate/legacy 各是否启用；明确「本 Work Item」＝是否限 `target_work_item_id` 为空或等于当前 WI；声明硬化保持通用 |
| P2-06 | P2 | `design.md:153-169`（共享面与排序）；`tasks.md:38-39` | **共享面登记与对方 change 不一致。** ① `isolate-plan-criterion-namespace/design.md:135` 声明 `work_item_plan_policy/*`（含 classify）「本 change 独占」，本 change 却改 `classify.rs`/`tests_classify.rs`，双方都未登记。② 本 change 排序（quota-pool → mechanicalize → 本 change）在 quota-pool、mechanicalize 全目录零引用（`grep close-contract-findings-before-pass` 在兄弟目录 0 命中），是单向声明。③ 第 4 行称与 quota-pool「共享 `tests_classify.rs`」，quota-pool 全目录无 `classify`/`tests_classify` 引用，无依据。④ `bound-sc-generation-teaching` 对 `review/routing*.rs`、`classify.rs`、`history_compaction.rs`、`parsers.rs` 零引用（仅动 `work_item_plan_policy/types.rs`、`evaluate.rs`），本 change 未列它是对的——与 controller 任务前提不符，需以文件为准。⑤ `routing_scope.rs` 本 change 实际无需编辑：成员资格经 `classify_finding`（`:88`）自动变化。 | 见左列各文件；`mechanicalize-plan-compile-repair/tasks.md:59`（9.1 为「待验证假设，不改契约」） | 与 isolate 的 owner 协商 classify.rs 归属（建议：isolate 先、本 change 后，或拆 PR）并双向登记；删除 ③；把 ⑤ 改成「仅回归验证，不改」；第 3 行不再宣称对方「改集合比较方式」 |
| P2-11 | P2 | 决策 §4.3 第 7 行；change 全目录 | **决策 §4.3 两条落地条款未进入 change。** ① 「原始模型文本保留用于审计，但 policy 不得重新解析 raw 后绕过 owner 排除」——design/proposal/delta/tasks 对「重新解析」「绕过」「审计」零命中。② 「模型 hint 不能单独给自动调用许可」——delta 只写「不得凭 `category` 或 `severity`」（WSC `:48`、TOP `:32`），**未含 `class_hint`**；而现码 `class_hint=repairable` 即直接 `Repairable`（`classify.rs:135-138`，`tests_classify.rs:58-69`「hint wins」无 category/field 也成立），与 P1-01 同源。③ 决策 §4.3 第 1 行的 `review/closure_policy.rs`、`closure_policy_tests.rs` 在 change 中零命中（design 未指定谓词落点）。 | `决策_WIG六项owner裁决_v1.0.md:284,288,289,301`；`grep closure_policy\|重新解析\|绕过` 于 change 目录 0 命中；`classify.rs:132-139` | 在 TOP delta「不得自造修复许可」场景中把 `class_hint` 并入禁止项并说明与 P1-01 判据的关系；design 增一句「policy/routing 一律消费 owner 归一后的 verdict，不重解析 raw」并在 tasks 2.4 加对应用例；D1 指明谓词落点模块 |
| P3-07 | P3 | `design.md:160`（`routing_platform_rescue.rs` 行）；proposal `:47` | 「平台救回路径需用同一谓词，否则被救回的 finding 仍可能被免阻断」论证不成立：`rescue_platform_compile_failure_repair` 只处理带 `candidate_compile_failure:` 前缀的**平台**finding，不涉及模型契约类 finding；tasks 也无对应工作包。 | `routing_platform_rescue.rs:93-126`；`compile_failure_surface.rs:41` | 删去该行与 proposal 影响面中的文件，或补出可复现的反例 |
| P3-08 | P3 | `tasks.md:34`；WSC delta `:72` | 矩阵维度 `class_hint（三类）` 漏了**缺省**，而 TOP 场景 2 恰以「无 class_hint」为前提；矩阵与基线样本的实际覆盖不含该格。 | `tasks.md:34` | 改为 `class_hint（repairable/human_required/advisory/缺省）` |
| P3-09 | P3 | `tasks.md:39,34`；`design.md:179` | ① 6.2 前置「现场 #3 归因结论稳定后」无负责人与判据，不可执行。② 基线 #1 `contract_field=None` ⇒ `category` 存在但无稳定 ID ⇒ `identity_unstable=true`，被 `routing_scope.rs:91` 与 `evaluate.rs:320` 过滤，**不参与**「同一指纹复评停止」，只走 `UnstableFindingIdentity`（`evaluate.rs:335-347`）；5.2 的「同一指纹复评停止」专项须用有稳定 ID 的 finding 构造，不能复用基线 #1。 | `fingerprint.rs:85-122`；`routing_scope.rs:84-94` | 6.2 改为「mechanicalize 9.1 结论文档存在且已评审」；5.2 注明专项 fixture 构造要求 |
| P3-10 | P3 | `design.md:129-139`（D5 historical 入口）；`tasks.md:18`（2.6） | **historical 入口未枚举。** 代码里历史重建有两类入口：① 重新解析型 `parse_historical_review_json` / `parse_historical_work_item_plan_review_json`（走 `parse_*_value*`，调用点 `routing.rs:1279,1286`、`owner_guidance.rs:335`）；② 纯 serde 反序列化型 `deserialize_historical_review_verdict`（不重新解析，调用点 `historical_review.inc.rs:38`、`lifecycle_recovery.rs:45`、`review_context.rs:546`、`routing_scope.rs:843`），其后只靠 `normalize_review_verdict_with_store`→`renormalize_verdict_after_disposition_change` 修 verdict。对「P0-1 硬化之后持久化」的基线类记录（durable 里 disposition 已是 actionable、verdict=pass），② 路径经 D3 改造后的 renormalize 即可重开，D5 结论成立；**残余缺口**：P0-1 硬化之前持久化的记录（disposition 仍为 advisory）在 ② 路径不会被补硬化。设计与 tasks 均未列出这些调用点，也未判断该残余是否在范围内。 | `parsers.rs:90-95,260-276`；`historical_review.inc.rs:103-109`；`disposition_override.rs:91-143,172-187`（持久化的是 parse 后 verdict） | D5/2.6 列出上述 7 个调用点；在 design 声明「P0-1 之前的记录不在范围 / 或在 normalize 内补做契约类硬化」二选一；增 ①② 两类入口同结论的 fixture |
| P3-12 | P3 | `tasks.md:16,22,33`；WSC delta `:45`；TOP delta `:19` | **场景→任务映射有两处漏接。** WSC 场景「缺可验证定位时转人工且不自动调用 author」（delta `:45-48`）无任务显式映射（3.1 只映射 TOP 同名场景，5.1 仅覆盖基线样本）；TOP 场景「分类优先级与原始 verdict 来源不冲突」（delta `:19-22`）无任务映射（2.4 只映射前两条）。 | `tasks.md:16,22-23,33` | 3.1 追加映射 WSC「缺可验证定位…」；2.4 追加映射 TOP「分类优先级…」 |
| P3-13 | P3 | `design.md:118-127`（D4）；`openspec/specs/work-item-plan-single-candidate/spec.md`（REQ-WSC-11） | **转人工后的 owner 可读性未交代。** REQ-WSC-11 的 `owner_question/owner_options` 强制只在 `effective_verdict==NeedsHuman` 且 Live 时检查（`parsers.rs:395-401`）。模型原判 `pass` 的契约类 finding 被重开后经 policy 落 `HumanRequired`，模型并未提供人话提问与选项；D4 只写「转既有人工处理通道」，未说明门卡如何满足 REQ-WSC-11/15 的 owner 可读口径（预计「人工门数量增加」风险正落在此）。 | `parsers.rs:395-402`；`evaluate.rs:379-391` | D4 增一句：该类人工门卡的 owner 可读呈现沿用哪一既有机制（例如平台渲染的技术细节折叠 + 通用指引），并加一条门卡渲染回归用例 |

## 2. A~F 逐维度结论

### A · MODIFIED 场景保全 —— 通过（引用 controller 已复核，未重查）

- REQ-WSC-17：主规格 3 场景 / delta 12 场景，原 3 个（指纹闸门与返修预算只认 actionable、revise 但无 actionable 不驱作者返修、契约类缺口不得以 advisory 漏检）全在，新增 9 个。
- REQ 编号无重号；四个 change 无同一 requirement 被两个 change 同时 MODIFIED（本 change＝REQ-WSC-17 + REQ-TOP-07）。
- delta 不含「自动补 reviewer_check」类表述（controller grep 0 命中）。
- 本人仅补做两项机械核对：delta 的 `### Requirement:` 标题与主规格 `:622` **逐字相同**（MODIFIED 可匹配归档）；delta 第一段与主规格 `:624` **逐字相同**（原条款文字未被改写，仅追加段落）。

### B · 决策报告 §4.1~§4.4 落地 —— 方向全落，细节 3 处缺口

| 决策条款 | 落点 | 结论 |
|---|---|---|
| §4.1 免阻断定性、保留原 severity 与 finding_key、共享谓词、`class_hint=advisory` 不盖过 | proposal `:13-15`；design D1/D3；WSC delta `:30-38`；TOP delta `:9-12` | 已落 |
| §4.2 判定式 `requires_closure` 与顺序「硬化→owner 排除→计算」 | design D1 `:55-77`；WSC delta `:40-43`；tasks 1.3 | 已落，公式与决策原式逐项一致；「排除后不得再硬化」写成可验收场景 |
| §4.3 必关闭集合：未被排除的契约类 MUST 进入 | WSC delta `:11,:30-33` | 已落 |
| 模型 `suggestion`/`advisory`/`pass` 均不得放行 | WSC delta `:11,:33`；TOP delta `:9-17` | 已落（TOP 场景 2 的可达性见 P2-03） |
| 四面同口径 | design D3；WSC delta `:50-53`；tasks 2.5 | 已落，但「四面」不完整（P2-04）、事实送达 policy 的机制缺失（P2-02） |
| 缺可验证定位/许可 ⇒ 转既有人工处理，不得自动调用 author | design D4；WSC delta `:45-48`；TOP delta `:24-32` | **已落但判据自相矛盾（P1-01）** |
| owner 排除优先 | design D1 顺序、Risks `:176`；WSC delta `:40-43` | 已落；现码 `disposition_override.rs:47-48` 顺序正确（先 `force_owner_excluded…` 后 `renormalize…`），design 与之相符 |
| 范围外 advisory 不变（仅 `contract_gap`/`self_contradiction` 两类） | design D6；WSC delta `:65-68`；tasks 4.1 | 已落 |
| 通用 Story/Design 门槛不扩大；scope 由可信入口传入、不由模型标签授予 | design D2；WSC delta `:55-58`；tasks 1.2 | 已落；取值表缺失（P2-05）。可信入口实证：`structured_output.rs:251` 以 `workspace_type == WorkItemPlan` 分流到 `parse_work_item_plan_review_value`，该事实非模型可控 |
| 历史只重新归一、不启动 provider、不回滚已确认/发布产物 | design D5、Migration `:186`；WSC delta `:60-63`；tasks 2.6 | 已落；入口枚举缺（P3-10）。现码历史读路径为纯读（`historical_review.inc.rs:103-109`；`disposition_override.rs` 模块注释 `:16-17`「不落 durable」），与声明相容 |
| §4.3 其余两条：raw 不得绕 owner 排除、hint 不得单独给自动许可、谓词落点模块 | —— | **未落（P2-11）** |
| §4.4 基线五条 fixture、矩阵、既有回归位置 | tasks 5.1/5.2/4.2；WSC delta `:70-73` | 已落；矩阵 `class_hint` 漏缺省（P3-08） |

**四条被否决方案均未开口（逐条核对 delta/design/tasks 全文）：**
1. 只升 severity：WSC delta `:35-38` 明令 `suggestion` 不得改写、key 逐字不变；tasks 1.1 有「不改写 severity 与 message」用例。
2. 只放宽 verdict 函数：delta `:9,:50-53`「四面 MUST 同口径」覆盖其余三面；但作者输入面等遗漏消费面仍会漂移（P2-04）——属**覆盖不全**而非**开口**。
3. 推迟到二期：全目录无「二期/后续/待定」式推迟（grep 仅命中否决理由与 `tasks.md:42` 的 Workflow follow-up 标题）。
4. 削弱规格承认 pass：delta 无任何放宽 `pass` 的条款；Migration `:185` 明令回退「不得静默恢复契约缺口自动 pass」。

> 注：任务书写「三条被否决方案」，决策 §4.4 实际列出**四条**，本报告按四条核对。

### C · 与主规格字面冲突 —— 无硬冲突，2 处需澄清

逐条比对 REQ-WSC-12 / 13 / 17 及 REQ-TOP-01 / 02 / 04：
- **REQ-WSC-12**（`:593`「只把 actionable 计入阻断判定、返修触发与必关闭清单」）：delta 以「有效 disposition＝平台硬化后」为前提，相容。场景「模型把 advisory 误标为 must_fix 也不阻断」（`:607-610`）与新增例外正交（例外仅作用于契约类 category 且 effective disposition 仍为 actionable），相容。
- **REQ-WSC-13**（`:643` key＝`sha256(severity+\u{1}+message)`）：delta `:38` 逐字复述，且设计不改 severity/message，key 不变。相容。
- **REQ-WSC-17**：第一段逐字保留；新增段落与原 3 场景无冲突（见 A）。
- **REQ-TOP-01**「分类 SHALL 使用 raw_verdict」vs TOP-07 场景 3「对有效必关闭的契约类 finding，分类 MUST NOT 由 class_hint 或 raw verdict 的兜底决定」：两句并存时，若 raw_verdict 为 `needs_human`/`revise` 而 finding 要求有效必关闭，**分类到底取哪个 class** 在 delta 内未给出正向规则，只给了「不得为 Advisory」的负向约束 → 见 P1-01（判据）与 P2-03（raw 语义）。属需澄清，非硬冲突。
- **REQ-TOP-02** 优先级「转换预算耗尽 → 重复指纹 → 返修需求 → human_required → valid」：design `:114` 声称新增分支落在 `human_required` 内、不改顺序。核对 `evaluate.rs:261-397` 属实（预算→verification 新 finding→重复→可返修→human→valid），相容。
- **REQ-TOP-04** Verification 新 finding 停止规则：初评相位把非 advisory 指纹写入 `original_fingerprints`（`evaluate.rs:433-440`）；本 change 使契约类 finding 在初评即为非 advisory，因此进入集合，规则本身未变。**副作用须知**：此前被判 Advisory 的契约类 finding 若首次出现在 Verification 相位，现将触发 `VerificationNewFindings`→人工门（delta TOP `:37` 已声明不改该规则，但「人工门数量增加」的具体来源应在 Risks 点名）。
- 主规格 TOP 场景「首次 aggregate repairable 自动返修一次」（`:139`）：与 P1-01 的第二分支（有定位的契约类 suggestion 变 Repairable）相互作用，见 P1-01。

### D · 与其他 change 的排序一致性 —— 本 change 内部自洽，对外登记不对称

| 对方 change | 对方文件实际声明 | 本 change 声明 | 一致性 |
|---|---|---|---|
| `fix-author-rewrite-quota-pool` | 触及 `review/routing.rs:623-637,717-721`、`routing_platform_rescue.rs:124`、`routing_scope.rs` reservation 预领；**未提及** `classify.rs`/`tests_classify.rs`/本 change | 排「对方先」；称共享 `tests_classify.rs` | 区域不重叠（本 change 在 `routing.rs:445-472`，对方在 `:623+`），顺序可行；但「共享 tests_classify」无依据，且对方未登记（P2-06） |
| `mechanicalize-plan-compile-repair` | 仅 `tasks.md 9.1`（待验证假设，不改契约）；零引用本 change | 排「对方先」；称对方「改集合比较方式」 | 对方并未承诺改动；排序前置条件不可执行（P2-06 ⑤、P3-09） |
| `bound-sc-generation-teaching` | 触及 `models/workspace.rs`、`work_item_plan_policy/types.rs:166`、`evaluate.rs:203-222`、`planning_failure.rs`；**不触** `review/routing*.rs`、`classify.rs`、`parsers.rs`、`history_compaction.rs` | 未列 | 本 change 未列是对的。**与任务书前提不符**：未发现它与本 change 共享 `routing.rs`/`routing_scope.rs`/`classify.rs`；仅与 `evaluate.rs` 邻近（本 change 不改 `evaluate.rs`） |
| `isolate-plan-criterion-namespace` | `design.md:135`：`work_item_plan_policy/*`（含 classify）「本 change 独占」 | 改 `classify.rs`/`tests_classify.rs`，未提 isolate | **冲突**（P2-06 ①）。指纹层面无漂移：`identity_for_finding` 对 `category=Some` 的 finding 不以 `class` 入哈希（`fingerprint.rs:85-122`），契约类均带 category，故本 change 改 class 不改变其指纹，isolate 的指纹代际不受影响 |
| `explicit-sc-author-effort` | 与本 change 无共享文件 | 未列 | 一致 |

### E · 代码接地 —— design/proposal 所引位置全部真实且语义相符，2 处引用口径需订正

| 引用 | 实测 | 结论 |
|---|---|---|
| `web/workspace_ws_types/review.rs:187-197` `with_platform_disposition_rules` | 仅 `Advisory + ContractGap/SelfContradiction → Actionable`，不碰 severity；`:145-159` key 算法与 design 复述一致 | 相符 |
| `parsers.rs:427-455` `effective_work_item_plan_review_verdict` | `Pass` 且存在 `is_actionable() && severity∈{Blocking,MustFix}` → Outline/Item→`Revise`、Batch→`ReviseBatch` | 相符；该函数是 verdict 面唯一判点，改它的落点明确 |
| `prompts/history_compaction.rs:85-122` `render_open_required_findings` | `:91` 要求 `review_gate == RequiresRevision`；`:101` `is_actionable() && is_required(severity)`；`is_required`（`:551`）只认 Blocking/MustFix；标题 `:118`「未关闭 blocking/must_fix finding（全文）」 | 相符，D7 所称两处漏点（gate 前置、标题）均属实 |
| `review/disposition_override.rs:43-77` | `:47-48` 先排除后 renormalize；`:69-77` `has_actionable_blocking` 只认 Blocking/MustFix；`:91-143` 分「WorkItemPlan 扩展分支」与「通用分支」 | 相符；D3 面 3「非本作用域不偷改」可由现有双分支结构实现 |
| `work_item_plan_policy/classify.rs:124-185` | `:132` 非 actionable→Advisory，否则 hint 优先，否则 `fallback_class`；`Pass→Advisory`（`:172`）；`Revise+契约类+非空 field→Repairable`（`:173-183`） | 相符，同时是 P1-01 的证据 |
| `routing_scope.rs:81-91` | 实为 `:81-94`（fn 体至 `:94`），少算 3 行且遗漏 `:91` 的 `identity_unstable` 过滤 | 微偏差（P3-09 已含） |
| `routing.rs:445-472` | `parse_review_envelope` + `classify_review` 调用，相符；但 `:344,:444` 以序列化后的 `ReviewVerdict.verdict` 作 `raw_verdict`（P2-03） | 位置相符，语义需订正 |
| `structured_output.rs:236-274` | 可信 scope 分流点（`:251`），相符 | 相符 |
| `historical_review.inc.rs:103-108` | `verdict.map(normalize_review_verdict_with_store…)`，相符 | 相符 |
| `routing_platform_rescue.rs`（design `:160`、proposal `:47`） | 仅处理平台编译失败 finding | **论证不成立（P3-07）** |
| `prompts/review.rs:178-182` | 调用 `render_open_required_findings`，相符 | 相符 |

遗漏的真实消费点（不在 design 表内）：`sc_revision.rs:118-148`、`plan_outline/revision.rs:627-638`、`prompts/review_context.rs:557`、`routing_platform_rescue.rs:71,267`（P2-04 / P2-02）。

### F · tasks 门禁与可执行性 —— 结构合格，映射有 2 处漏接

- tasks 全为高层工作包（5 组 + 集成验收），每条标注 `映射 requirement`，且不下沉到文件/命令（`tasks.md:3` 明示交 `writing-plans`）。`cadence/project-rules/delivery-completeness.md` 规则 1（owner 需求含「按钮/页面/界面/入口/能看到」措辞时 tasks 必须含前端任务与 UI 验收）：决策 §4 的 owner 措辞为「免阻断/必关闭/不改 severity」，**未触发**该规则，故无需前端任务；但若 P3-13 的修订引入新的门卡呈现，则规则 1 随即触发，须同步补 UI 验收用例。
- 验收可执行：1.1/1.2/1.3/2.x 为单元/一致性用例；4.2/6.3 为既有回归全绿；5.1 基线五条 fixture、5.2 覆盖矩阵均有明确断言。
- 缺口：场景→任务漏接（P3-12）；5.2「同一指纹复评停止」须用稳定 ID finding（P3-09）；6.2 前置不可执行（P3-09）；缺「作者输入面」「事实送达 policy 的参数化与 6 个调用点」「谓词落点」三类工作包（P2-02/04/11）。

### validate / show 真实输出

```text
$ openspec validate close-contract-findings-before-pass --strict
Change 'close-contract-findings-before-pass' is valid
EXIT=0
```

`openspec show close-contract-findings-before-pass --deltas-only`（文本模式）：本 CLI 版本在文本模式下**输出的是 proposal 全文**（`# Proposal … ## Why … ## What Changes … ## Capabilities … ## Impact`，EXIT=0），未单列 delta；改用 `--json --deltas-only` 取得 delta 结构：

```text
$ openspec show close-contract-findings-before-pass --json --deltas-only
close-contract-findings-before-pass 2
work-item-plan-single-candidate MODIFIED | Modify requirement: 非 advisory 指纹闸门、transition budget 与 `revise` 归一 MUST 与阻断判定同口径… | scn= 12
work-item-typed-outcome-policy ADDED | Add requirement: 中央策略 SHALL 以「有效必关闭」判定消费 finding：`class_hint=advisory` 与 raw `pass` fallba… | scn= 6
```

（以上第二段为对 JSON 的字段摘取：`id`、`deltaCount=2`、各 delta 的 `spec`/`operation`/`description` 前缀与场景数；`deltaCount=2`、12 与 6 个场景与文件实测一致。）

## 3. 已核验通过项

- `openspec validate close-contract-findings-before-pass --strict` 通过（原始输出见 §2）。
- WSC delta 的 MODIFIED 标题与主规格逐字相同、第一段逐字保留；REQ-TOP-07 在主规格 `work-item-typed-outcome-policy` 现有 TOP-01~06 之后，不与其他 change 重号（controller 已复核）。
- 判定式、三段顺序、`severity` 是充分非必要条件、不改写 severity/message 因而 finding key 不变——与决策 §4.2 逐项一致；`review.rs:145-159` key 算法与 design 复述一致。
- owner 排除优先：现码 `disposition_override.rs:47-48` 已是「先排除、后 renormalize」，且 `force_owner_excluded_findings_advisory` 在 `:58-66` 以现算 key 匹配，design 所述与现码相符；历史读路径不落 durable（`disposition_override.rs:16-17`），与「历史只展示归一」相容。
- 例外类别枚举仅 `contract_gap`/`self_contradiction` 两类：与 `review.rs:190-192`、`classify.rs:176` 现有枚举相同，未扩大。
- 通用 Story/Design 路径：`structured_output.rs:251-270` 以 `workspace_type` 分流，通用入口走 `parse_review_value`（`parsers.rs:175-247`）并经 `review_gate_for_findings` 的旧口径（`:862-889`），可在不改该函数的前提下保持「旧门槛逐字不变」。
- 指纹层：契约类 finding 带 category，`FindingFingerprint::identity_for_finding` 不以 `class` 入哈希（`fingerprint.rs:85-122`），class 变化不改变其指纹，无历史指纹漂移。
- 与 REQ-TOP-02 优先级顺序一致（`evaluate.rs:261-397`）；Verification 新 finding 规则本身未被改动（`evaluate.rs:281-309`）。
- 范围外 advisory：`classify.rs:132-134` 的 `!is_actionable() → Advisory` 先于 hint，本 change 不触及，D6 成立。
- 受影响文件清单中，除 `routing_platform_rescue.rs`（P3-07）外，其余真实存在且确与判定口径相关。
- 未改被审文件、`openspec/specs/**`、其他 change 目录；未 `git add`。

## 4. 未核实缺口

- **未运行任何 `cargo` 构建/测试**（评审任务不含）：所有「现码行为」均为静态阅读，未以运行结果佐证。
- **`routing.rs` 消费的 verdict 是否已含 owner 归一**：`evaluate_work_item_policy_route` 接收的 `verdict` 来自 `finalize_review_verdict`→`apply_session_owner_dispositions` 链路（`disposition_override.rs:175-197`），本人未逐个核实所有调用方都经该链路（决策 §4.3 第 289 行「policy 不得重新解析 raw」依赖这一点，见 P2-11）。
- **legacy（非 SingleCandidate）WorkItemPlan flow** 是否会走到契约类例外：仅从 `parse_work_item_plan_review_value` 入口推断为「会」，未核对各 flow 的 `flow_kind` 与 scope 构造（P2-05 的取值表需实施侧补）。
- **P0-1 之前持久化的历史记录**（disposition 仍为 advisory）占比与是否需要补硬化，无数据；仅指出入口差异（P3-10）。
- **`finding_disposition_tests.inc.rs:31,55,106,173-198` 与 `history_compaction.rs:692-734`**（决策 §4.4 列为既有回归）：确认文件存在，未逐行核对其断言是否会被本 change 翻转；`history_compaction.rs:723-733` 的 `advisory finding must not enter the required-closure list` 用例与新标题/新口径可能需随改。
- **`bound-sc-generation-teaching` 全部 spec/tasks 文件**：仅对其 design 与全目录做了 `grep`，未逐文件通读；「零引用 routing/classify/parsers/history_compaction」结论基于全目录 grep 命中为空，但与任务书「三者共享 routing.rs、routing_scope.rs、classify.rs」的前提相左，建议 controller 复核该前提来源。
- **作者输入面**（`sc_revision.rs`、`plan_outline/revision.rs`）：已确认其门槛仅认 actionable / blocking+must_fix，未核对其余作者提示路径（draft/batch 修订）是否有同类门槛。
- **前端/UI 影响**：未核对门卡前端对「必关闭清单标题」「NeedsHuman 门卡」的渲染依赖；若 D7 改变文案，需另核 `web/src` 断言。

## 5. 建议修订顺序（供 controller 派工）

1. **P1-01**：owner 二选一裁决 D4 判据（沿用现码「有 `contract_field` 即可」或收紧为 server-attested machine_repair），随后统一 D4/两份 delta/tasks 3.1 的逻辑与「自动重写次数」措辞。
2. **P2-02 / P2-03 / P2-04 / P2-11**：同属「事实如何一致送达各消费点」，一并补 design 的「谓词形态与 scope 参数」决定，并扩充 D3 表与 tasks 工作包。
3. **P2-05 / P2-06**：补 scope 取值表；与 `isolate-plan-criterion-namespace` 协商 classify.rs 归属并双向登记，删除无依据的共享声明。
4. P3-07~P3-13 为措辞与映射修补，可随上述一并处理。

> 以上修订均不改变本 change 的方向或四条被否决方案的结论，故状态定为 `PASS_WITH_ISSUES` 而非 `BLOCKED`。
