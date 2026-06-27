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
    testing_passes: Mutex<VecDeque<bool>>,
    review_decisions: Mutex<VecDeque<String>>,
    candidate_refs: Vec<String>,
    fail_testing_with_provider_error: bool,
    testing_has_only_out_of_scope_failure: bool,
    testing_artifact_ref: Option<String>,
}

impl ScriptedExecutionProvider {
    fn happy() -> Self {
        Self::new([true], ["pass"])
    }

    fn testing_fails_then_passes() -> Self {
        Self::new([false, true], ["pass"])
    }

    fn testing_has_only_out_of_scope_failure() -> Self {
        Self {
            testing_has_only_out_of_scope_failure: true,
            ..Self::new([false], ["pass"])
        }
    }

    fn review_revises_then_passes() -> Self {
        Self::new([true, true], ["revise", "pass"])
    }

    fn testing_always_fails() -> Self {
        Self::new([false, false, false, false], ["pass"])
    }

    fn testing_provider_errors() -> Self {
        Self {
            fail_testing_with_provider_error: true,
            ..Self::happy()
        }
    }

    fn with_candidate_refs<const C: usize>(candidate_refs: [&str; C]) -> Self {
        let mut provider = Self::happy();
        provider.candidate_refs = candidate_refs.into_iter().map(ToOwned::to_owned).collect();
        provider
    }

    fn with_testing_artifact_ref(mut self, artifact_ref: &str) -> Self {
        self.testing_artifact_ref = Some(artifact_ref.to_string());
        self
    }

    fn new<const T: usize, const R: usize>(testing_passes: [bool; T], reviews: [&str; R]) -> Self {
        Self {
            output_schemas: Mutex::new(Vec::new()),
            seen_prompts: Mutex::new(Vec::new()),
            testing_passes: Mutex::new(testing_passes.into_iter().collect()),
            review_decisions: Mutex::new(reviews.into_iter().map(ToOwned::to_owned).collect()),
            candidate_refs: Vec::new(),
            fail_testing_with_provider_error: false,
            testing_has_only_out_of_scope_failure: false,
            testing_artifact_ref: None,
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
            "schema://aria/artifacts/testing_report/v1" => {
                if self.fail_testing_with_provider_error {
                    return Err(ProviderAdapterError::execution_failed(
                        Some(1),
                        "",
                        "provider quota exhausted",
                        1,
                    ));
                }
                let passed = self
                    .testing_passes
                    .lock()
                    .expect("testing passes")
                    .pop_front()
                    .unwrap_or(true);
                json!({
                    "artifact_kind": "testing_report",
                    "artifact_ref": self.testing_artifact_ref.as_deref().unwrap_or("testing_report_worktask_001_0001"),
                    "worktask_id": "worktask_001",
                    "commands_run": ["cargo test --test execution_chain_fake_provider"],
                    "tests_passed": passed,
                    "failures": if passed {
                        json!([])
                    } else if self.testing_has_only_out_of_scope_failure {
                        json!([{
                            "test": "future_acceptance_target",
                            "failure_type": "out_of_scope_acceptance_failure",
                            "message": "完整测试失败在后续 worktask 的 acceptance target，当前 worktask 范围验证已通过。"
                        }])
                    } else {
                        json!([{"test": "execution_chain", "message": "fixture failure"}])
                    },
                    "scope_result": if self.testing_has_only_out_of_scope_failure {
                        json!("worktask_001_scoped_verification_passed")
                    } else {
                        json!(null)
                    },
                    "candidate_traceability_refs": []
                })
            }
            "schema://aria/artifacts/code_review_report/v1" => {
                let decision = self
                    .review_decisions
                    .lock()
                    .expect("review decisions")
                    .pop_front()
                    .unwrap_or_else(|| "pass".to_string());
                json!({
                    "artifact_kind": "code_review_report",
                    "artifact_ref": "code_review_report_worktask_001_0001",
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
            "provider log\n{STRUCTURED_OUTPUT_START}\n{}\n{STRUCTURED_OUTPUT_END}\n",
            serde_json::to_string(&payload).expect("payload json")
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
