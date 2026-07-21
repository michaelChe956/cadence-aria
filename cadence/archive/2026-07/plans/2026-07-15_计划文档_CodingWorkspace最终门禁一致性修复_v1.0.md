# Coding Workspace 最终门禁一致性修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让写入范围与验证证据在单个 Work Item 完成前完成确定性校验，并保证 Group Final Review 通过后只做幂等状态收尾，不再出现 `coding_start_failed`、范围漂移或验证证据缺失错误。

**Architecture:** 采用“CodeReviewer 结构化判定 → Unit Completion Preflight → Group Completion Preflight → Review Request → Group Final Review → 幂等完成收尾”的顺序。CodeReviewer 对 required gates 和非 Forbidden 的精确写入范围 amendment 给出结构化、可审计结论；平台在 Unit 完成前做确定性校验并持久化 attestation，整组 Review 前聚合为 preflight 记录。Group Final Review 通过后不再重新执行范围与验证业务门禁，只消费已通过且未失效的 preflight 记录并同步 Attempt、Work Item 元数据与共享 worktree 锁。

**Tech Stack:** Rust 2024、Serde JSON、Axum WebSocket、现有 CodingAttemptStore/LifecycleStore、React/Vitest 仅用于平台自身前端回归。

## Global Constraints

- 这是通用 AI 开发平台能力，不得把产品逻辑硬编码为 Rust、Cargo、Vite、pnpm 或任何单一技术栈。
- 平台仓库自身的验证可以继续使用 Rust/Cargo 与 React/Vitest；这些命令只是本仓库回归命令，不是产品对用户项目的限制。
- Code Reviewer 与 Group Final Review 均禁止提出 E2E、端到端测试、Playwright、浏览器自动化或浏览器环境安装相关 findings；普通单元测试、非浏览器集成测试、编译、构建、类型检查、静态分析、格式与 lint 均允许。
- Reviewer 与 Coder 可以使用 Verification Plan 之外的非 E2E 验证；`additional_checks` 必须被保留，不能把执行范围限制为 Work Item 已列命令。
- `forbidden_write_scopes` 永远不可 amendment、不可自动扩展、不可被 Group Final Review 覆盖。
- `exclusive_write_scopes` 外的合法返修只能通过精确文件路径 amendment 表达；禁止 amendment 使用 `*`、`**` 或目录前缀。
- 不重新引入已摘除的 Testing/Tester 执行阶段；CodeReviewer 的结构化 verification assessment 是精简链路的新权威证据。
- Group Final Review approve 后不得重新 Coding、Code Review 或 Group Final Review；完成收尾必须幂等。
- 不修改 Work Item Source Draft 历史记录；运行期 amendment 属于 Coding Attempt 的审计数据。
- 所有 Cargo 命令禁止 `-j 1`。

---

## 事故记录与根因

当前 `coding_attempt_0001` 的 10 个 Unit、Review Request 和 Group Final Review 均已完成，Group Final Review verdict 为 `approve`，但 approve 后的 `run_group_completion_gates` 仍重新读取原始 Work Item 与 `testing-reports/`，依次暴露以下问题：

1. Work Item 1 的 Code Review 必要返修修改了 `src/cross_cutting/provider_adapter.rs` 与 `src/protocol/provider_errors.rs`，两者不在原始 Exclusive Scopes，也不在 Forbidden Scopes。
2. Work Item 7 handoff 包含 `web/pnpm-workspace.yaml`，不在原始 Exclusive Scopes；单项 Review 曾说明它是应删除的环境产物，但最终 handoff 仍提交了该文件，说明 Group Final Review 不能替代提前的确定性范围门禁。
3. 2026-07-06 的精简改造已从执行编排摘除 Testing 阶段，但最终门禁仍强制要求每个 Verification Plan 存在 `TestingReport`，导致执行链与完成链的证据模型不一致。
4. Group Final Review approve 后才执行上述门禁，错误经 runner 冒泡成 `coding_start_failed`，用户看到“Review 已通过但 Attempt 仍 running”。
5. 历史数据修复曾清空 `current_work_item_id`，导致完成路径可能使用首个 Work Item 释放最后一个 Unit 的共享锁；Work Item 1 还出现 `execution_status=completed` 但 `completion_commit/handoff_summary_ref` 为空的部分同步状态。

本次兼容性数据修复已完成：补齐两个 Work Item 的精确 Exclusive Scopes、按现有 Unit handoff 重建 10 份带 `backend_verified=false` 标记的历史 TestingReport、恢复最后一个 `current_work_item_id`、通过正式 WebSocket `final_confirm` 完成 Attempt、释放共享锁，并补齐 Work Item 1 的 completion/handoff 元数据。永久方案不得把这种手工补数据作为正常流程。

## 方案选择

- 方案 A：在最终报错后继续修改 Work Item scopes、补 TestingReport，再重试最终门禁。只适合一次性数据恢复，会重复制造 Source Draft/Compiled/Attempt 漂移，不作为产品方案。
- 方案 B：保留当前顺序，但把 Group Final Review 的 approve 当成所有范围和验证问题的自动豁免。会让 Forbidden、无证据测试和意外文件失去确定性保护，不采用。
- 方案 C（采用）：CodeReviewer 输出结构化 verification assessment 与精确 scope amendments；Unit 完成前校验并持久化 attestation，Group Final Review 前聚合 preflight，approve 后只做幂等完成收尾。

---

### Task 1: 扩展 CodeReviewer 结构化审查契约

**Files:**

- Modify: `src/product/coding_models/review.rs:101-160`
- Modify: `src/product/coding_workspace_engine/review_parser.rs:1-150`
- Modify: `src/product/coding_workspace_engine/reports.rs:110-175`
- Modify: `src/product/coding_workspace_engine/prompts.rs:5-55, 290-330`
- Modify: `src/product/coding_models/tests.rs`
- Modify: `src/product/coding_workspace_engine/tests/coder_resume_recovery.rs`
- Modify: `src/product/coding_workspace_engine/tests/gate_coder_feedback.rs`
- Modify: `src/product/coding_workspace_engine/tests/gate_rework.rs`
- Modify: `src/product/coding_workspace_engine/tests/provider_driven.rs`
- Modify: `src/web/coding_ws_handler/tests.rs`
- Modify: `tests/it_product/product_coding_attempt_store/part_02.rs`
- Modify: `tests/it_web/web_coding_attempt_api/part_03.rs`
- Modify: `web/src/api/types/coding.ts:284-305`
- Modify: `web/src/api/types.test.ts`
- Modify: `web/src/state/coding-workspace-store.test.ts`
- Test: `src/product/coding_workspace_engine/tests/parser_prompt/review_parser.rs`
- Test: `src/product/coding_workspace_engine/tests/parser_prompt.rs`
- Test: `tests/it_product/product_coding_models.rs`

**Interfaces:**

- Consumes: 当前 Work Item、Verification Plan、CoderEvidencePack、当前 Unit diff。
- Produces: `CodeReviewReport.verification_assessment`、`approved_scope_amendments`、`work_item_id`、`unit_id`。

- [ ] **Step 1: 写反序列化与兼容性失败测试**

  增加测试，覆盖完整新字段、旧 JSON 缺字段仍可读取、amendment 含通配符时解析后由门禁拒绝。

  ```rust
  #[test]
  fn code_review_report_accepts_structured_verification_and_scope_amendments() {
      let report: CodeReviewReport = serde_json::from_value(serde_json::json!({
          "id": "code_review_0001",
          "attempt_id": "coding_attempt_0001",
          "round": 1,
          "verdict": "approve",
          "findings": [],
          "tested_evidence_refs": ["coder_output_0001"],
          "diff_refs": ["unit_diff_0001"],
          "summary": "通过",
          "created_at": "2026-07-15T00:00:00Z",
          "work_item_id": "work_item_0001",
          "unit_id": "coding_unit_0001",
          "verification_assessment": {
              "plan_id": "verification_plan_0001",
              "satisfied_required_gates": ["unit_tests"],
              "missing_required_gates": [],
              "additional_checks": ["custom-check --strict"],
              "evidence_refs": ["coder_output_0001"]
          },
          "approved_scope_amendments": [{
              "path": "src/shared/error.rs",
              "reason": "实现 reviewer 要求的公共错误码修复"
          }]
      })).expect("new review report");

      assert_eq!(report.unit_id.as_deref(), Some("coding_unit_0001"));
      assert_eq!(
          report.verification_assessment
              .as_ref()
              .expect("assessment")
              .additional_checks,
          vec!["custom-check --strict"]
      );
  }
  ```

- [ ] **Step 2: 运行定向测试确认 RED**

  Run: `cargo test --locked --lib code_review_report_accepts_structured_verification_and_scope_amendments`

  Expected: FAIL，新字段和类型尚不存在。

- [ ] **Step 3: 增加审查模型**

  在 `review.rs` 增加以下类型，并给 `CodeReviewReport` 的新字段添加 `#[serde(default)]`，保证旧数据可读取。

  ```rust
  #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
  pub struct CodeReviewVerificationAssessment {
      pub plan_id: String,
      #[serde(default)]
      pub satisfied_required_gates: Vec<String>,
      #[serde(default)]
      pub missing_required_gates: Vec<String>,
      #[serde(default)]
      pub additional_checks: Vec<String>,
      #[serde(default)]
      pub evidence_refs: Vec<String>,
  }

  #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
  pub struct CodeReviewScopeAmendment {
      pub path: String,
      pub reason: String,
  }
  ```

  `CodeReviewReport` 新增：

  ```rust
  #[serde(default)]
  pub work_item_id: Option<String>,
  #[serde(default)]
  pub unit_id: Option<String>,
  #[serde(default)]
  pub verification_assessment: Option<CodeReviewVerificationAssessment>,
  #[serde(default)]
  pub approved_scope_amendments: Vec<CodeReviewScopeAmendment>,
  ```

- [ ] **Step 4: 扩展 provider payload 与报告绑定**

  `RawCodeReviewProviderPayload` 增加同名 assessment/amendments 字段；`build_code_review_report` 从当前 Attempt 绑定 Work Item 与活动 Unit，而不是相信 provider 自报 ID。

  ```rust
  let active_unit = self.store.get_active_coding_unit(
      &attempt.project_id,
      &attempt.issue_id,
      &attempt.id,
  )?;

  let work_item_id = Some(self.active_work_item_id_for_attempt(attempt).to_string());
  let unit_id = active_unit.map(|unit| unit.id);
  let verification_assessment = payload.verification_assessment;
  let approved_scope_amendments = payload.approved_scope_amendments;
  ```

  将这四个局部变量赋给 `CodeReviewReport` 的同名字段。同步更新所有现有 `CodeReviewReport` Rust struct literal，为新字段写入 `None`/空数组；不能只依赖 Serde default，因为 Rust 编译期 struct literal 仍要求列出新增字段。前端 `CodeReviewReport` 类型新增对应可选字段：

  ```ts
  export type CodeReviewVerificationAssessment = {
    plan_id: string;
    satisfied_required_gates: string[];
    missing_required_gates: string[];
    additional_checks: string[];
    evidence_refs: string[];
  };

  export type CodeReviewScopeAmendment = {
    path: string;
    reason: string;
  };
  ```

- [ ] **Step 5: 调优 CodeReviewer prompt，不限制普通测试范围**

  在现有 `code_review_material_protocol()` 中加入：

  ```text
  - verification_assessment 必须逐项列出 Verification Plan 的 required_gates 是否已有可信证据。
  - Verification Plan 之外执行的普通非 E2E 验证写入 additional_checks，不得因其未列在 Work Item 中而否定。
  - 只有 Exclusive Scopes 外且不属于 Forbidden Scopes、并且确为实现目标或本轮 finding 必需的精确文件，才可写入 approved_scope_amendments。
  - approved_scope_amendments.path 必须是单个仓库相对文件路径，不得包含通配符或目录范围。
  - Forbidden Scopes 中的路径不得进入 approved_scope_amendments，必须 request_changes 或 blocked。
  ```

  输出 JSON 契约明确包含：

  ```json
  {
    "verdict": "approve",
    "summary": "全部 required gates 已有可信证据",
    "findings": [],
    "verification_assessment": {
      "plan_id": "verification_plan_0001",
      "satisfied_required_gates": ["unit_tests"],
      "missing_required_gates": [],
      "additional_checks": ["custom-check --strict"],
      "evidence_refs": ["coder_output_0001"]
    },
    "approved_scope_amendments": []
  }
  ```

- [ ] **Step 6: 验证 prompt 继续禁止 E2E，但允许其他测试**

  Run: `cargo test --locked --lib parser_prompt`

  Expected: PASS；CodeReviewer 与 Group Final Review prompt 都包含 E2E/Playwright 禁止条款，且包含 `additional_checks` 与“非 E2E 验证不受 Verification Plan 严格限制”。

- [ ] **Step 7: 提交**

  ```bash
  git add src/product/coding_models/review.rs \
          src/product/coding_workspace_engine/review_parser.rs \
          src/product/coding_workspace_engine/reports.rs \
          src/product/coding_workspace_engine/prompts.rs \
          src/product/coding_models/tests.rs \
          src/product/coding_workspace_engine/tests/coder_resume_recovery.rs \
          src/product/coding_workspace_engine/tests/gate_coder_feedback.rs \
          src/product/coding_workspace_engine/tests/gate_rework.rs \
          src/product/coding_workspace_engine/tests/provider_driven.rs \
          src/product/coding_workspace_engine/tests/parser_prompt.rs \
          src/product/coding_workspace_engine/tests/parser_prompt/review_parser.rs \
          src/web/coding_ws_handler/tests.rs \
          tests/it_product/product_coding_attempt_store/part_02.rs \
          tests/it_product/product_coding_models.rs \
          tests/it_web/web_coding_attempt_api/part_03.rs \
          web/src/api/types/coding.ts \
          web/src/api/types.test.ts \
          web/src/state/coding-workspace-store.test.ts
  git commit -m "feat(coding): structure reviewer scope and verification evidence"
  ```

### Task 2: 在 Unit 完成前执行确定性门禁并持久化 attestation

**Files:**

- Modify: `src/product/coding_models/group.rs`
- Create: `src/product/coding_workspace_engine/unit_completion_preflight.rs`
- Modify: `src/product/coding_workspace_engine/mod.rs:67-87`
- Modify: `src/product/coding_workspace_engine/handoffs.rs:580-615`
- Modify: `src/product/coding_workspace_engine/types.rs:1-55`
- Create: `src/product/coding_attempt_store/completion_preflight.rs`
- Modify: `src/product/coding_attempt_store/mod.rs:1-20`
- Modify: `src/product/coding_attempt_store/paths.rs:1-110`
- Test: `tests/it_product/product_coding_workspace_engine/part_13.rs`
- Test: `tests/it_product/product_coding_workspace_engine/part_14.rs`

**Interfaces:**

- Consumes: 当前 Unit、Work Item、handoff、最新 approve CodeReviewReport、Verification Plan。
- Produces: `UnitCompletionAttestation`；失败时 Unit 保持 active，不进入下一个 Unit。

- [ ] **Step 1: 写四个失败测试与一个通过测试**

  覆盖：无 amendment 的 Exclusive 越界、合法精确 amendment、Forbidden 路径即使有 amendment 也拒绝、required gate 缺失、required gate 全覆盖且 additional checks 存在时通过。

  ```rust
  #[tokio::test]
  async fn unit_preflight_never_allows_forbidden_scope_amendment() {
      let fixture = group_unit_preflight_fixture();
      fixture.save_approved_review(
          vec![CodeReviewScopeAmendment {
              path: "tests/browser/spec.ts".to_string(),
              reason: "review remediation".to_string(),
          }],
          assessment_all_required_gates(),
      );
      fixture.save_handoff_files(vec!["tests/browser/spec.ts"]);

      let error = fixture.engine
          .run_current_unit_completion_preflight(&fixture.attempt)
          .await
          .expect_err("forbidden path must fail");

      assert!(matches!(
          error,
          CodingWorkspaceEngineError::WorkItemDiffScopeViolation(path)
              if path == "tests/browser/spec.ts"
      ));
  }
  ```

- [ ] **Step 2: 运行定向测试确认 RED**

  Run: `cargo test --locked --test it_product unit_preflight_`

  Expected: FAIL，preflight 与 attestation 尚不存在。

- [ ] **Step 3: 增加 attestation 模型与存储**

  ```rust
  #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
  pub struct UnitCompletionAttestation {
      pub id: String,
      pub attempt_id: String,
      pub unit_id: String,
      pub work_item_id: String,
      pub completion_commit: String,
      pub handoff_ref: String,
      pub code_review_id: String,
      pub verification_plan_id: Option<String>,
      #[serde(default)]
      pub satisfied_required_gates: Vec<String>,
      #[serde(default)]
      pub additional_checks: Vec<String>,
      #[serde(default)]
      pub approved_scope_amendments: Vec<CodeReviewScopeAmendment>,
      pub created_at: String,
  }
  ```

  文件路径固定为：

  ```text
  coding-attempts/{attempt_id}/unit-completion-attestations/{unit_id}.json
  ```

- [ ] **Step 4: 实现有效写入范围计算**

  规则顺序必须固定：先检查 Forbidden，再验证 amendment 是精确文件路径，最后用 `exclusive_write_scopes + approved_scope_amendments.path` 校验 handoff files。

  ```rust
  fn amendment_is_exact_file(path: &str) -> bool {
      !path.is_empty()
          && !path.contains('*')
          && !path.ends_with('/')
          && !path.starts_with('/')
          && !path.split('/').any(|part| part == "..")
  }
  ```

  只有以下条件全部满足时 amendment 生效：report verdict 为 approve、report.work_item_id 与 Unit 匹配、report.unit_id 与 Unit 匹配、path 出现在 handoff.files_changed、path 不命中 Forbidden。

- [ ] **Step 5: 实现 verification assessment 校验**

  若 Work Item 有 required gates，则 `assessment.plan_id` 必须匹配，`missing_required_gates` 必须为空，且每个 required gate 都存在于 `satisfied_required_gates`。`additional_checks` 只记录，不参与 required gate 的集合相等判断。

  ```rust
  let missing = verification_plan
      .required_gates
      .iter()
      .filter(|gate| !assessment.satisfied_required_gates.contains(gate))
      .cloned()
      .collect::<Vec<_>>();
  if !missing.is_empty() || !assessment.missing_required_gates.is_empty() {
      return Err(CodingWorkspaceEngineError::VerificationGateFailed(
          missing.join(","),
      ));
  }
  ```

  兼容旧数据：若 assessment 缺失，可接受同 plan_id 的 Passed/PassedWithWarnings TestingReport；新生成的 review 必须使用 assessment，不再生成伪 TestingReport。

- [ ] **Step 6: 调整 Unit 完成顺序**

  `complete_group_unit_after_code_review` 改为：commit → handoff → unit preflight → attestation → complete unit。

  ```rust
  pub async fn complete_group_unit_after_code_review(
      &self,
      attempt: &CodingExecutionAttempt,
  ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
      let attempt = self.commit_current_group_unit_changes(attempt).await?;
      self.generate_and_save_work_item_handoff_if_missing(&attempt).await?;
      self.run_current_unit_completion_preflight(&attempt).await?;
      self.complete_current_group_unit(&attempt, Some("当前 Work Item 已完成".to_string()))
          .await
  }
  ```

- [ ] **Step 7: 门禁失败进入可恢复 blocked 状态**

  runner 捕获 scope/verification preflight 错误，创建现有 `CodingGateKind::Blocked`，stage 保持 `code_review`，actions 使用 `send_to_coder`、`retry_review`、`abort`。不得让错误冒泡为 `coding_start_failed`。

- [ ] **Step 8: 运行测试确认 GREEN**

  Run: `cargo test --locked --test it_product unit_preflight_`

  Expected: PASS；合法 amendment 通过，Forbidden 永远失败，additional checks 不被丢弃。

- [ ] **Step 9: 提交**

  ```bash
  git add src/product/coding_models/group.rs \
          src/product/coding_workspace_engine/unit_completion_preflight.rs \
          src/product/coding_workspace_engine/mod.rs \
          src/product/coding_workspace_engine/handoffs.rs \
          src/product/coding_workspace_engine/types.rs \
          src/product/coding_attempt_store/completion_preflight.rs \
          src/product/coding_attempt_store/mod.rs \
          src/product/coding_attempt_store/paths.rs \
          tests/it_product/product_coding_workspace_engine/part_13.rs \
          tests/it_product/product_coding_workspace_engine/part_14.rs
  git commit -m "feat(coding): gate each unit before completion"
  ```

### Task 3: 在 Group Final Review 前持久化整组 preflight

**Files:**

- Modify: `src/product/coding_models/group.rs`
- Create: `src/product/coding_workspace_engine/group_completion_preflight.rs`
- Modify: `src/product/coding_workspace_engine/mod.rs:67-87`
- Modify: `src/product/coding_attempt_store/completion_preflight.rs`
- Modify: `src/web/coding_ws_handler/runner.rs:545-640`
- Modify: `src/product/coding_workspace_engine/internal_pr_review.rs:36-115`
- Test: `tests/it_product/product_coding_workspace_engine/part_13.rs`
- Test: `tests/it_web/web_coding_ws_handler/part_04.rs`

**Interfaces:**

- Consumes: 全部 completed Units、每个 Unit 的 attestation、handoff、completion commit、Attempt HEAD、共享 worktree 状态。
- Produces: `GroupCompletionPreflight`；只有 passed preflight 才允许 Review Request 与 Group Final Review。

- [ ] **Step 1: 写“GFR 不应启动”的失败测试**

  构造一个缺 attestation 或 attestation commit 过期的整组 Attempt，断言不会调用 Internal Reviewer provider，不会创建 `internal_review_*`，而是返回 blocked gate。

  ```rust
  assert_eq!(internal_reviewer.call_count(), 0);
  assert!(store
      .list_internal_pr_reviews("project_0001", "issue_0001", "coding_attempt_0001")
      .expect("reviews")
      .is_empty());
  assert_eq!(attempt.status, CodingAttemptStatus::Blocked);
  ```

- [ ] **Step 2: 运行定向测试确认 RED**

  Run: `cargo test --locked --test it_web group_preflight_blocks_before_group_final_review`

  Expected: FAIL，当前 runner 会先启动 Group Final Review。

- [ ] **Step 3: 增加 GroupCompletionPreflight 模型**

  ```rust
  #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
  pub struct GroupCompletionPreflight {
      pub id: String,
      pub attempt_id: String,
      pub head_commit: String,
      pub unit_attestation_refs: Vec<String>,
      pub last_work_item_id: String,
      pub created_at: String,
  }
  ```

  存储路径固定为：

  ```text
  coding-attempts/{attempt_id}/group-completion-preflight.json
  ```

- [ ] **Step 4: 实现整组聚合校验**

  必须验证：所有 Unit completed、每个 Unit 有 handoff 与 attestation、attestation 的 unit/work_item/commit/handoff_ref 与当前数据一致、最后 Unit commit 等于 Attempt HEAD、worktree clean、共享锁 holder 等于最后 Work Item。

  不再从原始 Work Item 重新推导 amendment，也不再要求 TestingReport；这些已在 Unit preflight 中完成。

- [ ] **Step 5: 调整 runner 顺序**

  在 `execute_review_request` 前运行并保存 preflight：

  ```rust
  if current.scope == CodingAttemptScope::WorkItemGroup {
      match engine.run_group_completion_preflight(&current).await {
          Ok(preflight) => engine.emit_group_preflight_passed(&preflight).await,
          Err(error) => {
              engine.block_group_completion_preflight(&current, error).await?;
              current = coding_store.get_attempt(
                  &current.project_id,
                  &current.issue_id,
                  &current.id,
              )?;
              return emit_current_session_state(event_tx, coding_store, &current).await;
          }
      }
  }
  ```

  顺序固定为：最后 Unit completed → group preflight passed → Review Request push → Group Final Review。

- [ ] **Step 6: Group Final Review prompt 只消费 preflight 结果**

  在 Completed Units 区域增加 preflight ID、HEAD 与 attestation refs；保留现有 E2E 禁止条款。GFR 可以审查功能正确性、依赖闭环、PR 描述和实际 diff，但不得把 Exclusive amendment 当成隐式审批渠道。

- [ ] **Step 7: 运行测试确认 GREEN**

  Run: `cargo test --locked --test it_product group_preflight_`

  Run: `cargo test --locked --test it_web group_preflight_`

  Expected: PASS；preflight 失败时没有 GFR provider 调用，preflight 通过后才出现 Review Request 与 GFR。

- [ ] **Step 8: 提交**

  ```bash
  git add src/product/coding_models/group.rs \
          src/product/coding_workspace_engine/group_completion_preflight.rs \
          src/product/coding_workspace_engine/mod.rs \
          src/product/coding_attempt_store/completion_preflight.rs \
          src/web/coding_ws_handler/runner.rs \
          src/product/coding_workspace_engine/internal_pr_review.rs \
          tests/it_product/product_coding_workspace_engine/part_13.rs \
          tests/it_web/web_coding_ws_handler/part_04.rs
  git commit -m "feat(coding): preflight groups before final review"
  ```

### Task 4: Group Final Review approve 后只做幂等完成收尾

**Files:**

- Modify: `src/product/coding_workspace_engine/handoffs.rs:15-90, 580-704`
- Modify: `src/product/coding_workspace_engine/gates.rs:186-303`
- Modify: `src/product/lifecycle_store/work_item.rs`
- Modify: `src/product/coding_attempt_store/attempt.rs:388-435`
- Test: `tests/it_product/product_coding_workspace_engine/part_13.rs`
- Test: `tests/it_web/web_coding_ws_handler/part_04.rs`

**Interfaces:**

- Consumes: approve InternalPrReview、passed GroupCompletionPreflight。
- Produces: completed Attempt、10 个同步完成的 Work Items、释放后的共享锁；重复调用结果相同。

- [ ] **Step 1: 写 approve 后不再运行业务门禁的测试**

  注入一个 spy preflight evaluator，在 GFR 前计数为 1；GFR approve 后完成收尾不得再次调用 scope/verification evaluator。

  ```rust
  assert_eq!(preflight_probe.unit_evaluation_count(), 2);
  assert_eq!(preflight_probe.group_evaluation_count(), 1);
  assert_eq!(attempt.status, CodingAttemptStatus::Completed);
  ```

- [ ] **Step 2: 写元数据与锁的幂等测试**

  覆盖 `current_work_item_id=None` 的历史数据，完成函数必须使用 `shared.current_active_work_item_id`，其次使用最后 completed Unit，不得回退到首个 `attempt.work_item_id`。

  每个 Work Item 必须同步：

  ```rust
  execution_status = WorkItemStatus::Completed;
  handoff_summary_ref = unit.handoff_ref.clone();
  completion_commit = unit.completion_commit.clone();
  ```

- [ ] **Step 3: 删除 approve 后的 `run_group_completion_gates` 调用**

  `complete_group_attempt_after_final_review` 只验证 passed preflight 存在且 `preflight.head_commit == attempt.head_commit`，然后调用单一的幂等收尾函数。

  ```rust
  pub(crate) async fn complete_group_attempt_after_final_review(
      &self,
      attempt: &CodingExecutionAttempt,
  ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
      self.require_current_group_preflight(attempt)?;
      self.finalize_group_completion(attempt)
  }
  ```

  `require_current_group_preflight` 是状态不变量读取，不重新执行 scope、verification 或测试证据门禁。

- [ ] **Step 4: 实现幂等 `finalize_group_completion`**

  顺序为：同步 Work Item completion metadata → 标记 Attempt completed → 释放最后 Work Item 锁。任何一步重复执行均返回当前一致状态。

  若进程在中途退出，下一次 REST/WebSocket 构建 session state 时对 completed group Attempt 调用 reconciliation，补齐未完成的元数据或锁释放，不重新调用 provider。

- [ ] **Step 5: 调整错误语义**

  删除正常 group runner 中 approve 后可能抛出的 `WorkItemDiffScopeViolation` 与 `VerificationGateResultMissing` 路径。preflight 失败使用 blocked gate reason codes：

  ```text
  group_unit_scope_amendment_required
  group_unit_forbidden_scope_violation
  group_unit_verification_evidence_missing
  group_completion_preflight_inconsistent
  ```

  WebSocket 不再把这些错误包装为 `coding_start_failed`；用户看到可恢复 gate 与明确受影响 Unit。

- [ ] **Step 6: 运行测试确认 GREEN**

  Run: `cargo test --locked --test it_product group_final_review_approve_`

  Run: `cargo test --locked --test it_web group_final_review_approve_`

  Expected: PASS；GFR approve 后 Attempt 一次完成，Work Item 元数据齐全，共享锁释放，无重复 Coding/Review。

- [ ] **Step 7: 提交**

  ```bash
  git add src/product/coding_workspace_engine/handoffs.rs \
          src/product/coding_workspace_engine/gates.rs \
          src/product/lifecycle_store/work_item.rs \
          src/product/coding_attempt_store/attempt.rs \
          tests/it_product/product_coding_workspace_engine/part_13.rs \
          tests/it_web/web_coding_ws_handler/part_04.rs
  git commit -m "fix(coding): finalize approved groups without late gates"
  ```

### Task 5: 完成恢复路径与 WebSocket 回归

**Files:**

- Modify: `src/product/coding_workspace_engine/gates.rs:395-560`
- Modify: `src/product/coding_workspace_engine/group.rs`
- Modify: `src/web/coding_ws_handler/socket/admission.rs`
- Modify: `src/web/coding_ws_handler/socket/preparation.rs`
- Test: `tests/it_web/web_coding_ws_handler/part_04.rs`
- Test: `tests/it_web/web_coding_ws_handler/part_08.rs`

**Interfaces:**

- Consumes: preflight blocked gate response。
- Produces: 精确恢复受影响 Unit；不会重复启动两个 runner。

- [ ] **Step 1: 写恢复测试**

  覆盖：scope amendment 缺失后 send_to_coder、只重试 reviewer、双击重试只产生一个 runner、刷新后 gate 仍可操作。

- [ ] **Step 2: 实现精确 Unit reopen**

  `reopen_group_unit_for_completion_rework` 必须恢复指针和共享锁：

  ```rust
  unit.status = CodingExecutionUnitStatus::Running;
  unit.completed_at = None;
  unit.handoff_ref = None;
  unit.completion_commit = None;
  attempt.current_work_item_id = Some(unit.work_item_id.clone());
  attempt.active_unit_id = Some(unit.id.clone());
  attempt.stage = CodingExecutionStage::Coding;
  attempt.status = CodingAttemptStatus::Running;
  ```

  删除该 Unit 旧 attestation 与 group preflight；保留此前完成 Unit 的所有记录。若选择 retry_review，则 stage 为 `code_review`，不执行 coder。

- [ ] **Step 3: 复用 attempt mutation lease 与 runner reservation**

  Gate response 必须沿用当前 WebSocket 的 mutation lease/attempt reservation，确保多次点击只激活一个 runner；错误码继续使用明显的 `coding_runner_already_started`，不产生第二套特殊锁。

- [ ] **Step 4: 运行 WebSocket 定向测试**

  Run: `cargo test --locked --test it_web preflight_recovery_`

  Expected: PASS；刷新可恢复、双击不重复、修复后只从目标 Unit 继续。

- [ ] **Step 5: 提交**

  ```bash
  git add src/product/coding_workspace_engine/gates.rs \
          src/product/coding_workspace_engine/group.rs \
          src/web/coding_ws_handler/socket/admission.rs \
          src/web/coding_ws_handler/socket/preparation.rs \
          tests/it_web/web_coding_ws_handler/part_04.rs \
          tests/it_web/web_coding_ws_handler/part_08.rs
  git commit -m "fix(coding): recover group preflight failures safely"
  ```

### Task 6: 全量验证与文档闭环

**Files:**

- Verify: `src/product/coding_workspace_engine/**`
- Verify: `src/product/coding_attempt_store/**`
- Verify: `src/product/coding_models/**`
- Verify: `src/web/coding_ws_handler/**`
- Verify: `web/src/**`
- Update: `cadence/analysis-docs/2026-07-14_分析报告_WorkItemWorkspace生成缺陷_v1.0.md`

**Interfaces:**

- Consumes: Task 1-5 的实现。
- Produces: 平台级回归证据和问题记录互链。

- [ ] **Step 1: 运行 Rust 格式与定向测试**

  ```bash
  cargo fmt --check
  cargo test --locked --lib parser_prompt
  cargo test --locked --test it_product unit_preflight_
  cargo test --locked --test it_product group_preflight_
  cargo test --locked --test it_web group_preflight_
  cargo test --locked --test it_web preflight_recovery_
  ```

  Expected: 全部 PASS，无 E2E/Playwright 命令。

- [ ] **Step 2: 运行平台仓库标准 Rust 验证**

  ```bash
  cargo clippy --all-targets --all-features --locked -- -D warnings
  cargo check --locked
  cargo test --locked
  ```

  Expected: 全部 PASS，命令不携带 `-j 1`。

- [ ] **Step 3: 运行平台前端普通测试与类型检查**

  ```bash
  cd web
  pnpm test
  pnpm tsc -b
  ```

  Expected: 全部 PASS；不执行 `pnpm test:e2e`，不下载浏览器。

- [ ] **Step 4: 验证通用平台语义**

  测试 fixture 至少包含一个非 Rust/Vite 命令字符串，例如 `custom-build verify`，断言平台只按 Verification Plan gate ID 和证据模型判断，不解析或硬编码语言、包管理器与测试框架。

- [ ] **Step 5: 更新缺陷文档互链**

  在 Work Item Workspace 生成缺陷“事件五”后追加本计划链接，并明确区分：Work Item 生成范围不足是上游输入缺陷；approve 后才运行门禁、Testing 阶段已删除但门禁仍要求 TestingReport、完成元数据与锁非幂等是 Coding Workspace 缺陷。

- [ ] **Step 6: 最终自检**

  ```bash
  rg -n "T[B]D|T[O]DO|implement l[a]ter|fill in d[e]tails" \
    cadence/plans/2026-07-15_计划文档_CodingWorkspace最终门禁一致性修复_v1.0.md
  git diff --check
  git status --short
  ```

  Expected: 计划与实现文档无占位词，diff 无空白错误，仅包含本方案相关文件。

- [ ] **Step 7: 提交**

  ```bash
  git add cadence/analysis-docs/2026-07-14_分析报告_WorkItemWorkspace生成缺陷_v1.0.md
  git commit -m "docs: link coding final gate consistency fix"
  ```

## 完成判据

- 任一 Unit 的范围或验证证据问题都在 Unit 完成前阻断。
- 合法精确 amendment 可通过，Forbidden Scopes 即使 reviewer 输出 amendment 也必须拒绝。
- Verification Plan required gates 被结构化 assessment 覆盖；额外普通非 E2E 验证被保留且允许。
- Group Final Review 启动前已有 passed GroupCompletionPreflight。
- Group Final Review approve 后不再运行 scope/verification 业务门禁，不再出现 `coding_start_failed`。
- Attempt、全部 Work Items、handoff/commit 元数据与共享 worktree 锁最终一致。
- 刷新、重试和多次点击不会创建重复 runner 或重复 review。
- Code Reviewer 与 Group Final Review 均继续禁止 E2E/Playwright findings，但不限制普通单元测试、非浏览器集成测试或平台仓库的 Rust/Vitest 验证。
