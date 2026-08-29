# Tasks

> 本 change = 阶段 2（单仓 C′ MVP）。阶段 1（策略层/终态矩阵/持久化）见 `workitem-typed-outcome-policy`，本阶段复用其产物。阶段 3 才实现最小 WS 协议与对话流人工门；阶段 4 才处理多仓扩展与旧协议删除。`advance` 的 workitem pipeline 签名属于阶段 3，本 change 不定义。

## 1. 字段来源矩阵与 grammar

- [ ] 1.1 建立 `openspec/changes/rearch-workitem-plan-pipeline/field-source-matrix.md`：逐字段覆盖 `CanonicalWorkItemContract`、`WorkItemDraftVerificationPlan`、trusted commands、target repository、publication provenance。每行标注唯一来源（markdown 明写 / session 与已确认上下文 / 编译器确定性派生 / compile 事务运行时生成）、缺失行为、禁止的第二来源、覆盖测试；handoff 仅描述 schema，运行时值仍由 coding 后 HandoffRevision 产生
- [ ] 1.2 定义 `work-item-plan.md` grammar（稳定 section、EARS 行语法、ID、scope、依赖、verification、handoff schema、blocker、traceability）；结构化区域未知字段失败关闭，仅 Notes/Rationale 区容忍自由文本
- [ ] 1.3 编写 `openspec/changes/rearch-workitem-plan-pipeline/fixtures/work-item-plan-rep4.md`：backend/frontend/integration 三 item 的完整 source fixture；按 1.1 的每一字段覆盖全部契约要素。另建**仅限确属 grammar/lowering** 的 compiler diagnostic fixture，并为每例标注输入行、字段、期望诊断；不得把 reviewer finding 自动改写为 compiler diagnostic
- [ ] 1.4 为 grammar/source linter 写未知结构化字段、缺失字段、ID/EARS 非法、Notes/Rationale 自由文本允许的测试

## 2. 编译器（markdown → 逐 item typed IR）

- [ ] 2.1 parse：markdown → AST，文档级诊断必须含行号、字段与恰好一个修复示例
- [ ] 2.2 lower：AST → 顶层 `PlanCandidateIr { source_revision_hash, compiler_version, items: Vec<PlanCandidateItemIr> }`；逐 item `PlanCandidateItemIr { target_repository_id, contract: CanonicalWorkItemContract, verification_plan: WorkItemDraftVerificationPlan, trusted_commands }`。顶层 provenance 不得重复置于 item；不能以临时 JSON 或第二来源补齐字段
- [ ] 2.3 golden 分两类：阶段 1 的 14 条 classifier golden（rep2/3/4 的 9 条、rep1 round-1 的 2 条 Advisory、3 个带人工 class_hint 的变体）按其阶段 1 预期分类；仅确属 grammar/lowering 的**9 个 reviewer finding 映射集合**建对应 markdown 输入、错误行、字段与期望 compiler diagnostic。其余 finding 明确仅为 prompt few-shot 素材，不伪装为 compiler diagnostic；3 个带人工 class_hint 的变体只服务阶段 1 分类器
- [ ] 2.4 完整 lowering 验证：`fixtures/work-item-plan-rep4.md` 编译产物通过既有 `work_item_split_validator` 全量校验零 Error；复用阶段 1 golden fixture 的分类边界而不修改 validator 规则
- [ ] 2.5 publish 前 freshness 校验 source_revision_hash + compiler_version，写入不可变 publication provenance；coding 只读取该 immutable runtime binding，执行期绝不解析 markdown

## 3. compile 事务输入抽取（高风险边界，独立审查门）

- [ ] 3.1 先补 legacy parity/characterization 测试：按语义产物、事务状态序列、failpoint/recovery 三层比较；语义产物比较规范化后字段，忽略 `next_compile_id()` 与 `Utc::now()` 导致的动态 ID/时间，不要求原始 JSON 字节相同
- [ ] 3.2 定义唯一注入模型 `InitialPlanCompileInput`：

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
- [ ] 3.3 以现有 `WorkItemPlanCompileFinalizerCheckpoint::{PlanSummaryPrepared, FirstChildSessionEnsured, FirstChildBindingEnsured, FirstChildContextPrepared, CompileReportPersisted}` 逐项执行 failpoint/recovery parity；确认提取前后规范化产物、持久化状态序列、finalizer 结果一致
- [ ] 3.4 新路径 IR adapter：validated IR → `InitialPlanCompileInput` → 同一 prepare/execute 核心；对 legacy 与 IR adapter 分别做 parity 测试

## 4. prompt 重构与复评 invocation

- [ ] 4.1 author prompt 改 markdown 语法 + 边界约束 + 判例 few-shot（grammar 内容内联进 prompt——provider 工作目录是目标仓库，不能只给 aria 仓内路径）
- [ ] 4.2 删除行为教学层（B 层），仓库注入内容承担单一来源
- [ ] 4.3 【阶段 1 契约残留引用】复用阶段 1 的持久化 `ReviewPhase` / `ReviewInvocationScope`：服务端生成 Initial 或 Verification scope 及 canonical digest；prompt builder 从 scope 构造初评/复评指令，不允许 provider/campaign 自行提供范围。字段与 digest 算法以阶段 1 change 为唯一来源，本阶段不重定义
- [ ] 4.4 【阶段 1 契约残留引用】parser/分类器校验复评 finding 仅能重现 original fingerprints，且本 invocation 的 mechanical report 必须存在且匹配；不得做 changed-path 归因。缺机械报告、指纹越界、scope digest 不符均为 fatal/protocol error；复评后不再自动返修
- [ ] 4.5 prompt 与 scope 单测：必需内容存在、已删内容不存在、字节上限、scope 持久化/重连、范围违例失败关闭

## 5. 单候选 engine 端到端（flag 内，单仓）

- [ ] 5.1 新路径 engine：复用阶段 1 策略层、终态矩阵、`RunHistory` durable counters、`ReviewInvocationScope` 与 human gate snapshot；副作用幂等（CAS/事务先持久化 next state，再启动 provider）
- [ ] 5.2 运行时自选 batch/serial（内部决策，不暴露为 WS/UI 决策）
- [ ] 5.3 多仓边界：实施确定性 preflight。在尚未创建/持久化新路径 session、markdown、IR、run history 或 transaction，且 provider 尚未启动前才允许选择 legacy；preflight 成功后任一持久化或 provider 启动后的失败一律写新路径 durable fatal/recoverable，原 `flow_kind` 不变，绝不静默 fallback
- [ ] 5.4 恢复测试逐例写初始 JSON/transaction、重启或重连动作、预期 state、provider 启动次数、不可变事件断言：generate/repair 中断、approval pending 重连、已完成重放、compile/finalizer recovery

## 6. 验收（需操作者授权真实运行）

- [ ] 6.1 campaign driver 适配 + 操作者授权
- [ ] 6.2 codex + pi 各 1 案例实跑（naruto 单仓）：2/2 Confirmed、≤12 分钟、`initial_review_count ≤ 1`、`verification_review_count ≤ 1`、`repairs_used ≤ 1`
- [ ] 6.3 核对阶段 1 的 14 条 classifier golden（11 原始 + 3 个带人工 class_hint 的 Repairable 变体）；只核对 9 个 reviewer finding 映射集合中已明确归入 grammar/lowering 的 compiler diagnostic golden。其余 reviewer finding 只作为 prompt few-shot 素材；usage/token 基线对比（含注入内容占比测量）
- [ ] 6.4 验收报告落盘；达标后移交阶段 3 立项（对话流人工门与 `advance` 接口），未达标不得退役旧协议

> **2026-08-29 简化裁决补记**（用户批准）：工作包 5.2 的「轻量 outline→计数→selector→完整 author」两阶段生成简化为**单次 provider 生成完整 plan**；selector 降级为编译后内部诊断。工作包 2.5/5.x 期间落地的 outline 派生受信目录链（catalogfix b2aaf24e 及其后续教学）标记 superseded——trusted_commands 改为 plan Verification 段声明确定性投影，授权锚=plan 审批门（见 design.md 架构简化裁决节与 field-source-matrix FSM-024/038~041 更新）。
