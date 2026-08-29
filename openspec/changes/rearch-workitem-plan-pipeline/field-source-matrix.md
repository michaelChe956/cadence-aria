# Work Item 单候选 C′字段唯一来源矩阵

## 约束

本矩阵是 REQ-WSC-02、REQ-WSC-07 与 D1、D-D 的唯一字段来源约束，供 Task 1.2 grammar 和 Task 2.x compiler 直接实现。每个字段只接受表中 `source` 指定的来源：

- `markdown`：作者在 `work-item-plan.md` 的受约束结构化区域明写的语义；不得由 prompt、旧 draft、outline 或 runtime 补齐。
- `session_confirmed_context`：创建 session 时已确认并持久化的上下文；不得由 markdown 或 prompt 覆盖或猜测。
- `compiler_derived`：编译器从已验证的唯一输入确定性计算；不得由 markdown、prompt 或 runtime 覆盖。
- `compile_runtime`：compile/publish 事务外层注入的 ID、时钟和持久化结果；不得由 markdown、prompt 或编译器猜测。

`contract.depends_on[]` 是 markdown 中 work item 依赖声明的 lowering 输入；当前 `CanonicalWorkItemContract` 不承载该字段，后续 compiler 必须将它沿用为既有依赖关系派生的输入，而不是另造第二来源或 `DependencyGraph` 类型。

`contract.handoff_contract` 只定义 coding 前可审计的 schema。`HandoffRevision` 的 `id`、`coding_unit_run_id`、`provided_contracts`、`provided_capabilities`、`contract_hash`、`commit_sha` 和 `created_at` 都是 coding 完成后由 `src/product/models/work_item_revision.rs` 产生的 runtime value；它们不是 lowering 输入，也不是本矩阵的第二来源。

| field_path | source | missing_behavior | forbidden_second_source | lowering_rule | test_id |
| --- | --- | --- | --- | --- | --- |
| contract.schema_version | markdown | 缺失→diagnostic（required） | compiler 默认版本、runtime 覆盖 | 解析版本字面量并写入 canonical contract | FSM-001 |
| contract.identity.logical_work_item_id | markdown | 缺失→diagnostic（required） | outline ID、runtime 生成 ID | 解析合法逻辑 ID 并写入 identity | FSM-002 |
| contract.identity.title | markdown | 缺失→diagnostic（required） | issue 标题、prompt 补齐 | 解析标题并写入 identity | FSM-003 |
| contract.identity.kind | markdown | 缺失→diagnostic（required） | outline kind、runtime 推断 | 解析合法 kind 并写入 identity | FSM-004 |
| contract.goal.summary | markdown | 缺失→diagnostic（required） | prompt 摘要、outline goal | 解析 EARS goal summary 并写入 goal | FSM-005 |
| contract.non_goals[] | markdown | 缺失→显式空集合 | prompt 补齐、旧 draft | 解析列表；缺省时保留空集合 | FSM-006 |
| contract.depends_on[] | markdown | 缺失→显式空集合 | outline dependency graph、runtime 推断 | 解析逻辑 ID 列表，供既有依赖派生使用 | FSM-007 |
| contract.input_contracts[].contract_id | markdown | 条目缺失→diagnostic（required） | 上游 draft、runtime 生成 | 解析输入契约条目的 contract_id | FSM-008 |
| contract.input_contracts[].provider_logical_work_item_id | markdown | 条目缺失→diagnostic（required） | dependency graph、runtime 推断 | 解析输入契约 provider 逻辑 ID | FSM-009 |
| contract.input_contracts[].required_capabilities[] | markdown | 条目缺失→显式空集合 | 上游产物、prompt 补齐 | 解析 capability 列表 | FSM-010 |
| contract.input_contracts[].compatibility_policy | markdown | 条目缺失→diagnostic（required） | compiler 默认值、runtime 覆盖 | 解析 require_all 或 require_any | FSM-011 |
| contract.output_contracts[].contract_id | markdown | 条目缺失→diagnostic（required） | runtime 生成、下游推断 | 解析输出契约条目的 contract_id | FSM-012 |
| contract.output_contracts[].capabilities[] | markdown | 条目缺失→显式空集合 | prompt 补齐、下游需求推断 | 解析 capability 列表 | FSM-013 |
| contract.tasks[].task_id | markdown | 条目缺失→diagnostic（required） | runtime 生成、statement 派生 | 解析任务 ID | FSM-014 |
| contract.tasks[].statement | markdown | 条目缺失→diagnostic（required） | prompt 改写、goal 补齐 | 解析 EARS task statement | FSM-015 |
| contract.tasks[].requirement_refs[] | markdown | 条目缺失→显式空集合 | design 推断、prompt 补齐 | 解析 requirement reference 列表 | FSM-016 |
| contract.tasks[].done_when_refs[] | markdown | 条目缺失→显式空集合 | acceptance 推断、prompt 补齐 | 解析 done-when reference 列表 | FSM-017 |
| contract.write_policy.exclusive_scopes[] | markdown | 缺失→explicit 空集合 | runtime 猜测 path、outline scope | 解析受控 scope 列表 | FSM-018 |
| contract.write_policy.forbidden_scopes[] | markdown | 缺失→explicit 空集合 | runtime 猜测 path、prompt 补齐 | 解析受控 scope 列表 | FSM-019 |
| contract.acceptance_criteria[].criterion_id | markdown | 条目缺失→diagnostic（required） | runtime 生成、statement 派生 | 解析 acceptance criterion ID | FSM-020 |
| contract.acceptance_criteria[].statement | markdown | 条目缺失→diagnostic（required） | prompt 改写、task 派生 | 解析 EARS acceptance statement | FSM-021 |
| contract.acceptance_criteria[].required_evidence[] | markdown | 条目缺失→显式空集合 | validator 补齐、runtime 推断 | 解析 EvidenceKind 列表 | FSM-022 |
| contract.verification_checks[].check_id | markdown | 条目缺失→diagnostic（required） | runtime 生成、command 派生 | 解析 verification check ID | FSM-023 |
| contract.verification_checks[].command | markdown | 缺失→显式 None；与 manual_instruction 均缺失→diagnostic | prompt 补齐 | 解析命令；只保留作者明写值（授权由 plan 审批门承载，非独立目录） | FSM-024（2026-08-29 简化裁决） |
| contract.verification_checks[].manual_instruction | markdown | 缺失→显式 None；与 command 均缺失→diagnostic | prompt 补齐、runtime 指令 | 解析人工检查说明；只保留作者明写值 | FSM-025 |
| contract.verification_checks[].required | markdown | 条目缺失→diagnostic（required） | compiler 默认 true、runtime 覆盖 | 解析布尔值 | FSM-026 |
| contract.verification_checks[].non_zero_test_execution_required | markdown | 条目缺失→diagnostic（required） | compiler 默认值、runtime 覆盖 | 解析布尔值 | FSM-027 |
| contract.handoff_contract.required_fields[] | markdown | 缺失→显式空集合 | HandoffRevision runtime values、prompt 补齐 | 解析 coding 前 handoff schema 字段名 | FSM-028 |
| contract.handoff_contract.provided_contract_refs[] | markdown | 缺失→显式空集合 | HandoffRevision runtime values、runtime 推断 | 解析引用列表 | FSM-029 |
| contract.handoff_contract.reviewer_check_refs[] | markdown | 缺失→显式空集合 | reviewer finding、runtime 推断 | 解析引用列表 | FSM-030 |
| contract.blocker_rules[].reason_code | markdown | 条目缺失→diagnostic（required） | runtime 错误码、prompt 补齐 | 解析 blocker reason code | FSM-031 |
| contract.blocker_rules[].route | markdown | 条目缺失→diagnostic（required） | runtime 路由、prompt 补齐 | 解析合法 BlockerRoute | FSM-032 |
| contract.blocker_rules[].target_contract_refs[] | markdown | 条目缺失→显式空集合 | dependency graph、runtime 推断 | 解析目标契约引用列表 | FSM-033 |
| contract.design_traceability[].source_type | markdown | 条目缺失→diagnostic（required） | session context、runtime 推断 | 解析 traceability source type | FSM-034 |
| contract.design_traceability[].source_id | markdown | 条目缺失→diagnostic（required） | session context、runtime 推断 | 解析 traceability source ID | FSM-035 |
| contract.design_traceability[].requirement_id | markdown | 条目缺失→diagnostic（required） | prompt 补齐、runtime 推断 | 解析 requirement ID | FSM-036 |
| verification_plan.checks[] | compiler_derived | canonical verification_checks 缺失→diagnostic | markdown 第二份 checks、runtime 补齐 | 从 canonical verification_checks 确定性投影为 WorkItemDraftVerificationPlan.checks | FSM-037 |
| trusted_commands[].command | markdown（Verification 段声明；plan 审批门为授权锚） | 声明缺失→显式空集合 | prompt、仓库配置、独立目录文件 | 从本 item Verification.command 声明确定性投影（去重） | FSM-038（2026-08-29 简化裁决） |
| trusted_commands[].cwd | compiler_derived | — | markdown 明写、runtime 猜测 path | 固定投影为 "."（与 plan 相对） | FSM-039（同上） |
| trusted_commands[].purpose | compiler_derived | — | markdown 明写 | 由对应 check 的语句派生（确定性截断） | FSM-040（同上） |
| trusted_commands[].source_ref | compiler_derived | — | markdown 明写 | 由 source revision hash 确定性派生 | FSM-041（同上） |
| target_repository_id | session_confirmed_context | 缺失→diagnostic（required） | markdown、prompt、runtime 猜测 path | 从已确认 target repository binding 原样注入 item | FSM-042 |
| ir.source_revision_hash | compiler_derived | 源 markdown 不可读→diagnostic | prompt hash、runtime 覆盖 | 对规范化 source revision 字节确定性计算 hash 并只置于顶层 IR | FSM-043 |
| ir.compiler_version | compiler_derived | 编译器版本未定义→diagnostic | markdown、runtime 覆盖 | 由编译器常量确定性写入顶层 IR | FSM-044 |
| publication_provenance.id | compile_runtime | 未注入→事务失败 | markdown、compiler 猜测 ID | 外层事务注入不可变 publication ID | FSM-045 |
| publication_provenance.plan_id | compile_runtime | 未注入→事务失败 | markdown、IR item、compiler 猜测 ID | 外层事务注入待发布 plan ID | FSM-046 |
| publication_provenance.plan_revision_id | compile_runtime | 未注入→事务失败 | markdown、IR item、compiler 猜测 ID | 外层事务注入已分配 plan revision ID | FSM-047 |
| publication_provenance.source_revision_ref | compiler_derived | source revision 无法解析→diagnostic | markdown、runtime 覆盖 | 从已验证 source revision 确定性生成引用 | FSM-048 |
| publication_provenance.plan_candidate_ir_ref | compiler_derived | IR 未生成→diagnostic | markdown、runtime 覆盖 | 从已验证顶层 IR 确定性生成引用 | FSM-049 |
| publication_provenance.mechanical_report_ref | compile_runtime | mechanical report 未持久化→事务失败 | markdown、prompt、compiler 猜测持久化 ID | 外层事务注入已持久化 mechanical report 引用 | FSM-050 |
| publication_provenance.source_revision_hash | compiler_derived | IR source_revision_hash 缺失→diagnostic | markdown、runtime 覆盖 | 复制并校验顶层 IR 的确定性 hash | FSM-051 |
| publication_provenance.compiler_version | compiler_derived | IR compiler_version 缺失→diagnostic | markdown、runtime 覆盖 | 复制并校验顶层 IR 的确定性 compiler version | FSM-052 |
| publication_provenance.published_at | compile_runtime | 未注入→事务失败 | markdown、compiler 读取时钟 | 外层事务注入 publish 时刻 | FSM-053 |
| publication_provenance.content_hash | compiler_derived | provenance 内容不可规范化→diagnostic | markdown、runtime 覆盖 | 对待发布不可变 provenance 内容确定性计算 hash | FSM-054 |
| compile_id | compile_runtime | 未注入→事务失败 | markdown、compiler 猜测 ID | 外层事务在 prepare 前注入 compile ID | FSM-055 |
| now | compile_runtime | 未注入→事务失败 | markdown、compiler 读取时钟 | 外层事务在 prepare 前注入时间 | FSM-056 |
