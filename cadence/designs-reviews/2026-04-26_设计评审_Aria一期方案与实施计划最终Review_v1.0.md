# 设计评审：Aria 一期方案与实施计划最终 Review

## 1. 评审目标

本次评审面向研发落地，目标是确认研发人员能否仅根据设计方案和实施计划清楚知道：

1. 做什么。
2. 按什么顺序做。
3. 写到哪些模块。
4. 输入、输出、状态、错误码、事件、落盘路径和测试验收分别是什么。
5. 哪些问题必须先由设计侧裁定，不能由研发自行猜测。

评审原则：

- 以 `2026-04-23_技术方案_Aria一期MVP精简设计_v1.2.md` 与 `2026-04-23_技术方案_Aria一期实现总契约_v1.0.md` 为一期基准。
- 如果 MVP 或总契约自身存在冲突，标为“基准疑似问题”，需要先由你裁定。
- `IO协作协议`、`研发导读`、`评审后实施规格补齐`、P1-P4 计划必须服从上述基准；如需覆盖基准，应升版基准或记录裁定。

## 2. 评审结论

当前文档已经比早期版本更接近可实施状态，但仍存在若干会直接阻塞研发实现的问题。主要集中在：

1. 基准或补齐规格之间的字段、类型、阶段时序冲突。
2. P1/P2 的基础契约仍有缺口，后续 P3/P4 会被迫补洞。
3. P3/P4 对 Provider、执行失败路由、集成 commit 边界、最终收口语义仍有歧义。
4. 导读和总览对研发入口友好，但缺少几个关键实施线和准入边界。

建议：阻塞项未裁定前，不建议启动正式编码；最多允许做不产生正式产物的原型验证。

## 3. 必须先裁定的问题

### B-01 OpenSpec 约束可用时机与节点矩阵冲突

- 严重级别：阻塞，基准疑似问题
- 涉及文档：实现总契约、评审后实施规格补齐
- 证据：总契约节点矩阵要求 `N07` 消费 `design_constraints`，`N10/N11` 消费 `task_constraints`；但 OpenSpec 生命周期规则又表示 `design.md` 缺失阻断 `N11`，`tasks.md` 缺失阻断 `N12/N16`。
- 影响：新任务在尚未生成 design/tasks 前可能被 design/task constraints 阻断，规划链可能死锁。
- 建议裁定：`N07` 只强制 `requirement_constraints`；`design_constraints` 在 `N08` 通过并写回后供 `N10/N11` 使用；`task_constraints` 由 `N11` 写回后供 `N12/N16` 使用。

### B-02 Provider raw output 引用类型冲突

- 严重级别：阻塞
- 涉及文档：实现总契约、IO 协作协议、评审后实施规格补齐
- 证据：总契约要求 stdout/stderr/structured output 登记为 `ExternalArtifactRef`；补齐规格中 `ProviderRunRecord.raw_artifact_refs/stdout_ref/stderr_ref/structured_output_ref` 使用 `ArtifactRefId`；补齐规格又提到 structured output 登记为 `ExternalArtifactRef`。
- 影响：`ArtifactKind` 只有一期正式产物，无法合法表示 stdout/stderr。ProviderRunRecord、external-ref store、artifact index 会分叉。
- 建议裁定：raw provider 文件统一使用 `ExternalRefId` / `ExternalArtifactRef`，或新增 `RuntimeArtifactRef`；不要复用 canonical `ArtifactRefId`。

### B-03 Projection 类型、映射规则、golden fixture 不一致

- 严重级别：阻塞
- 涉及文档：评审后实施规格补齐、P2 计划
- 证据：`WorkPackageProjection.traceability_refs` 类型是 `Vec<TraceabilityRef>`，但 golden JSON 使用字符串数组；`acceptance_targets` 类型是 `Vec<AcceptanceTarget>`，golden 也使用字符串数组；`DesignDecisionProjection` 类型字段是 `summary`，golden 使用 `text`；`RiskProjection` 类型缺少 `severity` 和 `related_design_decision_ids`，但解析规则和 golden 使用这些字段；最小 `plan.md` fixture 只定义 `WT-001` 却依赖 `WT-002`。
- 影响：P2 projection compiler、validator、fixture 无法同时通过。
- 建议裁定：先确定 projection payload 的唯一 JSON 形态，再同步 Rust 类型、映射规则、fixture 和 validator。若采用结构化对象，golden 必须改；若采用字符串 ID，Rust 类型必须改。

### B-04 `openspec_bootstrap_status` 复用 `BundleStatus` 但缺少 `bootstrapped`

- 严重级别：阻塞
- 涉及文档：P1 计划、P2 计划、实现总契约、评审后实施规格补齐
- 证据：总契约要求 P1 写 `openspec_bootstrap_status=bootstrap_pending`，P2 更新为 `bootstrapped`；P1 示例把该字段声明为 `BundleStatus`；补齐规格的 `BundleStatus` 只有 `bootstrap_pending/ready/stale/blocked`。
- 影响：研发无法建模或编译，且 bootstrap 生命周期与 bundle 生命周期混在一起。
- 建议裁定：新增独立 `OpenSpecBootstrapStatus { bootstrap_pending, bootstrapped }`，并明确落盘在 task runtime state；`OpenSpecConstraintBundle.bundle_status` 继续表达 bundle readiness。

### B-05 `RuntimeUnit<CanonicalNodeInput>` 与 `N00/N01` pre-task 场景冲突

- 严重级别：阻塞，辅助规格疑似问题
- 涉及文档：P1 计划、评审后实施规格补齐
- 证据：补齐规格的 `CanonicalNodeInput`、`DaemonContext` 都要求 `task_id`，P1 又要求 `N00/N01` 也实现统一 `RuntimeUnit`；但 `N00 session_bootstrap`、`N01 intake_capture` 发生在 task 创建前。
- 影响：研发会被迫伪造 task，或绕开统一 handler。
- 建议裁定：为 `N00/N01` 定义 pre-task input；或允许 `CanonicalNodeInput.task_id/risk_registry_ref` 条件为空；或规定 `N00/N01` 不走该 `RuntimeUnit`，从 `N02` 开始统一。

### B-06 REPL reconnect/replay 字段位置与事件时间字段冲突

- 严重级别：阻塞
- 涉及文档：实现总契约、评审后实施规格补齐、P1 计划
- 证据：总契约规定 `hello` 带 `last_seen_event_id?`，`attach` 不带；补齐规格时序图要求 `attach` 带 `last_seen_event_id`，但 replay 判定又写 `hello` 带。总契约 wire event 使用 `occurred_at`，补齐规格 event log 使用 `created_at`。
- 影响：P1 wire schema、replay 测试、event log 到 envelope 的映射无法稳定实现。
- 建议裁定：固定 `last_seen_event_id` 的唯一位置；建议按总契约放在 `hello`。同时定义 event log record 的 `created_at` 如何映射到 wire envelope 的 `occurred_at`，或统一字段名。

### B-07 P4 N23 集成成功路径缺少 integration commit 事实边界

- 严重级别：阻塞
- 涉及文档：P4 计划、评审后实施规格补齐
- 证据：P4 Task 3 只写 `git cherry-pick --no-commit` 和生成 `integration_report`；补齐规格要求随后执行 `git commit -m "aria: integrate <worktask_id>"`，N24 rollback 写 `rollback_ref` 与 snapshot。
- 影响：integration worktree 会留下未提交状态，N24/final_review 无法可靠判断“已集成”的 commit 边界，rollback 缺审计事实。
- 建议修正：P4 Task 3 增加成功路径：pre-check 后 commit，记录 `integration_commit_sha/post_merge_sha`；N24 失败执行 `git reset --hard <pre_merge_sha>`，写 `rollback_ref` 和 snapshot。

### B-08 rejected followup 的最终收口语义不清

- 严重级别：阻塞
- 涉及文档：P4 计划、实现总契约
- 证据：P4 允许用户拒绝 followup 后进入 `N27`，并让 `final_summary.overall_status=closed_with_rejected_followup/manual_terminated`；但总契约最终阶段收口要求 `final_review.overall_decision=pass` 且不存在悬空 requirement/task/risk。
- 影响：可能把未闭合覆盖伪装成最终成功收口。
- 建议裁定：rejected followup 是非成功终态，还是带人工豁免的成功关闭。若允许关闭，必须生成 manual exemption / explicit manual transfer，并区分 `closed_successfully` 与 `manual_terminated`。

## 4. 高优先级问题

### H-01 P1 命令面出口与总契约不一致

- 涉及文档：P1 计划、实现计划总览、研发导读、实现总契约
- 证据：总契约最小命令面包含 `list_artifacts/approve_gate/reject_gate/reply_gate`；P1 出口只写到 `hello/attach/subscribe/new_task/get_status/detach`。
- 影响：P4 approval gate 和 artifact 查询缺 P1 通信基线。
- 建议：P1 增加这些命令的最小 handler 与测试；若只做 schema，需要明确不符合总契约并请求裁定。

### H-02 P1 缺少逐命令 payload struct 与响应字段测试

- 涉及文档：P1 计划、实现总契约
- 证据：P1 只列命令名和 envelope 校验，没有逐命令固定 `payload.args` 与成功响应字段。
- 影响：REPL 与 daemon 可能实现出不兼容 payload。
- 建议：P1 Task 2 增加每个 command 的 Rust payload struct、JSON fixture 和 round-trip 测试，覆盖总契约命令表。

### H-03 P1 缺少 daemon discovery / metadata / lock / auto-start 落地步骤

- 涉及文档：P1 计划、评审后实施规格补齐
- 证据：补齐规格定义 `aria daemon run/status`、`aria repl --no-start`、socket 路径、`daemon.json`、`daemon.lock`、`ARIA_DAEMON_SOCKET`；P1 计划只写 transport 和握手。
- 影响：daemon 常驻、REPL attach/detach、stale daemon 恢复无法稳定实现。
- 建议：把补齐规格第 7 章拆入 P1 任务和测试：CLI 参数、workspace_hash、路径优先级、metadata schema、stale lock、自动启动/禁用自动启动。

### H-04 P1 `intake_brief` 未明确 materialize 为 canonical artifact

- 涉及文档：P1 计划、实现总契约、评审后实施规格补齐
- 证据：P1 要生成 `intake_brief` 并返回 `intake_ref`，但未说明路径、`ArtifactRef`、版本、sha256、artifact index/latest、`artifact.materialized` event。
- 影响：P2 validator 接入后可能找不到 P1 入口产物。
- 建议：P1 增加最小 artifact store/materialize 规则，至少覆盖 `intake_brief`。

### H-05 P2 把 17 类产物都写成三层 validator，偏离 MVP

- 涉及文档：P2 计划、MVP 精简设计、实现总契约
- 证据：P2 写“17 类一期产物统一三层 validator：canonical -> projection -> phase1_profile”；MVP/总契约只要求 `spec/design/plan` 三类 projection，JSON report 用 `_aria`。
- 影响：研发可能给 JSON report/runtime_snapshot 硬造 projection。
- 建议：改为 artifact kind 校验矩阵：17 类都走 canonical；仅 `spec/design/plan` 走 projection；需要 `_aria` 的 JSON artifact 走 phase1 profile。

### H-06 P2 漏列 `tests/phase1_profile.rs` 与完成判定

- 涉及文档：P2 计划、实现计划总览
- 证据：P2 Task 4 要测试 `phase1_profile_validator`，但目标文件结构和 P2 DoD 未单列 `tests/phase1_profile.rs`；总览测试基线也漏列。
- 影响：`_aria` profile 可能漏验仍被认为 P2 完成。
- 建议：补入目标文件结构、测试目录和完成判定；明确 `phase1_profile.rs` 与 `traceability_binding.rs` 职责边界。

### H-07 P2 缺少跨模块 public API 签名、错误类型和事务边界

- 涉及文档：P2 计划
- 证据：Document Operation、projection compiler、OpenSpec compiler、traceability binding 多为职责描述。
- 影响：研发会在不同模块发明不兼容接口。
- 建议：补签名和错误类型，例如 `read_document_model`、`create_document`、`upsert_section`、`compile_spec_projection`、`validate_projection`、`bootstrap_openspec_skeleton`、`compile_constraint_bundle`、`normalize_traceability`，并明确事务边界。

### H-08 P2 traceability 算法未完整落到任务和测试

- 涉及文档：P2 计划、评审后实施规格补齐
- 证据：补齐规格第 10 章要求输入优先级、worktask routing 查找、conflict log、同 checkpoint 落盘、`traceability.updated` payload；P2 Task 6 只泛化描述。
- 影响：P3/P4 execution report 无法形成稳定追踪链。
- 建议：把第 10 章完整落进 Task 6 的测试矩阵和接口签名，特别是 `source_work_package_id`、冲突原因、binding 与 report 同事务。

### H-09 Provider structured output 与 canonical artifact 最小字段不对齐

- 涉及文档：MVP 精简设计、实现总契约、评审后实施规格补齐
- 证据：补齐规格 prompt/structured output 示例中，`N16` 使用 `files_modified/status`，而 canonical `coding_report` 需要 `changed_files/implementation_summary/known_risks`；`N25` 使用 `followup_required`，总契约使用 `followup_needed`；`N27` 使用 `overall_status`，不在 `final_summary` 最小字段中。
- 影响：adapter 解析成功后仍可能无法通过 canonical validator。
- 建议：明确 provider structured output 是候选 DTO，并补逐节点 normalization mapping；或直接要求 provider 输出 canonical 最小字段。

### H-10 N25/N27 Provider 角色与基准冲突

- 涉及文档：MVP 精简设计、实现总契约、评审后实施规格补齐、P4 计划
- 证据：MVP 和总契约定义 `N25/N27` 为 Claude Code orchestrator；补齐规格写 `Claude/Codex`。
- 影响：`NodeExecutionContract`、prompt registry、provider router 无法确定 provider_type/runtime_role。
- 建议：一期按基准固定 `N25/N27 = Claude Code orchestrator`；若允许 Codex，需要回改基准并给出选择规则。

### H-11 P4 缺少逐节点 contract 表

- 涉及文档：P4 计划、实现总契约、评审后实施规格补齐
- 证据：P4 Task 0 只列节点和写权限，未逐节点写 `provider_type/runtime_role/adapter_role/advisory_only/output_schema_ref/prompt_id`。
- 影响：研发可能把 advisory 节点当正式决策节点，或接错 provider。
- 建议：P4 增加 contract 表，逐节点固定 provider、role、advisory、schema、prompt id。

### H-12 P4 execution failure / rework 路由覆盖不足

- 涉及文档：P4 计划、实现总契约、MVP 精简设计
- 证据：P4 Task 2 主要覆盖 happy path；总契约要求 N17 失败启用 debugging、N18 revise 到 N19、N19 只修失败项；MVP 保留 rework counter 阈值。
- 影响：测试失败、评审要求返工、rework 超限路径可能缺失。
- 建议：补 `testing fail -> N19`、`code_review revise -> N19`、`rework_counter` 递增与默认 3 次阈值、失败项 superseded/checkpoint 测试。

### H-13 集成失败回流上限未在代码级规格固化

- 涉及文档：MVP 精简设计、评审后实施规格补齐、P4 计划
- 证据：MVP 要求同一 WorkTask 连续 2 次集成失败进入 `X08 manual_intervention`，reason=`integration_retry_limit_exceeded`；补齐规格未定义计数器和阈值。P4 有测试，但缺代码级类型/状态来源。
- 影响：可能出现 `N19 -> N20 -> N23/N24 -> N19` 无限回流。
- 建议：在 `loop_counters` 或 integration queue state 中补 `integration_failure_counter`、默认阈值 2、错误码和 validator。

### H-14 P3 ProviderRunRecord 字段清单不完整

- 涉及文档：P3 计划、评审后实施规格补齐
- 证据：P3 Task 1 字段清单缺少 `status`、`started_at/completed_at`、`approval_policy`、`sandbox_mode`、`constraint_check_ref`、`traceability_binding_refs` 等。
- 影响：provider baseline 先落不完整 record，后续 recovery、审计、失败路由返工。
- 建议：P3 直接引用完整 `ProviderRunRecord`，`provider_adapter_baseline` 做 golden JSON/serde round-trip，覆盖 lifecycle status 和空 stdout/stderr/structured-output 落盘。

### H-15 实施总览受控并行边界偏宽

- 涉及文档：实现计划总览、P2/P3/P4 计划、MVP 精简设计
- 证据：总览允许 P3 在 P2 Task 6 后启动、P4 在 P3 Task 5 后启动；但 P2 Task 7 才覆盖 superseded refs，P3 Task 6 才覆盖 Risk Registry，P4 final closure 依赖 risk/coverage 闭合。
- 影响：并行团队可能提前进入会产生正式产物的链路，后补回流和风险追踪。
- 建议：拆细并行规则：P3 provider baseline/context builder 可早启动；`N08-N12` revise/backtrack 等待 P2 Task 7；P4 final closure 等待 P3 Task 6 和 minimal-flow fixture。

### H-16 研发导读遗漏 `N13-N15 / M13 execution_setup`

- 涉及文档：研发导读、MVP 精简设计、P4 计划
- 证据：MVP 明确 `M13` 覆盖 `N13/N14/N15`；P4 Task 1 专门实现该链路；导读从 Planning Nodes 直接跳到 Execution Nodes。
- 影响：研发负责人缺少 worktask register、worktree lease、allowed_write_scope、execution route 的前置实施线。
- 建议：导读增加 `Execution Setup / Worktree Lifecycle` 实施线。

### H-17 四份技术方案包的职责和裁定关系未讲清

- 涉及文档：研发导读、实现计划总览
- 证据：导读只解释 MVP 和总契约，未清楚说明 IO 协议、补齐规格与二者的阅读顺序和冲突裁定。
- 影响：Provider、Document Operation、OpenSpec 负责人容易漏读代码级章节，或误把补齐规格覆盖基准。
- 建议：导读新增“四份技术方案包阅读顺序与裁定规则”。

## 5. 中低优先级问题

### M-01 runtime 存储拓扑存在总契约与补齐规格差异

- 涉及文档：实现总契约、评审后实施规格补齐、P2/P3 计划
- 证据：总契约建议 `projections/constraints/provider-runs/traceability` 在 `.aria/runtime/` 顶层；补齐规格改为 `.aria/runtime/tasks/<task_id>/...` task-scoped 目录；P2/P3 多处按 task-scoped 使用。
- 影响：研发可能实现两套 index 或路径。
- 建议：裁定 task-scoped 为代码级存储真相源，并把总契约的顶层路径改成“汇总缓存或旧稿建议”；否则 P2/P3 必须回改。

### M-02 循环阈值缺少代码级注册表

- 涉及文档：MVP 精简设计、实现总契约、评审后实施规格补齐
- 证据：MVP 有 `patch_round_counter=2`、rework/design_revision/clarification 默认 3；补齐规格只给 `BTreeMap<String, u32>`。
- 影响：研发可能使用单一阈值，违反 MVP 不可简化项。
- 建议：补 `LoopCounterName`、默认阈值注册表、超限错误码和 validator。

### M-03 P3/P4 准入门槛没有显式依赖上一阶段 DoD

- 涉及文档：P3 计划、P4 计划、实现计划总览
- 证据：P3/P4 准入主要列补齐规格章节，未写“P1/P2/P3 完成判定全部通过”。
- 影响：agent 或研发可能跳过基础验证直接开后续阶段。
- 建议：P3 准入加 P1/P2 DoD；P4 准入加 P3 DoD、active dispatch_package、非空 task_constraints、可重放 minimal-flow fixture。

### M-04 WorktreeLease 完整字段和事件未完全落入 P4 Task 1

- 涉及文档：P4 计划、评审后实施规格补齐
- 证据：补齐规格要求 `base_ref/branch_name/status/acquired_at/released_at`，获得 lease 后发 `worktree.lease_acquired` event；P4 Task 1 只写 create/lease/release、路径和 scope。
- 影响：recovery 和 event replay 缺字段。
- 建议：P4 Task 1 明确完整 `WorktreeLease` 字段、状态机、event payload、等待队列阻塞 lease id。

### M-05 事件注册表未覆盖补齐规格新增事件

- 涉及文档：实现总契约、评审后实施规格补齐、P1/P4 计划
- 证据：总契约事件表没有 `worktree.lease_acquired`、policy degrade、OpenSpec rollback 等；补齐规格和计划使用了这些语义。
- 影响：REPL event schema registry 可能漏事件。
- 建议：新增 event registry 扩展表，或规定这些事件先作为 internal runtime event，不进入 wire event。

### M-06 snake_case 裁定仍有残留字段

- 涉及文档：实现总契约、IO 协议、评审后实施规格补齐、P4 计划
- 证据：仍有 `constraint_bundleId/bundleVersion`、`normalizedArtifactRef/rejectionReason/createdAt`、`registeredWorktask_ids`。
- 影响：serde fixture、JSON schema 字段名漂移。
- 建议：全量改为 snake_case；P4 明确 `registered_worktask_ids`。

### M-07 P4 `node_specific_fields` 需要锁定 fixture

- 涉及文档：评审后实施规格补齐、P4 计划
- 证据：补齐规格第 11 章定义每节点必填字段，但 P4 计划未要求 snapshot fixture/validator 全覆盖。
- 影响：节点能跑通但 checkpoint 不可审计。
- 建议：P4 增加 snapshot fixture，至少覆盖 `N13-N24/N25-N28`。

### M-08 P2 OpenSpec compiler 输出字段有内部 helper 与类型缺口

- 涉及文档：实现总契约、评审后实施规格补齐、P2 计划
- 证据：补齐规格映射提到 `requirement_titles_by_id/scenario_titles_by_id`，但 `RequirementConstraints` 类型未定义这些字段；总契约说明这些只能作为内部 helper 或子字段。
- 影响：研发不知道是否要序列化这些 map。
- 建议：明确为内部 helper，不进 `OpenSpecConstraintBundle`；或补入类型并锁定 schema。

## 6. 建议的修订顺序

1. 先裁定 B-01 到 B-08，尤其是 OpenSpec 阶段约束、Provider raw output 引用、Projection payload、bootstrap status、RuntimeUnit pre-task、REPL replay。
2. 修订 `评审后实施规格补齐_v1.4` 或升版为 v1.5，先把类型、fixture、字段名、状态机冲突全部打平。
3. 回改 P1/P2：补齐命令面、daemon discovery、artifact materialize、validator 矩阵、phase1 profile 测试、traceability 算法和 public API。
4. 回改 P3/P4：补 ProviderRunRecord 完整字段、逐节点 contract 表、execution failure/rework 路由、N23 commit 边界、followup rejected 终态。
5. 最后更新研发导读和总览：增加文档包裁定关系、`M13` 实施线、受控并行边界、完整测试基线。

## 7. 最小验收门槛建议

在正式开发前，至少应满足：

- MVP / 总契约 / 补齐规格之间不再存在字段级或类型级冲突。
- P1 有完整 wire command schema、daemon discovery、intake artifact materialize 测试。
- P2 有 artifact kind 校验矩阵、projection golden、phase1 profile、OpenSpec bundle schema、traceability binding 测试。
- P3 有 ProviderRunRecord golden round-trip、CLI/fake adapter 共 DTO、context builder 覆盖 `N04-N12`。
- P4 有逐节点 contract 表、worktree lease recovery、integration commit/rollback、execution failure/rework、final followup routes 测试。

## 8. 待你裁定的关键问题

1. `N07/N10/N11` 是否允许在 `design_constraints/task_constraints` 为空时进入？建议按阶段产物可用性裁定。
2. Provider raw output 到底是 `ExternalArtifactRef`、`ArtifactRef`，还是新增 `RuntimeArtifactRef`？
3. Projection 的 `traceability_refs` / `acceptance_targets` 采用结构化对象还是字符串 ID 数组？
4. Provider structured output 是 canonical artifact 本体，还是候选 DTO？
5. `last_seen_event_id` 放在 `hello` 还是 `attach`？event log 时间字段用 `occurred_at` 还是 `created_at`？
6. `openspec_bootstrap_status` 是否独立于 `OpenSpecConstraintBundle.bundle_status`？
7. `N00/N01` 是否走 `RuntimeUnit<CanonicalNodeInput>`？若走，pre-task input 如何建模？
8. rejected followup 是非成功人工终止，还是带豁免成功关闭？
9. N25/N27 是否固定 Claude Code orchestrator？若允许 Codex，需要升版基准。
