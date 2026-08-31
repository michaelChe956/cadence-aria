# Tasks

> 本 change = 阶段 2（单仓 C′ MVP）。阶段 1（策略层/终态矩阵/持久化）见 `workitem-typed-outcome-policy`，本阶段复用其产物。阶段 3 才实现最小 WS 协议与对话流人工门；阶段 4 才处理多仓扩展与旧协议删除。`advance` 的 workitem pipeline 签名属于阶段 3，本 change 不定义。

## 1. 字段来源矩阵与 grammar

- [x] 1.1 建立 `openspec/changes/rearch-workitem-plan-pipeline/field-source-matrix.md`：逐字段覆盖 `CanonicalWorkItemContract`、`WorkItemDraftVerificationPlan`、trusted commands、target repository、publication provenance。每行标注唯一来源（markdown 明写 / session 与已确认上下文 / 编译器确定性派生 / compile 事务运行时生成）、缺失行为、禁止的第二来源、覆盖测试；handoff 仅描述 schema，运行时值仍由 coding 后 HandoffRevision 产生
- [x] 1.2 定义 `work-item-plan.md` grammar（稳定 section、EARS 行语法、ID、scope、依赖、verification、handoff schema、blocker、traceability）；结构化区域未知字段失败关闭，仅 Notes/Rationale 区容忍自由文本
- [x] 1.3 编写 `openspec/changes/rearch-workitem-plan-pipeline/fixtures/work-item-plan-rep4.md`：backend/frontend/integration 三 item 的完整 source fixture；按 1.1 的每一字段覆盖全部契约要素。另建**仅限确属 grammar/lowering** 的 compiler diagnostic fixture，并为每例标注输入行、字段、期望诊断；不得把 reviewer finding 自动改写为 compiler diagnostic
- [x] 1.4 为 grammar/source linter 写未知结构化字段、缺失字段、ID/EARS 非法、Notes/Rationale 自由文本允许的测试

## 2. 编译器（markdown → 逐 item typed IR）

- [x] 2.1 parse：markdown → AST，文档级诊断必须含行号、字段与恰好一个修复示例
- [x] 2.2 lower：AST → 顶层 `PlanCandidateIr { source_revision_hash, compiler_version, items: Vec<PlanCandidateItemIr> }`；逐 item `PlanCandidateItemIr { target_repository_id, contract: CanonicalWorkItemContract, verification_plan: WorkItemDraftVerificationPlan, trusted_commands }`。顶层 provenance 不得重复置于 item；不能以临时 JSON 或第二来源补齐字段
- [x] 2.3 golden 分两类：阶段 1 的 14 条 classifier golden（rep2/3/4 的 9 条、rep1 round-1 的 2 条 Advisory、3 个带人工 class_hint 的变体）按其阶段 1 预期分类；仅确属 grammar/lowering 的**9 个 reviewer finding 映射集合**建对应 markdown 输入、错误行、字段与期望 compiler diagnostic。其余 finding 明确仅为 prompt few-shot 素材，不伪装为 compiler diagnostic；3 个带人工 class_hint 的变体只服务阶段 1 分类器
- [x] 2.4 完整 lowering 验证：`fixtures/work-item-plan-rep4.md` 编译产物通过既有 `work_item_split_validator` 全量校验零 Error；复用阶段 1 golden fixture 的分类边界而不修改 validator 规则
- [x] 2.5 publish 前 freshness 校验 source_revision_hash + compiler_version，写入不可变 publication provenance；coding 只读取该 immutable runtime binding，执行期绝不解析 markdown

## 3. compile 事务输入抽取（高风险边界，独立审查门）

- [x] 3.1 先补 legacy parity/characterization 测试：按语义产物、事务状态序列、failpoint/recovery 三层比较；语义产物比较规范化后字段，忽略 `next_compile_id()` 与 `Utc::now()` 导致的动态 ID/时间，不要求原始 JSON 字节相同
- [x] 3.2 定义唯一注入模型 `InitialPlanCompileInput`：

```rust
pub struct InitialPlanCompileInput {
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub previous_plan: IssueWorkItemPlan,
    pub active_index: WorkItemPlanDraftActiveIndex,
    pub outline_candidate: WorkItemPlanOutlineCandidateDto,
    pub outline_order: Vec<String>,
    pub draft_records: Vec<WorkItemDraftRecord>,
    pub logical_targets: Option<BTreeMap<LogicalRepositoryId, String>>,
    pub repository_id: String,
    pub change_order: Vec<LogicalRepositoryId>,
    pub compile_id: String,
    pub now: String,
}
```

  `logical_targets` 必须对齐 `src/product/workspace_engine/draft_batch/compile_support.rs` 的现状：`Option<BTreeMap<LogicalRepositoryId, String>>`，使 logical repository 与其已解析 target 映射一起注入，纯核心不得猜测 path。legacy adapter 从现有 lifecycle/store 取得 `previous_plan`、active index、latest outline candidate、accepted active drafts、logical target/repository id 与 confirmed-design change order，并由外层注入 compile_id/now；IR adapter 从 validated `PlanCandidateIr` 的逐 item `PlanCandidateItemIr` 组装同一模型，禁止重写事务语义。

  `CompileStores`、`PreparedInitialPlanCompile` 及 prepare/execute 的字段、所有权和最终签名在实施时以 `src/product/workspace_engine/compile.rs` 的现状为准；本计划不锁定尚不存在的字段，唯一约束是 `prepare_*` 仅做确定性 projection、validator 输入和 transaction draft 构造，`execute_*` 复用既有 put/commit/finalizer/recovery 原子语义。ID 分配、时钟读取、store/lifecycle 读取在 adapter 外层完成，作为 input 注入；不得把 store 读取重新塞回纯核心。
- [x] 3.3 以现有 `WorkItemPlanCompileFinalizerCheckpoint::{PlanSummaryPrepared, FirstChildSessionEnsured, FirstChildBindingEnsured, FirstChildContextPrepared, CompileReportPersisted}` 逐项执行 failpoint/recovery parity；确认提取前后规范化产物、持久化状态序列、finalizer 结果一致
- [x] 3.4 新路径 IR adapter：validated IR → `InitialPlanCompileInput` → 同一 prepare/execute 核心；对 legacy 与 IR adapter 分别做 parity 测试

## 4. prompt 重构与复评 invocation

- [x] 4.1 author prompt 改 markdown 语法 + 边界约束 + 判例 few-shot（grammar 内容内联进 prompt——provider 工作目录是目标仓库，不能只给 aria 仓内路径）
- [x] 4.2 删除行为教学层（B 层），仓库注入内容承担单一来源
- [x] 4.3 【阶段 1 契约残留引用】复用阶段 1 的持久化 `ReviewPhase` / `ReviewInvocationScope`：服务端生成 Initial 或 Verification scope 及 canonical digest；prompt builder 从 scope 构造初评/复评指令，不允许 provider/campaign 自行提供范围。字段与 digest 算法以阶段 1 change 为唯一来源，本阶段不重定义
- [x] 4.4 【阶段 1 契约残留引用】parser/分类器校验复评 finding 仅能重现 original fingerprints，且本 invocation 的 mechanical report 必须存在且匹配；不得做 changed-path 归因。缺机械报告、指纹越界、scope digest 不符均为 fatal/protocol error；复评后不再自动返修
- [x] 4.5 prompt 与 scope 单测：必需内容存在、已删内容不存在、字节上限、scope 持久化/重连、范围违例失败关闭

## 5. 单候选 engine 端到端（flag 内，单仓）

- [x] 5.1 新路径 engine：复用阶段 1 策略层、终态矩阵、`RunHistory` durable counters、`ReviewInvocationScope` 与 human gate snapshot；副作用幂等（CAS/事务先持久化 next state，再启动 provider）
- [x] 5.2 运行时自选 batch/serial（内部决策，不暴露为 WS/UI 决策）
- [x] 5.3 多仓边界：实施确定性 preflight。在尚未创建/持久化新路径 session、markdown、IR、run history 或 transaction，且 provider 尚未启动前才允许选择 legacy；preflight 成功后任一持久化或 provider 启动后的失败一律写新路径 durable fatal/recoverable，原 `flow_kind` 不变，绝不静默 fallback
- [x] 5.4 恢复测试逐例写初始 JSON/transaction、重启或重连动作、预期 state、provider 启动次数、不可变事件断言：generate/repair 中断、approval pending 重连、已完成重放、compile/finalizer recovery

> **§1-§5 勾选证据（2026-08-31 终审，scout 逐项核验 + controller 复核，实施台账 = `.superpowers/sdd/2026-08-27_计划文档_WorkItem阶段2单候选C′MVP_v1.0/progress.md`）**：20 项中 17 项有实现锚点（grammar `STRUCTURED_KEYS`/诊断词汇表、`parse_work_item_plan`/`lower_work_item_plan`/`PlanCandidateIr`、freshness `verify_publish_freshness`、`InitialPlanCompileInput` 纯 prepare 核心、finalizer checkpoint 恢复矩阵、IR adapter parity、prompt 内容/预算断言、B 层删除负断言、`ReviewInvocationScope` 复用、preflight 禁静默 fallback、恢复矩阵与 provider-start dedup）；3 项备注勾选：
> - **2.3**：原文「9 个 reviewer finding 映射集合建对应 compiler diagnostic」实测归入 0 条（9 条全为 prompt few-shot，`compiler_fixture=null`），实际交付 4 条独立 grammar/lowering fixture——与 §6.3 核对结果同源（2026-08-31 表述对齐）。
> - **2.4**：rep4 编译产物过 validator 零 Error 已测；但 budget/session-fit/verification-source 等 6 条 validator 规则因 SC 恒定投影结构性不可触发（属规则未参与而非通过），登记为 residual（阶段 3 验证或专项轮覆盖）。
> - **5.2**：原「运行时自选 batch/serial」被 2026-08-29 简化裁决取代（单次 provider 生成完整 plan）；现为编译后内部诊断 selector（`select_internal_generation_mode`），不暴露为 WS/UI 决策，符合 REQ-WSC-01 内部选择语义。
> - 其他 residual 备注：4.4 category/class_hint 机械白名单绑定风险；5.1 生成段 crash recovery 为 MVP 已知边界；5.4 个别事件字段为间接断言。均不推翻已实现判定。

## 6. 验收（需操作者授权真实运行）

- [x] 6.1 campaign driver 适配 + 操作者授权
- [x] 6.2 codex + pi 各 1 案例实跑（naruto 单仓）：2/2 Confirmed、`initial_review_count ≤ 1`、`verification_review_count ≤ 1`、`repairs_used ≤ 1`
- [ ] 6.2-时长子项 单案例时长 ≤12 分钟——**未通过**：pi 实测 938.04s（15.63 分钟）；用户裁决（2026-08-31，选项 c）不放宽子项、pi 登记为已知例外、**旧协议本阶段不删除**（详见下方收口块与 design.md 退役门裁决节）

> **6.2 达标收口（2026-08-31，用户裁决 A：按本节判据原文收口）**：codex 与 pi 各 1 案例 Confirmed 实证达标，逐项亲验证据如下（campaign `result.json`，durable 证据勿删）：
>
> | 判据 | codex(r26) | pi(r26-rep2) | codex(r28，规则注入后) |
> |---|---|---|---|
> | `session_status` | confirmed | confirmed | confirmed |
> | `confirmed_count` | 1 | 1 | 1 |
> | `flow_kind` | single_candidate | single_candidate | single_candidate |
> | verdict / round | pass@r1 | pass@r1 | pass@r1 |
> | `repairs_used` | 0 | 0 | 0 |
> | `provider_start_count` | 1 | 1 | 1 |
> | `legacy_decision_messages` | [] | [] | [] |
> | must_fix validator findings | 0 | 0 | 0 |
> | 时长 | 142.96s | 938.04s ⚠️ | 627.14s |
>
> durable：`issue_0075`/`workspace_session_0080`（codex r26）、`issue_0077`/`workspace_session_0085`（pi r26）、`issue_0084`/`workspace_session_0095`（codex r28，中文 plan 验收）。产物：`/tmp/aria-phase2-results/r26/{codex,pi-rep2}/`、`/tmp/aria-phase2-results/r28/codex-rep2/`。
>
> **时长子项裁决（2026-08-31，用户选 c）**：pi 938.04s = 15.63 分钟 > 「≤12 分钟」——**该子项判为未通过**，不放宽、不改口径、不删除。推论：REQ-WSC-07 退役门未全部满足 → **旧协议（generation-mode 决策、逐段确认消息、review_decision 双选项语法）本阶段不得删除**。pi 的 Confirmed 仍为合法功能实证，仅性能子项偏离。codex 两轮（142.96s / 627.14s）均在门内。详见 design.md「退役门时长子项裁决」节。
>
> **kimi 状态登记（不入 6.2 判据）**：本节判据原文仅要求 codex + pi；§7.5/§8.5 阶段性写的「三 provider 全 Confirmed」为实施期加严口径，随本次收口回归本节原文。kimi 五轮失败根因均已定位并修复（`d192755e` 能力原子拆分教学 + nonce 首行教学 + SC 前言确定性修剪；`61876332` 半角分隔符教学），但**修复后的验证轮未实跑**（r28c-rep1 hard_timeout 900s、rep2 用户叫停）。用户裁决：kimi 归为「待验证增强」，延后至 6.4 后的专项测量轮与 95% 成功率测量一并实跑；后续验证优先使用 codex 与 pi。不得据此声称 kimi 达标或不达标。
- [x] 6.3 核对阶段 1 的 14 条 classifier golden（11 原始 + 3 个带人工 class_hint 的 Repairable 变体）；只核对 9 个 reviewer finding 映射集合中已明确归入 grammar/lowering 的 compiler diagnostic golden。其余 reviewer finding 只作为 prompt few-shot 素材；usage/token 基线对比（含注入内容占比测量）

> **6.3 核对结果（2026-08-31，controller 亲验，非 worker 自报）**
>
> **表述对齐（用户同意，不改判据）**：本节原文「9 个 reviewer finding 映射集合中已明确归入 grammar/lowering 的 compiler diagnostic golden」实测归入数量为 **0**：`openspec/changes/rearch-workitem-plan-pipeline/fixtures/reviewer-finding-channel-map.json` 中 9 条全部 `channel=prompt_few_shot`、`compiler_fixture=null`，由 `src/product/work_item_plan_compiler/tests/reviewer_finding_channel_boundary.rs:54-116` 守卫。因此实际核对的 compiler diagnostic golden 是另外 **4 条独立 grammar/lowering fixture**（`openspec/changes/rearch-workitem-plan-pipeline/fixtures/compiler-diagnostics/` + `expected.json`），这与本节「其余 reviewer finding 只作为 prompt few-shot 素材」的意图一致。
>
> | 核对项 | 命令 | 结果 |
> |---|---|---|
> | 14 条 classifier golden | `cargo test --locked --lib golden_findings_classify_to_the_expected_typed_outcomes` | ok 1 passed（测试内含 `assert_eq!(fixtures.len(), 14)`；11 原始 `provider_raw` + 3 条 `annotated_variant`）|
> | channel map 边界 | `cargo test --locked --lib reviewer_finding_channel_boundary` | ok 1 passed（守卫 14/11/3 数量、rep2/3/4 筛出 9 条、channel 与 compiler_fixture 字段事实）|
> | compiler diagnostic golden（4 条 fixture）| `--lib fixtures_diagnostic_sources_have_one_static_target_error`、`--lib fixtures_expected_json_has_the_diagnostic_schema`、`--lib source_linter_matches_every_diagnostic_fixture_field_by_field` | 三条均 ok 1 passed |
> | SC author 预算实测 | `--lib work_item_plan_markdown_prompt_inlines_grammar_boundaries_and_real_findings -- --nocapture` | ok；**`SC author prompt bytes=18934 margin=66`**（预算 19,000，余量 66B——见 §8 defer 账第 1 条）|
> | 全量 lib | `cargo test --locked --lib` | 2916 passed / 2 failed → 两条均为已登记 flaky 家族（`single_candidate_recovery_finalizer_checkpoint_matrix`、`task_3_4_single_candidate_crash_boundaries_reuse_durable_reservation`），定向复跑均 ok 1 passed，定性为并行时序 flaky，非产品缺陷 |
>
> **usage/token 基线对比**（数据源：campaign `result.json` 的 `usage_by_role`）：
>
> | 轮 | provider | 阶段 | reviewer input | reviewer output | cache_read |
> |---|---|---|---:|---:|---:|
> | r26 | codex | 注入前 | 27,753 | 1,313 | 25,344 |
> | r28 | codex | 注入后（`67aa456f`）| 29,374 | 1,177 | 27,392 |
> | r26 | pi | 注入前 | 474 | 1,728 | 33,408 |
>
> reviewer input 变化 +1,621 tokens（+5.84%）。**注入内容占比（静态字节口径）**：注入段合计 1,662B = language.md 全文 222B + 优先规则句 715B + code-usage 摘要 381B + code-reading 摘要 340B + 模板分隔 4B（`src/product/work_item_split_engine/prompts.rs:67-92`）；占预算 fixture prompt（18,934B）**8.78%**，占 r28 实跑 author prompt（33,942B，源 `ws.jsonl` 的 `timeline_node_002_prompt`）**4.90%**。
>
> **已登记缺口（defer，用户同意）**：`usage_by_role` 仅含 `reviewer`，无 `author` 条目（driver 从 `execution_event.kind=usage` 采集，author 侧不落 usage 事件）→ **author 侧 token 增量无实测值，不以缺失数据推算**。本节 usage 基线以 reviewer 侧 token + 静态字节占比结项；author 侧 usage 埋点归入阶段 3 可观测性工作，不阻塞 6.4。
- [x] 6.4 验收报告落盘；达标后移交阶段 3 立项（对话流人工门与 `advance` 接口），未达标不得退役旧协议

> **6.4 收口（2026-08-31）**：验收报告已落盘 `cadence/reports/workitem-coding-campaign/reports/2026-08-31_阶段2验收_单候选C′MVP实测报告.md`（263 行 9 节，controller 亲验无越界达标声明）；阶段 3 立项移交清单随本 change 归档交接输出（对话流人工门、`advance` 接口、SC manual revision、平台级规则设计 defer 项、kimi 待验证、95% 专项测量、发布后变更管理）；旧协议按 6.2-时长子项裁决（1c）**不删除**。

> **2026-08-29 简化裁决补记**（用户批准）：工作包 5.2 的「轻量 outline→计数→selector→完整 author」两阶段生成简化为**单次 provider 生成完整 plan**；selector 降级为编译后内部诊断。工作包 2.5/5.x 期间落地的 outline 派生受信目录链（catalogfix b2aaf24e 及其后续教学）标记 superseded——trusted_commands 改为 plan Verification 段声明确定性投影，授权锚=plan 审批门（见 design.md 架构简化裁决节与 field-source-matrix FSM-024/038~041 更新）。

## 7. reviewer 能力覆盖投影（2026-08-30 增补，D1=B3 用户批准）

> 依据：r23 实跑 codex/kimi 的 plan 过 review 却被 Final Compile canonical 校验拒（`required_capability_missing` 11/5 条），reviewer 判定面与 canonical validator 系统性错位。B1（author prompt 能力覆盖教学，commit `e78094a9`）已先行落地；本节为 B2。REQ-WSC-06 已增补对应 SHALL 条款与 scenario。

- [x] 7.1 reviewer context 只读能力覆盖投影：逐 WI→contract edge 的 required capabilities、所引契约输出 capabilities、compatibility_policy；数据由 `report_contract_requirements`（`src/product/work_item_contract/dependency.rs:156-235`）同源计算逻辑生成，不重述不重实现；仅注入单候选 reviewer 路径
- [x] 7.2 reviewer prompt 增补：逐 edge 机械核验教学；任何覆盖缺口产出 must_fix finding（归类建议 contract_gap，evidence 含具体 edge 与缺失能力）；canonical validator 与 scope/digest/CAS 机制零改动
- [x] 7.3 测试：投影正确性单测（与 validator 同输入同口径）、prompt 教学句存在性测试（先 RED 后 GREEN）、legacy/story/design 路径不接收投影的隔离测试
- [x] 7.4 SC markdown prompt 预算余量复核（B1 后余 5 字节；如 B2 触及 SC prompt 侧则同步放宽常量至整百级并注明 margin 惯例）
- [x] 7.5 完成并过审后 r24 重跑三 provider 验收（codex/kimi 900s、pi 1800s，D2 用户批准）：全部 Confirmed 方为 6.2 达标；D3（driver 卫生）本轮不做，留作三 provider 通过后的独立 harness 任务

> **口径回归（2026-08-31，用户裁决 A）**：7.5 的「全部（三 provider）Confirmed 方为 6.2 达标」为实施期加严口径，已回归 §6.2 判据原文（codex + pi）。B1/B2 能力覆盖投影本身的有效性由 r23→r26 实证（能力覆盖类违规消灭、codex/pi R1 直接过）；kimi 见 §6.2 收口块的待验证登记。

## 8. handoff 消费闭环与 trusted catalog 残留清理（2026-08-30 增补，用户批准 P2+P4）

> 依据：r24 实跑——codex R1 直接过 review（B1/B2 能力覆盖类消灭），Final Compile 拒 `unconsumed_required_handoff` ×2；pi 在 IR 校验被 `trusted_verification_command_catalog_field_too_large` ×1（2026-08-29 简化裁决后残留）+ `unknown_requirement_ref` ×11（provider 方差，教学已存在）。oracle 裁决 P2+P4、不做 P3 全量 checklist；P4 先行独立提交。

- [x] 8.1 （P4）SC 路径退役旧 trusted catalog 规则：`validate_plan_candidate_ir` 走 SC 专用 outline 校验 profile，跳过 `outline.rs:126-175` catalog 条目数/字段长度/投影 bytes 规则与 `draft.rs:230-270` missing_trusted/untrusted_required 残留（`parse.rs:177`、`work_item_plan_compiler/validate.rs:34-35` 联动点以代码现状为准）；legacy draft 路径及其测试零变化；SC 路径其他 outline 规则（IDs/traceability/scope/budget/依赖/环）行为不变；不全局放开命令字段边界，长度安全门若保留须迁移为通用 bounded-field 校验、不再使用带 outline 语义的旧规则名
- [x] 8.2 （P2-author）handoff 消费闭环教学：`work_item_split_engine/prompts.rs` SC full-author prompt 增补 `provided_contract_refs` 必须被下游 `input_contracts` 逐字消费的纪律与反例；预算常量 16,200→17,000
- [x] 8.3 （P2-reviewer）覆盖投影扩展：依赖图事实（depends_on/边/环/重复边/未知 provider）+ handoff 消费闭环（消费者集合与消费状态，空消费者显式空集）+ 跨 item 写范围冲突事实；同源复用 `dependency.rs` 共享逻辑，不重述不重实现；unconsumed handoff must_fix/contract_gap 教学；仅单候选注入；reviewer 64KiB 预算双点检查不变
- [x] 8.4 测试：P4 RED（SC 路径长命令字段通过且无 outline catalog finding；legacy 路径该规则仍生效）；投影消费闭环逐字段测试（同源一致）；SC prompt 教学句测试；隔离回归
- [x] 8.5 实施过审后 r25 重跑三 provider（codex/kimi 900s、pi 1800s）：全部 Confirmed 方为 6.2 达标；needs_human 为合法终态，不降级不伪造；pi 方差按有界重跑，连续复现另立议题报用户

> **口径回归（2026-08-31，用户裁决 A）**：同 7.5——「全部（三 provider）Confirmed」回归 §6.2 判据原文。P2+P4 本身的有效性由 r25→r26 实证（`unconsumed_required_handoff` 与 catalog 残留类消灭）。

> **§7/§8 勾选证据（2026-08-31 终审，逐项核对）**：7.1-7.3=commit `0f47060d`（reviewer Approved+Important 补强；r26 kimi 轮 reviewer 拦截能力缺口=投影在工作实证；测试含入全量 lib 绿）。7.4=B2 未触及 SC author 侧条件未触发，后续 8.2-fix/8.3-fix 链已将常量上调至 19,000 整百级（实测 bytes=18,934 margin=66）。7.5=r24 已跑（codex 暴露 `unconsumed_required_handoff` → 引出 §8），终局 r26 codex+pi Confirmed；D3 driver 卫生登记为独立 harness 任务未做。8.1=`6d74242a`（Approved；r25 后 catalog 残留拦截消灭）。8.2-8.4=`4e031552`（Approved 3 Low defer；预算链 16,200→17,000→…→19,000；r26 后 `unconsumed_required_handoff` 消灭）。8.5=r25 已跑（暴露终端 `[]` 泛化 → 8.2-fix 链修复），终局 r26 codex+pi Confirmed、kimi needs_human 合法终态。

> **登记（2026-08-30，用户裁决方案 b）**：plan/draft/review 产物 95% 成功率验收**不入 6.2 判据**；待全流程（6.2→6.3→6.4→终审→push）完成后以专项测量轮执行（测量形态终审裁量）。

> **8.2-fix 教学修复（2026-08-30 二次增补，用户批准 C′）**：8.2 教学补「Handoff Schema 三字段必须显式输出；无下游消费者（含链路末端与单 WI 计划）→ `provided_contract_refs: []`（合法空数组，非省略）；禁止以省略/删字段/写 blocker/改 ID/自然语言回避校验」；预算常量 17,000→18,000（改后实测 prompt bytes）。TDD：RED=教学句存在性断言；行为锁=终端 `[]` 编译通过、省略字段仍 `missing_section`、非终端未消费非空 ref 仍被 validator 拒。validator/grammar/lowering 零改动（`[]` 机制本就合法）。
> **登记（同批）**：legacy draft prompt 的「无消费者空数组」约定在架构简化重写 SC prompt 时丢失，本项为复原该约定。

> **8.3-fix 项目规则内容式注入（2026-08-31 增补，用户批准简化方案）**：SC author 的 [cadence_project_rules] 从指针式（「按需查阅+读不到就忽略」）改为内容式：服务端生成启动前读取目标仓库 `.claude/rules/language.md` 全文注入（222B 级），并注入 aria 侧维护的 code-usage/code-reading 关键段固定摘要与「结构字面量优先于语言规则」优先句；language.md 缺失/不可读→生成拒绝（fail-closed）；质量预算 18,000→19,000。旧 Legacy outline/draft/Story/Design/Coding 链路保持指针式不动。背景：r26 实跑 codex 英文 plan——规则内容从不进 prompt 的机制性根因（8-21 relax-gate 生成类弱指针）。
> **登记（同批 defer）**：interactive 人工返修的 SC revision 路径缺口（现走 legacy 中文-heading artifact 约束，r27-interactive 实证）→ 阶段 3 对话式人工门一并重做。
