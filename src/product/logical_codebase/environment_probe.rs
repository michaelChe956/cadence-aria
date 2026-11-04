//! 聚合冷启动的环境前置探测。
//!
//! 三个环境事实都是「在任何确定性步骤开始之前就该确定下来」的：
//!
//! - **codegraph 可用性与版本**（`AggregateEnvironmentProbe`）——建索引与
//!   provider turn 都依赖它；冷启动无 LKG 时必须致命（REQ-BOOT-09）。
//! - **聚合根可写**（[`ensure_aggregate_root_writable`]）——`preflight` 之后
//!   每一步都要往聚合根写文件。
//! - **Cadence-skills 四条命令的链接层完整**（[`ensure_skills_commands_linked`]）
//!   ——`pre-check` / `rule-config` / `mcp-configuration` /
//!   `project-rules-examples` 必须能在 shared / codex / claude 三层解析到。
//!
//! 失败一律以
//! [`AggregateInitializationError::EnvironmentPrecondition`] 返回：`code` 即
//! 最终 `reason_code`，operation 以 `step: None` 失败，**不把任何 step 标
//! Failed**，因此用户只看到一次可行动的报错，也不会触发索引构建与
//! provider turn。

use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::cross_cutting::bounded_command_runner::BoundedCommandRunner;
use crate::product::cadence_skills::CadenceSkillsPaths;
use crate::product::logical_codebase::AggregateInitializationError;
use crate::product::logical_codebase::aggregate_index::{AggregateIndexError, CodeGraphCli};

/// 四条 Cadence-skills 斜杠命令对应的 skill 目录名。`pre-check` /
/// `rule-config` / `mcp-configuration` / `project-rules-examples` 必须能在
/// shared / codex / claude 三层解析到，否则聚合根的 provider turn 必然解析
/// 不到命令。
pub const AGGREGATE_SKILL_COMMANDS: [&str; 4] = [
    "pre-check",
    "rule-config",
    "mcp-configuration",
    "project-rules-examples",
];

/// 聚合冷启动的 codegraph 前置探测。
///
/// 实现必须在**不触碰 `aggregate_index/**` 语义**的前提下复用同一份 CLI
/// 配置：生产实现只是把 `CodeGraphCli` 包一层，runner 与 executable 名与
/// 索引操作完全一致（`CodeGraphCli` 自身不克隆实例、不加 `Clone`，D-16）。
pub trait AggregateEnvironmentProbe: Send + Sync {
    /// `Ok(version)` = CLI 可用且版本落在兼容区间；`Err` 一律是
    /// `EnvironmentPrecondition { code: "codegraph_missing" |
    /// "codegraph_version_mismatch" | <CLI 稳定码>, .. }`。
    fn ensure_codegraph(&self) -> Result<String, AggregateInitializationError>;
}

/// 生产探针：包一层与索引操作同配置的 [`CodeGraphCli`]。
#[derive(Clone)]
pub struct CodeGraphEnvironmentProbe {
    cli: CodeGraphCli,
}

impl CodeGraphEnvironmentProbe {
    pub fn new(runner: Arc<dyn BoundedCommandRunner>, executable: String) -> Self {
        Self {
            cli: CodeGraphCli::new(runner, executable),
        }
    }
}

impl AggregateEnvironmentProbe for CodeGraphEnvironmentProbe {
    fn ensure_codegraph(&self) -> Result<String, AggregateInitializationError> {
        self.cli.verify_version().map_err(environment_precondition)
    }
}

/// 把 `CodeGraphCli` 的稳定错误码原样提升为环境前置码（不压成通用失败）。
fn environment_precondition(error: AggregateIndexError) -> AggregateInitializationError {
    let (code, message) = match error {
        AggregateIndexError::Degraded { code, message } => (code, message),
        AggregateIndexError::Failed { code, message } => (code, message),
    };
    AggregateInitializationError::EnvironmentPrecondition {
        code: code.to_string(),
        reason: message,
        retryable: !matches!(code, "codegraph_version_mismatch" | "codegraph_cancelled"),
    }
}

static WRITE_PROBE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 在 canonical 聚合根下建删一个 `.aria-probe-<nonce>` 文件，证明该根真的
/// 可写。任何一步失败（含删除失败）都报 `aggregate_root_not_writable`，且
/// 尽力清理残留。
pub fn ensure_aggregate_root_writable(
    canonical_root: &Path,
) -> Result<(), AggregateInitializationError> {
    let nonce = WRITE_PROBE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let probe = canonical_root.join(format!(".aria-probe-{}-{nonce}", std::process::id()));
    let write_attempt = std::fs::File::create(&probe).and_then(|mut file| {
        use std::io::Write;
        file.write_all(b"aria aggregate root write probe")?;
        file.sync_all()
    });
    let failure = match write_attempt {
        Ok(()) => std::fs::remove_file(&probe).err(),
        Err(error) => {
            let _ = std::fs::remove_file(&probe);
            Some(error)
        }
    };
    match failure {
        None => Ok(()),
        Some(error) => Err(AggregateInitializationError::EnvironmentPrecondition {
            code: "aggregate_root_not_writable".to_string(),
            reason: format!(
                "aggregate root {} is not writable: {error}",
                canonical_root.display()
            ),
            retryable: true,
        }),
    }
}

/// 校验四条 Cadence-skills 命令在 shared / codex / claude 三层的
/// `SKILL.md` 都可解析。`machine_skills` 是确定性代码、不跑 provider，因此
/// 这里的失败**绝不能**归因为「provider 未登录」。
pub fn ensure_skills_commands_linked(home: &Path) -> Result<(), AggregateInitializationError> {
    let paths = CadenceSkillsPaths::from_home(home);
    for (layer, root) in [
        ("shared", paths.shared_skills_root()),
        ("codex", paths.codex_skills_root()),
        ("claude", paths.claude_skills_root()),
    ] {
        for skill in AGGREGATE_SKILL_COMMANDS {
            let manifest = root.join(skill).join("SKILL.md");
            if !manifest.is_file() {
                return Err(AggregateInitializationError::EnvironmentPrecondition {
                    code: "skills_commands_missing".to_string(),
                    reason: format!(
                        "Cadence skills link layer {layer} is missing the {skill} command (expected {}); rerun the aggregate initialization so machine_skills can relink the three layers",
                        manifest.display()
                    ),
                    retryable: true,
                });
            }
        }
    }
    Ok(())
}

/// 测试替身：只回放预先给定的探测结果，不做任何 I/O。
#[derive(Debug, Clone)]
pub struct StubEnvironmentProbe {
    outcome: StubOutcome,
}

#[derive(Debug, Clone)]
enum StubOutcome {
    Available(String),
    Failure { code: &'static str, reason: String },
}

impl StubEnvironmentProbe {
    /// 探测通过，返回给定版本号。
    pub fn available(version: impl Into<String>) -> Self {
        Self {
            outcome: StubOutcome::Available(version.into()),
        }
    }

    /// 模拟 `codegraph` 不在 PATH。
    pub fn codegraph_missing() -> Self {
        Self {
            outcome: StubOutcome::Failure {
                code: "codegraph_missing",
                reason: "codegraph: command not found".to_string(),
            },
        }
    }

    /// 模拟版本越界（`expected >=1.6.0, <2.0.0`）。
    pub fn version_mismatch(actual: impl Into<String>) -> Self {
        Self {
            outcome: StubOutcome::Failure {
                code: "codegraph_version_mismatch",
                reason: format!("expected >=1.6.0, <2.0.0, got {}", actual.into()),
            },
        }
    }
}

impl AggregateEnvironmentProbe for StubEnvironmentProbe {
    fn ensure_codegraph(&self) -> Result<String, AggregateInitializationError> {
        match &self.outcome {
            StubOutcome::Available(version) => Ok(version.clone()),
            StubOutcome::Failure { code, reason } => {
                Err(AggregateInitializationError::EnvironmentPrecondition {
                    code: (*code).to_string(),
                    reason: reason.clone(),
                    retryable: *code != "codegraph_version_mismatch",
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::aggregate_initialization::{
        AggregateInitializationOperation, AggregateInitializationOperationInput,
        AggregateInitializationOperationStatus, AggregateInitializationStepStatus,
    };
    use crate::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore;
    use crate::product::logical_codebase::store::{LogicalCodebaseManifest, LogicalCodebaseStore};
    use crate::product::logical_codebase::{
        AggregateInitializationCoordinator, AggregateInitializationError,
        AggregatePreflightService, AggregatePreflightSnapshot, AggregateProviderTurnDriver,
        AggregateProviderTurnRequest, AggregateSkillsPreparation, MachineSkillsPreparation,
    };
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;

    const CREATED_AT: &str = "2026-11-03T00:00:00Z";

    struct NoopSkills {
        calls: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl AggregateSkillsPreparation for NoopSkills {
        async fn prepare_skills(
            &self,
            _project_id: &str,
            _operation_id: &str,
            _cancellation: CancellationToken,
        ) -> Result<MachineSkillsPreparation, AggregateInitializationError> {
            self.calls
                .lock()
                .unwrap()
                .push("machine_skills".to_string());
            Ok(MachineSkillsPreparation {
                source_digest: "sha256:source".to_string(),
                link_digest: "sha256:link".to_string(),
                skills_root: PathBuf::from("/skills"),
                warnings: Vec::new(),
            })
        }
    }

    struct NoopPreflight {
        aggregate_root: String,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl AggregatePreflightService for NoopPreflight {
        fn inspect(
            &self,
            _project_id: &str,
            _manifest: &LogicalCodebaseManifest,
            _cancellation: &CancellationToken,
        ) -> Result<AggregatePreflightSnapshot, AggregateInitializationError> {
            self.calls
                .lock()
                .unwrap()
                .push("aggregate_preflight".to_string());
            Ok(AggregatePreflightSnapshot {
                aggregate_root: self.aggregate_root.clone(),
                index_excludes_assets: true,
                members: Vec::new(),
                manifest_revision: 1,
                manifest_digest: "sha256:manifest".to_string(),
                ..Default::default()
            })
        }
    }

    struct NoopProvider {
        calls: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl AggregateProviderTurnDriver for NoopProvider {
        async fn run_turn(
            &self,
            request: AggregateProviderTurnRequest<'_>,
        ) -> Result<String, AggregateInitializationError> {
            self.calls
                .lock()
                .unwrap()
                .push(request.step.as_str().to_string());
            Ok(format!("{} summary", request.step.as_str()))
        }
    }

    struct ProbeFixture {
        _temp: tempfile::TempDir,
        paths: ProductAppPaths,
        coordinator: AggregateInitializationCoordinator,
        skills_calls: Arc<Mutex<Vec<String>>>,
        preflight_calls: Arc<Mutex<Vec<String>>>,
        provider_calls: Arc<Mutex<Vec<String>>>,
    }

    impl ProbeFixture {
        fn new(probe: Arc<dyn AggregateEnvironmentProbe>) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let paths = ProductAppPaths::new(temp.path().join(".aria"));
            let store = AggregateInitializationOperationStore::new(paths.clone());
            let skills_calls = Arc::new(Mutex::new(Vec::new()));
            let preflight_calls = Arc::new(Mutex::new(Vec::new()));
            let provider_calls = Arc::new(Mutex::new(Vec::new()));
            let mut manifest = LogicalCodebaseManifest::new(
                "project_0001",
                temp.path().join("aggregate-root"),
                Vec::new(),
            );
            manifest.created_at = CREATED_AT.to_string();
            manifest.updated_at = CREATED_AT.to_string();
            LogicalCodebaseStore::new(paths.clone())
                .save_manifest("project_0001", &manifest)
                .unwrap();
            let skills: Arc<dyn AggregateSkillsPreparation> = Arc::new(NoopSkills {
                calls: skills_calls.clone(),
            });
            let preflight: Arc<dyn AggregatePreflightService> = Arc::new(NoopPreflight {
                aggregate_root: manifest
                    .provider_context_root
                    .to_string_lossy()
                    .into_owned(),
                calls: preflight_calls.clone(),
            });
            let provider: Arc<dyn AggregateProviderTurnDriver> = Arc::new(NoopProvider {
                calls: provider_calls.clone(),
            });
            let clock: Arc<dyn Fn() -> String + Send + Sync> = Arc::new(|| CREATED_AT.to_string());
            let coordinator = AggregateInitializationCoordinator::new(
                paths.clone(),
                store,
                skills,
                preflight,
                provider,
                clock,
            )
            .with_environment_probe(probe);
            coordinator
                .begin(
                    "aggregate_initialization_0001".to_string(),
                    "project_0001",
                    AggregateInitializationOperationInput {
                        idempotency_key: "0001".to_string(),
                        manifest_revision: manifest.membership_revision,
                        policy_digest: "sha256:policy".to_string(),
                        profile_evidence_digest: Some("sha256:profile".to_string()),
                        provider_context_root: manifest.provider_context_root.clone(),
                        provider: "claude_code".to_string(),
                    },
                )
                .unwrap();
            Self {
                _temp: temp,
                paths,
                coordinator,
                skills_calls,
                preflight_calls,
                provider_calls,
            }
        }

        async fn execute(
            &self,
        ) -> Result<AggregateInitializationOperation, AggregateInitializationError> {
            self.coordinator
                .execute(
                    "project_0001",
                    "aggregate_initialization_0001",
                    CancellationToken::new(),
                )
                .await
        }

        fn operation(&self) -> AggregateInitializationOperation {
            self.coordinator
                .get("project_0001", "aggregate_initialization_0001")
                .unwrap()
        }

        fn preflight_json_path(&self) -> PathBuf {
            self.paths
                .aggregate_initializations_root("")
                .join("aggregate_initialization_0001")
                .join("preflight.json")
        }

        fn downstream_calls(&self) -> usize {
            self.skills_calls.lock().unwrap().len()
                + self.preflight_calls.lock().unwrap().len()
                + self.provider_calls.lock().unwrap().len()
        }
    }

    fn assert_all_steps_pending(operation: &AggregateInitializationOperation) {
        for step in &operation.steps {
            assert_eq!(
                step.status,
                AggregateInitializationStepStatus::Pending,
                "step {} must stay pending when an environment precondition fails",
                step.step_id.as_str()
            );
        }
    }

    /// #1：codegraph 缺失 → operation Failed、`reason_code=codegraph_missing`、
    /// 全部 step 仍 Pending，且下游（machine_skills / preflight / provider
    /// turn）一次都没触发。
    #[tokio::test]
    async fn codegraph_missing_fails_operation_before_any_step_starts() {
        let fixture = ProbeFixture::new(Arc::new(StubEnvironmentProbe::codegraph_missing()));
        let error = fixture.execute().await.expect_err("codegraph 缺失必须失败");
        assert!(
            matches!(
                &error,
                AggregateInitializationError::EnvironmentPrecondition { code, .. }
                    if code == "codegraph_missing"
            ),
            "expected EnvironmentPrecondition codegraph_missing, got {error:?}"
        );

        let operation = fixture.operation();
        assert_eq!(
            operation.status,
            AggregateInitializationOperationStatus::Failed
        );
        assert_eq!(
            operation
                .error
                .as_ref()
                .map(|record| record.reason_code.as_str()),
            Some("codegraph_missing")
        );
        assert_eq!(operation.failed_step, None);
        assert_all_steps_pending(&operation);
        assert_eq!(
            fixture.downstream_calls(),
            0,
            "codegraph 探测失败时不得开始任何确定性步骤或 provider turn"
        );
    }

    /// #2：版本越界 → `codegraph_version_mismatch`，同样全部 step Pending。
    #[tokio::test]
    async fn codegraph_version_mismatch_is_reported_without_starting_steps() {
        let fixture = ProbeFixture::new(Arc::new(StubEnvironmentProbe::version_mismatch("2.0.1")));
        let error = fixture.execute().await.expect_err("版本越界必须失败");
        assert!(
            matches!(
                &error,
                AggregateInitializationError::EnvironmentPrecondition { code, .. }
                    if code == "codegraph_version_mismatch"
            ),
            "expected codegraph_version_mismatch, got {error:?}"
        );

        let operation = fixture.operation();
        assert_eq!(
            operation.status,
            AggregateInitializationOperationStatus::Failed
        );
        assert_eq!(
            operation
                .error
                .as_ref()
                .map(|record| record.reason_code.as_str()),
            Some("codegraph_version_mismatch")
        );
        assert_eq!(operation.failed_step, None);
        assert_all_steps_pending(&operation);
        assert_eq!(fixture.downstream_calls(), 0);
    }

    /// #3：探测通过 → 五步正常推进，且 `preflight.json` 记录
    /// `codegraph_available=true` + 实测版本号（REQ-BOOT-09 的 durable 证据）。
    #[tokio::test]
    async fn available_codegraph_is_recorded_in_durable_preflight_snapshot() {
        let fixture = ProbeFixture::new(Arc::new(StubEnvironmentProbe::available("1.6.0")));
        let operation = fixture.execute().await.expect("探测通过时应跑完五步");
        assert_eq!(
            operation.status,
            AggregateInitializationOperationStatus::Completed
        );
        assert_eq!(
            operation
                .steps
                .iter()
                .map(|step| step.status)
                .collect::<Vec<_>>(),
            vec![
                AggregateInitializationStepStatus::Completed,
                AggregateInitializationStepStatus::Completed,
                AggregateInitializationStepStatus::Completed,
                AggregateInitializationStepStatus::Completed,
                AggregateInitializationStepStatus::Completed,
            ]
        );

        let snapshot: AggregatePreflightSnapshot =
            crate::product::json_store::read_json(&fixture.preflight_json_path())
                .expect("preflight.json 必须存在且可读");
        assert!(snapshot.codegraph_available);
        assert_eq!(snapshot.codegraph_version.as_deref(), Some("1.6.0"));
    }

    /// 未装配探针时不瞎报版本：`codegraph_available=false`、`version=None`。
    #[tokio::test]
    async fn absent_probe_records_codegraph_unavailable_without_inventing_a_version() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let store = AggregateInitializationOperationStore::new(paths.clone());
        let mut manifest = LogicalCodebaseManifest::new(
            "project_0001",
            temp.path().join("aggregate-root"),
            Vec::new(),
        );
        manifest.created_at = CREATED_AT.to_string();
        manifest.updated_at = CREATED_AT.to_string();
        LogicalCodebaseStore::new(paths.clone())
            .save_manifest("project_0001", &manifest)
            .unwrap();
        let coordinator = AggregateInitializationCoordinator::new(
            paths.clone(),
            store,
            Arc::new(NoopSkills {
                calls: Arc::new(Mutex::new(Vec::new())),
            }),
            Arc::new(NoopPreflight {
                aggregate_root: manifest
                    .provider_context_root
                    .to_string_lossy()
                    .into_owned(),
                calls: Arc::new(Mutex::new(Vec::new())),
            }),
            Arc::new(NoopProvider {
                calls: Arc::new(Mutex::new(Vec::new())),
            }),
            Arc::new(|| CREATED_AT.to_string()),
        );
        coordinator
            .begin(
                "aggregate_initialization_0001".to_string(),
                "project_0001",
                AggregateInitializationOperationInput {
                    idempotency_key: "0001".to_string(),
                    manifest_revision: manifest.membership_revision,
                    policy_digest: "sha256:policy".to_string(),
                    profile_evidence_digest: Some("sha256:profile".to_string()),
                    provider_context_root: manifest.provider_context_root.clone(),
                    provider: "claude_code".to_string(),
                },
            )
            .unwrap();
        coordinator
            .execute(
                "project_0001",
                "aggregate_initialization_0001",
                CancellationToken::new(),
            )
            .await
            .unwrap();

        let snapshot: AggregatePreflightSnapshot = crate::product::json_store::read_json(
            &paths
                .aggregate_initializations_root("")
                .join("aggregate_initialization_0001")
                .join("preflight.json"),
        )
        .unwrap();
        assert!(!snapshot.codegraph_available);
        assert_eq!(snapshot.codegraph_version, None);
    }

    /// 可写探测：可写目录通过且不留残留；只读目录 → `aggregate_root_not_writable`。
    #[cfg(unix)]
    #[test]
    fn writable_probe_passes_on_writable_root_and_leaves_no_residue() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let writable = temp.path().join("writable");
        std::fs::create_dir_all(&writable).unwrap();
        ensure_aggregate_root_writable(&writable).unwrap();
        let residue = std::fs::read_dir(&writable)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert!(residue.is_empty(), "可写探测不得留下残留文件: {residue:?}");

        let readonly = temp.path().join("readonly");
        std::fs::create_dir_all(&readonly).unwrap();
        std::fs::set_permissions(&readonly, std::fs::Permissions::from_mode(0o555)).unwrap();
        let error = ensure_aggregate_root_writable(&readonly).expect_err("只读聚合根必须被拒绝");
        std::fs::set_permissions(&readonly, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            matches!(
                &error,
                AggregateInitializationError::EnvironmentPrecondition { code, .. }
                    if code == "aggregate_root_not_writable"
            ),
            "expected aggregate_root_not_writable, got {error:?}"
        );
    }

    /// skills 链接层：三层 × 四条命令的 SKILL.md 齐全才通过；缺任一条 →
    /// `skills_commands_missing`（而不是被误归因为 provider 未登录 /
    /// machine_skills 失败）。
    #[test]
    fn skills_link_layer_requires_all_three_layers_of_four_commands() {
        let home = tempfile::tempdir().unwrap();
        assert!(
            ensure_skills_commands_linked(home.path()).is_err(),
            "空 home 没有任何链接层，必须报缺失"
        );

        let paths = crate::product::cadence_skills::CadenceSkillsPaths::from_home(home.path());
        for root in [
            paths.shared_skills_root(),
            paths.codex_skills_root(),
            paths.claude_skills_root(),
        ] {
            for skill in [
                "pre-check",
                "rule-config",
                "mcp-configuration",
                "project-rules-examples",
            ] {
                let dir = root.join(skill);
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(dir.join("SKILL.md"), "# skill").unwrap();
            }
        }
        ensure_skills_commands_linked(home.path()).unwrap();

        let missing = paths
            .claude_skills_root()
            .join("pre-check")
            .join("SKILL.md");
        std::fs::remove_file(&missing).unwrap();
        let error = ensure_skills_commands_linked(home.path()).expect_err("缺链接必须被拒绝");
        assert!(
            matches!(
                &error,
                AggregateInitializationError::EnvironmentPrecondition { code, reason, .. }
                    if code == "skills_commands_missing" && reason.contains("pre-check")
            ),
            "expected skills_commands_missing naming pre-check, got {error:?}"
        );
    }

    /// 生产探针把 `CodeGraphCli` 的错误码原样带出（`codegraph_missing` /
    /// `codegraph_version_mismatch`），不压成通用失败。
    #[test]
    fn environment_precondition_keeps_codegraph_error_codes() {
        let missing = environment_precondition(
            crate::product::logical_codebase::aggregate_index::AggregateIndexError::Degraded {
                code: "codegraph_missing",
                message: "codegraph: not found".to_string(),
            },
        );
        assert!(matches!(
            &missing,
            AggregateInitializationError::EnvironmentPrecondition { code, .. }
                if code == "codegraph_missing"
        ));
        let mismatch = environment_precondition(
            crate::product::logical_codebase::aggregate_index::AggregateIndexError::Degraded {
                code: "codegraph_version_mismatch",
                message: "got 2.0.1".to_string(),
            },
        );
        assert!(matches!(
            &mismatch,
            AggregateInitializationError::EnvironmentPrecondition { code, retryable, .. }
                if code == "codegraph_version_mismatch" && !retryable
        ));
    }
}
