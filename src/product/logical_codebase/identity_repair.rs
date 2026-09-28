//! C4 Task 7：Failed identity journal 的独立诊断与安全 repair。
//!
//! repair 入口绕过普通成员列表和成功身份解析，直接读取 journal、
//! repos.json（legacy 事实）、identity registry 与 authority metadata；
//! 不删除 journal、不直接编辑权威 JSON、不自动吞并 mapping 冲突、
//! 不在用户确认前切 read authority。安全 prefix（source digest 一致且
//! 无冲突）也必须由用户显式 POST 确认；确认后的推进复用既有
//! `IdentityMigrationExecutor::ensure_*` 原链，repair 事实以 audit 条目
//! 追加在原 journal 上（`#[serde(default)]`，旧 JSON 兼容）。

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::logical_codebase::migration::IdentityMigrationExecutor;
use crate::product::logical_codebase::{
    IdentityMigrationJournal, IdentityMigrationJournalStore, IdentityMigrationPhase,
    IdentityRegistryEntry, IdentityRegistryState, IdentityRegistryStore,
    IdentityRepairAuditEntry, RepositoryIdentityMapping,
};

/// Failed journal 的只读诊断（GET 投影的唯一来源）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IdentityJournalDiagnostic {
    pub project_id: String,
    pub logical_codebase_id: String,
    pub migration_id: String,
    pub phase: IdentityMigrationPhase,
    pub source_repos_digest: String,
    pub observed_source_repos_digest: Option<String>,
    pub read_mode: Option<String>,
    pub completed_keys: Vec<String>,
    pub mappings: Vec<RepositoryIdentityMapping>,
    pub candidates: Vec<RepositoryIdentityMapping>,
    pub conflicts: Vec<String>,
    pub impact: Vec<String>,
    pub allowed_actions: Vec<IdentityRepairActionKind>,
    /// C4 Task 10：journal 当前 `updated_at`——显式 repair 动作 POST 的
    /// `expected_journal_updated_at` 唯一权威来源（GET 只读补读同一值）。
    pub journal_updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityRepairActionKind {
    ContinueSafePrefix,
    SubmitMapping,
    Revalidate,
}

/// 用户提交的 mapping 裁决（必须与 registry 候选完全一致，不可捏造）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IdentityMappingSubmission {
    pub legacy_repository_id: String,
    pub source_identity_digest: String,
    pub logical_repository_id: crate::product::logical_codebase::LogicalRepositoryId,
    pub primary_checkout_id: crate::product::logical_codebase::RepositoryCheckoutId,
    pub physical_repository_id: String,
    /// 为空时由服务端按 canonical 规则计算；非空时必须与 canonical 一致。
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IdentityRepairActionRequest {
    pub command_id: String,
    pub project_id: String,
    pub logical_codebase_id: String,
    pub expected_journal_updated_at: String,
    pub action: IdentityRepairActionKind,
    pub mapping: Option<IdentityMappingSubmission>,
}

/// 独立于普通成员列表的 repair 服务。
pub struct IdentityRepairService {
    paths: ProductAppPaths,
}

impl IdentityRepairService {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    /// 只读诊断：不调用 `ensure_identity_schema`，不触碰普通
    /// member/identity 解析路径，零写入。
    pub fn diagnostic(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
    ) -> Result<IdentityJournalDiagnostic, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(logical_codebase_id)?;
        let journal = self.load_journal(project_id)?;
        let executor = IdentityMigrationExecutor::new(self.paths.clone());
        let (repositories, observed_digest) = executor.read_repair_sources(project_id)?;
        let registry = IdentityRegistryStore::new(self.paths.clone());

        // 候选：每个 observed 源仓在 registry 中的既有条目（Active 或
        // Tombstoned——tombstone 即需用户裁决的 mapping 候选）。
        let mut candidates = Vec::new();
        let mut conflicts = Vec::new();
        for repository in &repositories {
            let source = crate::product::logical_codebase::migration::repository_source_identity(repository)?;
            if let Some(entry) = registry.find_by_source(project_id, &source)? {
                if entry.state == IdentityRegistryState::Tombstoned {
                    conflicts.push(format!(
                        "registry_source_tombstoned:{}",
                        entry.physical_repository_id
                    ));
                }
                candidates.push(candidate_mapping(project_id, repository.id.clone(), &entry));
            }
        }
        if observed_digest != journal.source_repos_digest {
            conflicts.push("source_repos_digest_drift".to_string());
        }
        if crate::product::logical_codebase::migration::duplicate_repository_id(&repositories)
            .is_some()
        {
            conflicts.push("duplicate_legacy_repository".to_string());
        }
        if crate::product::logical_codebase::migration::duplicate_mapping_legacy_id(
            &journal.mappings,
        )
        .is_some()
        {
            conflicts.push("duplicate_mapping_legacy_id".to_string());
        }
        for mapping in &journal.mappings {
            let repository = repositories
                .iter()
                .find(|repository| repository.id == mapping.legacy_repository_id);
            match repository {
                None => conflicts.push(format!(
                    "mapping_repository_missing:{}",
                    mapping.legacy_repository_id
                )),
                Some(repository) => {
                    let source = crate::product::logical_codebase::migration::repository_source_identity(repository)?;
                    if source.key_digest != mapping.source_identity_digest {
                        conflicts.push(format!(
                            "mapping_source_identity_mismatch:{}",
                            mapping.legacy_repository_id
                        ));
                    }
                }
            }
        }

        let safe_prefix_eligible =
            journal.phase == IdentityMigrationPhase::Failed && conflicts.is_empty();
        let allowed_actions = if journal.phase == IdentityMigrationPhase::Failed {
            let mut actions = Vec::new();
            if safe_prefix_eligible {
                actions.push(IdentityRepairActionKind::ContinueSafePrefix);
            }
            if !candidates.is_empty() {
                actions.push(IdentityRepairActionKind::SubmitMapping);
            }
            actions.push(IdentityRepairActionKind::Revalidate);
            actions
        } else {
            Vec::new()
        };

        let impact = impact_scope(&journal);

        Ok(IdentityJournalDiagnostic {
            project_id: project_id.to_string(),
            logical_codebase_id: logical_codebase_id.to_string(),
            migration_id: journal.migration_id.clone(),
            phase: journal.phase.clone(),
            source_repos_digest: journal.source_repos_digest.clone(),
            observed_source_repos_digest: Some(observed_digest),
            read_mode: journal.read_mode.clone(),
            completed_keys: journal.completed_keys.clone(),
            mappings: journal.mappings.clone(),
            candidates,
            conflicts,
            impact,
            allowed_actions,
            journal_updated_at: journal.updated_at.clone(),
        })
    }

    /// 显式 repair 动作：同 command 先查 audit（幂等重放）；确认语义后
    /// 才落 audit 并复用原迁移链推进。被拒绝的动作零写入。
    pub fn apply(
        &self,
        request: IdentityRepairActionRequest,
    ) -> Result<IdentityJournalDiagnostic, ProductStoreError> {
        validate_relative_id(&request.project_id)?;
        validate_relative_id(&request.logical_codebase_id)?;
        if request.command_id.trim().is_empty() {
            return Err(ProductStoreError::Conflict {
                kind: "identity_repair_invalid_command",
                id: request.command_id,
            });
        }
        let journal = self.load_journal(&request.project_id)?;
        // 同 command 重放：返回同一诊断，不重复推进/不重复 backfill。
        if journal
            .repair_audit
            .iter()
            .any(|entry| entry.command_id == request.command_id)
        {
            return self.diagnostic(&request.project_id, &request.logical_codebase_id);
        }
        if journal.updated_at != request.expected_journal_updated_at {
            return Err(ProductStoreError::Conflict {
                kind: "identity_repair_stale_journal",
                id: format!(
                    "expected:{}:observed:{}",
                    request.expected_journal_updated_at, journal.updated_at
                ),
            });
        }

        let executor = IdentityMigrationExecutor::new(self.paths.clone());
        match request.action {
            IdentityRepairActionKind::ContinueSafePrefix => {
                let diagnostic =
                    self.diagnostic(&request.project_id, &request.logical_codebase_id)?;
                if diagnostic.phase != IdentityMigrationPhase::Failed {
                    return Err(ProductStoreError::Conflict {
                        kind: "identity_repair_not_failed",
                        id: diagnostic.migration_id,
                    });
                }
                if !diagnostic.conflicts.is_empty() {
                    return Err(ProductStoreError::Conflict {
                        kind: "identity_repair_conflict",
                        id: diagnostic.conflicts.join(";"),
                    });
                }
                let previous_error = journal
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "identity migration failed".to_string());
                // 用户已确认安全 prefix：追加 audit（保留原失败原因）、拨回
                // Scanning，并复用原链推进到 authority 写（不切读）。
                let audit = IdentityRepairAuditEntry {
                    command_id: request.command_id.clone(),
                    action: "continue_safe_prefix".to_string(),
                    outcome: "accepted".to_string(),
                    detail: previous_error,
                    applied_at: chrono_now(),
                };
                executor.repair_from_failed(
                    &request.project_id,
                    audit,
                    RepairContinuation::ThroughAuthority,
                )?;
            }
            IdentityRepairActionKind::SubmitMapping => {
                let Some(submission) = request.mapping.as_ref() else {
                    return Err(ProductStoreError::Conflict {
                        kind: "identity_repair_mapping_required",
                        id: request.command_id,
                    });
                };
                if journal.phase != IdentityMigrationPhase::Failed {
                    return Err(ProductStoreError::Conflict {
                        kind: "identity_repair_not_failed",
                        id: journal.migration_id,
                    });
                }
                // 核验（失败零写入）：digest/physical/候选一致、幂等键
                // canonical；tombstone 候选在用户确认后复活（原链操作）。
                let mapping = executor.repair_validate_mapping(
                    &request.project_id,
                    submission,
                )?;
                let audit = IdentityRepairAuditEntry {
                    command_id: request.command_id.clone(),
                    action: "submit_mapping".to_string(),
                    outcome: "staged".to_string(),
                    detail: format!(
                        "mapping staged for {} -> {}",
                        mapping.legacy_repository_id, mapping.physical_repository_id
                    ),
                    applied_at: chrono_now(),
                };
                executor.repair_stage_mapping(&request.project_id, mapping, audit)?;
            }
            IdentityRepairActionKind::Revalidate => {
                let diagnostic =
                    self.diagnostic(&request.project_id, &request.logical_codebase_id)?;
                if diagnostic.phase != IdentityMigrationPhase::Failed {
                    // 非 Failed 状态：journal 无需 repair，原链自身幂等。
                    executor.ensure_identity_schema(&request.project_id)?;
                } else if diagnostic.conflicts.iter().all(|conflict| {
                    conflict.starts_with("registry_source_tombstoned")
                }) && !diagnostic.mappings.is_empty()
                {
                    // mapping 已 staged 且无未决冲突：显式继续原链（含核验
                    // 后的读切换）。
                    let audit = IdentityRepairAuditEntry {
                        command_id: request.command_id.clone(),
                        action: "revalidate".to_string(),
                        outcome: "accepted".to_string(),
                        detail: "continue original migration chain after repair".to_string(),
                        applied_at: chrono_now(),
                    };
                    executor.repair_from_failed(
                        &request.project_id,
                        audit,
                        RepairContinuation::FullSchema,
                    )?;
                } else {
                    return Err(ProductStoreError::Conflict {
                        kind: "identity_repair_conflict",
                        id: diagnostic.conflicts.join(";"),
                    });
                }
            }
        }
        self.diagnostic(&request.project_id, &request.logical_codebase_id)
    }

    fn load_journal(&self, project_id: &str) -> Result<IdentityMigrationJournal, ProductStoreError> {
        IdentityMigrationJournalStore::new(self.paths.clone())
            .load(project_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "identity_migration_journal",
                id: project_id.to_string(),
            })
    }
}

/// Revalidate/ContinueSafePrefix 后的原链推进范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairContinuation {
    /// 推进到 authority 写完成（不切读）。
    ThroughAuthority,
    /// 完整原链（backfill → dual read → 核验后切读）。
    FullSchema,
}

fn chrono_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn candidate_mapping(
    project_id: &str,
    legacy_repository_id: String,
    entry: &IdentityRegistryEntry,
) -> RepositoryIdentityMapping {
    RepositoryIdentityMapping {
        idempotency_key: crate::product::logical_codebase::migration::mapping_idempotency_key(
            project_id,
            &legacy_repository_id,
            &entry.source_identity.key_digest,
        ),
        legacy_repository_id,
        source_identity_digest: entry.source_identity.key_digest.clone(),
        logical_repository_id: entry.logical_repository_id,
        primary_checkout_id: entry.primary_checkout_id,
        physical_repository_id: entry.physical_repository_id.clone(),
        authority_written: false,
        compatibility_backfilled: false,
    }
}

fn impact_scope(journal: &IdentityMigrationJournal) -> Vec<String> {
    let mut impact = vec![format!("read_mode:{}", journal.read_mode.clone().unwrap_or_default())];
    for mapping in &journal.mappings {
        impact.push(format!("authority_member:{}", mapping.logical_repository_id.0));
        impact.push(format!("checkout:{}", mapping.primary_checkout_id.0));
    }
    impact
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::json_store::write_json;
    use crate::product::logical_codebase::LogicalCodebaseStore;
    use crate::product::models::RepositoryRecord;
    use crate::product::project_store::{CreateProjectInput, ProjectStore};
    use std::path::PathBuf;
    use std::sync::Arc;

    fn git(repository: &std::path::Path, arguments: &[&str]) {
        let output = std::process::Command::new("git")
            .args(arguments)
            .current_dir(repository)
            .output()
            .expect("git starts");
        assert!(
            output.status.success(),
            "git {arguments:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    struct RepairFixture {
        _root: tempfile::TempDir,
        paths: ProductAppPaths,
        repository_path: PathBuf,
    }

    fn legacy_record(id: &str, path: &std::path::Path) -> RepositoryRecord {
        RepositoryRecord {
            id: id.to_string(),
            project_id: "project_0001".to_string(),
            name: id.to_string(),
            path: path.to_path_buf(),
            repo_hash: format!("legacy-hash-{id}"),
            runtime_root: PathBuf::from("/unused/.aria/runtime"),
            default_policy_preset: "manual-write".to_string(),
            default_provider_mode: "fake".to_string(),
            created_at: "2026-09-29T00:00:00Z".to_string(),
            updated_at: "2026-09-29T00:00:00Z".to_string(),
            logical_repository_id: None,
            primary_checkout_id: None,
            identity_schema_version: 0,
        }
    }

    fn repair_fixture() -> RepairFixture {
        let root = tempfile::tempdir().expect("root");
        let repository_path = root.path().join("repository");
        std::fs::create_dir_all(&repository_path).unwrap();
        git(&repository_path, &["init", "-q", "-b", "main"]);
        git(&repository_path, &["config", "user.email", "repair@test.local"]);
        git(&repository_path, &["config", "user.name", "Repair Test"]);
        std::fs::write(repository_path.join("README.md"), "# member\n").unwrap();
        git(&repository_path, &["add", "."]);
        git(&repository_path, &["commit", "-q", "-m", "init"]);
        git(
            &repository_path,
            &["remote", "add", "origin", "ssh://git@example.test/acme/api.git"],
        );

        let paths = ProductAppPaths::new(root.path());
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "repair-project".to_string(),
                description: None,
            })
            .unwrap();
        write_json(
            &paths.project_root("project_0001").join("repos.json"),
            &vec![legacy_record("repository_0001", &repository_path)],
        )
        .unwrap();
        RepairFixture {
            _root: root,
            paths,
            repository_path,
        }
    }

    fn mark_journal_failed(paths: &ProductAppPaths) -> IdentityMigrationJournal {
        let store = IdentityMigrationJournalStore::new(paths.clone());
        let mut journal = store.load("project_0001").unwrap().expect("journal");
        journal.phase = IdentityMigrationPhase::Failed;
        journal.last_error =
            Some("identity mismatch: identity_registry repository_0001".to_string());
        store.save("project_0001", &journal).unwrap();
        journal
    }

    fn journal(paths: &ProductAppPaths) -> IdentityMigrationJournal {
        IdentityMigrationJournalStore::new(paths.clone())
            .load("project_0001")
            .unwrap()
            .expect("journal")
    }

    fn lc_id() -> String {
        crate::product::logical_codebase::store::legacy_logical_codebase_id("project_0001")
    }

    fn aria_inventory(root: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        let mut inventory = std::collections::BTreeMap::new();
        let mut stack = vec![root.join(".aria")];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let relative = path.strip_prefix(root).unwrap().to_path_buf();
                    inventory.insert(
                        relative.to_string_lossy().into_owned(),
                        std::fs::read(&path).unwrap_or_default(),
                    );
                }
            }
        }
        inventory
    }

    #[test]
    fn failed_journal_diagnostic_does_not_require_normal_member_list() {
        let fixture = repair_fixture();
        let executor = IdentityMigrationExecutor::new(fixture.paths.clone());
        executor
            .ensure_through_authority("project_0001")
            .expect("seed authority facts first");
        mark_journal_failed(&fixture.paths);

        // 普通 ensure 链在 Failed journal 上直接拒绝——repair 诊断不依赖它。
        assert!(executor.ensure_identity_schema("project_0001").is_err());

        let before = aria_inventory(fixture._root.path());
        let diagnostic = IdentityRepairService::new(fixture.paths.clone())
            .diagnostic("project_0001", &lc_id())
            .expect("diagnostic must not require the normal member list");
        let after = aria_inventory(fixture._root.path());
        assert_eq!(before, after, "diagnostic must be read-only");

        assert_eq!(diagnostic.phase, IdentityMigrationPhase::Failed);
        assert!(!diagnostic.source_repos_digest.is_empty());
        assert_eq!(
            diagnostic.observed_source_repos_digest.as_deref(),
            Some(diagnostic.source_repos_digest.as_str())
        );
        assert_eq!(diagnostic.completed_keys, journal(&fixture.paths).completed_keys);
        assert_eq!(diagnostic.mappings.len(), 1);
        assert!(!diagnostic.candidates.is_empty());
        assert!(diagnostic.conflicts.is_empty());
        assert!(!diagnostic.impact.is_empty());
        assert!(diagnostic.allowed_actions.contains(&IdentityRepairActionKind::ContinueSafePrefix));
    }

    #[test]
    fn safe_prefix_requires_user_confirmation_and_replay_is_idempotent() {
        let fixture = repair_fixture();
        let executor = IdentityMigrationExecutor::new(fixture.paths.clone());
        executor
            .ensure_through_authority("project_0001")
            .expect("seed authority facts first");
        mark_journal_failed(&fixture.paths);

        let service = IdentityRepairService::new(fixture.paths.clone());
        let lc = lc_id();
        // 未 POST 前：GET 只读，journal 字节不变，安全 prefix 仅展示。
        let before = journal(&fixture.paths);
        let inventory_before = aria_inventory(fixture._root.path());
        let diagnostic = service.diagnostic("project_0001", &lc).unwrap();
        assert_eq!(before, journal(&fixture.paths));
        assert!(diagnostic.allowed_actions.contains(&IdentityRepairActionKind::ContinueSafePrefix));

        // 过期 expected_journal_updated_at：拒绝且零写入。
        let rejected = service.apply(IdentityRepairActionRequest {
            command_id: "cmd-safe-prefix-stale".to_string(),
            project_id: "project_0001".to_string(),
            logical_codebase_id: lc.clone(),
            expected_journal_updated_at: "2000-01-01T00:00:00Z".to_string(),
            action: IdentityRepairActionKind::ContinueSafePrefix,
            mapping: None,
        });
        assert!(rejected.is_err());
        assert_eq!(before, journal(&fixture.paths));

        // 用户确认：audit 追加（保留原失败原因）、原链推进到 authority。
        let result = service
            .apply(IdentityRepairActionRequest {
                command_id: "cmd-safe-prefix-1".to_string(),
                project_id: "project_0001".to_string(),
                logical_codebase_id: lc.clone(),
                expected_journal_updated_at: before.updated_at.clone(),
                action: IdentityRepairActionKind::ContinueSafePrefix,
                mapping: None,
            })
            .expect("confirmed safe prefix continues the original chain");
        assert_ne!(result.phase, IdentityMigrationPhase::Failed);
        let after = journal(&fixture.paths);
        assert_eq!(after.repair_audit.len(), 1);
        assert_eq!(after.repair_audit[0].command_id, "cmd-safe-prefix-1");
        assert!(
            after.repair_audit[0]
                .detail
                .contains("identity mismatch: identity_registry repository_0001"),
            "original failure reason must be preserved in the audit"
        );
        // 原 journal 文件保留（同一条 durable 记录演进，不删档）。
        let members = LogicalCodebaseStore::new(fixture.paths.clone())
            .list_members("project_0001")
            .unwrap_or_default();
        assert_eq!(members.len(), 1, "no duplicate backfill after repair");

        // 同 command 重放：同一诊断结果，不重复推进/不重复 audit。
        let replay = service
            .apply(IdentityRepairActionRequest {
                command_id: "cmd-safe-prefix-1".to_string(),
                project_id: "project_0001".to_string(),
                logical_codebase_id: lc.clone(),
                expected_journal_updated_at: after.updated_at.clone(),
                action: IdentityRepairActionKind::ContinueSafePrefix,
                mapping: None,
            })
            .expect("replay must be idempotent");
        assert_eq!(replay.migration_id, result.migration_id);
        assert_eq!(replay.phase, result.phase);
        assert_eq!(journal(&fixture.paths).repair_audit.len(), 1);
        assert_eq!(
            LogicalCodebaseStore::new(fixture.paths.clone())
                .list_members("project_0001")
                .unwrap_or_default()
                .len(),
            1
        );
        let _ = inventory_before;
    }

    #[test]
    fn digest_drift_or_mapping_conflict_never_switches_reads() {
        // 场景 A：source digest 漂移。
        let fixture = repair_fixture();
        let executor = IdentityMigrationExecutor::new(fixture.paths.clone());
        executor
            .ensure_through_authority("project_0001")
            .expect("seed authority facts first");
        mark_journal_failed(&fixture.paths);
        write_json(
            &fixture.paths.project_root("project_0001").join("repos.json"),
            &vec![
                legacy_record("repository_0001", &fixture.repository_path),
                legacy_record(
                    "repository_0002",
                    &fixture.repository_path.join("..").join("repository"),
                ),
            ],
        )
        .unwrap();

        let service = IdentityRepairService::new(fixture.paths.clone());
        let lc = lc_id();
        let diagnostic = service.diagnostic("project_0001", &lc).unwrap();
        assert!(diagnostic.conflicts.iter().any(|c| c == "source_repos_digest_drift"));
        assert!(
            !diagnostic
                .allowed_actions
                .contains(&IdentityRepairActionKind::ContinueSafePrefix)
        );
        let drift_rejected = service.apply(IdentityRepairActionRequest {
            command_id: "cmd-drift-1".to_string(),
            project_id: "project_0001".to_string(),
            logical_codebase_id: lc.clone(),
            expected_journal_updated_at: journal(&fixture.paths).updated_at.clone(),
            action: IdentityRepairActionKind::ContinueSafePrefix,
            mapping: None,
        });
        assert!(drift_rejected.is_err());
        let drifted = journal(&fixture.paths);
        assert_eq!(drifted.phase, IdentityMigrationPhase::Failed);
        assert_eq!(drifted.read_mode, None);

        // 场景 B：tombstone 候选 + 多候选 mapping —— 未确认前不切读。
        let fixture_b = repair_fixture();
        let executor_b = IdentityMigrationExecutor::new(fixture_b.paths.clone());
        // 先 seed registry active 条目再 tombstone，制造需要用户裁决的候选。
        executor_b
            .ensure_through_authority("project_0001")
            .expect("seed authority facts first");
        let registry = IdentityRegistryStore::new(fixture_b.paths.clone());
        let source = crate::product::logical_codebase::migration::repository_source_identity(
            &legacy_record("repository_0001", &fixture_b.repository_path),
        )
        .unwrap();
        registry
            .tombstone("project_0001", &source, "delete_op_0001", "2026-09-29T01:00:00Z")
            .unwrap();
        // 重置 journal 为未映射状态并触发原链失败。
        let store = IdentityMigrationJournalStore::new(fixture_b.paths.clone());
        let mut fresh = IdentityMigrationJournal::new("project_0001", "");
        fresh.source_repos_digest = store
            .load("project_0001")
            .unwrap()
            .unwrap()
            .source_repos_digest;
        store.save("project_0001", &fresh).unwrap();
        assert!(executor_b.ensure_through_authority("project_0001").is_err());
        let failed = journal(&fixture_b.paths);
        assert_eq!(failed.phase, IdentityMigrationPhase::Failed);
        assert!(failed.mappings.is_empty());

        let service_b = IdentityRepairService::new(fixture_b.paths.clone());
        let lc_b = lc_id();
        let diagnostic = service_b.diagnostic("project_0001", &lc_b).unwrap();
        assert!(
            diagnostic
                .conflicts
                .iter()
                .any(|c| c.starts_with("registry_source_tombstoned")),
            "tombstone candidate must be visible: {:?}",
            diagnostic.conflicts
        );
        assert!(!diagnostic
            .allowed_actions
            .contains(&IdentityRepairActionKind::ContinueSafePrefix));
        assert!(diagnostic
            .allowed_actions
            .contains(&IdentityRepairActionKind::SubmitMapping));
        let candidate = diagnostic.candidates[0].clone();

        // 提交错误 mapping（digest 不匹配）：仍拒绝，registry/journal 不变。
        let wrong = service_b.apply(IdentityRepairActionRequest {
            command_id: "cmd-wrong-mapping-1".to_string(),
            project_id: "project_0001".to_string(),
            logical_codebase_id: lc_b.clone(),
            expected_journal_updated_at: journal(&fixture_b.paths).updated_at.clone(),
            action: IdentityRepairActionKind::SubmitMapping,
            mapping: Some(IdentityMappingSubmission {
                legacy_repository_id: candidate.legacy_repository_id.clone(),
                source_identity_digest: "sha256:forged".to_string(),
                logical_repository_id: candidate.logical_repository_id,
                primary_checkout_id: candidate.primary_checkout_id,
                physical_repository_id: candidate.physical_repository_id.clone(),
                idempotency_key: candidate.idempotency_key.clone(),
            }),
        });
        assert!(wrong.is_err(), "forged digest must be rejected");
        assert!(
            registry
                .find_by_source("project_0001", &source)
                .unwrap()
                .map(|entry| entry.state == IdentityRegistryState::Tombstoned)
                .unwrap_or(false),
            "read authority must not switch before confirmation"
        );
        assert!(journal(&fixture_b.paths).mappings.is_empty());

        // 提交完整匹配 mapping：staging + tombstone 复活（原链操作）。
        let staged = service_b
            .apply(IdentityRepairActionRequest {
                command_id: "cmd-mapping-1".to_string(),
                project_id: "project_0001".to_string(),
                logical_codebase_id: lc_b.clone(),
                expected_journal_updated_at: journal(&fixture_b.paths).updated_at.clone(),
                action: IdentityRepairActionKind::SubmitMapping,
                mapping: Some(IdentityMappingSubmission {
                    legacy_repository_id: candidate.legacy_repository_id.clone(),
                    source_identity_digest: candidate.source_identity_digest.clone(),
                    logical_repository_id: candidate.logical_repository_id,
                    primary_checkout_id: candidate.primary_checkout_id,
                    physical_repository_id: candidate.physical_repository_id.clone(),
                    idempotency_key: candidate.idempotency_key.clone(),
                }),
            })
            .expect("fully matching mapping stages");
        assert_eq!(staged.phase, IdentityMigrationPhase::Failed, "staging must not advance the chain");
        assert_eq!(journal(&fixture_b.paths).mappings.len(), 1);
        assert_eq!(journal(&fixture_b.paths).repair_audit.len(), 1);

        // Revalidate：核验通过后原链继续（含读切换），journal 保留 + audit 追加。
        let revalidated = service_b
            .apply(IdentityRepairActionRequest {
                command_id: "cmd-revalidate-1".to_string(),
                project_id: "project_0001".to_string(),
                logical_codebase_id: lc_b.clone(),
                expected_journal_updated_at: journal(&fixture_b.paths).updated_at.clone(),
                action: IdentityRepairActionKind::Revalidate,
                mapping: None,
            })
            .expect("revalidate continues the original chain after confirmation");
        assert_eq!(
            revalidated.read_mode.as_deref(),
            Some("logical_authoritative"),
            "read authority switches only after successful revalidation"
        );
        assert_ne!(revalidated.phase, IdentityMigrationPhase::Failed);
        let final_journal = journal(&fixture_b.paths);
        assert_eq!(final_journal.repair_audit.len(), 2);
        assert_eq!(
            LogicalCodebaseStore::new(fixture_b.paths.clone())
                .list_members("project_0001")
                .unwrap_or_default()
                .len(),
            1,
            "exactly one authority member after repair"
        );
        let _ = Arc::new(0usize);
    }
}
