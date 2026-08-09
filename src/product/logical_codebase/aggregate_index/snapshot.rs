use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::bounded_command_runner::{
    BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    TokioBoundedCommandRunner,
};
use crate::product::json_store::validate_relative_id;
use crate::product::logical_codebase::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalCodebaseManifest,
    LogicalCodebaseStore, MemberStatus, RepositoryCheckoutRecord,
};

use super::{AggregateIndexBudget, AggregateIndexError, AggregateIndexMemberSnapshot};

const GIT_TIMEOUT_SECS: u64 = AggregateIndexBudget::INCREMENTAL.fail_secs;
const GIT_OUTPUT_LIMIT_BYTES: usize = 1024 * 1024;

/// Collects immutable Git revision and working-tree evidence for aggregate members.
#[derive(Clone)]
pub struct AggregateIndexSnapshotCollector {
    logical: LogicalCodebaseStore,
    runner: Arc<dyn BoundedCommandRunner>,
}

impl AggregateIndexSnapshotCollector {
    pub fn new(runner: Arc<dyn BoundedCommandRunner>) -> Self {
        Self::with_dependencies(
            LogicalCodebaseStore::new(crate::product::app_paths::ProductAppPaths::new(".")),
            runner,
        )
    }

    pub fn with_dependencies(
        logical: LogicalCodebaseStore,
        runner: Arc<dyn BoundedCommandRunner>,
    ) -> Self {
        Self { logical, runner }
    }

    pub fn for_paths(paths: crate::product::app_paths::ProductAppPaths) -> Self {
        Self::with_dependencies(
            LogicalCodebaseStore::new(paths),
            Arc::new(TokioBoundedCommandRunner),
        )
    }

    pub fn capture_included(
        &self,
        project_id: &str,
        manifest: &LogicalCodebaseManifest,
    ) -> Result<Vec<AggregateIndexMemberSnapshot>, AggregateIndexError> {
        validate_relative_id(project_id)?;
        let members = self.logical.list_members(project_id)?;
        let checkouts = self.logical.list_checkouts(project_id)?;
        let members_by_id = members
            .iter()
            .map(|member| (member.logical_repository_id, member))
            .collect::<BTreeMap<_, _>>();
        let mut seen_members = BTreeSet::new();
        let mut snapshots = Vec::with_capacity(manifest.member_ids.len());

        for member_id in &manifest.member_ids {
            if !seen_members.insert(*member_id) {
                return Err(AggregateIndexError::Failed {
                    code: "aggregate_index_member_invalid",
                    message: format!("manifest repeats member {}", member_id.0),
                });
            }
            let member = members_by_id.get(member_id).copied().ok_or_else(|| {
                AggregateIndexError::Failed {
                    code: "aggregate_index_member_invalid",
                    message: format!("manifest member {} has no authority record", member_id.0),
                }
            })?;
            self.validate_active_member_main_checkout(member, *member_id, &checkouts)?;
            let checkout = unique_main_checkout(*member_id, &checkouts)?;
            snapshots.push(self.current(checkout)?);
        }

        Ok(snapshots)
    }

    pub fn current(
        &self,
        checkout: &RepositoryCheckoutRecord,
    ) -> Result<AggregateIndexMemberSnapshot, AggregateIndexError> {
        if checkout.kind != CheckoutKind::Main
            || checkout.availability != CheckoutAvailability::Available
        {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_checkout_unavailable",
                message: checkout.canonical_path.display().to_string(),
            });
        }

        let revision = self.git(&checkout.canonical_path, &["rev-parse", "HEAD"])?;
        let status = self.git(&checkout.canonical_path, &["status", "--porcelain=v1"])?;
        let revision = revision.trim().to_string();
        if revision.len() != 40 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_invalid_revision",
                message: revision,
            });
        }

        Ok(AggregateIndexMemberSnapshot::indexed(
            checkout.logical_repository_id,
            checkout.checkout_id,
            revision,
            !status.trim().is_empty(),
            Utc::now().to_rfc3339(),
        ))
    }

    fn validate_active_member_main_checkout(
        &self,
        member: &CodebaseMemberRecord,
        member_id: crate::product::logical_codebase::LogicalRepositoryId,
        checkouts: &[RepositoryCheckoutRecord],
    ) -> Result<(), AggregateIndexError> {
        if member.status != MemberStatus::Active {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_member_invalid",
                message: format!("manifest member {} is not active", member_id.0),
            });
        }
        let checkout = unique_main_checkout(member_id, checkouts)?;
        if !member.checkout_ids.contains(&checkout.checkout_id)
            || member.physical_repository_id != checkout.physical_repository_id
            || checkout.availability != CheckoutAvailability::Available
        {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_member_invalid",
                message: format!(
                    "main checkout {} is not an available checkout of member {}",
                    checkout.checkout_id.0, member_id.0
                ),
            });
        }
        Ok(())
    }

    fn git(&self, cwd: &Path, argv: &[&str]) -> Result<String, AggregateIndexError> {
        let request = BoundedCommandRequest {
            executable: "git".to_string(),
            argv: argv
                .iter()
                .map(|argument| (*argument).to_string())
                .collect(),
            working_dir: cwd.to_path_buf(),
            timeout: Duration::from_secs(GIT_TIMEOUT_SECS),
            cancellation: CancellationToken::new(),
            environment: sanitized_environment(),
            stdout_limit: GIT_OUTPUT_LIMIT_BYTES,
            stderr_limit: GIT_OUTPUT_LIMIT_BYTES,
        };
        let runner = self.runner.clone();
        let output = std::thread::spawn(move || -> Result<_, AggregateIndexError> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| AggregateIndexError::Failed {
                    code: "aggregate_index_git_runtime",
                    message: error.to_string(),
                })?;
            runtime
                .block_on(runner.run(request))
                .map_err(map_runner_error)
        })
        .join()
        .map_err(|_| AggregateIndexError::Failed {
            code: "aggregate_index_git_runner_panic",
            message: "bounded Git runner thread panicked".to_string(),
        })??;
        if output.timed_out {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_git_timeout",
                message: format!("git {} timed out after {GIT_TIMEOUT_SECS}s", argv.join(" ")),
            });
        }
        if output.cancelled {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_git_cancelled",
                message: format!("git {} was cancelled", argv.join(" ")),
            });
        }
        if output.exit_code != Some(0) {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_git_failed",
                message: command_failure_message(argv, &output),
            });
        }
        Ok(output.stdout)
    }
}

fn unique_main_checkout(
    member_id: crate::product::logical_codebase::LogicalRepositoryId,
    checkouts: &[RepositoryCheckoutRecord],
) -> Result<&RepositoryCheckoutRecord, AggregateIndexError> {
    let main_checkouts = checkouts
        .iter()
        .filter(|checkout| {
            checkout.logical_repository_id == member_id && checkout.kind == CheckoutKind::Main
        })
        .collect::<Vec<_>>();
    let [checkout] = main_checkouts.as_slice() else {
        return Err(AggregateIndexError::Failed {
            code: "aggregate_index_member_invalid",
            message: format!(
                "manifest member {} must have exactly one main checkout, found {}",
                member_id.0,
                main_checkouts.len()
            ),
        });
    };
    Ok(checkout)
}

fn sanitized_environment() -> BTreeMap<String, String> {
    std::env::var("PATH")
        .map(|path| BTreeMap::from([("PATH".to_string(), path)]))
        .unwrap_or_default()
}

fn map_runner_error(error: BoundedCommandError) -> AggregateIndexError {
    match error {
        BoundedCommandError::CommandMissing {
            executable,
            details,
        } => AggregateIndexError::Failed {
            code: "aggregate_index_git_missing",
            message: format!("{executable}: {details}"),
        },
        BoundedCommandError::Io { details } => AggregateIndexError::Failed {
            code: "aggregate_index_git_io",
            message: details,
        },
    }
}

fn command_failure_message(argv: &[&str], output: &BoundedCommandResult) -> String {
    let stderr = output.stderr.trim();
    let stdout = output.stdout.trim();
    let details = if !stderr.is_empty() { stderr } else { stdout };
    format!(
        "git {} exited with {:?}: {details}",
        argv.join(" "),
        output.exit_code
    )
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use uuid::Uuid;

    use super::*;
    use crate::cross_cutting::bounded_command_runner::{
        BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    };
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::{
        LogicalRepositoryId, RepositoryCheckoutId, RepositorySourceIdentity, RepositoryType,
    };

    #[test]
    fn snapshot_reads_head_and_dirty_from_each_main_checkout_not_aggregate_root() {
        let runner = RecordingGitRunner::with_output(
            "api",
            "0123456789012345678901234567890123456789\n",
            " M src/A.rs\n",
        );
        let collector = AggregateIndexSnapshotCollector::new(Arc::new(runner.clone()));
        let snapshot = collector.current(&main_checkout("api")).unwrap();

        assert_eq!(
            snapshot.revision,
            "0123456789012345678901234567890123456789"
        );
        assert!(snapshot.dirty);
        assert_eq!(
            runner.cwd_calls(),
            vec![PathBuf::from("api"), PathBuf::from("api")]
        );
    }

    #[test]
    fn capture_included_keeps_manifest_order_and_reads_each_active_main_checkout() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let logical = LogicalCodebaseStore::new(paths);
        let first = member_with_main_checkout("first");
        let second = member_with_main_checkout("second");
        let manifest = LogicalCodebaseManifest::new(
            "project_0001",
            temp.path().join("aggregate"),
            vec![
                second.0.logical_repository_id,
                first.0.logical_repository_id,
            ],
        );
        logical.save_manifest("project_0001", &manifest).unwrap();
        for (member, checkout) in [&first, &second] {
            logical.save_member("project_0001", member).unwrap();
            logical.save_checkout("project_0001", checkout).unwrap();
        }
        let runner = RecordingGitRunner::with_outputs([
            result("b".repeat(40)),
            result(String::new()),
            result("a".repeat(40)),
            result(" M src/A.rs\n".to_string()),
        ]);
        let collector =
            AggregateIndexSnapshotCollector::with_dependencies(logical, Arc::new(runner.clone()));

        let snapshots = collector
            .capture_included("project_0001", &manifest)
            .unwrap();

        assert_eq!(
            snapshots
                .iter()
                .map(|snapshot| snapshot.logical_repository_id)
                .collect::<Vec<_>>(),
            manifest.member_ids
        );
        assert_eq!(
            runner.cwd_calls(),
            vec![
                PathBuf::from("second"),
                PathBuf::from("second"),
                PathBuf::from("first"),
                PathBuf::from("first"),
            ]
        );
        assert!(!snapshots[0].dirty);
        assert!(snapshots[1].dirty);
    }

    #[test]
    fn capture_included_rejects_missing_or_ambiguous_main_checkouts_before_git() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let logical = LogicalCodebaseStore::new(paths);
        let (member, checkout) = member_with_main_checkout("api");
        let manifest = LogicalCodebaseManifest::new(
            "project_0001",
            temp.path().join("aggregate"),
            vec![member.logical_repository_id],
        );
        logical.save_manifest("project_0001", &manifest).unwrap();
        logical.save_member("project_0001", &member).unwrap();
        let runner = RecordingGitRunner::default();
        let collector = AggregateIndexSnapshotCollector::with_dependencies(
            logical.clone(),
            Arc::new(runner.clone()),
        );

        assert!(matches!(
            collector.capture_included("project_0001", &manifest),
            Err(AggregateIndexError::Failed { code, .. }) if code == "aggregate_index_member_invalid"
        ));
        assert!(runner.cwd_calls().is_empty());

        logical.save_checkout("project_0001", &checkout).unwrap();
        let mut duplicate = checkout.clone();
        duplicate.checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        logical.save_checkout("project_0001", &duplicate).unwrap();
        assert!(matches!(
            collector.capture_included("project_0001", &manifest),
            Err(AggregateIndexError::Failed { code, .. }) if code == "aggregate_index_member_invalid"
        ));
        assert!(runner.cwd_calls().is_empty());
    }

    #[test]
    fn current_rejects_non_main_or_unavailable_checkout_before_git() {
        let runner = RecordingGitRunner::default();
        let collector = AggregateIndexSnapshotCollector::new(Arc::new(runner.clone()));
        let mut checkout = main_checkout("api");
        checkout.kind = CheckoutKind::IssueWorktree;

        assert!(matches!(
            collector.current(&checkout),
            Err(AggregateIndexError::Failed { code, .. }) if code == "aggregate_index_checkout_unavailable"
        ));
        checkout.kind = CheckoutKind::Main;
        checkout.availability = CheckoutAvailability::Missing;
        assert!(matches!(
            collector.current(&checkout),
            Err(AggregateIndexError::Failed { code, .. }) if code == "aggregate_index_checkout_unavailable"
        ));
        assert!(runner.cwd_calls().is_empty());
    }

    #[test]
    fn current_rejects_invalid_head_and_git_failure() {
        let invalid = RecordingGitRunner::with_outputs([
            result("not-a-sha\n".to_string()),
            result(String::new()),
        ]);
        let collector = AggregateIndexSnapshotCollector::new(Arc::new(invalid));
        assert!(matches!(
            collector.current(&main_checkout("api")),
            Err(AggregateIndexError::Failed { code, .. }) if code == "aggregate_index_invalid_revision"
        ));

        let failing = RecordingGitRunner::with_outputs([BoundedCommandResult {
            exit_code: Some(128),
            stdout: String::new(),
            stderr: "not a git repository".to_string(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 1,
        }]);
        let collector = AggregateIndexSnapshotCollector::new(Arc::new(failing));
        assert!(matches!(
            collector.current(&main_checkout("api")),
            Err(AggregateIndexError::Failed { code, .. }) if code == "aggregate_index_git_failed"
        ));
    }

    fn member_with_main_checkout(name: &str) -> (CodebaseMemberRecord, RepositoryCheckoutRecord) {
        let checkout = main_checkout(name);
        let now = "2026-08-09T00:00:00Z".to_string();
        let member = CodebaseMemberRecord {
            logical_repository_id: checkout.logical_repository_id,
            physical_repository_id: checkout.physical_repository_id.clone(),
            alias: name.to_string(),
            role: "repository".to_string(),
            ordinal: 1,
            source_identity: RepositorySourceIdentity {
                scheme: "test".to_string(),
                key_digest: format!("sha256:source-{name}"),
                canonical_git_dir: PathBuf::from(name).join(".git"),
                canonical_origin: None,
                first_seen_path_hash: format!("sha256:path-{name}"),
            },
            repo_type: RepositoryType::Unknown,
            tech_stack: Vec::new(),
            owner: None,
            tags: Vec::new(),
            default_ref: None,
            checkout_ids: vec![checkout.checkout_id],
            status: MemberStatus::Active,
            created_at: now.clone(),
            updated_at: now,
        };
        (member, checkout)
    }

    fn main_checkout(name: &str) -> RepositoryCheckoutRecord {
        let now = "2026-08-09T00:00:00Z".to_string();
        RepositoryCheckoutRecord {
            checkout_id: RepositoryCheckoutId(Uuid::new_v4()),
            logical_repository_id: LogicalRepositoryId(Uuid::new_v4()),
            physical_repository_id: format!("repository_{name}"),
            kind: CheckoutKind::Main,
            canonical_path: PathBuf::from(name),
            checkout_path_hash: format!("sha256:checkout-{name}"),
            git_dir_identity: format!("sha256:git-{name}"),
            revision: None,
            availability: CheckoutAvailability::Available,
            observed_at: now.clone(),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    fn result(stdout: String) -> BoundedCommandResult {
        BoundedCommandResult {
            exit_code: Some(0),
            stdout,
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 1,
        }
    }

    #[derive(Clone, Default)]
    struct RecordingGitRunner {
        state: Arc<Mutex<RecordingGitState>>,
    }

    #[derive(Default)]
    struct RecordingGitState {
        outputs: VecDeque<BoundedCommandResult>,
        cwd_calls: Vec<PathBuf>,
    }

    impl RecordingGitRunner {
        fn with_output(_cwd: &str, head: &str, status: &str) -> Self {
            Self::with_outputs([result(head.to_string()), result(status.to_string())])
        }

        fn with_outputs(outputs: impl IntoIterator<Item = BoundedCommandResult>) -> Self {
            Self {
                state: Arc::new(Mutex::new(RecordingGitState {
                    outputs: outputs.into_iter().collect(),
                    cwd_calls: Vec::new(),
                })),
            }
        }

        fn cwd_calls(&self) -> Vec<PathBuf> {
            self.state.lock().unwrap().cwd_calls.clone()
        }
    }

    #[async_trait::async_trait]
    impl BoundedCommandRunner for RecordingGitRunner {
        async fn run(
            &self,
            request: BoundedCommandRequest,
        ) -> Result<BoundedCommandResult, BoundedCommandError> {
            assert_eq!(request.executable, "git");
            if !matches!(
                request.argv.as_slice(),
                [rev_parse, head] if rev_parse == "rev-parse" && head == "HEAD"
            ) && !matches!(
                request.argv.as_slice(),
                [status, porcelain] if status == "status" && porcelain == "--porcelain=v1"
            ) {
                panic!("unexpected Git argv: {:?}", request.argv);
            }
            let mut state = self.state.lock().unwrap();
            state.cwd_calls.push(request.working_dir);
            Ok(state
                .outputs
                .pop_front()
                .unwrap_or_else(|| panic!("missing scripted Git output")))
        }
    }
}
