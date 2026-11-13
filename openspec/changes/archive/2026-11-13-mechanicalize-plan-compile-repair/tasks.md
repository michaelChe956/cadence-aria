# Tasks: mechanicalize-plan-compile-repair

> 纪律：每个工作包先改/加测试再改实现（TDD）。波次 1 可在 1 天内并行落地；波次 3 需拆文件，单独立项。

## 1. 止血：计划意图分支等价解析（REQ-CRP-03）

- [x] 1.1 先红：`kind=single_repository` + 值在 `intent_target_logical_repo`（唯一可映射到成员物理仓）→ 编译通过且产 `equivalence_canonicalization` 审计；值不在成员表 → typed 诊断且**不采用**；值同时命中逻辑键与物理值 → **不采用**（双命中 fail-closed）；legacy（`member_targets=None`）→ 取会话 target
- [x] 1.2 实现：`lower_plan_intent`（`src/product/work_item_plan_compiler/lower.rs:443-480`）等价解析；复用 `resolve_item_target_repository_id`（`lower.rs:262`）与 member targets
- [x] 1.3 回归：`kind=logical_codebase` 缺 codebase id 仍 typed 报错；单仓/多仓既有 lowering 测试全绿

## 2. 止血：lowering typed fault + 教学资格（REQ-CRP-02、MODIFIED REQ-WSC-09）

- [x] 2.1 先红（三处钉死旧语义的测试，必须先改）：`src/web/workspace_ws_handler/run/single_candidate.rs:1525-1534`、`src/web/workspace_ws_handler/tests/single_candidate_provider_run.rs:1003`（`single_candidate_batched_traceability_lowering_error_fails_terminal_without_reredrive`）、`single_candidate_provider_run/ears_reredrive.rs:8,32-34,293-297`（Task 4：三处已按 typed 口径改红→改绿；`teaching_verdict` 未定义的编译红 + 旧期望不再成立的用例红均已观察）
- [x] 2.2 实现：`CompilerDiagnostic` 加 `lowering_fault: Option<LoweringFault>`（`types.rs:61-80`，无 serde，零 wire 污染）；`lower.rs:1121 diagnostic()` 工厂赋 fault，禁止从 code/message 派生
- [x] 2.3 实现：教学资格判据由 `TEACHABLE_PARSE_FAILURE_CODES`（`run/single_candidate.rs:84-100`）改为「fault 在场且 field+行+期望形态齐全」；重驱 prompt 直接渲染 typed payload（Task 4：`teaching_verdict`/`TeachingPayload`/`TeachingVerdict` + `PreflightVerdict` 落 `run/single_candidate_compile_preflight.rs`；码表保留供 `parse_reasons`）
- [x] 2.4 不变量：同一 candidate 总重驱次数恒 ≤1（既有单槽不变），加断言（Task 4：`second_teachable_category_consumes_no_second_slot` + 既有 invalid_ears 用例双绿；`structure_rerun_used` 分支零改动）
- [x] 2.5 回归：无 fault 的 lowering 仍按既有终态；既有 parse 教学重驱用例全绿（Task 4：`wording_or_prefix_alone_grants_no_eligibility` + ears_reredrive lowering 用例改写为 typed 口径后全绿）
- [x] 2.6 实现：四类 typed IR 诊断 producer（裁决 §1.5）——`src/product/work_item_contract/validation.rs:245-312` 与拆出的 handler 预检产 `unknown_done_when_ref`、`unknown_reviewer_check_ref`、`acceptance_criterion_without_reviewer_check`、`unknown_requirement_ref`；新增内部 `src/product/work_item_plan_compiler/ir_fault.rs` 承载 typed payload（WI、task/check/criterion、错误 ref、本 WI 合法集合或真实 spec 集合、原行——无行即「不可定位」、所需字段形态）；MUST NOT 从现 `:265-267` message 反解析 task ID
- [x] 2.7 先红：数据库/基线/成员解析失败（原 handler `:288-357`）判 Operational——MUST NOT 复用「Some(reasons) 即重驱」；失败集合混合 operational 与可教类时零重驱（禁止 `any(可教)` 放行）；「恰好一次」以前置授权、合法 provider 与剩余机会为条件（Task 4：`PreflightVerdict.operational_failures` + `mixed_operational_failure_set_grants_zero_reredrive` + `teaching_verdict_rejects_mixed_operational_set`）
- [x] 2.8 不变量：新增可教类别必须显式进入规格/typed 白名单（负向测试：`code` 前缀或「缺少」措辞不自动获权）；其余结构 kind 复用 `planning_failure.rs` 事实完整性规则（Task 4：`teaching_verdict_rejects_wording_or_prefix_without_typed_fault` + `wording_or_prefix_alone_grants_no_eligibility`）
- [ ] 2.9（遗留，构造点完备性）：teaching_verdict 对 BranchKeyMismatch 仅校验 expected_field 非空（single_candidate_compile_preflight.rs 的 locatable_lowering_fault），actual_field/value 为空不拒；当前唯一构造点恒写全、无触发路径；后续新增构造点时须在构造点侧补完备断言（2026-11-12 整分支评审 Minor-2 补登记：此前仅在验收记录 §6-E 与 gitignored 台账，不在任何未勾选工作项 ⇒ 后续新增构造点的实施者未必读到）

## 3. 止血：可观测性最小版（REQ-WSC-25）

- [x] 3.1 先红：同一 issue 连续 3 次失败后能读出 3 条记录（阶段/稳定码/fault/指纹/已应用修复/处置）——`src/product/lifecycle_store/tests/candidate_attempts.rs`：3 次失败 + 1 次通过共 4 行，逐条断言阶段/稳定码/typed 种类/结构指纹/自动重写阶段；同一 `attempt_key` 重复落盘零膨胀（幂等）
- [x] 3.2 实现：追加写 `candidate-attempts.jsonl`（与 `candidate-compile-status.json` 同目录）——新 store `src/product/lifecycle_store/candidate_attempts.rs`（追加写 + 读取 + 度量读数：首次通过率 / 同 issue 连续失败次数 P50/P90 / 失败码 Top N / lowering 类终态失败计数）；写入点 `src/product/workspace_engine/compile/single_candidate.rs`（SC Final Compile 边界，失败与成功两侧各追加一行；写失败只降级为静默并记入 policy diagnostics，不影响主流程）
- [x] 3.3 实现：`equivalence_canonicalization` 审计事件（kind/原 key/值/映射依据）落同一序列（Task 4：generate 段 handler 为唯一写入点，`record_equivalence_canonicalizations` 经既有 candidate-attempts store 追加，幂等键=node+内容摘要；completion 复验侧不写——`equivalence_canonicalization_audited_to_candidate_attempts` 断言两次编译仅 1 条）

## 4. 图奇偶性（REQ-WSC-23）

- [x] 4.1 先红：把现场 #1 形态固化成脱敏 fixture（多仓、gateway 消费 frontend 产出、`change_order` 为 gateway→frontend）（Task 5：`tests/single_candidate/structure_failure_passthrough.rs` 同构 fixture（gateway/frontend 两成员仓、gateway 消费 frontend 产出、change_order gateway→frontend），红态实测 `dependency_cycle: 「WI-GATEWAY」 → 「WI-FRONTEND」 → 「WI-GATEWAY」…` + 结构指纹 + 机器 id 段）
- [x] 4.2 断言：同一 fixture 在 Stage1 typed 图与 Final Compile 图的「有效边集合 / 成环结论 / witness 首跳」三者一致；不一致者登记缺陷单（ignored + 说明），修复后转绿（Task 5 核验：`--lib candidate_gate_and_final_compile_agree` 2 用例全绿（effective_edges / dependency_cycle_witness 各一），归档 parity change 交付面零重写）
- [x] 4.3 cleanup：清掉 `compile_support.rs:395`、`:425` 失真的「尚未接线」注释与失效的 `#[allow(dead_code)]`（Task 5：实测共十处（:287-:698 区间），接线证据 `plan_projection.rs` 生产消费 `project_effective_planning_graph`；全部删除后 `cargo check` 零 dead_code 告警，2308→2288 行净减）

## 5. 环的结构化教学（REQ-WSC-24、REQ-CG-08）

- [x] 5.1 先红：环失败时 durable detail 携带逐跳 `from/to/sources`；教学指令含「逆了 `change_order` 哪两个位次」+「保留全部 Inputs/AC、不得改仓序」三条保护项（Task 5 核验：`--lib author_stage_cycle_failure` 1 用例绿（`author_stage_cycle_failure_carries_witness_facts_and_two_owner_options`），归档交付面零重写）
- [x] 5.2 先红（负向）：任何自动删边路径都不存在——构造 `canonical_input` 参与的环，断言平台不产出删边修复（Task 5 核验：`candidate_gate_and_final_compile_agree_on_dependency_cycle_witness` 断言 canonical 边参与环检出后仍以 `CanonicalInput` 来源保留在有效图与 witness 中；`--lib candidate_compile_failure` 8 用例绿；本 Task 新用例断言环 cause 原文（含全部边事实）随 Structure 渲染保留）
- [x] 5.3 实现：`compile/single_candidate.rs:770` 透传 `PlanningCompileFailureKind::Structure`；`compile_support.rs:783-826` 判据改 typed witness（需先拆文件）（Task 5：`types.rs` `InitialPlanCompileFailure{Operational,Structure}` + 新文件 `compile/prepare_failure.rs`（分流单源 `into_compile_failure` + 投影包装）+ `compile/single_candidate.rs` prepare 包裹点分流；witness 以「机器标记段原文进 evidence.details + witness 留空 fail-closed」落地——compile_support 本任务只删不增，typed witness 判据改造随拆文件另行立项；`--lib structure_failure_passthrough` 2 用例绿）
- [x] 5.4 回归：同环重复停判定仍按结构指纹，不按文案（Task 5：`--lib candidate_compile_failure` 8 用例绿（含同环 fingerprint 重复停判定）；新用例断言 `CYCLE_FINGERPRINT_MARKER`/`CYCLE_IDS_MARKER` 随 Structure 渲染保留，指纹解析锚点不回退）

## 6. 一次答完（REQ-CG-09）

- [x] 6.1 先红：需要 owner 决定的失败 verdict 必须含 `owner_question` + ≥2 `owner_options`，每个选项带后果（Task 6 核验：`--lib auto_rewrite_verdict_carries_owner_question` 匹配 1/通过 1——问句与 2 个带后果选项全部由 typed witness 生成，不交模型自由发挥；parity change 已交付，零重写）
- [x] 6.2 实现：`compile_failure_surface.rs:143-144` 由 typed 事实生成问句与选项（不交模型自由发挥）（Task 6 核验：环面 owner 选项 `--lib author_stage_cycle_failure` 匹配 1/通过 1（`structure_parity.rs` 既有用例）；生成面在 `cycle_owner_options` / `auto_rewrite_verdict`，已交付面零改动）
- [x] 6.3 先红：已处置事实不再作为 `owner_question` 呈现（复用既有已处置清单）（Task 6 钉子：新 `tests/candidate_compile_owner_question_regression.rs` 用例 `answered_fact_not_reasked_as_owner_question`——构造含环 owner_question 的失败面 → 经既有 `record_human_dispositions` 通路写入 `computed_finding_key` → 同一未变事实（同结构指纹）下一轮失败面经会话级裁决不再作为 actionable owner 决定面重复提问；改前即绿（parity 已钉），记为回归护栏，不算新增行为；`--lib candidate_compile_owner_question_regression` 匹配 1/通过 1）
- [x] 6.4 D4 裁决落地：已处置清单 `HumanFindingDispositionKind` 现只有 `Excluded`（`src/product/models/human_gate.rs:143-145`），`deny_unknown_fields`，且 key 必须是 16 位 hex（`human_gate.rs:162`）。因此「不再重复问」可复用清单（key 取 typed 事实的结构指纹，不取措辞），但 owner 的**具体选择**无处存放：新增 `Decided{option_id}` 变体须带 `serde(default)` 兼容并单列测试；若不愿改枚举，则本 change 只承诺「同一未变事实不重复提问」，不承诺「持久化选项内容」（Task 6：选收窄支——同一未变事实不重复提问已由 6.3 钉子测试承载；`Decided{option_id}` 变体不新增（避免无写入点的死代码）；持久化选项内容不在本 change 承诺范围）
- [ ] 6.5（**遗留缺口，另开 change**）：web 呈现层未镜像处置过滤 —— `web/src/components/chat-workspace/finding-list.tsx:63` 的 `reviewOwnerGuidance` **遍历全部 findings、不按 `disposition` 过滤**（后端归一只改 disposition、保留问句原文，见 `review/disposition_override.rs:62-66`）⇒ 已处置（advisory）finding 的 `owner_question` 仍被 `ReviewVerdictEntry.tsx:37` / `GatePromptEntry.tsx:140` 渲染为「需要你决定一件事」裁决块。**后端决定面已由 `candidate_compile_owner_question_regression` 钉住（不再重复提问）**；前端需补 `disposition != "advisory"` 过滤或等价镜像。**登记为未勾选项 ⇒ 归档审阅会暴露、不会静默消失。**（2026-11-12 Task 6 评审发现）

## 7. prompt 分层瘦身（≈15.5K → ≤8K）

- [x] 7.1 先加计量测试（REQ-CRP-05）：固定教学块字符数 + 硬措辞计数断言（当前基线 15.5K / 63，目标 ≤8K / ≤25），**先锁目标再裁剪**；每删一条机械条款须在变更说明中指向覆盖它的校验或自修测试（Task 7：`teaching_block_volume_within_budget` 落 `prompt_contract_volume_budget.rs`，常量 8_000/25 逐字；字符闸门度量会话无关固定教学块（单仓形态），硬措辞闸门双形态生效；实测基线 单仓 14,861/63、LC 16,176/69（2026-11-12 红态实测，15.5K/63 与 15,765/53 为不同口径历史值）；逐条映射表见本包提交信息）
- [x] 7.2 按清单删/压：重复 trusted command 段、未知 key 白名单、Plan Intent 分支复述与 `lowering_error` 引文、模板枚举展开、few-shot 11 条 → 3 条、成环预防段各留 1 处（Task 7：清单逐项落地——trusted command 3 句收 1 句、40-key 白名单枚举整行删除、LC 分支复述收短、lowering_error 引文 ×2 删除、值域枚举展开压缩（kind/intent 保留）、few-shot 11→3（rep1-f1/rep1-f2/rep2-f3，golden 全量保留）、仓序成环预防保留单处；同闸门另删 capability 正反例对、AC/输出契约/成对书写长正例、`」` 判例、诊断码清单、LC 写范围段等并逐条登记平台侧承接）
- [x] 7.3 回归：既有 prompt 契约测试按新契约更新（不许删测试）；本包**单独提交**（Task 7：`--lib prompt_contract` 全族 61 passed/0 failed——prompt_contract/baseline_tree/weak_model 断言按新契约更新并补「不回潮」负向断言，零测试删除；plan_intent_target/sc_revision/plan_options/anti_preamble 原断言全保留；计量闸门绿态实测 单仓 7,691 字符/20 硬措辞、LC 8,592/23（LC 字符含数据驱动成员面，作观测值进验收记录））

## 8. O1 落地：内容 autorepair 迁原授权 author（REQ-CRP-01 收窄；裁决 §2.2/§2.3/§2.4）

> O1 已由 owner 裁决（2026-11-08 §2.2/§2.3）：**不改 `REQ-SG-02A`，内容 autorepair 迁原授权 author**。规划裁决完成不等于代码完成，以下工作包均未实施（不预勾）。

- [x] 8.1 移除八轮 source mutation 收敛及其私有 apply/rewrite 死代码：`src/product/workspace_engine/contract_autorepair.rs:54-104,630-702`；四类 producer 按裁决 §2.2 收窄，不保同名旧行为 shim
- [x] 8.2 `contract_autorepair.rs:105-267,416-625` 与 `contract_prerevision.rs:353-356`：把「本地收敛消费」注释与接口清切为 author 指令——`required_capability_missing` 取消自动追加 capability；`unconsumed_required_handoff` 取消删 ref 或替换为 `[]`；`unknown_structured_key` 不再按诊断行删业务承载行，只生成「保留内容、迁往合法字段」的指令；重复 trusted command 取消本地删 command/整 check
- [x] 8.3 两个生产 caller 同时切到 compile/typed 诊断→原授权指令链：`src/web/workspace_ws_handler/run/single_candidate.rs:1214-1219`、`src/product/workspace_engine/single_candidate.rs:465-479`——不能只改 handler 而 completion 又二次暗修
- [x] 8.4 测试迁移：新 `src/product/workspace_engine/tests/contract_autorepair_migration.rs`（承接原 `contract_autorepair.rs` 内联测试块的收窄口径改写，另按 8.4 口径改写 4 处 caller 面旧「自动补/删成功」用例；`contract_prerevision.rs:1733-1848` 两个 sg_07 调用测试原地保留、断言未动——其断言与 mutation 语义正交，收窄后仍有效，identity 绑定面由原位 sg_07 用例继续覆盖）：旧「自动补/删成功」断言改为 source 不变、指令完整、author 新产物及复验
- [x] 8.5 验收（裁决 §2.5）：四类规则各测成功指令与拒绝场景——source 字节不变；持久/已 review 产物不改；原 author 身份及同一剩余 attempt；新产物全量复验；未知 key 携业务正文不得消失，整 check 不得无故消失

## 9. 收口

- [x] 9.1 `openspec validate mechanicalize-plan-compile-repair --strict` exit 0（Task 8 实测 2026-11-12：`Change 'mechanicalize-plan-compile-repair' is valid`，exit 0；归档演练 /tmp 复核 81 passed / 0 failed，演练目录已删）
- [x] 9.2 全量门禁（fmt / clippy -D / check / test）绿；预存基线按名对照（Task 8 复核 2026-11-12，`854dab48` 修复后**自跑**：fmt --all --check exit 0；`--lib` 6338 passed/0 failed/7 ignored（运行 6345）；it_core 221/0/1（运行 222，行数守卫含内）；it_web 631/0/17（运行 648）；clippy -D warnings exit 0、0 error。初测 `--lib` 曾 1 failed＝Task 7 措辞改写（`，`→`、`）漏更的族外陈旧断言（single_candidate_provider_run.rs:1305，非行为回归），登记验收记录 §5-G 后由 854dab48 一行修复、全仓同类漂移扫描零新增）

## 10. 待验证假设（不改契约，取证后再决定是否追加 requirement）

- [x] 10.1 现场 #3「reviewer 连报三轮」归因：①`single_candidate_consecutive_repeated_finding_count`（`src/product/workspace_engine/review/routing_scope.rs:815-849`）要求相邻两轮非 advisory 指纹集合**完全相等**，任一增减 finding 即不触发，且仅比较相邻两轮；②指纹由 category/class/message/contract_field 组成（定义 `work_item_plan_policy/fingerprint.rs:73 identity_for_finding`，调用点 `classify.rs:141-146`），无 category 时措辞敏感；③owner 排除 key=`sha256(severity+message)`（REQ-WSC-13，`web/workspace_ws_types/review.rs:157-159`），换措辞即新 key。用现场三轮 verdict 回放核对哪个生效，结论写入 designs 文档后再决定是否新增「finding 身份稳定性」requirement（Task 8：取证记录落档 `cadence/notes/2026-11-11_取证记录_reviewer连报三轮归因_v1.0.md`——锚点实测行号 routing_scope.rs:814-848 / fingerprint.rs:73 + classify.rs:141-146 / review.rs:148-159；合成三轮 verdict fixture 回放（排除键为按 ③ 算法当场 sha256 实算的 16 位 hex）；现场 durable history 未提供 ⇒ 按裁决 §8.4「挂起待材料」，结论仅限代码级归因；是否新增 requirement 待 owner 决定）
- [x] 10.2 `outbound_queue_full`：另开工单（不在本 change），工单引用 `src/web/workspace_session/manager/degraded_diagnostics.rs:101`（Task 8：已在取证记录 §5「关联工单」登记——degraded_diagnostics.rs:101 锚点实测（`record_enter` 稳定码）；proposal Non-Goals :36 已声明）
