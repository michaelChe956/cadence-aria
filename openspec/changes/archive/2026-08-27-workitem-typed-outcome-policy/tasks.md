# Tasks

## 1. 核心类型、归档与 golden fixture

- [x] 1.1 先归档 `/tmp/wic-campaign/codex/rep1/result.json`、`rep2/result.json`、`rep3/result.json`、`rep4/result.json` 到 `cadence/reports/workitem-coding-campaign/reports/codex-rep{1,2,3,4}-result.json`；归档覆盖四次运行。rep1 的 30 条 finding 全部归档，其中仅 round-1 的 2 条真实 pass/suggestion finding 进入 golden 覆盖 Advisory，其余 28 条不进入 golden
- [x] 1.2 提取脚本接受显式 campaign 输入根目录参数，不将 `/tmp/wic-campaign` 写死；提取 rep2/3/4 的 9 条原始 finding 与 rep1 round-1 的 2 条 Advisory suggestion，生成 `src/product/work_item_plan_policy/fixtures/golden_findings.json`
- [x] 1.3 原始 rep2/rep4 的 8 条 `needs_human` finding 按 P1-D2 无 hint 兜底标为 `HumanRequired`；rep1 round-1 的 2 条真实 pass/suggestion 标为 Advisory；新增 3 条带 `category`、`class_hint`、`contract_field` 的**人工标注变体**覆盖 Repairable，明确 hint 不来自原始 provider；删除 `auto_repair_allowed`，golden 总数为 14 条（11 原始 + 3 标注变体）
- [x] 1.4 新建 `work_item_plan_policy` 基础模块：`FindingClass`、`ReviewFindingCategory`、`FindingClassHint`、`ClassifiedFinding`、`FindingFingerprint`、`RunBudgets`、`RunHistory`、`RunPolicy`、`WorkItemPlanFlowKind`、`ReviewPhase`、`ReviewInvocationScope`；所有持久化类型列明完整 serde/derive。`RunHistory` 必须为 `#[serde(default, deny_unknown_fields)]` 并实现 Default，保留旧 JSON 缺任意 history 字段恢复为空历史的回归测试
- [x] 1.5 `ReviewInvocationScope` 使用 `Initial { initial_revision_id, scope_digest } | Verification { original_fingerprints, repaired_revision_id, mechanical_report_ref, scope_digest }`；禁止用空字符串表达初评不适用复评字段。scope digest 采用 `review-invocation-scope/v1` 固定字节编码（字段字典序、标量长度前缀、BTreeSet 稳定序）、SHA-256 和 `review_scope_v1:<64 lowercase hex>` 前缀，并测试 canonical 重算/非法值拒绝
- [x] 1.6 指纹归一化（lowercase + Unicode NFC + 连续空白折叠 + trim）与稳定性/区分度测试；`FindingFingerprint` 使用 `#[serde(transparent)]` 并在构造/反序列化校验 64 位小写 hex；更新 `Cargo.toml`/`Cargo.lock` 加入 `unicode-normalization`

## 2. finding schema、显式 parser 与分类器

- [x] 2.1 `ReviewFinding` 增 category/class_hint/contract_field 三字段（缺字段 serde default 兼容），保持 `deny_unknown_fields`；不新增 path、region、affected_paths 或其他无确定性来源的字段
- [x] 2.2 唯一 parser adapter 为 `parse_review_envelope(&serde_json::Value) -> Result<ParsedReviewEnvelope, ReviewStructuredOutputError>`：先预检/解析 RawReviewFinding 的原始字符串，再显式转换 category/class_hint。不得依赖 `Option<Enum>` 的默认 serde 行为处理未知值；未知 category/class_hint 分别产生 `UnknownFindingCategory(raw)`/`UnknownFindingClassHint(raw)`，类型错误产生 `InvalidFindingField`
- [x] 2.3 实现 `classify_review`（hint 优先 + 确定性兜底 + 不可归类 Err）；固定四层错误映射：`UnknownFindingCategory(parser) → ClassificationError::UnknownCategory → FatalReason::UnknownCategory → unknown_finding_category`，以及 class_hint 的对应 `unknown_class_hint` 链路。所有结构化错误 durable failed，绝不降级 NeedsHuman/HumanRequired
- [x] 2.4 服务端生成、持久化 Initial/Verification scope 并由 prompt builder 独占消费。Verification 仅校验原指纹重现和本 invocation 的机械报告重跑；删除 changed-path 归因分支，不以展示用 changed path 参与校验。机械报告缺失/不符、digest 不符等结构性 scope 违例为 `FatalReason::ProtocolViolation`；原指纹越界作为 `HumanRequired(VerificationNewFindings)`，按终态矩阵分流，不得自动返修
- [x] 2.5 golden 14 例（11 原始 + 3 标注变体）、旧 finding、未知 category、未知 class_hint、字段类型错误、`needs_human` raw verdict 保留、scope canonical digest、原指纹越界和机械报告违例测试全过

## 3. 纯策略评估器与预算语义

- [x] 3.1 定义并测试完整 `PlanOutcome`、`FatalReason`（含 serde + Display）、`HumanReason` 三来源、`PolicyDiagnostic`、`RunHistoryDelta` 字段与 checked merge 规则；实现纯 `evaluate(input: &ReviewEvaluationInput, history: &RunHistory, budgets: &RunBudgets) -> EvaluationDecision`
- [x] 3.2 固定五档优先级：transition 预算 → 重复指纹 → aggregate repairable（机械与 review 共用 `max_repairs=1`）→ native human_required → valid；repair budget 耗尽产生 `HumanRequired(RepairBudgetExhausted)`，不产生 Fatal
- [x] 3.3 测试预算精确边界：`max_repairs=0` 立即 HumanRequired；`max_transitions=0`/已达上限立即 Fatal；`max_manual_repairs=0` 时人工门只留批准/终止；持久化计数已超限、checked_add 溢出或 merge 后超限均为 StateCorruption；advisory 不进 seen；纯 evaluator 的 transition delta 恒为 0
- [x] 3.4 单元测试覆盖交互/auto × 首次 repairable/重复指纹/原生 human_required/repair budget 耗尽/transition budget 耗尽/状态损坏，以及 initial/verification/manual repair 的计数上限；Task 3 不读写 store、不创建 durable reservation、不启动 provider

## 4. session 持久化、CAS 协议与 state wire

- [x] 4.1 `WorkspaceSessionRecord`、`CreateWorkspaceSessionInput` 与 lifecycle store 接入 `flow_kind`、`run_policy`、`run_history`、review invocation scope、human gate snapshot、repair reservation、policy diagnostics、provider start ledger；旧 JSON serde default（特别是 RunHistory 缺字段）回归测试
- [x] 4.2 新建 `WorkItemPlanSessionOptions` 并以 `Option<...>` 接入 `CreateWorkspaceSessionInput`；仅 WorkItemPlan 接受 `Some`，其他 WorkspaceType 显式拒绝；rollout flag 由 `WebAppState::work_item_plan_single_candidate` 创建时固化
- [x] 4.3 定义 durable CAS：以 expected session revision 原子提交 history delta、reservation、next state/status、scope、gate、diagnostic 与 provider-start ledger；CAS 冲突重新读取并重评。stage transition 成功持久化后才加 `transitions_used`
- [x] 4.4 定义 `RepairReservation { token, owner_session_id, owner_run_id, provider_start_idempotency_key, state, commit_id }` 与 `Reserved → ProviderStarted → Committed | Released`：provider 启动失败按 token 幂等释放且 repairs_used 不增；重启从 ledger 识别 Reserved/ProviderStarted/Committed 并复用 key，最多启动一次
- [x] 4.5 `WorkspaceSessionStatus` 增 `StoppedNeedsHuman` 与 `Failed`（stable 值 `stopped_needs_human`/`failed`）；`WorkspaceSession`、`WorkspaceEngine::build_session_state`、`WsOutMessage::SessionState`、DTO/type/store 透传 durable `session_status`、`flow_kind`、`run_policy`、`run_history`、`review_invocation_scope`、`human_gate_snapshot`、`policy_diagnostics`、`provider_start_ledger`。不改人工门产品 UI，且 build_session_state 不得从内存补造 gate
- [x] 4.6 为 `$.session_status`、`$.flow_kind`、`$.run_policy`、`$.run_history`、`$.review_invocation_scope`、`$.human_gate_snapshot`、`$.policy_diagnostics`、`$.provider_start_ledger` 写 store/WS/TypeScript compatibility 测试；运行中变更 run_policy/flow_kind 返回 protocol_error

## 5. legacy 路径接入策略层（止血核心）

- [x] 5.1 legacy parity/characterization 测试先行，采用规范化产物比较（忽略动态 ID/时间）并覆盖现有 failpoint/recovery 状态序列
- [x] 5.2 review routing 改造：verdict → explicit parser adapter → classify → evaluate → `route_outcome`，不再由原始 verdict 直接跳转；分类错误按唯一 fatal 链路 durable failed
- [x] 5.3 `GateSnapshotContext` 必须提供 history、budgets、scope、findings、repeated fingerprints、trigger；`route_outcome` 纯映射构造 `HumanGateSnapshot`，其中 `attempts_used = repairs_used + manual_repairs_used`、`manual_repairs_remaining = max_manual_repairs.saturating_sub(manual_repairs_used)`；resumable 仅 auto 的 stopped 且无 fatal/persistence diagnostic
- [x] 5.4 两阶段 review：机械校验 → 初评 → 一次聚合返修 → 限范围复评 → 终态；人工返修复用机械校验 + Verification scope，复评后不再自动返修。manual budget 为 0/耗尽时门只允许批准/终止
- [x] 5.5 按 Task 4 CAS 启动 provider：先 durable next state，再按 reservation token/key 启动；成功 stage transition 后加 transition，启动失败幂等释放；`StoppedNeedsHuman` 快照持久化、指标与 fatal 分离；接管创建 interactive 新运行，不改写原运行

## 6. 人工门快照、恢复与 campaign 适配（真实运行需操作者授权）

- [x] 6.1 新建 `HumanGateSnapshot` 并同 session record 原子写入；恢复测试逐条给出初始 JSON、重连/重启动作、期望 state、provider 启动次数与事件不可变断言：awaiting_human 重连、stopped 接管、completed/failed 重放、generate/repair 中断恢复
- [x] 6.2 provider 启动次数只从 `provider_start_ledger` 已启动 idempotency key 的去重值读取；测试在 reservation 状态和 CAS failure 下均验证最多一次启动、无重复计数
- [x] 6.3 driver 适配：prepare 传 run_policy；首条 session state 校验 `$.flow_kind`/`$.run_policy`；计数只读 `$.run_history.*`；stopped/fatal 只读 `$.session_status`；未知分类只读 `$.policy_diagnostics[*].code`；启动次数只读 `$.provider_start_ledger[*].provider_start_idempotency_key`
- [x] 6.4 为上述 driver protocol JSON path 写 fixture/dry-run 测试，覆盖 needs_human、UnknownCategory、UnknownClassHint、StoppedNeedsHuman、failed、policy 不漂移、无静默重审及启动账本去重
- [x] 6.5 【授权门】操作者授权后 codex + pi 各 1 案例实跑（naruto 语料，auto_if_valid）；验收终态无死循环、**每个 review cycle 内 initial_count ≤ 1 且 verification_count ≤ 1 且 repairs_used ≤ cycle 预算**（session 级总计数仅展示不门控）、golden 14 例（11 原始 + 3 标注变体）分类正确、恢复矩阵与 legacy 回归绿；报告落盘且不依赖 `/tmp`
