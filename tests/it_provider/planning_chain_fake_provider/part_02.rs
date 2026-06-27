impl ProviderAdapter for ScriptedPlanningProvider {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.output_schemas
            .lock()
            .expect("schemas")
            .push(input.output_schema.clone());
        self.prompts
            .lock()
            .expect("prompts")
            .push((input.output_schema.clone(), input.prompt.clone()));
        let payload = match input.output_schema.as_str() {
            "schema://aria/artifacts/clarification_record/v1" => json!({
                "artifact_kind": "clarification_record",
                "goal_summary": "实现 Aria 规划链起始节点",
                "constraints": ["使用 Docker Rust 环境"],
                "assumptions": ["P1 已完成"],
                "open_questions": [],
                "suggested_scope": "N04-N07 fake provider chain"
            }),
            "schema://aria/artifacts/spec/v1" => json!({
                "artifact_kind": "spec",
                "markdown": canonical_spec_markdown()
            }),
            "schema://aria/advisory/spec_gate_review/v1" => json!({
                "artifact_kind": "advisory_review",
                "findings": [],
                "blocking_issues": [],
                "decision_recommendation": "pass"
            }),
            "schema://aria/artifacts/design/v1" => json!({
                "artifact_kind": "design",
                "markdown": canonical_design_markdown()
            }),
            "schema://aria/artifacts/design_review/v1" => {
                let decision = self
                    .review_decisions
                    .lock()
                    .expect("review decisions")
                    .pop_front()
                    .unwrap_or_else(|| "pass".to_string());
                json!({
                    "artifact_kind": "design_review",
                    "review_decision": decision,
                    "findings": if decision == "revise" {
                        json!([{"finding_id": "finding-001", "summary": "补充修订设计"}])
                    } else {
                        json!([])
                    }
                })
            }
            "schema://aria/artifacts/design_revision_record/v1" => json!({
                "artifact_kind": "design_revision_record",
                "revision_summary": "根据评审补充设计决策。",
                "resolved_findings": ["finding-001"],
                "revised_design_markdown": revised_design_markdown()
            }),
            "schema://aria/artifacts/readiness_check/v1" => json!({
                "artifact_kind": "readiness_check",
                "ready": true,
                "blocking_items": []
            }),
            "schema://aria/artifacts/plan/v1" => json!({
                "artifact_kind": "plan",
                "markdown": canonical_plan_markdown()
            }),
            "schema://aria/artifacts/dispatch_package/v1" => json!({
                "artifact_kind": "dispatch_package",
                "worktask_routing": []
            }),
            other => panic!("unexpected schema {other}"),
        };
        let stdout = format!(
            "provider log\n{STRUCTURED_OUTPUT_START}\n{}\n{STRUCTURED_OUTPUT_END}\n",
            serde_json::to_string(&payload).expect("payload json")
        );
        let structured_output = parse_last_structured_output(&stdout)?;
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout,
            stderr: String::new(),
            structured_output,
            files_modified: vec![],
            duration_ms: 1,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}

fn canonical_spec_markdown() -> &'static str {
    "# Spec\n\n## 功能需求\n\n- [REQ-001] 用户可以通过 REPL 创建任务。Priority: must\n\n## 成功标准\n\n- [AC-001] 输入 new_task 后返回 task_id、phase、intake_ref、change_id。Refs: REQ-001\n"
}

fn canonical_design_markdown() -> &'static str {
    "# Design\n\n## 设计决策\n\n- [DD-001] REPL 只作为客户端，daemon 是 runtime truth。Refs: REQ-001\n\n## 公共组件\n\n- [CMP-001] repl_wire JSON envelope schema\n\n## 风险\n\n- [RISK-001] REPL 断连后 daemon 状态不一致。Severity: high; Refs: DD-001\n"
}

fn no_id_design_markdown() -> &'static str {
    "# Design\n\n\
## 设计决策\n\n\
| 决策点 | 选择 | 理由 |\n\
|--------|------|------|\n\
| runtime truth | daemon | REQ-001 要求运行时状态统一 |\n\n\
## 公共组件\n\n\
### Runtime Session Store\n\n\
- **职责**: 保存任务运行态\n"
}

fn revised_design_markdown() -> &'static str {
    "# Design\n\n## 设计决策\n\n- [DD-001] 修订后 REPL 只作为客户端，daemon 是 runtime truth。Refs: REQ-001\n\n## 公共组件\n\n- [CMP-001] repl_wire JSON envelope schema\n\n## 风险\n\n- [RISK-001] REPL 断连后 daemon 状态不一致。Severity: high; Refs: DD-001\n"
}

fn canonical_plan_markdown() -> &'static str {
    "# Plan\n\n## 工作包\n\n| ID | Description | Execution Mode | Human Reason | Traceability | Acceptance |\n|----|-------------|----------------|--------------|--------------|------------|\n| WT-001 | 实现 REPL wire schema | agent_only | | REQ-001, DD-001, TASK-001 | AC-001 |\n| WT-002 | 实现 daemon handshake | agent_only | | REQ-001, DD-001, TASK-002 | AC-001 |\n\n## 依赖关系\n\n| From | To | Type |\n|------|----|------|\n| WT-001 | WT-002 | blocks |\n"
}

fn package_json_plan_markdown() -> &'static str {
    "# Plan\n\n## 工作包\n\n| ID | Description | Execution Mode | Human Reason | Traceability | Acceptance |\n|----|-------------|----------------|--------------|--------------|------------|\n| WT-001 | 实现 climbStairs 源码模块 | agent_only | | REQ-001, DD-001, TASK-001 | AC-001 |\n| WT-002 | 实现 tests/climbStairs.test.js 测试套件 | agent_only | | REQ-001, DD-001, TASK-002 | AC-002 |\n| WT-003 | 在 package.json 注册 test 脚本 node --test，缺失则创建 package.json | agent_only | | REQ-001, DD-001, TASK-003 | AC-007 |\n\n## 依赖关系\n\n| From | To | Type |\n|------|----|------|\n| WT-002 | WT-003 | blocks |\n"
}

fn dec_traceability_plan_markdown() -> &'static str {
    "# Plan\n\n## 工作包\n\n| ID | Description | Execution Mode | Human Reason | Traceability | Acceptance |\n|----|-------------|----------------|--------------|--------------|------------|\n| WT-001 | 实现 runtime session store | agent_only | | REQ-001, DEC-001, TASK-001 | AC-001 |\n\n## 依赖关系\n\n| From | To | Type |\n|------|----|------|\n"
}

fn prepare_change_dir(workspace_root: &Path, change_id: &str) -> std::path::PathBuf {
    let change_dir = workspace_root.join("openspec/changes").join(change_id);
    copy_dir(
        Path::new("tests/fixtures/openspec/changes/sample-change"),
        &change_dir,
    );
    fs::write(
        change_dir.join("specs/main/spec.md"),
        "# Main Spec\n\n### ADDED Requirements\n\n",
    )
    .expect("empty initial spec");
    change_dir
}

fn planning_input(workspace_root: &Path, change_id: &str) -> PlanningStartChainInput {
    let change_dir = workspace_root.join("openspec/changes").join(change_id);
    let initial_manifest =
        cadence_aria::cross_cutting::openspec_constraints::build_openspec_source_manifest(
            &change_dir,
        )
        .expect("initial manifest");
    PlanningStartChainInput {
        session_id: "sess_planning".to_string(),
        task_id: "task_0001".to_string(),
        change_id: change_id.to_string(),
        workspace_root: workspace_root.to_path_buf(),
        worktree_path: None,
        intake_brief: json!({
            "artifact_kind": "intake_brief",
            "request_summary": "实现 Aria 规划链起始节点",
            "raw_user_request": "继续 MVP 内容开发",
            "repo_context": {"branch": "feature/aria-phase1-p2"},
            "initial_constraints": ["使用 Docker Rust 环境"],
            "requested_goal": "N04-N12 fake provider chain"
        }),
        initial_constraint_bundle: OpenSpecConstraintBundle {
            constraint_bundle_id: "constraint_bundle_initial".to_string(),
            bundle_version: "openspec.constraint_bundle.v1".to_string(),
            bundle_status: BundleStatus::Ready,
            change_id: change_id.to_string(),
            proposal_constraints: ProposalConstraints {
                business_intent: vec![
                    "Users need to create runtime tasks from the REPL.".to_string(),
                ],
                scope: vec![
                    "Compile task creation rules into the Phase 1 runtime contract.".to_string(),
                ],
                non_goals: vec![],
                impacted_areas: vec![],
            },
            requirement_constraints: RequirementConstraints {
                requirement_ids: vec![],
                scenario_ids: vec![],
                success_criteria_ids: vec![],
            },
            design_constraints: DesignConstraints {
                design_decision_ids: vec![],
                component_ids: vec![],
                risk_ids: vec![],
            },
            task_constraints: TaskConstraints {
                task_ids: vec![],
                task_sequence: vec![],
                related_requirement_ids_by_task: Default::default(),
                related_design_decision_ids_by_task: Default::default(),
                acceptance_target_ids_by_task: Default::default(),
            },
            traceability_requirements: TraceabilityRequirements {
                required_requirement_ids: vec![],
                required_design_decision_ids: vec![],
                required_task_ids: vec![],
                required_acceptance_target_ids: vec![],
            },
            coverage_model: CoverageModel {
                required_ids: vec![],
                covered_ids: vec![],
                uncovered_ids: vec![],
            },
            source_manifest: initial_manifest,
            compiled_from_projection_refs: vec![],
            compiled_at: "2026-04-27T00:00:00Z".to_string(),
            compiled_by_node: "N03".to_string(),
        },
    }
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create target dir");
    for entry in fs::read_dir(from).expect("read source dir") {
        let entry = entry.expect("dir entry");
        let source_path = entry.path();
        let target_path = to.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir(&source_path, &target_path);
        } else {
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent).expect("create file parent");
            }
            fs::copy(&source_path, &target_path).expect("copy file");
        }
    }
}
