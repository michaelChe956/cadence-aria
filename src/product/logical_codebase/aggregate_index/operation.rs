use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use serde_json::Value;
use uuid::Uuid;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::with_exact_exclusive_lock;
use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::logical_codebase::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalCodebaseManifest,
    LogicalCodebaseStore, MemberStatus, RepositoryCheckoutRecord,
};

use super::codegraph_coverage::{
    AggregateCoverageProbe, CliFallbackCoverageProbe, CodeGraphCoverage, CoverageOutcome,
    summarize_coverage,
};
use super::{
    AggregateIndexError, AggregateIndexRecord, AggregateIndexSnapshotCollector,
    AggregateIndexStatus, AggregateIndexStore, CodeGraphCli, CodeGraphExcludeGenerator,
};

const EXCLUDED_QUERIES: [&str; 4] = [
    "SHOULD_NOT_INDEX_NONMEMBER",
    "SHOULD_NOT_INDEX_WORKTREE",
    "SHOULD_NOT_INDEX_ARIA",
    "SHOULD_NOT_INDEX_BUILD",
];

/// Evidence produced only after every CodeGraph scope assertion succeeds.
///
/// G4（终局关闸缺口）：激活语义＝成员覆盖＋负向排除；不再要求任何
/// 硬编码代表符号（canonical fixture 的 `crossRepoGreeting` 语料）
/// 命中≥2 成员——真实业务仓无该符号时 planning_ready 仍可达。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregateIndexAcceptance {
    /// 每成员覆盖判定（SDK `hasFilesUnder` / CLI `starts_with` 归一为布尔；
    /// 未持久化，仅验收证据）。
    pub member_covered: BTreeMap<String, bool>,
    pub excluded_queries: BTreeMap<String, Vec<PathBuf>>,
    /// 健康闸/CLI 回退的审计 warning（不阻断激活）。
    pub warning: Option<String>,
}

impl AggregateIndexAcceptance {
    fn verify(
        cli: &CodeGraphCli,
        coverage: &dyn AggregateCoverageProbe,
        root: &Path,
        member_names: &[String],
    ) -> Result<Self, AggregateIndexError> {
        let sdk_warning: Option<String>;
        // 成员覆盖优先走 codegraph SDK（hasFilesUnder 判定，载荷几百字节；
        // CLI files 的全量清单在 3.1 GB 级索引上 ~230 MB，必然截断）。
        let mut fallback_trigger: Option<&'static str> = None;
        let member_covered_from_sdk = match coverage.coverage(root, member_names) {
            Ok(outcome) => {
                // 健康闸：stale/pending/state/fileCount 异常只记 warning，
                // 不因健康问题阻断激活（保持既有语义）。
                sdk_warning = index_health_warning(&outcome);
                Some(outcome.member_covered)
            }
            // 回退保底：SDK 路径解析不到 / SDK 不兼容 ⇒ 走既有 CLI files
            // 验收（小仓与旧行为不回归），并留下可审计的回退痕迹。
            Err(error) => {
                let Some(trigger) = coverage_fallback_trigger(&error) else {
                    return Err(error);
                };
                let (fallback_code, fallback_message) = match &error {
                    AggregateIndexError::Failed { code, message } => (*code, message.clone()),
                    other => (error_code(other), other.to_string()),
                };
                // 一条 warn：回退原因 + 回退到哪条路径（客户机据此判断
                // 「为什么走了慢路径」）。
                tracing::warn!(
                    trigger,
                    fallback = "cli_files",
                    error_code = fallback_code,
                    sdk_message = %fallback_message,
                    "aggregate coverage: SDK unavailable, falling back to CLI files"
                );
                fallback_trigger = Some(trigger);
                sdk_warning = Some(format!(
                    "codegraph SDK 覆盖验收不可用（{fallback_message}），已回退 CLI files 验收路径"
                ));
                None
            }
        };
        let (member_covered, warning) = match member_covered_from_sdk {
            Some(covered) => (covered, sdk_warning),
            None => {
                let files = match cli.files(root) {
                    Ok(files) => files,
                    // 空索引＝每个成员都没有任何可索引文件：把上游人读提示
                    // 归并为业务语义的覆盖不足错误并点名全部成员，而不是让
                    // `expected value at line 1 column 1` 这类 JSON 解析天书透出。
                    Err(AggregateIndexError::Failed {
                        code: "codegraph_files_empty_index",
                        ..
                    }) => return Err(member_coverage_failed(member_names)),
                    Err(error) => {
                        // 一条 warn：截断（载荷不完整）——实际字节不可得
                        // （错误只带上限），影响范围在 fail_building 里按
                        // 模式补 `lkg_degraded`。
                        if error_code(&error) == "codegraph_files_output_truncated" {
                            tracing::warn!(
                                error_code = "codegraph_files_output_truncated",
                                path = "cli_files",
                                actual_bytes = "unknown",
                                message = %error,
                                "codegraph files output truncated: file list incomplete"
                            );
                        }
                        return Err(error);
                    }
                };
                let covered = verify_member_coverage(&files, member_names)?;
                let summary = summarize_coverage(&covered);
                // 一条 info：CLI 回退路径的覆盖判定（与 SDK 路径同一组字段）。
                tracing::info!(
                    path = "cli_files",
                    fallback_trigger = %fallback_trigger.unwrap_or("unknown"),
                    member_count = summary.member_count,
                    covered_count = summary.covered_count,
                    uncovered_members = %summary.uncovered.join(","),
                    file_count = files.len(),
                    "codegraph aggregate coverage decision"
                );
                (covered, sdk_warning)
            }
        };
        require_member_coverage(&member_covered, member_names)?;
        // 排除性查询载荷小，保持既有 CLI query_json 路径不动。
        let mut excluded_queries = BTreeMap::new();
        for query in EXCLUDED_QUERIES {
            let result = cli.query_json(root, query)?;
            let offending_paths = result_paths(&result);
            if !is_empty_query_result(&result) {
                return Err(exclusion_failed(query, offending_paths));
            }
            excluded_queries.insert(query.to_string(), offending_paths);
        }

        Ok(Self {
            member_covered,
            excluded_queries,
            warning,
        })
    }

    fn soft_warning(&self) -> Option<String> {
        self.warning.clone()
    }
}

/// Orchestrates immutable configuration publication, CodeGraph initialization,
/// scope verification, and durable active-record publication.
pub struct AggregateIndexOperation {
    logical: LogicalCodebaseStore,
    store: AggregateIndexStore,
    excludes: CodeGraphExcludeGenerator,
    cli: CodeGraphCli,
    snapshots: AggregateIndexSnapshotCollector,
    /// 成员覆盖探针：生产=new() 装配 SDK 探针；DI 构造器默认 CLI 回退
    /// 探针（保持既有 CLI 验收行为，测试可注入桩）。
    coverage: Arc<dyn AggregateCoverageProbe>,
}

impl AggregateIndexOperation {
    pub fn new(
        paths: ProductAppPaths,
        cli: CodeGraphCli,
        excludes: CodeGraphExcludeGenerator,
    ) -> Self {
        Self::with_snapshot_dependencies(
            LogicalCodebaseStore::new(paths.clone()),
            AggregateIndexStore::new(paths.clone()),
            cli,
            excludes,
            AggregateIndexSnapshotCollector::for_paths(paths),
        )
        .with_coverage_probe(Arc::new(CodeGraphCoverage::production()))
    }

    pub fn with_dependencies(
        logical: LogicalCodebaseStore,
        store: AggregateIndexStore,
        cli: CodeGraphCli,
        excludes: CodeGraphExcludeGenerator,
    ) -> Self {
        Self::with_snapshot_dependencies(
            logical.clone(),
            store,
            cli,
            excludes,
            AggregateIndexSnapshotCollector::with_dependencies(
                logical,
                std::sync::Arc::new(
                    crate::cross_cutting::bounded_command_runner::TokioBoundedCommandRunner,
                ),
            ),
        )
    }

    pub fn with_snapshot_dependencies(
        logical: LogicalCodebaseStore,
        store: AggregateIndexStore,
        cli: CodeGraphCli,
        excludes: CodeGraphExcludeGenerator,
        snapshots: AggregateIndexSnapshotCollector,
    ) -> Self {
        Self {
            logical,
            store,
            excludes,
            cli,
            snapshots,
            coverage: Arc::new(CliFallbackCoverageProbe),
        }
    }

    /// 覆盖探针注入 seam（测试桩 / for_lc 探针继承）。
    pub fn with_coverage_probe(mut self, coverage: Arc<dyn AggregateCoverageProbe>) -> Self {
        self.coverage = coverage;
        self
    }

    /// Re-scopes manifest/member reads, index generations and the single-writer
    /// lock to one logical codebase subtree, reusing the same CLI and snapshot
    /// command runner (D5 semantics unchanged).
    pub fn for_lc(&self, lc_id: impl Into<String>) -> Self {
        let lc_id = lc_id.into();
        let paths = self.logical.paths().clone();
        Self::with_snapshot_dependencies(
            LogicalCodebaseStore::for_lc(paths.clone(), lc_id.clone()),
            AggregateIndexStore::for_lc(paths.clone(), lc_id.clone()),
            self.cli.clone(),
            self.excludes,
            self.snapshots.for_lc(lc_id),
        )
        .with_coverage_probe(self.coverage.clone())
    }

    /// Clones the logical-codebase store handle for sibling services such as
    /// the freshness service that need independent read access.
    pub fn logical_clone(&self) -> LogicalCodebaseStore {
        self.logical.clone()
    }

    /// Clones the aggregate-index store handle.
    pub fn store_clone(&self) -> AggregateIndexStore {
        self.store.clone()
    }

    /// Clones the snapshot collector handle.
    pub fn snapshots_clone(&self) -> AggregateIndexSnapshotCollector {
        self.snapshots.clone()
    }

    pub fn build(
        &self,
        project_id: &str,
        expected_membership_revision: u64,
    ) -> Result<AggregateIndexRecord, AggregateIndexError> {
        validate_relative_id(project_id)?;
        let manifest = self
            .logical
            .load_manifest(project_id)?
            .ok_or_else(|| missing_manifest(project_id))?;
        if manifest.membership_revision != expected_membership_revision {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_membership_revision_mismatch",
                message: format!(
                    "project {project_id} membership revision is {}, expected {expected_membership_revision}",
                    manifest.membership_revision
                ),
            });
        }

        self.with_single_writer(project_id, || {
            self.apply_index(
                project_id,
                &manifest,
                IndexApplicationMode::Initialize,
                None,
            )
        })
    }

    /// Idempotent first build bound to a durable command identity (C4 Task 5):
    /// replays of the same `command_id` return the recorded generation without
    /// issuing any CodeGraph CLI request; a conflicting expected membership
    /// revision fails closed instead of issuing a second build.
    pub fn build_with_command_id(
        &self,
        project_id: &str,
        expected_membership_revision: u64,
        command_id: &str,
    ) -> Result<AggregateIndexRecord, AggregateIndexError> {
        validate_relative_id(project_id)?;
        validate_relative_id(command_id)?;
        self.with_single_writer(project_id, || {
            for record in self.store.records(project_id)? {
                if record.command_id.as_deref() != Some(command_id) {
                    continue;
                }
                if record.membership_revision != expected_membership_revision {
                    return Err(AggregateIndexError::Failed {
                        code: "aggregate_index_command_revision_mismatch",
                        message: format!(
                            "command {command_id} was durably recorded against membership revision {}, expected {expected_membership_revision}",
                            record.membership_revision
                        ),
                    });
                }
                return match record.status {
                    AggregateIndexStatus::Active
                    | AggregateIndexStatus::Degraded
                    | AggregateIndexStatus::Superseded => Ok(record),
                    AggregateIndexStatus::Building => Err(AggregateIndexError::Failed {
                        code: "aggregate_index_rebuild_in_progress",
                        message: format!(
                            "command {command_id} is already building for project {project_id}"
                        ),
                    }),
                    AggregateIndexStatus::Stale | AggregateIndexStatus::Failed => {
                        Err(AggregateIndexError::Failed {
                            code: "aggregate_index_command_replayed_failure",
                            message: format!(
                                "command {command_id} previously failed: {}",
                                record.warning.as_deref().unwrap_or("unknown failure")
                            ),
                        })
                    }
                };
            }
            let manifest = self
                .logical
                .load_manifest(project_id)?
                .ok_or_else(|| missing_manifest(project_id))?;
            if manifest.membership_revision != expected_membership_revision {
                return Err(AggregateIndexError::Failed {
                    code: "aggregate_index_membership_revision_mismatch",
                    message: format!(
                        "project {project_id} membership revision is {}, expected {expected_membership_revision}",
                        manifest.membership_revision
                    ),
                });
            }
            self.apply_index(
                project_id,
                &manifest,
                IndexApplicationMode::Initialize,
                Some(command_id),
            )
        })
    }

    /// Rebuilds the aggregate index under the single-writer lock, preserving the
    /// last-known-good generation on failure.
    ///
    /// On success the freshly verified record supersedes the prior active record
    /// via [`AggregateIndexStore::replace_active`]. On failure the newly-created
    /// generation is marked [`AggregateIndexStatus::Stale`] with the failure
    /// reason, while the before-snapshot generation remains the readable active
    /// pointer. The active pointer is never switched on the failure path.
    pub fn rebuild(&self, project_id: &str) -> Result<AggregateIndexRecord, AggregateIndexError> {
        self.with_single_writer(project_id, || {
            let manifest = self
                .logical
                .load_manifest(project_id)?
                .ok_or_else(|| missing_manifest(project_id))?;
            match self.apply_index(project_id, &manifest, IndexApplicationMode::Rebuild, None) {
                Ok(next) => Ok(next),
                Err(error) => Err(error),
            }
        })
    }

    /// Serializes `operation` against concurrent rebuild/sync writers for the
    /// project by holding an exclusive flock on the project-scoped aggregate
    /// index lock file. Half-written indexes and racing active-pointer flips
    /// are thereby impossible: only one writer mutates the index generation at a
    /// time.
    ///
    /// The original [`AggregateIndexError`] from `operation` is preserved
    /// verbatim (including its `&'static str` code); only lock-acquisition
    /// failures are mapped to a distinct `aggregate_index_lock_error` code.
    pub fn with_single_writer<T>(
        &self,
        project_id: &str,
        operation: impl FnOnce() -> Result<T, AggregateIndexError>,
    ) -> Result<T, AggregateIndexError> {
        validate_relative_id(project_id)?;
        let path = self.store.lock_path(project_id)?;
        // Capture the domain error produced inside the locked section so its
        // `&'static str` code survives the lock boundary; lock-acquisition
        // errors fall through to a distinct code below.
        let mut captured: Option<AggregateIndexError> = None;
        let result = with_exact_exclusive_lock(&path, || match operation() {
            Ok(value) => Ok(value),
            Err(error) => {
                captured = Some(error.clone());
                Err(ProductStoreError::Io(error.to_string()))
            }
        });
        match result {
            Ok(value) => Ok(value),
            Err(_) => {
                if let Some(error) = captured {
                    Err(error)
                } else {
                    Err(AggregateIndexError::Failed {
                        code: "aggregate_index_lock_error",
                        message: format!(
                            "acquire aggregate-index single-writer lock for {project_id} failed"
                        ),
                    })
                }
            }
        }
    }

    /// Refreshes an existing active index after freshness detected a drift.
    ///
    /// The caller (freshness service) supplies the previously-active record as
    /// freshness evidence. The durable active pointer is re-read by the shared
    /// operation, so on success the freshly verified record supersedes the
    /// current active generation. On any failure the newly-created generation
    /// is marked stale with the error and the original error is propagated; the
    /// before-snapshot active generation is never silently replaced.
    pub fn sync_and_verify(
        &self,
        project_id: &str,
        _prior: AggregateIndexRecord,
    ) -> Result<AggregateIndexRecord, AggregateIndexError> {
        validate_relative_id(project_id)?;
        self.with_single_writer(project_id, || {
            let manifest = self
                .logical
                .load_manifest(project_id)?
                .ok_or_else(|| missing_manifest(project_id))?;

            match self.apply_index(project_id, &manifest, IndexApplicationMode::Sync, None) {
                Ok(record) => Ok(record),
                Err(error) => Err(error),
            }
        })
    }

    /// Shared body that regenerates the CodeGraph configuration, executes the
    /// index command (`init` for fresh builds, `sync` for incremental refresh),
    /// re-runs member-coverage and negative-query acceptance, captures fresh
    /// member snapshots, and publishes a new active record superseding any prior.
    fn apply_index(
        &self,
        project_id: &str,
        manifest: &LogicalCodebaseManifest,
        mode: IndexApplicationMode,
        command_id: Option<&str>,
    ) -> Result<AggregateIndexRecord, AggregateIndexError> {
        let members = self.logical.list_members(project_id)?;
        let checkouts = self.logical.list_checkouts(project_id)?;
        let included = included_main_checkouts(manifest, &members, &checkouts)?;
        let before = self.snapshots.capture_included(project_id, manifest)?;
        let member_names = included
            .iter()
            .map(|(_, checkout)| checkout_root_name(&manifest.provider_context_root, checkout))
            .collect::<Result<Vec<_>, _>>()?;

        let now = Utc::now().to_rfc3339();
        let building = AggregateIndexRecord::building(
            new_index_id(),
            project_id.to_string(),
            manifest.membership_revision,
            before,
            now,
        );
        let mut building = building;
        building.command_id = command_id.map(|value| value.to_string());
        let building = self.store.create(project_id, building)?;
        let index_id = building.aggregate_index_id.clone();

        let command_result = (|| {
            let codegraph_version = self.cli.verify_version()?;
            let config = self.excludes.generate(manifest, &members, &checkouts)?;
            let config_digest = self
                .excludes
                .write_atomically(&manifest.provider_context_root, &config)?;
            match mode {
                IndexApplicationMode::Initialize | IndexApplicationMode::Rebuild => {
                    // G4（终局关闸缺口）：codegraph 1.6 实证——对已存在的
                    // 索引库重复 `init` 不重扫新增成员目录（补注册成员后
                    // rebuild 仍 member_coverage_failed 的现场根因），
                    // `sync` 才做增量重扫。Initialize/Rebuild 统一以
                    // init 建库＋sync 收尾：库不存在时 init 全量建库，
                    // 库已存在时 sync 把成员集变化纳入覆盖。
                    self.cli.init(&manifest.provider_context_root)?;
                    self.cli.sync(&manifest.provider_context_root)?;
                }
                IndexApplicationMode::Sync => {
                    // 防漂移：durable Active 记录可能指向已被清理/重建的
                    // 磁盘库（.codegraph 缺失或未初始化），sync-only 对
                    // 未初始化库必然 exit 1——回退 init＋sync 自愈。
                    self.cli.sync_or_reinit(&manifest.provider_context_root)?;
                }
            }
            Ok::<_, AggregateIndexError>((codegraph_version, config_digest))
        })();

        let after_result = self.snapshots.capture_included(project_id, manifest);
        let after = match after_result {
            Ok(after) => after,
            Err(error) => {
                return self.fail_building(project_id, &index_id, mode, error, Vec::new());
            }
        };
        let (codegraph_version, config_digest) = match command_result {
            Ok((codegraph_version, config_digest)) => (codegraph_version, config_digest),
            Err(error) => return self.fail_building(project_id, &index_id, mode, error, after),
        };
        if snapshots_differ(&building.member_snapshots, &after) {
            return self.fail_building(
                project_id,
                &index_id,
                mode,
                AggregateIndexError::Failed {
                    code: "aggregate_index_member_drifted",
                    message: "member checkout changed while aggregate index was building".into(),
                },
                after,
            );
        }

        let acceptance = match AggregateIndexAcceptance::verify(
            &self.cli,
            self.coverage.as_ref(),
            &manifest.provider_context_root,
            &member_names,
        ) {
            Ok(acceptance) => {
                let summary = summarize_coverage(&acceptance.member_covered);
                // 一条 info：验收结论（成功 + 成员汇总 + 是否有健康闸 warning）。
                tracing::info!(
                    project_id,
                    index_id = %index_id,
                    mode = mode.as_str(),
                    status = "ok",
                    member_count = summary.member_count,
                    covered_count = summary.covered_count,
                    warning = acceptance.warning.is_some(),
                    "aggregate index acceptance"
                );
                acceptance
            }
            Err(error) => return self.fail_building(project_id, &index_id, mode, error, after),
        };

        let now = Utc::now().to_rfc3339();
        let mut record = building;
        record.observed_after_member_snapshots = after;
        record.status = AggregateIndexStatus::Active;
        record.codegraph_root = manifest.provider_context_root.clone();
        // 实测版本透传：激活世代记录真实 CLI 版本（版本门实测值），
        // building/failed 世代保持空值，不伪造未实测的版本号。
        record.codegraph_version = codegraph_version;
        record.config_digest = config_digest;
        record.warning = acceptance.soft_warning();
        record.updated_at = now;
        let active = self.store.replace_active(project_id, record)?;
        // 激活落点回写 manifest 指针：evidence 链首查按
        // `manifest.active_aggregate_index_id` 写 pin（`load_or_pin_index`），
        // 该指针生产上无人写时，重建成功后每个 attempt 的首次检索仍恒
        // 503——而 pin 文件要靠一次成功首查才生成（鸡生蛋）。回写时机＝该
        // 世代 `replace_active` 成为 active 的此刻（不是 rebuild 入口）；
        // 旧世代已被翻 Superseded，指针只认最新 active，与 patrol 对该字段
        // 的引用巡检口径一致。回写失败显式失败：静默成功会留下「索引可用
        // 但证据链恒 503」的假象。
        self.logical
            .set_active_aggregate_index(project_id, &active.aggregate_index_id)?;
        Ok(active)
    }

    fn fail_building(
        &self,
        project_id: &str,
        index_id: &str,
        mode: IndexApplicationMode,
        error: AggregateIndexError,
        after: Vec<super::AggregateIndexMemberSnapshot>,
    ) -> Result<AggregateIndexRecord, AggregateIndexError> {
        let failure_code = error_code(&error);
        let status = match mode {
            IndexApplicationMode::Initialize => AggregateIndexStatus::Failed,
            IndexApplicationMode::Sync | IndexApplicationMode::Rebuild => {
                AggregateIndexStatus::Stale
            }
        };
        // Rebuild/Sync 的工具面失败（CLI/acceptance）：把 last-known-good 标记
        // degraded（可读 + 审计 warning）。member drift 不 degrade——LKG 数据
        // 相对现实已过时，交给 freshness 评估为 stale。
        let lkg_degraded = matches!(
            mode,
            IndexApplicationMode::Rebuild | IndexApplicationMode::Sync
        ) && !matches!(
            &error,
            AggregateIndexError::Failed {
                code: "aggregate_index_member_drifted",
                ..
            }
        );
        // 一条 info：验收结论（失败 + 失败码 + 影响范围）。
        tracing::info!(
            project_id,
            index_id,
            mode = mode.as_str(),
            status = "failed",
            failure_code,
            lkg_degraded,
            "aggregate index acceptance"
        );
        // 一条 warn：载荷截断（文件清单不完整）——上限已由错误文案带上，
        // 实际字节不可得；影响范围 = 既有 LKG 是否被标 degraded。
        if failure_code == "codegraph_files_output_truncated" {
            tracing::warn!(
                project_id,
                index_id,
                mode = mode.as_str(),
                error_code = failure_code,
                actual_bytes = "unknown",
                lkg_degraded,
                message = %error,
                "codegraph files output truncated: file list incomplete"
            );
        }
        self.store
            .mark_status(project_id, index_id, status, Some(error.to_string()))?;
        if lkg_degraded {
            self.store
                .degrade_last_known_good(project_id, error.to_string())?;
        }
        if !after.is_empty() {
            let mut record = self.store.get(project_id, index_id)?.ok_or_else(|| {
                AggregateIndexError::Failed {
                    code: "aggregate_index_not_found",
                    message: format!("aggregate index {index_id} was not found"),
                }
            })?;
            record.observed_after_member_snapshots = after;
            record.warning = Some(error.to_string());
            self.store.update_record(project_id, record)?;
        }
        Err(error)
    }
}

fn snapshots_differ(
    before: &[super::AggregateIndexMemberSnapshot],
    after: &[super::AggregateIndexMemberSnapshot],
) -> bool {
    before.len() != after.len()
        || before.iter().zip(after).any(|(before, after)| {
            before.logical_repository_id != after.logical_repository_id
                || before.checkout_id != after.checkout_id
                || before.revision != after.revision
                || before.dirty != after.dirty
                || before.included != after.included
        })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IndexApplicationMode {
    Initialize,
    Sync,
    Rebuild,
}

impl IndexApplicationMode {
    /// 日志字段 `mode`（稳定可 grep）。
    fn as_str(self) -> &'static str {
        match self {
            Self::Initialize => "initialize",
            Self::Sync => "sync",
            Self::Rebuild => "rebuild",
        }
    }
}

/// 错误的稳定 code（日志字段 `error_code` / `failure_code`）。
pub(crate) fn error_code(error: &AggregateIndexError) -> &'static str {
    match error {
        AggregateIndexError::Degraded { code, .. } | AggregateIndexError::Failed { code, .. } => {
            code
        }
    }
}

/// SDK 不可用 ⇒ CLI files 回退的触发原因（日志 `trigger`；`None` = 不回退）。
///
/// 只有 `Failed` 的两种码可回退：`Degraded` 语义与其它失败码必须原样上抛
/// （保持既有行为，不把截断之类的失败悄悄降级）。
pub(crate) fn coverage_fallback_trigger(error: &AggregateIndexError) -> Option<&'static str> {
    match error {
        AggregateIndexError::Failed {
            code: "codegraph_sdk_not_found",
            ..
        } => Some("sdk_not_found"),
        AggregateIndexError::Failed {
            code: "codegraph_sdk_incompatible",
            ..
        } => Some("sdk_incompatible"),
        _ => None,
    }
}

fn included_main_checkouts<'a>(
    manifest: &LogicalCodebaseManifest,
    members: &'a [CodebaseMemberRecord],
    checkouts: &'a [RepositoryCheckoutRecord],
) -> Result<Vec<(&'a CodebaseMemberRecord, &'a RepositoryCheckoutRecord)>, AggregateIndexError> {
    let members_by_id = members
        .iter()
        .map(|member| (member.logical_repository_id, member))
        .collect::<BTreeMap<_, _>>();
    let mut included = Vec::with_capacity(manifest.member_ids.len());
    let mut seen_members = BTreeSet::new();

    for member_id in &manifest.member_ids {
        if !seen_members.insert(*member_id) {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_member_invalid",
                message: format!("manifest repeats member {}", member_id.0),
            });
        }
        let member =
            members_by_id
                .get(member_id)
                .copied()
                .ok_or_else(|| AggregateIndexError::Failed {
                    code: "aggregate_index_member_invalid",
                    message: format!("manifest member {} has no authority record", member_id.0),
                })?;
        if member.status != MemberStatus::Active {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_member_invalid",
                message: format!("manifest member {} is not active", member_id.0),
            });
        }
        let main_checkouts = checkouts
            .iter()
            .filter(|checkout| {
                checkout.logical_repository_id == *member_id && checkout.kind == CheckoutKind::Main
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
        included.push((member, *checkout));
    }
    Ok(included)
}

fn checkout_root_name(
    root: &Path,
    checkout: &RepositoryCheckoutRecord,
) -> Result<String, AggregateIndexError> {
    let root = std::fs::canonicalize(root).map_err(|error| layout_error(root, error))?;
    let checkout_path = std::fs::canonicalize(&checkout.canonical_path)
        .map_err(|error| layout_error(&checkout.canonical_path, error))?;
    let name = checkout_path
        .strip_prefix(&root)
        .ok()
        .and_then(|relative| {
            let mut components = relative.components();
            let first = components.next()?;
            components
                .next()
                .is_none()
                .then(|| first.as_os_str().to_str())?
        })
        .ok_or_else(|| AggregateIndexError::Failed {
            code: "aggregate_index_layout_unsupported",
            message: format!(
                "main checkout {} is not a direct child of aggregate root {}",
                checkout_path.display(),
                root.display()
            ),
        })?;
    validate_relative_id(name)?;
    Ok(name.to_string())
}

fn layout_error(path: &Path, error: std::io::Error) -> AggregateIndexError {
    AggregateIndexError::Failed {
        code: "aggregate_index_layout_unsupported",
        message: format!("cannot canonicalize {}: {error}", path.display()),
    }
}

fn verify_member_coverage(
    files: &[PathBuf],
    member_names: &[String],
) -> Result<BTreeMap<String, bool>, AggregateIndexError> {
    let mut result = BTreeMap::new();
    for member_name in member_names {
        validate_relative_id(member_name)?;
        let prefix = Path::new(member_name);
        let covered = files.iter().any(|file| file.starts_with(prefix));
        result.insert(member_name.clone(), covered);
    }
    Ok(result)
}

/// SDK/CLI 归一后的成员覆盖判定：点名全部未覆盖成员再失败（bootstrap
/// 失败卡要呈现完整的「无可索引代码文件的成员」清单，不止第一个）。
fn require_member_coverage(
    member_covered: &BTreeMap<String, bool>,
    member_names: &[String],
) -> Result<(), AggregateIndexError> {
    let uncovered = member_names
        .iter()
        .filter(|name| !member_covered.get(*name).copied().unwrap_or(false))
        .cloned()
        .collect::<Vec<_>>();
    if uncovered.is_empty() {
        Ok(())
    } else {
        Err(member_coverage_failed(&uncovered))
    }
}

/// 健康闸：SDK 返回的 index.state / pending / stale / stats.fileCount 有
/// 异常信号时记 warning（不阻断激活——保持「不因健康问题阻断」语义；
/// codegraph 1.6 实测健康 state 为 "complete"，未知 state 值也计入信号，
/// 宁可多一条可审计 warning 也不静默）。
fn index_health_warning(outcome: &CoverageOutcome) -> Option<String> {
    let mut signals: Vec<String> = Vec::new();
    if let Some(index) = &outcome.index {
        if index.get("stale") == Some(&Value::Bool(true)) {
            signals.push("索引已过期（stale=true）".to_string());
        }
        if let Some(pending) = index.get("pending").and_then(Value::as_u64)
            && pending > 0
        {
            signals.push(format!("仍有 {pending} 个文件待索引（pending）"));
        }
        if let Some(Value::String(state)) = index.get("state") {
            const HEALTHY_STATES: [&str; 10] = [
                "ready",
                "idle",
                "complete",
                "completed",
                "ok",
                "healthy",
                "clean",
                "indexed",
                "active",
                "current",
            ];
            if !HEALTHY_STATES.contains(&state.trim().to_ascii_lowercase().as_str()) {
                signals.push(format!("index state: {state}"));
            }
        }
    }
    if outcome
        .stats
        .as_ref()
        .and_then(|stats| stats.get("fileCount"))
        .and_then(Value::as_u64)
        == Some(0)
    {
        signals.push("stats.fileCount=0（成员覆盖却无文件）".to_string());
    }
    if signals.is_empty() {
        None
    } else {
        Some(format!(
            "codegraph 索引健康闸（不阻断激活）：{}",
            signals.join("；")
        ))
    }
}

/// 覆盖不足的业务语义错误：点名无可索引代码文件的成员清单（空索引时
/// 即全部成员），并携带「每成员至少 1 个可索引代码文件」修复指引。
fn member_coverage_failed(uncovered: &[String]) -> AggregateIndexError {
    AggregateIndexError::Failed {
        code: "aggregate_index_member_coverage_failed",
        message: format!(
            "no indexable code file under included members [{}]; each included member \
             needs at least 1 indexable code file",
            uncovered.join(", ")
        ),
    }
}

fn is_empty_query_result(value: &Value) -> bool {
    matches!(value, Value::Array(values) if values.is_empty())
}

fn exclusion_failed(query: &str, paths: Vec<PathBuf>) -> AggregateIndexError {
    AggregateIndexError::Failed {
        code: "aggregate_index_exclusion_failed",
        message: format!(
            "excluded unique symbol {query} was indexed at: {}",
            format_paths(&paths)
        ),
    }
}

fn result_paths(value: &Value) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    collect_result_paths(value, &mut paths);
    paths.sort();
    paths.dedup();
    paths
}

fn collect_result_paths(value: &Value, paths: &mut Vec<PathBuf>) {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_result_paths(value, paths);
            }
        }
        Value::Object(object) => {
            for key in ["file", "path", "filePath"] {
                if let Some(Value::String(path)) = object.get(key) {
                    paths.push(PathBuf::from(path));
                }
            }
            for value in object.values() {
                collect_result_paths(value, paths);
            }
        }
        _ => {}
    }
}

fn format_paths(paths: &[PathBuf]) -> String {
    if paths.is_empty() {
        "<no paths returned>".to_string()
    } else {
        paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn missing_manifest(project_id: &str) -> AggregateIndexError {
    AggregateIndexError::Failed {
        code: "aggregate_index_manifest_missing",
        message: format!("logical-codebase manifest is missing for project {project_id}"),
    }
}

fn new_index_id() -> String {
    format!("aggregate_index_{}", Uuid::new_v4())
}

#[cfg(test)]
include!("operation_tests.inc.rs");
