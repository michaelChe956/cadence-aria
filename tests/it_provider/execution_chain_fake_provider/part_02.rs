fn execution_worktask_input(workspace_root: &std::path::Path) -> ExecutionWorktaskInput {
    ExecutionWorktaskInput {
        session_id: "session_001".to_string(),
        task_id: "task_001".to_string(),
        worktask_id: "worktask_001".to_string(),
        source_work_package_id: "WP-001".to_string(),
        worktree_path: workspace_root.join("worktree"),
        allowed_write_scope: vec!["src/feature/".to_string()],
        dispatch_package: json!({
            "artifact_kind": "dispatch_package",
            "_aria": {
                "worktask_routing": [
                    {
                        "worktask_id": "worktask_001",
                        "source_work_package_id": "WP-001",
                        "execution_mode": "agent_only",
                        "allowed_write_scope": ["src/feature/"],
                        "traceability_refs": ["req-001", "dd-001", "task-001"],
                        "verification_commands": ["cargo test --test execution_chain_fake_provider"]
                    }
                ]
            }
        }),
        plan_projection: PlanProjection {
            work_packages: vec![WorkPackageProjection {
                work_package_id: "WP-001".to_string(),
                description: "实现执行链".to_string(),
                execution_mode: ExecutionMode::AgentOnly,
                human_required_reason: None,
                traceability_refs: vec![
                    "req-001".to_string(),
                    "dd-001".to_string(),
                    "task-001".to_string(),
                ],
                acceptance_targets: vec![
                    "cargo test --test execution_chain_fake_provider".to_string(),
                ],
            }],
            dependencies: vec![],
            parallelism_groups: vec![],
        },
        projection_refs: vec![
            "proj_spec_projection_001".to_string(),
            "proj_design_projection_001".to_string(),
            "proj_plan_projection_001".to_string(),
        ],
        constraint_bundle_ref: "constraint_bundle_task_001".to_string(),
        risk_registry_ref: "risk_registry_001".to_string(),
        context_files: vec![
            "tests/fixtures/artifacts/spec.md".to_string(),
            "tests/fixtures/projections/plan_projection.json".to_string(),
            "tests/fixtures/openspec/constraint_bundle.json".to_string(),
        ],
    }
}

#[derive(Debug)]
struct ScriptedExecutionProvider {
    output_schemas: Mutex<Vec<String>>,
    seen_prompts: Mutex<Vec<(String, String)>>,
    review_decisions: Mutex<VecDeque<String>>,
    candidate_refs: Vec<String>,
    fail_review_with_provider_error: bool,
    review_artifact_ref: Option<String>,
}

impl ScriptedExecutionProvider {
    fn happy() -> Self {
        Self::new(["pass"])
    }

    fn review_revises_then_passes() -> Self {
        Self::new(["revise", "pass"])
    }

    fn review_always_revises() -> Self {
        Self::new(["revise", "revise", "revise", "revise"])
    }

    fn review_provider_errors() -> Self {
        Self {
            fail_review_with_provider_error: true,
            ..Self::happy()
        }
    }

    fn with_candidate_refs<const C: usize>(candidate_refs: [&str; C]) -> Self {
        let mut provider = Self::happy();
        provider.candidate_refs = candidate_refs.into_iter().map(ToOwned::to_owned).collect();
        provider
    }

    fn with_review_artifact_ref(mut self, artifact_ref: &str) -> Self {
        self.review_artifact_ref = Some(artifact_ref.to_string());
        self
    }

    fn new<const R: usize>(reviews: [&str; R]) -> Self {
        Self {
            output_schemas: Mutex::new(Vec::new()),
            seen_prompts: Mutex::new(Vec::new()),
            review_decisions: Mutex::new(reviews.into_iter().map(ToOwned::to_owned).collect()),
            candidate_refs: Vec::new(),
            fail_review_with_provider_error: false,
            review_artifact_ref: None,
        }
    }

    fn seen_output_schemas(&self) -> Vec<String> {
        self.output_schemas.lock().expect("schemas").clone()
    }

    fn seen_prompts_for_schema(&self, schema: &str) -> Vec<String> {
        self.seen_prompts
            .lock()
            .expect("prompts")
            .iter()
            .filter(|(seen_schema, _)| seen_schema == schema)
            .map(|(_, prompt)| prompt.clone())
            .collect()
    }
}

impl ProviderAdapter for ScriptedExecutionProvider {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.output_schemas
            .lock()
            .expect("schemas")
            .push(input.output_schema.clone());
        self.seen_prompts
            .lock()
            .expect("prompts")
            .push((input.output_schema.clone(), input.prompt.clone()));
        let payload = match input.output_schema.as_str() {
            "schema://aria/artifacts/coding_report/v1" => json!({
                "artifact_kind": "coding_report",
                "artifact_ref": "coding_report_worktask_001_0001",
                "worktask_id": "worktask_001",
                "files_modified": ["src/feature/lib.rs"],
                "commands_run": ["cargo test --test execution_chain_fake_provider"],
                "candidate_traceability_refs": self.candidate_refs.clone(),
                "status": "completed"
            }),
            "schema://aria/artifacts/code_review_report/v1" => {
                if self.fail_review_with_provider_error {
                    return Err(ProviderAdapterError::execution_failed(
                        Some(1),
                        "",
                        "provider quota exhausted",
                        1,
                    ));
                }
                let decision = self
                    .review_decisions
                    .lock()
                    .expect("review decisions")
                    .pop_front()
                    .unwrap_or_else(|| "pass".to_string());
                json!({
                    "artifact_kind": "code_review_report",
                    "artifact_ref": self.review_artifact_ref.as_deref().unwrap_or("code_review_report_worktask_001_0001"),
                    "worktask_id": "worktask_001",
                    "findings": if decision == "revise" {
                        json!([{"finding_id": "finding-001", "summary": "补充失败项修复"}])
                    } else {
                        json!([])
                    },
                    "blocking": decision == "revise",
                    "candidate_traceability_refs": []
                })
            }
            other => panic!("unexpected schema {other}"),
        };
        let stdout = format!(
            "provider log\n{}\n",
            structured_output_sentinel("fix00001", &payload)
        );
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: stdout.clone(),
            stderr: String::new(),
            structured_output: parse_last_structured_output(&stdout)?,
            files_modified: payload
                .get("files_modified")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .collect(),
            duration_ms: 1,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}
