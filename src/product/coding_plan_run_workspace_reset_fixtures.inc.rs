/// H2a 共享测试夹具（web 层零成员闭包测试与本地 store 测试同源，避免第二
/// 份 fixture）：PlanRepair 种子 + 两 logical 仓 + per-unit target 草稿 +
/// Manual advance 到 durable Ready（per-target attempts Created、units
/// Pending、advance record/外层 journal/per-target journals 在场）。
#[cfg(test)]
pub(crate) mod workspace_reset_fixtures {
    use super::*;
    use crate::product::checkpoint_store::CheckpointStore;
    use crate::product::logical_codebase::{
        AggregatePolicyArtifactStore, IssueCodebaseSelection, IssueCodebaseSelectionStore,
        LogicalCodebaseFeature, LogicalCodebaseStore,
    };
    use crate::product::models::{
        WorkItemDraftCandidate, WorkItemDraftRecord, WorkItemDraftStatus,
        WorkItemDraftVerificationPlan, WorkItemGenerationMode, WorkspaceType,
    };
    use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
    use crate::product::work_item_plan_store::WorkItemPlanStore;
    use crate::product::workspace_engine::{
        AdvanceInput, AdvanceOutcome, WorkspaceEngine, WorkspaceSession,
    };
    use crate::web::test_controls::{PlanRepairFixtureControl, PlanRepairFixtureRuntime};

    pub(crate) const PROJECT_ID: &str = "project_0001";
    pub(crate) const ISSUE_ID: &str = "issue_plan_0001";
    pub(crate) const PLAN_ID: &str = "work_item_plan_0001";

    fn run_git(cwd: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .status()
            .expect("start git");
        assert!(status.success(), "git {} failed", args.join(" "));
    }

    fn plan_engine(paths: &ProductAppPaths, checkpoints_root: &std::path::Path) -> WorkspaceEngine {
        let lifecycle = LifecycleStore::new(paths.clone());
        let session_record = lifecycle
            .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .into_iter()
            .find(|session| {
                session.workspace_type == WorkspaceType::WorkItemPlan
                    && session.entity_id == PLAN_ID
            })
            .expect("plan workspace session");
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
        WorkspaceEngine::new_persistent(
            std::sync::Arc::new(CheckpointStore::new(checkpoints_root.join("checkpoints"))),
            lifecycle,
            event_tx,
            WorkspaceSession::from_record(session_record),
        )
    }

    pub(crate) struct N2Fixture {
        pub(crate) _root: tempfile::TempDir,
        pub(crate) paths: ProductAppPaths,
        pub(crate) api: crate::product::logical_codebase::LogicalRepositoryId,
        pub(crate) web: crate::product::logical_codebase::LogicalRepositoryId,
        pub(crate) revision_id: String,
        pub(crate) bindings: Vec<AdvanceTargetAttemptBinding>,
    }

    impl N2Fixture {
        pub(crate) fn coding_store(&self) -> CodingAttemptStore {
            CodingAttemptStore::new(self.paths.clone())
        }

        pub(crate) fn run_store(&self) -> CodingPlanRunStore {
            CodingPlanRunStore::new(self.paths.clone())
        }

        pub(crate) fn run_file(&self) -> PathBuf {
            self.paths
                .issue_root(PROJECT_ID, ISSUE_ID)
                .join("coding-plan-runs")
                .join(format!("{PLAN_ID}.json"))
        }

        pub(crate) fn api_attempt(&self) -> CodingExecutionAttempt {
            self.coding_store()
                .get_attempt_for_work_item_group(PROJECT_ID, ISSUE_ID, PLAN_ID, Some(self.api))
                .unwrap()
                .expect("api target attempt")
        }

        pub(crate) fn web_attempt(&self) -> CodingExecutionAttempt {
            self.coding_store()
                .get_attempt_for_work_item_group(PROJECT_ID, ISSUE_ID, PLAN_ID, Some(self.web))
                .unwrap()
                .expect("web target attempt")
        }

        pub(crate) fn core_unit(
            &self,
            attempt: &CodingExecutionAttempt,
        ) -> crate::product::coding_models::CodingExecutionUnit {
            self.coding_store()
                .list_coding_units(PROJECT_ID, ISSUE_ID, &attempt.id)
                .unwrap()
                .into_iter()
                .find(|unit| unit.logical_work_item_id == "wi_core")
                .expect("wi_core unit materialized in the api attempt")
        }

        pub(crate) fn core_selection(&self, command_id: &str) -> CodingPlanSelection {
            let attempt = self.api_attempt();
            let unit = self.core_unit(&attempt);
            CodingPlanSelection {
                unit_id: unit.id,
                logical_work_item_id: "wi_core".to_string(),
                attempt_id: attempt.id,
                target_repository_id: self.api.0.to_string(),
                start_command_id: CodingPlanSelection::deterministic_start_command_id(
                    PROJECT_ID,
                    ISSUE_ID,
                    PLAN_ID,
                    command_id,
                    &self.api.0.to_string(),
                ),
            }
        }
    }

    pub(crate) async fn n2_fixture() -> N2Fixture {
        let root = tempfile::tempdir().unwrap();
        PlanRepairFixtureRuntime::seed(root.path(), PlanRepairFixtureControl::default())
            .await
            .expect("seed plan repair fixture");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let coding_store = CodingAttemptStore::new(paths.clone());
        let seeded = coding_store
            .get_attempt_for_work_item_group(PROJECT_ID, ISSUE_ID, PLAN_ID, None)
            .unwrap()
            .expect("seeded legacy group attempt");
        coding_store
            .delete_attempt(PROJECT_ID, ISSUE_ID, &seeded.id)
            .unwrap();

        let mut targets = Vec::new();
        for name in ["n2-api", "n2-web"] {
            let canonical_path = root.path().join(name);
            std::fs::create_dir_all(&canonical_path).unwrap();
            run_git(&canonical_path, &["init", "--quiet"]);
            run_git(
                &canonical_path,
                &["config", "user.email", "n2@example.test"],
            );
            run_git(&canonical_path, &["config", "user.name", "N2 Plan Run"]);
            std::fs::write(canonical_path.join("README.md"), format!("# {name}\n")).unwrap();
            run_git(&canonical_path, &["add", "README.md"]);
            run_git(
                &canonical_path,
                &["commit", "--quiet", "-m", "initial commit"],
            );
            let repository = RepositoryStore::with_logical_codebase_feature(
                paths.clone(),
                LogicalCodebaseFeature::enabled(),
            )
            .create(CreateRepositoryInput {
                project_id: PROJECT_ID.to_string(),
                name: name.to_string(),
                path: canonical_path,
                default_policy_preset: None,
                default_provider_mode: None,
                idempotency_key: format!("n2-plan-run-{name}"),
            })
            .unwrap();
            targets.push(
                repository
                    .logical_repository_id
                    .expect("logical repository ID"),
            );
        }
        let [api, web] = [targets[0], targets[1]];
        let manifest = LogicalCodebaseStore::new(paths.clone())
            .load_manifest(PROJECT_ID)
            .unwrap()
            .expect("manifest");
        AggregatePolicyArtifactStore::new(paths.clone())
            .ensure_bootstrap(&manifest)
            .unwrap();
        IssueCodebaseSelectionStore::new(paths.clone())
            .save(&IssueCodebaseSelection::explicit(
                PROJECT_ID,
                ISSUE_ID,
                targets.clone(),
                Vec::new(),
                Vec::new(),
                None,
            ))
            .unwrap();

        let revision_store = WorkItemRevisionStore::new(paths.clone());
        let lineage = revision_store
            .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        let plan_store = WorkItemPlanStore::new(paths.clone());
        for (logical_id, revision_id, target) in [
            ("wi_core", "work_item_revision_wi_core_0001", api),
            (
                "wi_registration",
                "work_item_revision_wi_registration_0001",
                api,
            ),
            ("wi_unrelated", "work_item_revision_wi_unrelated_0001", web),
        ] {
            let revision = revision_store
                .get_work_item_revision(&lineage, logical_id, revision_id)
                .unwrap();
            plan_store
                .put_draft_record(&WorkItemDraftRecord {
                    project_id: PROJECT_ID.to_string(),
                    issue_id: ISSUE_ID.to_string(),
                    plan_id: PLAN_ID.to_string(),
                    draft_id: revision.source_draft_revision_id.clone(),
                    outline_id: format!("outline_{logical_id}"),
                    generation_round_id: "round_n2_plan_run".to_string(),
                    batch_id: None,
                    attempt_index: 1,
                    outline_version_ref: "outline_version_n2".to_string(),
                    generation_mode: WorkItemGenerationMode::Serial,
                    generation_diagnostics: None,
                    candidate: WorkItemDraftCandidate {
                        target_repository_id: Some(target),
                        outline_id: format!("outline_{logical_id}"),
                        logical_work_item_id: logical_id.to_string(),
                        canonical_contract_candidate: revision.canonical_contract,
                        verification_plan: WorkItemDraftVerificationPlan { checks: Vec::new() },
                    },
                    status: WorkItemDraftStatus::Accepted,
                    active: true,
                    superseded_by_draft_id: None,
                    supersede_reason: None,
                    copied_from_draft_id: None,
                    review_node_id: None,
                    review_verdict_ref: None,
                    generated_from_node_id: "n2_plan_run_fixture".to_string(),
                    accepted_at: Some("2026-10-13T00:00:00Z".to_string()),
                    superseded_at: None,
                    created_at: "2026-10-13T00:00:00Z".to_string(),
                    updated_at: "2026-10-13T00:00:00Z".to_string(),
                })
                .unwrap();
        }

        let revision_id = lineage
            .active_revision_id
            .clone()
            .expect("active plan revision");

        let mut engine = plan_engine(&paths, root.path());
        let outcome = engine
            .handle_advance(AdvanceInput {
                command_id: "cmd_n2_advance_seed".to_string(),
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                plan_id: PLAN_ID.to_string(),
            })
            .await
            .expect("manual advance reaches durable Ready");
        let AdvanceOutcome::Completed {
            target_attempts, ..
        } = outcome
        else {
            panic!("expected advance Completed, got {outcome:?}");
        };
        assert_eq!(target_attempts.len(), 2, "one attempt per logical target");

        N2Fixture {
            _root: root,
            paths,
            api,
            web,
            revision_id,
            bindings: target_attempts,
        }
    }
}
