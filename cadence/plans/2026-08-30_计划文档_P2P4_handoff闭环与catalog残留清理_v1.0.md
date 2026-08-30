# P2+P4 handoff 闭环与 trusted catalog 残留清理实施计划 v1.0

**Goal：** 按 OpenSpec change `rearch-workitem-plan-pipeline` 的用户批准增补 §8.1–§8.4，先以独立提交粒度让 Single Candidate（下称 SC）IR 机械校验不再受已退役 trusted verification command catalog 语义约束，同时保持 SC 的 ID、traceability、scope、budget、依赖、环与写范围规则，以及 legacy draft 行为原样；随后补齐 author 的 handoff 消费闭环教学，并将 reviewer 的 B2 同源能力覆盖投影扩展为依赖图、handoff 消费闭环和跨 item 写范围冲突事实，使 `unconsumed_required_handoff` 在 Final Compile 前成为可行动的 `must_fix` / `contract_gap` finding。

**Architecture：** P4 不删除任何全局 validator 函数或规则，而是在 `validate_plan_candidate_ir` 使用显式 SC 校验 profile：复用 outline 的非 catalog 校验集合，并在 SC draft 校验中跳过 `missing_trusted_verification_command_catalog` / `untrusted_required_verification_command`；legacy 的 `WorkItemPlanOutlineValidator::validate`、`WorkItemDraftLocalValidator::validate` 与既有测试继续走原 profile。P2-author 只修改 SC full-author 的 reference discipline，并把唯一质量预算从 16,200 上调为 17,000。P2-reviewer 延续 B2 已落地的 `DependencyContractGraph` → typed projection → `append_review_context_section` 链路：把 capability、依赖图、handoff consumer 集合及写范围冲突汇总为确定性只读 reviewer projection；其中 handoff consumer 计算从 `report_unconsumed_handoffs` 抽成共享纯 helper，validator 与 reviewer 共用，不复制判定；reviewer 初评与复评继续共用 `build_single_candidate_plan_review_input`，64 KiB builder 末尾与 scope 追加后双点门禁保持不动。

**Spec（执行前必须全文复核，唯一范围与契约来源）：**

- `openspec/changes/rearch-workitem-plan-pipeline/tasks.md`：§8.1–§8.4；§8.5 的 r25 三 provider 重跑不在本 Plan。
- `openspec/changes/rearch-workitem-plan-pipeline/specs/work-item-plan-single-candidate/spec.md`：REQ-WSC-06 最新 SHALL 与三个 reviewer/author scenario；本 change 不存在根级 `spec.md`，以该 capability spec 为准。
- `openspec/changes/rearch-workitem-plan-pipeline/design.md`：`P2+P4 裁决（2026-08-30，用户批准）`，并遵守 `架构简化裁决` 与 B2 既有边界。
- 前置完成项：B1 commit `e78094a9`（author 能力覆盖教学）与 B2 commit `0f47060d`（reviewer 能力覆盖投影）；本 Plan 只在其上增量扩展，不重做、不回退。

## Global Constraints

- **范围固定：** 只实施 `tasks.md` §8.1–§8.4；不实施 §8.5 r25、P3 九组全量 checklist、driver 卫生、95% 专项测量或 provider 实跑；不新增公共协议、持久化格式、依赖或前端行为。
- **实施顺序与提交粒度：** 必须先完成并验证 Task 8.1（P4），将其作为独立提交粒度与 P2 隔离，便于单独审查/回滚；本文不包含任何 `git commit` step，提交由 orchestration 执行。Task 8.2 → 8.3 → 8.4 按用户批准顺序继续。
- **SC 专用退役，不全局删除：** `validate_plan_candidate_ir` 必须显式选择 SC profile，跳过 trusted catalog 的条目数、字段长度、投影字节、membership/count 及 `missing_trusted_verification_command_catalog` / `untrusted_required_verification_command` 规则；全局 validator 函数、错误码与 legacy 调用入口不删除、不改名。默认直接跳过旧字段长度门，因为它属于已退役 outline catalog 的序列化预算、错误码也携带 outline 语义；本工作包不另造通用 bounded-field 规则。若实施中发现独立安全边界确实依赖该长度，立即停止并回到 OpenSpec 裁决，不得自行扩大范围。
- **SC 非 catalog 行为不变：** ID、traceability、scope、context budget、依赖存在性、依赖图投影、环、direct-dependency scope 与跨 item exclusive/forbidden 写范围规则继续执行；Task 8.1 RED/GREEN 必须同时证明 blank ID 与 budget 超限仍失败。
- **legacy 零回归：** legacy outline/draft 路径和其既有测试不改；legacy 的 trusted catalog 条目数、字段长度、投影 bytes、缺 catalog、required command membership 规则继续 fail closed。既有测试零修改且全绿是零回归证据；禁止通过改 legacy fixture 或删断言达成 GREEN。
- **canonical validator 红线：** 除 §8.1 明确退役的 SC catalog 残留外，canonical validator 对外 findings code、severity、字段、消息、排序及 fail-closed 行为零改动；`required_capability_missing`、`unconsumed_required_handoff`、依赖/环/重复边/未知 provider 与写范围冲突语义保持不变。
- **同源投影：** reviewer 只消费 `dependency.rs` 的 typed、确定性共享计算结果；handoff consumer 集合必须由 `report_unconsumed_handoffs` 同源 helper 计算，不能在 `review.rs` 或 prompt 文本复制 membership 算法。空消费者必须序列化为显式 `consumers: []`、`consumed: false`；所有集合与输出顺序稳定排序、去重。
- **依赖与 scope 事实：** reviewer projection 必须包含逐 item `depends_on`、确定性 edges、cycles、duplicate edges、unknown providers，以及跨 item exclusive/exclusive、exclusive/forbidden 写范围重叠事实；尽量复用既有 `build_dependency_contract_graph` / `validate_dependency_contract_graph` 与 outline scope 校验的确定性产物，不修改其 validator 对外结果。
- **reviewer 行为边界：** `unconsumed handoff` 必须教学为 `severity=must_fix`、`category=contract_gap`、`class_hint=repairable`，evidence 点名 provider、contract_ref 与空 consumer 集；能力缺口 B2 教学保持不变。仅 SC reviewer 注入，初评与 Verification 复评共用；legacy/story/design reviewer 不出现新增投影标题或教学。
- **prompt 预算：** split-engine 预算唯一上调为 `WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES: 16_200 → 17_000`，约留 800 bytes；`WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES = 65_536` 不动，legacy draft 的 15,600 预算不动。reviewer 的 `SINGLE_CANDIDATE_REVIEW_PROMPT_MAX_BYTES = 64 * 1024`、builder 末尾与 scope 追加后双点检查、同一 helper 均不动。
- **scope/digest/CAS：** `ReviewInvocationScope` 内容、scope instructions、fingerprint、digest、CAS、durable IR/mechanical report binding 与 publication freshness 零改动；新 projection 是只读 context，不成为第二事实来源。
- **目录与产物禁区：** 不碰 `.aria/`；不修改 legacy/story/design 生产路径；不修改 coding engine、runtime binding、provider schema、WS/UI；实现只允许触碰各 Task `Files` 列明的 Rust 源码/测试。本文档之外本次规划任务不改任何代码。
- **large file guard：** 任一源码或测试文件修改后不得超过 1200 行。尤其 `src/product/workspace_engine/prompts/review.rs` 与 `src/product/workspace_engine/tests/single_candidate_prompt.rs` 必须先量行；新增 projection 类型/构建放 `dependency.rs` 或 `review_context.rs`，`review.rs` 只保留最小构建与一行 section 注入；B2/P2 测试继续放 `single_candidate_reviewer_coverage.rs`，不得向 `single_candidate_prompt.rs` 追加。
- **TDD：** 每个行为先加 RED 并实际运行确认目标断言失败，再做最小 GREEN；先用 `cargo test --locked --lib` 加目标测试过滤名和 `-- --list` 确认匹配数大于 0，不接受 0-test 假绿或无关编译错误冒充 RED。定向 cargo 命令必须带 `--lib`。
- **Cargo 命令：** 任何 cargo 命令禁止显式 `-j`（包括 `-j 1`）；宿主机 worktree 根目录直接运行，不用 Docker。定向测试仅使用 `cargo test --locked --lib` 加一个真实测试过滤名。
- **全量门禁（全文、顺序固定）：**

  ````bash
  cargo fmt --check
  cargo clippy --all-targets --all-features --locked -- -D warnings
  cargo check --locked
  cargo test --locked
  ````

- **flaky 定性：** 任一全量/定向失败先保留首轮日志并按同一命令重跑一次；若同一测试同一断言稳定复现，按回归处理；只有失败项、线程/端口/时序症状变化且单测隔离通过时，才可标为 flaky 家族，并必须记录首轮与复跑结果，不能以“历史 flaky”跳过门禁或修改产品语义迁就测试。
- **真实 Provider：** 不自动调用 Provider。由于 Task 8.2 修改 Work Item Draft/SC author prompt，交付前必须提醒操作者按 `cadence/designs/2026-07-24_技术方案_WorkItemDraftPrompt测试基线_v1.0.md` 授权执行 Case A、Case B 各 10 个有效首次输出；该运行与 r25 均不属于本 Plan。
- **实施前复核：** 编辑前再次用 `ast-grep outline` 复核所有符号与真实行号；本文行号来自 2026-08-30 当前 worktree 侦察，漂移只更新定位，不允许扩大范围。

## Task 8.1：P4——SC 路径退役旧 trusted catalog 残留（先行、独立提交粒度）

**可追溯性：** `tasks.md` §8.1；REQ-WSC-06 的 handoff/validator fail-closed 边界与 REQ-WSC-02 typed IR 校验边界；design.md `P2+P4 裁决`、`架构简化裁决`；P4 必须先于 P2 独立审查。目标是把 2026-08-29 已 superseded 的 outline→trusted catalog 规则从 SC IR 调用路径移除，而不是修改 canonical validator 全局语义。

**现状侦察基线（已重新核对当前 worktree，非照抄需求）：**

- `src/product/work_item_plan_compiler/validate.rs:24-83` 的 `validate_plan_candidate_ir(&PlanCandidateIr, &PlanCandidateValidationContext<'_>) -> Result<PlanCandidateMechanicalReport, Vec<CompilerDiagnostic>>` 先将 IR 投影为 `WorkItemPlanOutline`/draft/plan/work item/verification plan，再在第 34 行调用 `WorkItemPlanOutlineValidator::validate(&outline)`，之后对每个 draft 在第 49 行调用 `WorkItemDraftLocalValidator::validate(draft, &accepted_dependencies, &outline)`。
- `src/product/work_item_split_validator/types.rs:41-52` 当前 `WorkItemPlanOutlineValidator::validate(&WorkItemPlanOutline) -> WorkItemSplitValidationReport` 固定依次调用 `validate_outline_ids`、`validate_outline_traceability_and_scopes`、依赖、环和 scope 冲突；其中 `src/product/work_item_split_validator/outline.rs:44-124` 的 `validate_outline_traceability_and_scopes` 在第 83 行无条件调用 `validate_trusted_verification_command_catalog`。
- `src/product/work_item_split_validator/outline.rs:126-177` 当前 catalog 规则确实检查三类残留：`trusted_verification_commands.len()` 的条目数、command/cwd/purpose/source_ref 字段长度（错误码 `trusted_verification_command_catalog_field_too_large`），以及 `trusted_draft_verification_command_catalog_prompt_bytes` 投影预算（错误码 `trusted_verification_command_catalog_projection_too_large`）。该函数不是死代码：CodeGraph 重新确认它被 `validate_outline_traceability_and_scopes` 与 `src/product/work_item_split_engine/parse.rs` 的 legacy outline authoring 路径调用。
- `src/product/work_item_split_validator/types.rs:55-83` 当前 `WorkItemDraftLocalValidator::validate` 在 canonical/identity/provider/scope/verification 校验后，于第 70-75 行无条件调用 `draft::validate_draft_trusted_verification_commands`；`src/product/work_item_split_validator/draft.rs:225-280` 仍会生成 `missing_trusted_verification_command_catalog` 与 `untrusted_required_verification_command`，并通过 `src/product/work_item_split_engine/parse.rs` 的 legacy draft 编排及 `src/product/workspace_engine/draft_batch` 调用链保留。
- `src/product/work_item_split_validator/outline.rs:179-408` 仍包含 SC 必须保留的 dependency-not-in-outline、dependency graph projection、cycle 与 `write_scope_conflict`/`parallel_scope_overlap` 规则；这些函数不能因移除 catalog 调用而跳过。当前 `project_outline`（`validate.rs:96-152`）为 SC 生成固定 `estimated_context_tokens: Some(30_000)`、`FitsSingleAgentSession`，因此 budget 正常 fixture 不应误报；RED 必须显式改成超预算 IR 才证明规则仍接通。
- 行数基线已量取：`validate.rs` 358、`types.rs` 84、`outline.rs` 408、`draft.rs` 318；SC 相关既有 fixture 在 `src/product/work_item_plan_compiler/tests/full_lowering_validator.rs:1-193`。仓库中不存在用户文字所称的 `src/product/work_item_split_validator/parse.rs`；真实联动文件是 `src/product/work_item_split_engine/parse.rs`，实施时以 outline/CodeGraph 复核后的现行路径为准。

**Files：**

- **Modify:** `src/product/work_item_plan_compiler/validate.rs:24-83,96-152`：引入 SC 专用 outline/draft validation profile 的选择与调用；保持 `project_outline` 的 IR→legacy DTO 投影仅用于复用非 catalog 机械规则，不改变 `PlanCandidateIr`、`PlanCandidateMechanicalReport`、diagnostic 结构或排序。
- **Modify:** `src/product/work_item_split_validator/types.rs:41-83`：在不改变既有 public validator 签名/行为的前提下，补充 crate 内 SC 专用 profile 的最小入口（或将 profile 组装留在 compiler 并只调用既有公开的细粒度函数）；不得让 `WorkItemPlanOutlineValidator::validate` 或 `WorkItemDraftLocalValidator::validate` 的 legacy 默认入口变成 SC 语义。
- **Modify:** `src/product/work_item_split_validator/outline.rs:11-177`：把 traceability/scope/budget 循环抽成不含 catalog 的共享 helper，legacy wrapper 继续调用 `validate_trusted_verification_command_catalog`；不得删除/改名 catalog 函数或旧常量。
- **Modify:** `src/product/work_item_split_validator/draft.rs:9-280`：供两个 validator 入口复用现有 canonical/identity/provider/scope/verification/direct-dependency 函数；`validate_draft_trusted_verification_commands` 保持实现与 legacy 调用不变。
- **Read-only call-site verification:** `src/product/work_item_split_engine/parse.rs:177` 附近及 `src/product/workspace_engine/draft_batch`：确认 legacy outline/draft authoring 仍调用完整 legacy validator；不编辑这些路径，除非编译错误证明仅需最小导入调整。
- **Test:** `src/product/work_item_plan_compiler/tests/full_lowering_validator.rs:1-193`：新增 SC P4 RED/GREEN 测试，构造长 command 字段、blank ID 与超预算 outline 投影；既有 rep4、unknown provider、byte-stability 测试不改。
- **Test:** `src/product/work_item_split_validator/tests.rs`（以 `ast-grep outline` 取得当前实际符号/行号）：复跑而不改 legacy catalog validator 测试；如需补充 legacy 路径回归，只能在现有测试模块新增测试，不能改已有断言或 fixture 语义。

**Interfaces：**

- **Consumes：** SC `validate_plan_candidate_ir(ir: &PlanCandidateIr, context: &PlanCandidateValidationContext<'_>)` 的 typed IR 与 compiler context；现有 `project_outline` 产生的 `WorkItemPlanOutline`；现有 `project_drafts` 产生的 `Vec<WorkItemDraftCandidate>`；每个 item 的 `CanonicalWorkItemContract`、`WorkItemDraftVerificationPlan`、`trusted_commands` 仅作为既有投影输入。
- **Produces：** `Result<PlanCandidateMechanicalReport, Vec<CompilerDiagnostic>>`；SC 路径继续输出除退役 catalog finding 外的全部现有 finding，catalog membership/count/field/projection finding 不再出现。legacy `WorkItemPlanOutlineValidator::validate(&WorkItemPlanOutline) -> WorkItemSplitValidationReport` 与 `WorkItemDraftLocalValidator::validate(&WorkItemDraftCandidate, &[WorkItemDraftCandidate], &WorkItemPlanOutline) -> WorkItemSplitValidationReport` 的输出契约不变。

```rust
impl WorkItemPlanOutlineValidator {
    pub fn validate(outline: &WorkItemPlanOutline) -> WorkItemSplitValidationReport;

    pub(crate) fn validate_for_single_candidate(
        outline: &WorkItemPlanOutline,
    ) -> WorkItemSplitValidationReport;
}

impl WorkItemDraftLocalValidator {
    pub fn validate(
        current: &WorkItemDraftCandidate,
        accepted_dependencies: &[WorkItemDraftCandidate],
        outline: &WorkItemPlanOutline,
    ) -> WorkItemSplitValidationReport;

    pub(crate) fn validate_for_single_candidate(
        current: &WorkItemDraftCandidate,
        accepted_dependencies: &[WorkItemDraftCandidate],
        outline: &WorkItemPlanOutline,
    ) -> WorkItemSplitValidationReport;
}
```

`validate` 是 legacy 完整 profile，签名、调用顺序与结果不变；`validate_for_single_candidate` 是 crate 内 SC profile，供 compiler 和本模块单测调用。outline.rs 将当前循环拆为 `validate_outline_traceability_scopes_and_budget(outline: &WorkItemPlanOutline, findings: &mut Vec<WorkItemSplitFinding>)`（不含 catalog）和 legacy wrapper：legacy wrapper 调共享 helper 后再逐 item 调 `validate_trusted_verification_command_catalog`；SC 只调共享 helper。draft 的两入口共同调用私有 `validate_draft_common(current: &WorkItemDraftCandidate, accepted_dependencies: &[WorkItemDraftCandidate], outline: &WorkItemPlanOutline, include_trusted_catalog: bool)`，该布尔只控制 `validate_draft_trusted_verification_commands`，其它调用顺序不变。SC 分支必须执行 IDs、traceability、scope、budget、dependencies、cycles、scope conflicts，以及 draft 的 canonical/identity/provider/scopes/verification/direct-dependency checks，唯一不执行 catalog 三项与 draft catalog 两项。

**步骤：**

- [ ] **Step 1 — 记录当前符号与写 RED 测试。** 在编辑任何实现前执行：

  ````bash
  ast-grep outline src/product/work_item_plan_compiler/validate.rs
  ast-grep outline src/product/work_item_split_validator/types.rs
  ast-grep outline src/product/work_item_split_validator/outline.rs
  ast-grep outline src/product/work_item_split_validator/draft.rs
  ast-grep outline src/product/work_item_split_engine/parse.rs
  ast-grep outline src/product/work_item_plan_compiler/tests/full_lowering_validator.rs
  wc -l src/product/work_item_plan_compiler/validate.rs src/product/work_item_split_validator/types.rs src/product/work_item_split_validator/outline.rs src/product/work_item_split_validator/draft.rs src/product/workspace_engine/prompts/review.rs
  ````

  在 `full_lowering_validator.rs` 先增加三个可执行 RED case（使用既有 `compile_work_item_plan` 与 `rep4_validation_context` fixture，所有其他字段保持合法）：

  ````rust
  #[test]
  fn p4_sc_long_verification_command_does_not_emit_catalog_field_finding() {
      let mut ir = compile_work_item_plan(
          REP4_FIXTURE,
          &WorkItemPlanSourceContext { target_repository_id: "repo-levels".to_string() },
      ).expect("rep4 fixture must lower");
      let long_command = format!("cargo test {}", "x".repeat(49));
      ir.items[0].verification_plan.checks[0].command = Some(long_command.clone());
      ir.items[0].contract.verification_checks[0].command = Some(long_command.clone());
      ir.items[0].trusted_commands[0].command = long_command;
      let story_ids = vec!["story_spec_levels_0001".to_string()];
      let design_ids = vec!["design_spec_levels_0001".to_string()];
      let result = validate_plan_candidate_ir(
          &ir,
          &rep4_validation_context(Some(&rep4_repository_profile()), &story_ids, &design_ids),
      );
      let catalog_field_finding_exists = match result {
          Ok(report) => report.findings.iter().any(|finding|
              finding.code == "trusted_verification_command_catalog_field_too_large"),
          Err(diagnostics) => diagnostics.iter().any(|diagnostic|
              diagnostic.code == "trusted_verification_command_catalog_field_too_large"),
      };
      assert!(!catalog_field_finding_exists);
  }

  #[test]
  fn p4_sc_outline_identity_and_budget_rules_remain_active() {
      let mut ir = compile_work_item_plan(
          REP4_FIXTURE,
          &WorkItemPlanSourceContext { target_repository_id: "repo-levels".to_string() },
      ).expect("rep4 fixture must lower");
      ir.items[0].contract.identity.logical_work_item_id.clear();
      let story_ids = vec!["story_spec_levels_0001".to_string()];
      let design_ids = vec!["design_spec_levels_0001".to_string()];
      let diagnostics = validate_plan_candidate_ir(
          &ir,
          &rep4_validation_context(Some(&rep4_repository_profile()), &story_ids, &design_ids),
      ).expect_err("blank canonical identity must fail closed");
      assert!(diagnostics.iter().any(|diagnostic|
          diagnostic.code == "blank_logical_work_item_id"));
  }

  #[test]
  fn p4_legacy_draft_trusted_catalog_rules_remain_active() {
      let mut outline = valid_outline();
      outline.work_item_outlines[0].trusted_verification_commands.clear();
      let candidate = canonical_draft_candidate(&outline.work_item_outlines[0]);
      let report = WorkItemDraftLocalValidator::validate(&candidate, &[], &outline);
      assert!(report.findings.iter().any(|finding|
          finding.code == "missing_trusted_verification_command_catalog"));
  }

  #[test]
  fn p4_sc_outline_budget_rules_remain_active() {
      let mut outline = valid_outline();
      outline.work_item_outlines[0].estimated_context_tokens = Some(50_001);
      outline.work_item_outlines[0].session_fit = Some(
          WorkItemOutlineSessionFit::TooLargeMustSplit,
      );
      let report = WorkItemPlanOutlineValidator::validate_for_single_candidate(&outline);
      for code in [
          "outline_exceeds_single_session_budget",
          "outline_too_large_must_split",
      ] {
          assert!(report.findings.iter().any(|finding| finding.code == code));
      }
      assert!(report.findings.iter().all(|finding|
          !finding.code.starts_with("trusted_verification_command_catalog_")));
  }
  ````

  第一、二个测试写入 `full_lowering_validator.rs`；legacy 与 SC budget 测试写入 `work_item_split_validator/tests.rs`，因此可直接使用该模块已有私有 `valid_outline`/`canonical_draft_candidate`。长 command 的 `49` 字符来源是当前 `work_item_plan_outline_validator_rejects_overlong_trusted_command_catalog_fields` 已核实边界（49 会触发当前 field finding）；`cwd=17`、`purpose=33`、`source_ref=33` 的既有边界测试保持不动。测试代码不得使用省略号、伪 fixture 或不确定错误码；budget case 必须使用 SC profile 入口，而不是固定投影 30,000 的 compiler IR 伪造预算。

  先确认过滤器非零并运行 RED：

  ````bash
  cargo test --locked --lib p4_sc -- --list
  cargo test --locked --lib p4_sc
  cargo test --locked --lib p4_legacy_draft_trusted_catalog_rules_remain_active
  ````

  预期：SC 长 command case 在当前实现失败且包含 `trusted_verification_command_catalog_field_too_large`；blank ID/budget case 在 profile 尚未接入前保持既有错误，并在加入断言后确保测试 fixture 确实命中目标；legacy case 全绿或若测试尚不存在则按 `-- --list` 的真实过滤名重跑，绝不接受 0-test。若 RED 失败来自 fixture 构造/编译而非 catalog finding，先修正 fixture，不把非目标错误算作 RED。

- [ ] **Step 2 — 实现显式 SC outline validation profile。** 在 `work_item_split_validator` 内抽出“不含 catalog”的 outline 规则组合，或新增 `validate_outline_with_profile`：Legacy 分支严格调用现有 `WorkItemPlanOutlineValidator::validate` 完整链；SingleCandidate 分支只调用 `validate_outline_ids`、traceability 的非 catalog部分、`validate_outline_dependencies`、`validate_outline_dependency_cycles`、`validate_outline_scope_conflicts`。不要改 `validate_outline_traceability_and_scopes` 的 legacy 行为，不要删除/重命名 `validate_trusted_verification_command_catalog`。在 `validate_plan_candidate_ir` 把第 34 行的默认调用替换为 SC profile 调用，保留后续 `WorkItemSplitValidator::validate`、finding sort、report/error conversion 完整不动。

- [ ] **Step 3 — 实现显式 SC draft validation profile。** 在 SC profile 中复用 draft 的 canonical contract、identity、provider logical IDs、scope、verification plan、direct dependency scope 规则，但不调用 `validate_draft_trusted_verification_commands`。Legacy `WorkItemDraftLocalValidator::validate` 保持原调用顺序与 catalog 两个 finding。确认 SC 不通过删除全局函数或全局常量来放开命令边界；`trusted_commands` 仍可由 `project_verification_plans` 投影为安全执行输入，安全执行 gate 不在本 Task 改动。

- [ ] **Step 4 — GREEN 与 legacy parity。** 运行新测试并逐项检查“仅移除 catalog finding、其他 finding 不变”：

  ````bash
  cargo test --locked --lib p4_sc
  cargo test --locked --lib p4_legacy_draft_trusted_catalog_rules_remain_active
  cargo test --locked --lib full_lowering_validator
  cargo test --locked --lib work_item_split_validator
  ````

  预期：SC 长 command 不再产生 `trusted_verification_command_catalog_field_too_large`（若仍有其它独立 Error，则断言该具体 catalog finding 不存在）；SC blank ID、budget、dependency/cycle/scope conflict 仍 fail closed；legacy 空 catalog/required untrusted command finding 与既有代码完全一致。`full_lowering_validator` 的原有 rep4、unknown provider、byte-stability 测试必须全绿且不改；若过滤器命中 0，运行该模块的 `-- --list` 并使用列表中显示的真实测试名重跑。

- [ ] **Step 5 — P4 独立审查前的门禁与提交边界。** 确认所有修改文件均属于本 Task Files、每个文件不超过 1200 行、`git diff --check` 无空白错误；只在 P4 评审通过后由 orchestration 创建独立提交，本 Plan 不写 commit 命令。执行：

  ````bash
  cargo fmt --check
  cargo clippy --all-targets --all-features --locked -- -D warnings
  cargo check --locked
  cargo test --locked
  git diff --check
  find src/product/work_item_plan_compiler src/product/work_item_split_validator -type f -name '*.rs' -print0 | xargs -0 wc -l | awk '$1 > 1200 {print}'
  ````

  四条 Cargo 门禁预期全部通过；large-file 命令无输出。若任一门禁失败，按 Global Constraints 的 flaky 定性规则重跑同一命令并记录稳定回归/可复现 flaky，不得绕过或提交。

## Task 8.2：P2-author——SC full-author handoff 消费闭环教学

**可追溯性：** `tasks.md` §8.2；REQ-WSC-06 author handoff SHALL 与 scenario「provider handoff 引用无消费者时 reviewer/canonical 不得无保留通过」；design.md `P2+P4 裁决`、D6 prompt 分层；前置 B1 `e78094a9` 的能力覆盖教学风格。此 Task 只教 provider 如何在生成前修正 source，不新增 parser/lowering 语义或平行 `handoff_expectations` 来源。

**Files：**

- **Modify:** `src/product/work_item_split_engine/prompts.rs:52-61,143-163`：在 `work_item_plan_markdown_reference_discipline` 的 `[cross_reference_discipline]` 段增加 handoff 消费闭环纪律、逐字二元组定义、正反例与 canonical fail-closed 后果；把 `WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES` 从 `16_200` 改为 `17_000`，更新注释，legacy `WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES = 15_600` 与硬上限不变。
- **Modify/Test:** `src/product/work_item_split_engine/tests/prompt_contract.rs:787-995`：在现有 SC full-author prompt contract 测试中同步更新预算断言至 17,000，并增加 handoff 教学存在性、逐字匹配规则、正例和反例唯一出现次数断言；保持已有 grammar/capability/few-shot/legacy 隔离断言原样。
- **Read-only boundary:** `src/product/work_item_contract/model.rs:136-148` 与 `src/product/work_item_plan_compiler/lower.rs:193-244`：确认 `HandoffContract { provided_contract_refs, reviewer_check_refs }` 是唯一 schema，且 lowering 只从 markdown Handoff Schema 生成；不修改字段、lowering、canonical validator 或 reviewer projection。

**Interfaces：**

- **Consumes：** `build_work_item_plan_markdown_prompt(&GenerateWorkItemsRequest, &IssueRecord, &RepositoryRecord, WorkItemPlanMarkdownAuthorContext<'_>) -> Result<String, String>`；该函数通过 `work_item_plan_markdown_reference_discipline(Some(context.design_requirement_ids))` 生成 SC full-author prompt；现有 `CanonicalWorkItemContract.handoff_contract.provided_contract_refs` 与下游 `CanonicalWorkItemContract.input_contracts` 的 schema 语义。
- **Produces：** SC author 的 prompt 字符串中 `[cross_reference_discipline]` 的确定性教学文本；质量预算常量 `#[cfg(test)] pub(crate) const WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES: usize = 17_000`。不产生运行时 handoff 值、不改变 `PlanCandidateIr` 或 validator findings。

```rust
// 教学必须表达的稳定语义（文案可按既有中文风格微调，但这些字段和值必须逐字可检索）：
provided_contract_refs 中每项必须被至少一个下游 Work Item 的 input_contracts
以 (provider_logical_work_item_id, contract_id) 逐字二元组消费；无消费者时，
必须在生成前修正，否则 canonical validator 以 unconsumed_required_handoff 拒绝。

// 最小正反例必须同时出现：
// 反例：WI-002 handoff 提供 CT-005，但任何下游 input_contracts 没有
// (provider_logical_work_item_id=WI-002, contract_id=CT-005) → 拒绝/生成前修正。
// 正例：WI-002 提供 CT-005，WI-003 Inputs 逐字写 provider_logical_work_item_id: WI-002
// 与 contract_id: CT-005 → consumed=true；不要依赖 title、depends_on 或自然语言描述推断消费。
```

二元组必须明确 provider 是提供该 handoff 的 item 的 `logical_work_item_id`，contract_id 是 `provided_contract_refs` 中的完整字符串；consumer 不是由 `depends_on` 或 title 推导，只有下游 `input_contracts` 中二者同时逐字相等才计消费。一个 provided ref 可以有多个消费者；每个 provided ref 至少一个消费者；空消费者不是“未知”而是显式拒绝事实。教学必须提示 `unconsumed_required_handoff` 是 fail-closed Error，不能建议写 blocker、改写 contract id 或依赖自动推断掩盖缺口。

**步骤：**

- [ ] **Step 1 — 复核前置教学与先写 RED。** 编辑前执行：

  ````bash
  ast-grep outline src/product/work_item_split_engine/prompts.rs --match work_item_plan_markdown_reference_discipline --view expanded
  ast-grep outline src/product/work_item_split_engine/tests/prompt_contract.rs --match work_item_plan_markdown_prompt_inlines_grammar_boundaries_and_real_findings --view expanded
  codegraph explore "WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES prompt contract budget assertion" --max-files 8
  ````

  在现有 `work_item_plan_markdown_prompt_inlines_grammar_boundaries_and_real_findings`（当前 `prompt_contract.rs:820`）加入以下目标断言后先运行 RED；若要隔离 Task 名，可新增 sibling test `p2_author_handoff_consumption_prompt_teaches_exact_pair_and_counterexample`，但不得移动/删除既有断言：

  ````rust
  for required in [
      "provided_contract_refs 中每项必须被至少一个下游 Work Item 的 input_contracts",
      "provider_logical_work_item_id",
      "contract_id",
      "逐字二元组",
      "unconsumed_required_handoff",
      "反例：WI-002 handoff 提供 CT-005",
      "正例：WI-002 提供 CT-005",
      "不能依赖 title、depends_on 或自然语言描述推断消费",
  ] {
      assert!(prompt.contains(required), "SC author handoff 教学缺少 {required}");
  }
  assert_eq!(prompt.matches("unconsumed_required_handoff").count(), 1);
  assert!(prompt.len() < 17_000, "SC author quality budget margin must remain");
  ````

  先确认非零并运行：

  ````bash
  cargo test --locked --lib p2_author_handoff_consumption_prompt -- --list
  cargo test --locked --lib p2_author_handoff_consumption_prompt
  cargo test --locked --lib work_item_plan_markdown_prompt_inlines_grammar_boundaries_and_real_findings
  ````

  预期 RED 失败必须是 handoff 句/反例/正例未存在（当前 prompt 已有 capability 教学，但没有 handoff consumption teaching）；不能通过修改测试期望、在 legacy prompt 注入文字、删掉旧 assertion 或把阈值临时放宽制造假绿。若过滤器真实名称不同，使用 `cargo test --locked --lib work_item_split_engine::tests::prompt_contract -- --list` 读取并采用实际名称，不接受 0-test。

- [ ] **Step 2 — 注入单一 handoff 教学段。** 在 `work_item_plan_markdown_reference_discipline` 的 capability 教学之后追加固定段落，明确：每个 `provided_contract_refs` 必须由下游 `input_contracts` 通过 `(provider_logical_work_item_id, contract_id)` 完整、逐字匹配消费；provider item、consumer item、contract_id 的正例；WI-002/CT-005 无消费者的反例；无消费者必须在生成前修正；canonical `unconsumed_required_handoff` fail closed。内容使用 B1 现有短句/反例风格，避免重复 parser grammar、能力覆盖口径或仓库通用教学。`build_work_item_plan_markdown_prompt` 不新增第二个 source/context 参数，仍只通过同一 reference discipline helper 注入。

- [ ] **Step 3 — 更新唯一质量预算并守住余量。** 将 `WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES` 唯一改动设为 `17_000`，注释写明本次为 handoff 消费闭环教学扩容、较 16,200 增加 800；不要修改 65,536 hard max、15,600 legacy draft quality budget、context budget 或 reviewer 64 KiB。既有 prompt test 的 `< WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES` 断言同步使用新常量；新增断言记录实际 UTF-8 `prompt.len()` 与 `17_000 - prompt.len()` margin，预期约 800 但不得把 800 写成硬编码通过条件。

- [ ] **Step 4 — GREEN 与 prompt 隔离。** 运行：

  ````bash
  cargo test --locked --lib p2_author_handoff_consumption_prompt
  cargo test --locked --lib work_item_plan_markdown_prompt
  cargo test --locked --lib work_item_split_engine::tests::prompt_contract
  cargo test --locked --lib design_reviewer_boundary_non_design_prompts_exclude_examples
  ````

  预期 SC full-author 包含 handoff 教学且低于 17,000，正反例字段出现且 `unconsumed_required_handoff` 文案只在该教学中出现一次；legacy draft prompt 不出现 SC full-author 教学，story/design reviewer 不出现该文本；现有能力覆盖教学、grammar、few-shot 和解析 fixture 全绿。任何测试过滤器命中 0 时先 `-- --list`，不得继续。

- [ ] **Step 5 — 修改边界与交付提醒。** 以 `git diff --check` 检查文档化代码实现将使用的范围；Task 8.2 只会涉及上述 prompt 与测试文件，行数必须 ≤1200。因触发 `work-item-draft-prompt-validation.md`，实施交付时必须向操作者明确提示 Case A、Case B 各 10 个有效首次 Claude Code 输出的授权问题；未授权不得调用 Provider，本 Plan 不包含真实 Provider 运行。

## Task 8.3：P2-reviewer——覆盖投影扩展为依赖图、handoff 消费闭环与跨 item 写范围事实

**可追溯性：** `tasks.md` §8.3；REQ-WSC-06 最新 SHALL（reviewer 在复评前获得 dependency/handoff/scope 只读投影，缺口 must-fix）及 scenario「投影与 validator 同源且仅单候选注入」；design.md `reviewer 能力覆盖投影增补`、`P2+P4 裁决`。前置 B2 `0f47060d` 已提供 capability projection；本 Task 只扩展同一 section 和同源计算，不重述/重实现 canonical 规则。

**Files：**

- **Modify:** `src/product/work_item_contract/dependency.rs:1-445`：在现有 `ContractCapabilityCoverage`/`project_contract_capability_coverage`（当前约 31-239）基础上新增 reviewer contract coverage envelope、handoff consumer 共享 helper、dependency graph facts helper、cross-item write-scope facts helper；把 `report_duplicate_edges`、`report_dependency_cycles`、`report_unconsumed_handoffs` 的纯判定抽为共享计算，保留现有 finding 生成、码值、severity、消息、排序和 fail-closed 行为。
- **Modify:** `src/product/workspace_engine/prompts/review_context.rs:1-470`：提供 `append_single_candidate_contract_gap_teaching`（及必要的只读渲染辅助），把 projection 中 `consumed=false` 的每个 handoff 渲染为明确 provider/contract/empty consumers 证据和 must-fix 处置；保留 `append_review_context_section` 与 `single_candidate_dependency_graph` 的既有格式和 parity。
- **Modify:** `src/product/workspace_engine/prompts/review.rs:1-18,505-630`：只在 `build_single_candidate_plan_review_input` 的既有 dependency graph 构建成功后调用新的 envelope builder，并继续注入现有标题 `Reviewer Capability Coverage Projection`；以一行调用接入 handoff 教学 helper，避免该文件超过 1200 行。非 SC builder 不改，scope 后第二次 64 KiB 检查不改。
- **Modify/Test:** `src/product/work_item_contract/tests/dependency.rs:14-700`：增加 envelope、handoff、dependency facts、scope facts 的逐字段测试，以及 projection handoff 与 `validate_dependency_contract_graph` findings 同源一致性测试；不改既有 capability、handoff、cycle、duplicate、unknown provider validator 测试。
- **Modify/Test:** `src/product/workspace_engine/tests/single_candidate_reviewer_coverage.rs:1-221`：沿已有 durable IR/report/scope fixture 增加完整 projection section、handoff 教学、复评共用、预算与排序断言；不向已接近 large-file guard 的 `single_candidate_prompt.rs` 追加测试。
- **Read-only boundary:** `src/product/work_item_split_validator/outline.rs:179-408`、`src/product/work_item_split_validator/plan.rs:16-205`、`src/cross_cutting/worktree.rs:309-331`：核对 dependency/cycle/scope 的既有事实来源；如需抽纯 helper，仅保持既有 validator 输入/输出和错误文本 parity，不改变 validator 对外行为。

**Interfaces：**

- **Consumes：** 单候选 IR item 的 `CanonicalWorkItemContract`；由 `build_dependency_contract_graph(&[CanonicalWorkItemContract]) -> Result<DependencyContractGraph, ContractValidationReport>` 产生的 graph；既有 `ContractCapabilityCoverage`；`CanonicalWorkItemContract.input_contracts`、`handoff_contract.provided_contract_refs`、`depends_on`、`write_policy.exclusive_scopes`/`forbidden_scopes`；`scopes_may_overlap` 的同一 wildcard/directory/case-sensitive 判断。
- **Produces：** `project_contract_reviewer_coverage(&DependencyContractGraph) -> ContractReviewerCoverageProjection`；reviewer section `### Reviewer Capability Coverage Projection` 中一次性序列化 capability、dependency、handoff、write-scope 四组只读事实；`append_single_candidate_contract_gap_teaching` 追加 handoff 缺口教学；`validate_dependency_contract_graph` 继续产生原有 `ContractValidationReport`。

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ContractHandoffConsumption {
    pub provider: String,
    pub contract_ref: String,
    pub consumers: Vec<String>,
    pub consumed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ContractDependencyItemFact {
    pub work_item_id: String,
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ContractDependencyDuplicateFact {
    pub from: String,
    pub to: String,
    pub contract_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ContractUnknownProviderFact {
    pub provider: String,
    pub consumer: String,
    pub contract_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ContractDependencyEdgeFact {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ContractDependencyGraphFacts {
    pub depends_on: Vec<ContractDependencyItemFact>,
    pub declared_edges: Vec<ContractDependencyEdgeFact>,
    pub contract_edges: Vec<DependencyContractEdge>,
    pub cycles: Vec<Vec<String>>,
    pub duplicate_edges: Vec<ContractDependencyDuplicateFact>,
    pub unknown_providers: Vec<ContractUnknownProviderFact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ContractWriteScopeKind {
    Exclusive,
    Forbidden,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ContractWriteScopeConflict {
    pub left_work_item_id: String,
    pub left_kind: ContractWriteScopeKind,
    pub left_scope: String,
    pub right_work_item_id: String,
    pub right_kind: ContractWriteScopeKind,
    pub right_scope: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ContractReviewerCoverageProjection {
    pub capability_coverage: Vec<ContractCapabilityCoverage>,
    pub dependency_graph: ContractDependencyGraphFacts,
    pub handoff_consumption: Vec<ContractHandoffConsumption>,
    pub write_scope_conflicts: Vec<ContractWriteScopeConflict>,
}

pub(crate) fn project_contract_handoff_consumption(
    graph: &DependencyContractGraph,
) -> Vec<ContractHandoffConsumption>;

pub(crate) fn project_contract_reviewer_coverage(
    graph: &DependencyContractGraph,
) -> ContractReviewerCoverageProjection;

pub(super) fn single_candidate_reviewer_coverage(
    items: &[PlanCandidateItemIr],
) -> Result<ContractReviewerCoverageProjection, String>;
```

`project_contract_handoff_consumption` 必须按 graph 中每个 provider 的 `handoff_contract.provided_contract_refs` 去重后逐 ref 产出一项；consumer 集合仅收集所有 item 的 `input_contracts` 中同时满足 `provider_logical_work_item_id == provider` 与 `contract_id == contract_ref` 的逻辑 ID，排序、去重，空集合必须是 `consumers: []` 且 `consumed: false`。它是 `report_unconsumed_handoffs` 的唯一数据源：validator 对每个 `consumed=false` entry 继续生成一条既有 `unconsumed_required_handoff` Error，`logical_work_item_id`、`contract_ref`、消息逐字保持当前语义；不得在 reviewer 侧另算 consumers。

`ContractDependencyGraphFacts.depends_on` 逐 graph.contracts item 按 logical ID 排序并保留 canonical `depends_on`（去重、排序）；`declared_edges` 从每个 item 的 `depends_on` 投影为 `dependency -> item` 并排序；`contract_edges` 按既有 `DependencyContractEdge` 确定性顺序逐项投影，二者不能混成一个含糊 `edges` 字段。`cycles` 由抽出的既有 `report_dependency_cycles` 对 contract edges 的计算结果产生，循环节点排序/去重方式与当前 finding 相同；declared edge 的 cycle/未知依赖继续在既有 SC `Dependency Contract Graph`/mechanical report 中展示。`duplicate_edges` 由抽出的既有 `report_duplicate_edges` 计算结果产生，既报告重复 `(from,to)` contract edge，也报告同一 edge 内重复 `contract_id`（后者 `contract_id: Some`）；`unknown_providers` 按既有 `report_contract_requirements` 的 unknown provider 分支逐 required contract 产生，排序与现有 report 一致。不要把 `required_contract_missing` 当作 unknown provider，也不要以 reviewer projection 取代 validator finding。

`ContractWriteScopeConflict` 逐逻辑 item pair、逐 scope pair 产出，使用 `scopes_may_overlap(&[left_scope], &[right_scope], true)` 的既有 predicate；覆盖 `exclusive↔exclusive`、`exclusive↔forbidden` 与对称的 `forbidden↔exclusive` 三类，**不把 forbidden↔forbidden 当冲突**，按 `(left_work_item_id, left_kind, left_scope, right_work_item_id, right_kind, right_scope)` 稳定排序。只读投影不新增 finding、不改变现有“依赖顺序/并行顺序”判定；已有 outline/plan validator 的 `write_scope_conflict`、`parallel_scope_overlap` 和同 item `overlapping_exclusive_and_forbidden_scope` 对外结果保持原样。跨 item exclusive/forbidden 是 reviewer projection fact；复用既有 overlap predicate，但不得借 Task 8.3 增加 canonical Error。

`ContractReviewerCoverageProjection` 复用 B2 的 `capability_coverage`，不复制 capability 比较；现有 section 标题保持 `Reviewer Capability Coverage Projection` 以兼容 B2 测试，JSON 内增加 `dependency_graph`、`handoff_consumption`、`write_scope_conflicts` 三字段。`review_context.rs` 的 `single_candidate_reviewer_coverage` 负责 items → canonical contracts → `build_dependency_contract_graph` → envelope 的完整构建和错误映射；`review.rs` 只保留 `let reviewer_coverage = single_candidate_reviewer_coverage(&ir_record.ir.items)?;` 与既有 section append。该 envelope 只是 reviewer evidence，不写 IR/report/session/CAS，也不扩展 `ReviewerWorkItemProjection` 持久化模型。

**步骤：**

- [ ] **Step 1 — 复核行数与同源函数，写投影 RED。** 编辑前执行：

  ````bash
  ast-grep outline src/product/work_item_contract/dependency.rs
  ast-grep outline src/product/workspace_engine/prompts/review_context.rs
  ast-grep outline src/product/workspace_engine/prompts/review.rs --match build_single_candidate_plan_review_input --view expanded
  ast-grep outline src/product/work_item_contract/tests/dependency.rs
  ast-grep outline src/product/workspace_engine/tests/single_candidate_reviewer_coverage.rs
  wc -l src/product/workspace_engine/prompts/review.rs src/product/workspace_engine/prompts/review_context.rs src/product/work_item_contract/dependency.rs src/product/work_item_contract/tests/dependency.rs src/product/workspace_engine/tests/single_candidate_reviewer_coverage.rs
  ````

  在 `src/product/work_item_contract/tests/dependency.rs` 先加入 RED 测试，使用该文件现有 `provider_contract_fixture`、`consumer_contract_fixture` 与 `build_dependency_contract_graph`，不要构造未持久化 reviewer JSON：

  ````rust
  #[test]
  fn p2_reviewer_handoff_projection_reports_empty_consumer_set_field_exactly() {
      let mut provider = provider_contract_fixture(&["capability.present"]);
      provider.handoff_contract.provided_contract_refs = vec!["CT-005".to_string()];
      let consumer = consumer_contract_fixture(
          &[], ContractCompatibilityPolicy::RequireAny,
      );
      let graph = build_dependency_contract_graph(&[provider, consumer]).expect("graph");
      let projection = project_contract_reviewer_coverage(&graph);
      assert_eq!(projection.handoff_consumption, vec![ContractHandoffConsumption {
          provider: "WI-01".to_string(),
          contract_ref: "CT-005".to_string(),
          consumers: Vec::new(),
          consumed: false,
      }]);
      let report = validate_dependency_contract_graph(&graph);
      assert_eq!(report.findings.iter().filter(|finding|
          finding.code == "unconsumed_required_handoff"
              && finding.logical_work_item_id.as_deref() == Some("WI-01")
              && finding.contract_ref.as_deref() == Some("CT-005")
      ).count(), 1);
  }

  #[test]
  fn p2_reviewer_handoff_projection_consumers_match_validator_for_multiple_consumers() {
      let mut provider = provider_contract_fixture(&["capability.present"]);
      provider.output_contracts[0].contract_id = "CT-005".to_string();
      provider.handoff_contract.provided_contract_refs = vec!["CT-005".to_string()];
      let mut first = consumer_contract_fixture(
          &["capability.present"], ContractCompatibilityPolicy::RequireAll,
      );
      first.input_contracts[0].contract_id = "CT-005".to_string();
      let mut second = first.clone();
      second.identity.logical_work_item_id = "WI-03".to_string();
      let graph = build_dependency_contract_graph(&[provider, first, second]).expect("graph");
      let projection = project_contract_reviewer_coverage(&graph);
      assert_eq!(projection.handoff_consumption, vec![ContractHandoffConsumption {
          provider: "WI-01".to_string(),
          contract_ref: "CT-005".to_string(),
          consumers: vec!["WI-02".to_string(), "WI-03".to_string()],
          consumed: true,
      }]);
      assert!(validate_dependency_contract_graph(&graph).findings.iter().all(|finding|
          finding.code != "unconsumed_required_handoff"
              || finding.contract_ref.as_deref() != Some("CT-005")
      ));
  }

  #[test]
  fn p2_reviewer_dependency_and_scope_projection_facts_are_field_exact_and_stable() {
      let mut first = provider_contract_fixture(&["capability.present"]);
      first.depends_on = vec!["WI-02".to_string()];
      first.write_policy.exclusive_scopes = vec!["src/shared/**".to_string()];
      first.write_policy.forbidden_scopes.clear();
      let mut second = consumer_contract_fixture(
          &["capability.present"], ContractCompatibilityPolicy::RequireAll,
      );
      second.depends_on = vec!["WI-01".to_string()];
      second.write_policy.exclusive_scopes.clear();
      second.write_policy.forbidden_scopes = vec!["src/shared/**".to_string()];
      let mut contracts = BTreeMap::new();
      contracts.insert("WI-01".to_string(), first);
      contracts.insert("WI-02".to_string(), second);
      let required = RequiredDependencyContract {
          contract_id: "contract.workflow".to_string(),
          required_capabilities: vec!["capability.present".to_string()],
          compatibility_policy: ContractCompatibilityPolicy::RequireAll,
      };
      let graph = DependencyContractGraph {
          contracts,
          edges: vec![
              DependencyContractEdge {
                  from: "WI-01".to_string(),
                  to: "WI-02".to_string(),
                  required_contracts: vec![required.clone(), required.clone()],
              },
              DependencyContractEdge {
                  from: "WI-01".to_string(),
                  to: "WI-02".to_string(),
                  required_contracts: vec![required.clone()],
              },
              DependencyContractEdge {
                  from: "WI-02".to_string(),
                  to: "WI-01".to_string(),
                  required_contracts: vec![required.clone()],
              },
              DependencyContractEdge {
                  from: "WI-404".to_string(),
                  to: "WI-02".to_string(),
                  required_contracts: vec![required],
              },
          ],
      };
      let first_projection = project_contract_reviewer_coverage(&graph);
      let second_projection = project_contract_reviewer_coverage(&graph);
      assert_eq!(first_projection, second_projection);
      assert_eq!(first_projection.dependency_graph.cycles,
          vec![vec!["WI-01".to_string(), "WI-02".to_string()]]);
      assert_eq!(first_projection.dependency_graph.unknown_providers[0].provider, "WI-404");
      assert!(first_projection.dependency_graph.duplicate_edges.iter().any(|fact|
          fact.from == "WI-01" && fact.to == "WI-02"));
      assert!(first_projection.write_scope_conflicts.iter().any(|fact|
          fact.left_work_item_id == "WI-01"
              && fact.left_scope == "src/shared/**"
              && fact.right_work_item_id == "WI-02"
              && fact.right_scope == "src/shared/**"));
      let report = validate_dependency_contract_graph(&graph);
      for code in [
          "duplicate_dependency_contract_edge",
          "dependency_cycle",
          "unknown_provider_logical_work_item",
      ] {
          assert!(report.findings.iter().any(|finding| finding.code == code));
      }
  }
  ````

  当前测试文件已有 helper 的真实签名就是 `consumer_contract_fixture(required_capabilities: &[&str], compatibility_policy: ContractCompatibilityPolicy)`，上面示例可直接落地；新增类型逐项加入该文件现有 `use crate::product::work_item_contract` 导入列表。每一测试先运行 `-- --list` 确认非零，再运行：

  ````bash
  cargo test --locked --lib p2_reviewer_handoff_projection -- --list
  cargo test --locked --lib p2_reviewer_handoff_projection
  ````

  预期 RED 是新 envelope/helper/type 不存在或字段断言未满足；不得因改 finding code、跳过空消费者、把消费者从 `depends_on` 推导或删除既有 validator 测试而假绿。

- [ ] **Step 2 — 抽取 handoff consumer 单一计算源。** 将当前 `report_unconsumed_handoffs` 的 `consumed_contracts` 集合计算替换为 `project_contract_handoff_consumption` 结果过滤；保留 provider `BTreeMap` 遍历、provided ref 去重、`unconsumed_required_handoff` 的 finding 参数与消息。共享 helper 不访问 prompt、session 或 provider 输出；对 unknown provider 的 input 仍可作为 consumer 事实存在，但不会改变 provider 不存在时既有 requirement finding。

- [ ] **Step 3 — 抽取 dependency/cycle/duplicate facts并保持 validator parity。** 将 `report_duplicate_edges`、`report_dependency_cycles` 和 unknown-provider 分支的纯事实计算提炼为私有/`pub(crate)` helper；另从 `graph.contracts` 的 canonical `depends_on` 生成 `depends_on`/`declared_edges`。`validate_dependency_contract_graph` 仍按原顺序用共享 facts 生成完全相同的 findings，`project_contract_reviewer_coverage` 读取同一 facts。对 cycle 节点、重复 edge、重复 required contract、unknown provider 的测试逐字段断言；不得让 reviewer projection 改变 `sorted_report` 的去重或排序。`single_candidate_dependency_graph` 对 empty/duplicate identity 仍 fail closed；对未知 dependency/self/cycle 继续按其既有机械报告与现有 section 呈现，不得在 reviewer projection 构建前静默丢弃这些事实。

- [ ] **Step 4 — 生成 cross-item write scope facts。** 在 dependency.rs 以 graph.contracts 的 BTreeMap 顺序对 item pair 和四类 scope kind 逐一比较，调用既有 `scopes_may_overlap`，将命中的两个具体 scope 与 kind 写入 `ContractWriteScopeConflict` 并按规定 tuple 排序。不要把 graph projection 错误映射为 validator Error；不要将新的跨 item facts写回 canonical contract、outline、plan 或 projection bundle。

- [ ] **Step 5 — 接入 SC reviewer envelope 与 handoff 教学。** 将 B2 当前在 `review.rs:525-535` 的 canonical contract clone、graph build 与 capability projection 构建移入 `review_context.rs::single_candidate_reviewer_coverage`；`build_single_candidate_plan_review_input` 仅用一行调用取得 envelope，并把它作为既有 `Reviewer Capability Coverage Projection` section 的 value。保留 `Dependency Contract Graph`、`Cross-Item Contract Supply / Demand`、mechanical report、artifact refs 的原 section 和 JSON。调用 `append_single_candidate_contract_gap_teaching(&mut prompt, &reviewer_coverage)`：对每个 `consumed=false` entry 明确输出 `severity=must_fix`、`category=contract_gap`、`class_hint=repairable`、`evidence` 必须含 provider、contract_ref 与 `consumers: []`；不得让 reviewer 以 pass/advisory 掩盖。该调用只发生于 SC builder，初评和 Verification 复评自然共享；legacy/story/design builder 不接收 envelope 或教学。

- [ ] **Step 6 — GREEN、初评/复评共用与 64 KiB 双点回归。** 先运行：

  ````bash
  cargo test --locked --lib p2_reviewer_handoff_projection
  cargo test --locked --lib reviewer_capability_coverage
  cargo test --locked --lib single_candidate_reviewer_coverage
  cargo test --locked --lib single_candidate_prompt
  cargo test --locked --lib design_reviewer_boundary_non_design_prompts_exclude_examples
  ````

  预期 handoff projection 的空集/多消费者/consumed bool、dependency 五事实、scope 六字段、validator finding 同源一致全绿；single-candidate prompt 同时包含四组 projection 字段、`consumers: []`、`must_fix`/`contract_gap`/`evidence` 和具体 provider/ref；Initial 与 Verification 都含同一标题，且最终 scope 追加后仍经同一 64 KiB helper 检查。legacy/story/design 标题隔离和 `single_candidate_prompt` 既有 scope/digest/CAS parity 全绿。任何过滤器匹配 0，先用对应模块的 `-- --list` 读取真实测试名后重跑。

- [ ] **Step 7 — large-file guard 与无范围扩张检查。** 执行：

  ````bash
  wc -l src/product/workspace_engine/prompts/review.rs src/product/workspace_engine/prompts/review_context.rs src/product/work_item_contract/dependency.rs src/product/work_item_contract/tests/dependency.rs src/product/workspace_engine/tests/single_candidate_reviewer_coverage.rs
  awk '$1 > 1200 {print}' < <(wc -l src/product/workspace_engine/prompts/review.rs src/product/workspace_engine/prompts/review_context.rs src/product/work_item_contract/dependency.rs src/product/work_item_contract/tests/dependency.rs src/product/workspace_engine/tests/single_candidate_reviewer_coverage.rs)
  git diff --check
  ````

  预期所有目标文件均 ≤1200 行，large-file 命令无输出；若 `review.rs` 接近/超过 1200，必须将 envelope 构建与 handoff 教学继续放入 `dependency.rs`/`review_context.rs`，review.rs 只留导入、调用和既有 section append，不得顶破 guard。

## Task 8.4：测试——P4 RED、投影逐字段同源、author 教学与路径隔离回归

**可追溯性：** `tasks.md` §8.4；REQ-WSC-06 的 author/reviewer handoff SHALL、reviewer dependency/handoff/scope projection SHALL、legacy/story/design isolation scenario；design.md `P2+P4 裁决`。本 Task 只增加/更新测试，不以测试改动替代生产实现；所有新增测试都要先 RED 后 GREEN。

**Files：**

- **Modify/Test:** `src/product/work_item_plan_compiler/tests/full_lowering_validator.rs:1-193`：增加 P4 SC 集成断言：超长 Verification command 不产生 `trusted_verification_command_catalog_field_too_large`，canonical blank identity 仍产生 `blank_logical_work_item_id`，SC 专用 outline profile 对 `estimated_context_tokens = 50_001` 仍产生 `outline_exceeds_single_session_budget`；旧 rep4/unknown provider/byte-stability 测试原样保留。
- **Modify/Test:** `src/product/work_item_split_validator/tests.rs:129-288,416-445`：不改已有 legacy catalog、budget、scope 测试；必要时增加带 `p4_legacy` 前缀的新调用测试，使用已有 `valid_outline`/`canonical_draft_candidate`，逐项证明空 catalog、untrusted required command 和 catalog projection/field/count 仍 fail closed。
- **Modify/Test:** `src/product/work_item_contract/tests/dependency.rs:56-173,457-653`：增加 capability projection 扩展后的 handoff consumption、dependency facts、duplicate/cycle/unknown-provider、write-scope facts 测试；每个 handoff 字段和 validator finding 做逐字段/同源断言。
- **Modify/Test:** `src/product/workspace_engine/tests/single_candidate_reviewer_coverage.rs:24-221`：增加 SC reviewer prompt 的四组 projection 字段、空 consumers、must_fix/contract_gap/evidence 教学、Initial/Verification 共用、64 KiB exact/over-limit 和最终 scope 后长度断言。
- **Modify/Test:** `src/product/workspace_engine/tests/design_reviewer_boundary.rs:148-188`：扩展既有非 design prompt boundary 测试，断言 `WorkspaceType::Story`、`WorkspaceType::Design`、普通 WorkItem reviewer 和 `WorkItemPlanFlowKind::Legacy` 均不含 `Reviewer Capability Coverage Projection`、`handoff_consumption` 与新的 `contract_gap` 教学；不要修改生产 prompt builder。
- **Modify/Test:** `src/product/work_item_split_engine/tests/prompt_contract.rs:787-995`：更新唯一 SC quality-budget 断言，新增 handoff 教学正反例字段和单次出现断言；不新增第二个预算常量。
- **Modify:** `src/product/workspace_engine/tests.rs:40-47`：若此前 B2 sibling module 已存在则仅确认注册，不重复注册；新增测试文件必须通过现有扁平 `mod` 机制加载。

**Interfaces：**

- **Consumes：** `PlanCandidateIr`/`PlanCandidateItemIr` 与 `rep4_validation_context`；`WorkItemPlanOutlineValidator::validate` legacy fixture；`DependencyContractGraph`、`project_contract_reviewer_coverage`、`validate_dependency_contract_graph`；现有 durable `PlanCandidateIrRecord`、`PlanCandidateMechanicalReportRecord`、`ReviewInvocationScope` 和 `WorkspaceEngine::build_review_input()`；author prompt builder 与 `WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES`。
- **Produces：** 非零、可复现的 test assertions：SC catalog finding absence + non-catalog finding presence；legacy catalog finding presence；projection `consumers`/`consumed`/dependency/scope 字段准确且 validator 同源；SC prompt 教学/section/预算正确；story/design/legacy isolation；author quality budget = 17,000。

**步骤：**

- [ ] **Step 1 — 先建立完整 RED 矩阵。** 编辑测试前重新执行当前实际行号和测试大纲：

  ````bash
  ast-grep outline src/product/work_item_plan_compiler/tests/full_lowering_validator.rs
  ast-grep outline src/product/work_item_split_validator/tests.rs
  ast-grep outline src/product/work_item_contract/tests/dependency.rs
  ast-grep outline src/product/workspace_engine/tests/single_candidate_reviewer_coverage.rs
  ast-grep outline src/product/workspace_engine/tests/design_reviewer_boundary.rs
  ast-grep outline src/product/work_item_split_engine/tests/prompt_contract.rs
  cargo test --locked --lib p4_sc -- --list
  cargo test --locked --lib p2_reviewer_handoff_projection -- --list
  cargo test --locked --lib p2_author_handoff_consumption_prompt -- --list
  ````

  `-- --list` 必须列出至少一个真实测试名；若过滤名未命中，使用对应模块的精确测试前缀重新查询，不把空列表当通过。随后按顺序运行三个 RED 过滤器：

  ````bash
  cargo test --locked --lib p4_sc
  cargo test --locked --lib p2_reviewer_handoff_projection
  cargo test --locked --lib p2_author_handoff_consumption_prompt
  ````

  预期 RED 分别是 SC 旧 catalog finding 仍出现、新 handoff/dependency/scope projection 类型或字段不存在、SC author handoff 教学断言不存在。RED 失败必须来自目标断言；若先遇到 fixture 编译错误，修复测试 fixture 后重新运行同一命令，不能把编译失败记录为行为 RED。

- [ ] **Step 2 — P4 三项断言必须覆盖正负面。** 将 `full_lowering_validator.rs` 的 P4 测试写成可执行代码，不使用伪调用或省略参数：以 `REP4_FIXTURE` + `WorkItemPlanSourceContext { target_repository_id: "repo-levels".to_string() }` lower；以 `ir.items[0].verification_plan.checks` 的既有 command 和对应 `trusted_commands` 同步替换为 `"cargo test "` 加上超过当前 catalog 字段阈值的 ASCII 字符；调用 `validate_plan_candidate_ir(&ir, &rep4_validation_context(Some(&rep4_repository_profile()), &vec!["story_spec_levels_0001".to_string()], &vec!["design_spec_levels_0001".to_string()]))`。断言 diagnostics 中没有 `trusted_verification_command_catalog_field_too_large`，并单独断言 canonical blank identity 的 `blank_logical_work_item_id` 存在。预算超限使用 `src/product/work_item_split_validator/tests.rs` 已有 valid outline 的真实 `estimated_context_tokens = Some(50_001)` 与 `WorkItemOutlineSessionFit::TooLargeMustSplit`，调用 SC profile 入口断言 `outline_exceeds_single_session_budget` 和 `outline_too_large_must_split`；这是因为当前 `project_outline` 固定投影 30,000，不能伪造 compiler 输入注入预算值。

  用以下命令 GREEN：

  ````bash
  cargo test --locked --lib p4_sc_long_verification_command
  cargo test --locked --lib p4_sc_outline_rules
  cargo test --locked --lib p4_legacy_catalog
  cargo test --locked --lib full_lowering_validator
  cargo test --locked --lib work_item_split_validator
  ````

  预期长 command 的具体 catalog field finding 消失但仍受其他合法安全/contract 校验约束；blank ID 与 budget 仍 fail closed；legacy 空 catalog/required untrusted/field/count/projection tests 全绿。若实现选择不暴露 profile helper，则预算 case 通过 `WorkItemPlanOutlineValidator::validate` 的 legacy fixture 只能作为 legacy parity，必须另说明 SC profile 复用了完全相同的预算逻辑而不把 SC catalog 规则带回；不允许删除预算断言。

- [ ] **Step 3 — 投影逐字段与 validator findings 同源测试。** 在 dependency tests 中构造至少三种图：
  1. WI-001 提供 CT-005，WI-002 无对应 input，断言 `{ provider: "WI-001", contract_ref: "CT-005", consumers: [], consumed: false }`，并断言 report 有且仅有同 provider/ref 的 `unconsumed_required_handoff`；
  2. WI-001 提供 CT-005，WI-002 与 WI-003 的 `RequiredInputContract` 都以 `provider_logical_work_item_id: "WI-001"`、`contract_id: "CT-005"` 逐字引用，断言 consumers 按 `["WI-002", "WI-003"]` 排序且 consumed=true，report 没有 CT-005 unconsumed finding；
  3. 手工构造带重复 `(from,to)` edge、重复 required contract、cycle、unknown provider 与 exclusive/forbidden overlap 的 graph，逐字段断言 `dependency_graph.depends_on`、`edges`、`cycles`、`duplicate_edges`、`unknown_providers` 和 `write_scope_conflicts`，并确认 validator 仍输出 `duplicate_dependency_contract_edge`、`dependency_cycle`、`unknown_provider_logical_work_item` 等既有码值。

  每个 capability/handoff case 同时调用 `project_contract_capability_coverage` 或 envelope 的 capability 部分与 `validate_dependency_contract_graph`，使用 `BTreeSet` 比较 `required_capability_missing`/`unconsumed_required_handoff` 的 key 集，再以 `Vec` 断言确定性排序；不能只比较 JSON 字符串或只断言总数。运行：

  ````bash
  cargo test --locked --lib p2_reviewer_handoff_projection
  cargo test --locked --lib reviewer_capability_coverage
  cargo test --locked --lib work_item_contract::tests::dependency::canonical_work_item_dependency_validation
  ````

  预期空消费者显式存在、多个消费者排序稳定、validator finding 与 projection 同源；未知 provider、重复边/契约、cycle 和 scope conflict facts 不吞错、不改变既有 finding。过滤器命中 0 时按模块 `-- --list` 的真实名称替换，不接受 0-test。

- [ ] **Step 4 — SC prompt 教学、初评/复评、预算和隔离。** 复用 `single_candidate_reviewer_coverage.rs` 当前 durable fixture 持久化顺序：source revision → IR record → mechanical report → session refs；调用真实 `engine.build_review_input()`，逐字段断言 prompt 含 `Reviewer Capability Coverage Projection`、`capability_coverage`、`dependency_graph`、`handoff_consumption`、`write_scope_conflicts`、`consumers`、`consumed`、`WI-001`、`CT-005`。空 consumers case 还要断言 prompt 含 `consumers: []`、`consumed: false`、`severity=must_fix`、`category=contract_gap`、`class_hint=repairable`、`evidence` 和“provider/contract ref/edge”证据句；完整 consumer case 断言 `consumed: true` 与排序消费者均出现。以 `ReviewInvocationScope::verification` 替换同一 durable session scope 后再次调用真实入口，断言相同标题/字段仍存在，原 fingerprints/mechanical report scope 文本不被新投影替换，且 `input.prompt.len() <= SINGLE_CANDIDATE_REVIEW_PROMPT_MAX_BYTES`。

  用已有 `design_reviewer_boundary_non_design_prompts_exclude_examples` fixture 扩展 table/assertions，逐一对 Story、Design、普通 WorkItem 和 Legacy WorkItemPlan reviewer prompt 断言新增 section title、`handoff_consumption`、`write_scope_conflicts` 均不存在；SC case 单独断言它们存在。运行：

  ````bash
  cargo test --locked --lib single_candidate_reviewer_coverage
  cargo test --locked --lib single_candidate_reviewer_prompt_budget
  cargo test --locked --lib design_reviewer_boundary_non_design_prompts_exclude_examples
  cargo test --locked --lib single_candidate_prompt
  ````

  预期初评/复评共用同一 projection，legacy/story/design 仍不接收；exact 64 KiB helper 通过、+1 byte fail closed、scope 后最终 prompt 双点预算测试全绿。64 KiB 双点检查实现与测试不因本次 envelope 扩展而修改。

- [ ] **Step 5 — SC author prompt 与 17,000 质量预算。** 通过现有 `split_prompt_fixture()` 构造 `WorkItemPlanMarkdownAuthorContext`，调用真实 `build_work_item_plan_markdown_prompt`，断言 handoff 教学字段全部出现、`unconsumed_required_handoff` 只出现一次、现有 B1 capability 教学/grammar/few-shot 仍在，且 `prompt.len() < WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES`、常量值等于 17,000。另用 legacy draft builder 的现有测试确认 `WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES` 仍为 15,600；不要修改硬 max 或 legacy builder。

  ````bash
  cargo test --locked --lib p2_author_handoff_consumption_prompt
  cargo test --locked --lib work_item_plan_markdown_prompt_inlines_grammar_boundaries_and_real_findings
  cargo test --locked --lib work_item_split_engine::tests::prompt_contract
  ````

  预期 SC prompt handoff 正反例、二元组逐字消费纪律和 fail-closed 后果存在，prompt 位于 17,000 质量预算内；legacy/story/design prompt 不含新增 SC 教学。因本 Task 触发 Work Item Draft Prompt 测试提醒，测试完成汇报必须附带“Case A、Case B 各 10 个有效首次 Claude Code 输出需操作者授权”的明确提醒，不执行 Provider。

- [ ] **Step 6 — 隔离回归、完整门禁与残余 flaky 定性。** 运行本 Plan Global Constraints 的四条完整门禁，随后执行：

  ````bash
  cargo fmt --check
  cargo clippy --all-targets --all-features --locked -- -D warnings
  cargo check --locked
  cargo test --locked
  git diff --check
  wc -l src/product/workspace_engine/prompts/review.rs src/product/workspace_engine/prompts/review_context.rs src/product/work_item_contract/dependency.rs src/product/work_item_contract/tests/dependency.rs src/product/workspace_engine/tests/single_candidate_reviewer_coverage.rs src/product/work_item_split_engine/prompts.rs src/product/work_item_split_engine/tests/prompt_contract.rs
  awk '$1 > 1200 {print}' < <(wc -l src/product/workspace_engine/prompts/review.rs src/product/workspace_engine/prompts/review_context.rs src/product/work_item_contract/dependency.rs src/product/work_item_contract/tests/dependency.rs src/product/workspace_engine/tests/single_candidate_reviewer_coverage.rs src/product/work_item_split_engine/prompts.rs src/product/work_item_split_engine/tests/prompt_contract.rs)
  ````

  预期全量四条门禁、diff check 通过，`awk` 无输出；`prompts/review.rs` 保持 ≤1200（当前 1168，新增逻辑应放 helper 文件），测试文件也均 ≤1200。若任何命令失败，按 flaky 家族规则以同一完整命令立即复跑：稳定同断言失败记录为回归并停止；仅失败测试/线程/端口/时序症状变化且定向测试通过才定性 flaky，并报告两轮日志与未解决风险，不能静默忽略。

## 汇报

- **Plan 路径：** `cadence/plans/2026-08-30_计划文档_P2P4_handoff闭环与catalog残留清理_v1.0.md`
- **Task 8.1：** P4 先行，以显式 SC outline/draft validation profile 仅退役 trusted catalog 残留，保留所有全局函数、legacy 行为和其他 outline/draft 规则，并用超长 command、blank ID、budget 与 legacy catalog RED/GREEN/parity 证据锁定边界。
- **Task 8.2：** P2-author 在 B1 同风格的 SC reference discipline 中加入 `provided_contract_refs` → 下游 `input_contracts` 的 `(provider_logical_work_item_id, contract_id)` 逐字消费教学、WI-002/CT-005 正反例和生成前修正，质量预算 16,200→17,000。
- **Task 8.3：** P2-reviewer 从 `dependency.rs` 同源抽取 handoff consumers、dependency/cycle/duplicate/unknown provider 与跨 item exclusive/forbidden scope facts，扩展既有 `Reviewer Capability Coverage Projection`，并以 `must_fix`/`contract_gap`/具体 evidence 教学阻断 unconsumed handoff；初评/复评共用且 review.rs 不超过 1200 行。
- **Task 8.4：** 覆盖 P4 三项断言、projection 四组逐字段/排序/同源 finding、空消费者、多消费者、SC author 教学、Initial/Verification prompt、legacy/story/design 隔离、17,000 author budget 和不变的 64 KiB reviewer 双点预算。
- **实施提醒：** Task 8.2 触发 Work Item Draft Prompt 测试规则；交付实施时须向操作者明确提示 Case A、Case B 各 10 个有效首次 Claude Code 输出的授权，未授权不得调用 Provider。本 Plan 不含 r25 provider 重跑。
## 自检记录

- **OpenSpec 对齐：** 本文严格展开 `rearch-workitem-plan-pipeline` `tasks.md` §8.1–§8.4；§8.5 r25、P3 全量 checklist、D3 driver 卫生和 95% 专项测量均明确排除。REQ-WSC-06 最新段落的 author handoff 教学、reviewer 四组投影、缺口 finding 和单候选隔离均有对应 Task/测试。
- **顺序与独立提交：** Task 8.1 为 P4，正文首个 Task，先 RED/GREEN、完整门禁并作为独立提交粒度；Task 8.2 author、Task 8.3 reviewer、Task 8.4 tests 按用户批准顺序；全文没有 `git commit` step。
- **现状复核差异：** 与用户提供的基线相比有三处需要执行者注意：①实际 `src/product/work_item_split_validator/outline.rs:44-124` 的 catalog 调用位于 `validate_outline_traceability_and_scopes` 第 83 行，`validate.rs:34` 通过 `WorkItemPlanOutlineValidator::validate` 间接触发；②实际 `src/product/work_item_split_validator/draft.rs:225-280` 的 draft catalog 残留由 `WorkItemDraftLocalValidator::validate` 第 70-75 行触发；③用户基线提到的 `src/product/work_item_split_validator/parse.rs` 不存在，真实 legacy 联动点是 `src/product/work_item_split_engine/parse.rs`，已在 Task 8.1 的现状与 Files 写明。其余现状与本文基线一致：`validate_plan_candidate_ir` 第 34/49 行调用链、旧错误串、B2 已有 projection、review.rs 当前 1168 行、single_candidate_prompt.rs 当前 1196 行均已复核。
- **profile 边界：** 旧 catalog 函数、常量、legacy public validator 和测试不删除；SC profile 仅避开 outline catalog 三规则与 draft 两规则，非 catalog outline/draft/canonical/plan validator 继续执行。字段安全门默认随退役 catalog 跳过，理由和“不得自行迁移/扩大”为 Task 8.1 Global Constraint；若发现独立安全依赖则停止并重新裁决。
- **同源证据：** capability 继续复用 B2 `project_contract_capability_coverage`；handoff consumer 从 `report_unconsumed_handoffs` 抽共享 helper；dependency duplicate/cycle/unknown provider 与 scope overlap 只读投影均由现有确定性 facts 生成；测试要求逐字段、排序、空集及既有 findings 同源一致，避免 JSON 总数假绿。
- **budget 核对：** author quality budget 唯一由 16,200 上调到 17,000，legacy 15,600、SC author hard max 65,536、reviewer 64 KiB 双点检查不动；文中未引入第二预算常量。review.rs 新逻辑若触及 1200 行警戒必须下沉到 dependency.rs/review_context.rs。
- **隔离核对：** `.aria/`、legacy/story/design 生产路径、coding/runtime binding、scope/digest/CAS、前端/WS/provider schema 均列为禁区；隔离测试只读真实 reviewer prompt 并断言新标题/字段不泄漏。
- **文档格式核对：** 目标文件为 `cadence/plans/2026-08-30_计划文档_P2P4_handoff闭环与catalog残留清理_v1.0.md`，代码块使用外层四反引号容纳内层三反引号；每个 Task 均包含 Files、Interfaces、checkbox 步骤、实际路径与命令/预期；已扫描没有省略号、伪调用或未完成占位符。`#` 出现在 Rust 代码块中的 derive/attribute 属代码，不是额外 Markdown section。
- **实际自检命令：** 已执行 `codegraph init`（报告 worktree 已初始化）、多次 `codegraph explore`（验证调用链/影响面）、目标源码 `ast-grep outline`、`wc -l` 行数核对、目标文档结构/fence/占位符脚本扫描、`git diff --check -- cadence/plans/2026-08-30_计划文档_P2P4_handoff闭环与catalog残留清理_v1.0.md`（无输出）。未执行 cargo，因为本任务只写 Plan、明确禁止改代码；实施者必须按本文步骤执行完整门禁。
- **工作树核对：** 本次只创建目标 Plan 文件；工作树中 `openspec/changes/rearch-workitem-plan-pipeline/design.md`、`specs/work-item-plan-single-candidate/spec.md`、`tasks.md` 的修改是进入本任务前已存在的上游批准契约变更，本次未编辑；无 staged files。

```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "目标 Plan 仅展开用户批准的 tasks.md §8.1–§8.4，明确 P4 先行独立提交粒度、P2 author/reviewer 教学与投影扩展、测试/门禁/隔离边界；未改任何代码或其他文件。"
    }
  ],
  "changedFiles": [
    "cadence/plans/2026-08-30_计划文档_P2P4_handoff闭环与catalog残留清理_v1.0.md"
  ],
  "testsAddedOrUpdated": [],
  "commandsRun": [
    {
      "command": "codegraph init",
      "result": "passed",
      "summary": "当前 worktree 已初始化 CodeGraph。"
    },
    {
      "command": "codegraph explore <目标符号>",
      "result": "passed",
      "summary": "已核对 validate_plan_candidate_ir、legacy catalog、handoff、reviewer projection 的调用链与影响面。"
    },
    {
      "command": "ast-grep outline <目标源码/测试文件>",
      "result": "passed",
      "summary": "已核对真实函数、调用入口和文件结构；review.rs 当前 1168 行，single_candidate_prompt.rs 当前 1196 行。"
    },
    {
      "command": "wc -c -l cadence/plans/2026-08-30_计划文档_P2P4_handoff闭环与catalog残留清理_v1.0.md",
      "result": "passed",
      "summary": "目标 Plan 已增量落盘，当前约 77 KiB、约 723 行。"
    },
    {
      "command": "git diff --check -- cadence/plans/2026-08-30_计划文档_P2P4_handoff闭环与catalog残留清理_v1.0.md",
      "result": "passed",
      "summary": "无空白错误；未运行 cargo，因为本任务只写 Plan。"
    }
  ],
  "validationOutput": [
    "已先写文件头与 Global Constraints 落盘，再按 Task 8.1、8.2、8.3、8.4 增量追加，最后追加汇报、自检与本 acceptance report。",
    "已复核基线差异：实际不存在 src/product/work_item_split_validator/parse.rs；真实 legacy 联动点为 src/product/work_item_split_engine/parse.rs；catalog 调用在 outline.rs:83，draft 残留调用在 types.rs:70-75。",
    "已扫描目标文档没有省略号、未完成占位符或伪调用标记；所有 cargo 示例禁止 -j，定向测试包含 --lib，完整门禁四条命令齐全。"
  ],
  "residualRisks": [
    "实施者必须在编辑前按各 Task Step 1 重跑 ast-grep outline，行号漂移只更新定位，不扩大范围。",
    "本任务未运行 cargo；P4/P2 RED/GREEN、全量门禁及 flaky 家族定性属于实施阶段。",
    "Task 8.2 触发 Work Item Draft Prompt 测试规则，Case A 与 Case B 各 10 个有效首次 Claude Code 输出需操作者显式授权；本文不调用 Provider。",
    "工作树中上游 OpenSpec 契约文件存在既存修改，未纳入本次 changedFiles。"
  ],
  "noStagedFiles": true,
  "diffSummary": "新增一份只含 P2+P4 §8.1–§8.4 实施计划、精确路径/接口/步骤/命令、测试矩阵、门禁、自检和汇报的 Markdown 文档；无代码变更。",
  "reviewFindings": [
    "no blockers；未发现 scope 扩张。"
  ],
  "manualNotes": "父流程应先审查并独立提交 Task 8.1（P4），再进入 P2；提交由 orchestration 执行，本文不含 commit step。"
}
```
