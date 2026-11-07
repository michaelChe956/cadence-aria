//! 聚合索引夹具：假 CodeGraph CLI（集成测试**不**跑真实 codegraph 同步）。
//!
//! 为什么夹具不能真跑 `codegraph sync .`：那是本机重活——吃 CPU、抢全局
//! `/tmp/.codegraph` 文件锁、依赖本机 CLI 版本；并行夹具互踩会把增量预算
//! （`AggregateIndexBudget::INCREMENTAL.fail_secs = 30`）耗尽 ⇒ 422
//! `codegraph_timeout` ⇒ 整批用例红在**夹具前置**上（与被测行为无关）。
//!
//! 假 CLI 只替换「CLI 应答」这一层：版本探测 / init / sync / files / query
//! 由 fake 应答；其余全走生产链路——真实 git 成员快照、真实成员覆盖校验、
//! 真实 durable 世代落盘、真实 active 投影。rebuild 端点本身的行为覆盖由
//! `web_aggregate_index_api` 的用例经同一假 CLI 承担。

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use cadence_aria::cross_cutting::bounded_command_runner::{
    BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::logical_codebase::aggregate_index::{
    AggregateIndexOperation, AggregateIndexRecord, AggregateIndexSnapshotCollector,
    AggregateIndexStatus, AggregateIndexStore, CodeGraphCli, CodeGraphExcludeGenerator,
};
use cadence_aria::product::logical_codebase::store::LogicalCodebaseStore;
use cadence_aria::product::logical_codebase::{CheckoutAvailability, CheckoutKind, MemberStatus};
use serde_json::json;

/// 假 CLI 的可执行名（与生产 `"codegraph"` 区分：夹具误配真 CLI 时立即暴露）。
pub(crate) const FAKE_CODEGRAPH_EXECUTABLE: &str = "fake-codegraph";

/// 假 CodeGraph CLI：按 LC 权威（manifest + checkouts）推导成员根名，
/// `files` 时为每个成员报一个可索引源码文件；`query` 恒空（负向排除语义：
/// 被排除的符号不得出现在索引里）。
#[derive(Clone)]
pub(crate) struct FakeCodeGraphCli {
    paths: ProductAppPaths,
    project_id: String,
    lc_id: String,
}

impl FakeCodeGraphCli {
    pub(crate) fn new(paths: ProductAppPaths, project_id: &str, lc_id: &str) -> Self {
        Self {
            paths,
            project_id: project_id.to_string(),
            lc_id: lc_id.to_string(),
        }
    }

    /// LC 权威推出的成员根名（聚合根的直接子目录名）——与生产
    /// `AggregateIndexOperation` 的 `checkout_root_name` 同源语义。
    fn member_root_names(&self) -> Vec<String> {
        let logical = LogicalCodebaseStore::for_lc(self.paths.clone(), self.lc_id.clone());
        let Ok(Some(manifest)) = logical.load_manifest(&self.project_id) else {
            return Vec::new();
        };
        let Ok(members) = logical.list_members(&self.project_id) else {
            return Vec::new();
        };
        let Ok(checkouts) = logical.list_checkouts(&self.project_id) else {
            return Vec::new();
        };
        let mut names = Vec::new();
        for member_id in &manifest.member_ids {
            let Some(member) = members
                .iter()
                .find(|member| member.logical_repository_id == *member_id)
            else {
                continue;
            };
            if member.status != MemberStatus::Active {
                continue;
            }
            let Some(checkout) = checkouts.iter().find(|checkout| {
                checkout.logical_repository_id == *member_id
                    && checkout.kind == CheckoutKind::Main
                    && checkout.availability == CheckoutAvailability::Available
                    && member.checkout_ids.contains(&checkout.checkout_id)
            }) else {
                continue;
            };
            if let Some(name) =
                checkout_root_name(&manifest.provider_context_root, &checkout.canonical_path)
            {
                names.push(name);
            }
        }
        names
    }
}

/// 成员 checkout 在聚合根下的直接子目录名（非直接子目录 ⇒ 无覆盖，返 None）。
fn checkout_root_name(root: &Path, checkout_path: &Path) -> Option<String> {
    let root = std::fs::canonicalize(root).ok()?;
    let checkout = std::fs::canonicalize(checkout_path).ok()?;
    let relative = checkout.strip_prefix(&root).ok()?;
    let mut components = relative.components();
    let first = components.next()?;
    if components.next().is_some() {
        return None;
    }
    Some(first.as_os_str().to_str()?.to_string())
}

/// 假 CLI 应答的版本：取生产兼容区间下界（与 it_web 既有 fake 同源，
/// 区间上移时自动跟随）。
fn fake_version() -> String {
    let (major, minor, patch) =
        cadence_aria::product::logical_codebase::aggregate_index::CODEGRAPH_MIN_VERSION;
    format!("{major}.{minor}.{patch}")
}

#[async_trait]
impl BoundedCommandRunner for FakeCodeGraphCli {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        let command = request.argv.first().map(String::as_str).unwrap_or_default();
        let ok = |stdout: String| BoundedCommandResult {
            exit_code: Some(0),
            stdout,
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 0,
        };
        match command {
            "--version" => Ok(ok(fake_version())),
            "init" => {
                // 真实 `codegraph init` 在聚合根建库目录（`.codegraph`）；假 CLI
                // 如实物化该目录，「库目录在位」判据（`codegraph_library_present`）
                // 与真实链路保持一致（证据链 fail-closed 语义不因假 CLI 变形）。
                let _ = std::fs::create_dir_all(request.working_dir.join(".codegraph"));
                Ok(ok(String::new()))
            }
            "sync" => Ok(ok(String::new())),
            "files" => {
                let files = self
                    .member_root_names()
                    .iter()
                    .map(|name| json!({ "path": format!("{name}/src/main.rs") }))
                    .collect::<Vec<_>>();
                Ok(ok(
                    serde_json::to_string(&files).unwrap_or_else(|_| "[]".to_string())
                ))
            }
            "query" => Ok(ok("[]".to_string())),
            _ => Err(BoundedCommandError::Io {
                details: format!("unexpected fake codegraph command: {command}"),
            }),
        }
    }
}

/// 假 CLI 驱动的聚合索引操作（生产同构：真实 git 快照 + 真实 durable 落盘 +
/// 真实成员覆盖校验；只把 CLI 应答换成 fake）。
pub(crate) fn fake_index_operation(
    paths: &ProductAppPaths,
    project_id: &str,
    lc_id: &str,
) -> Arc<AggregateIndexOperation> {
    Arc::new(AggregateIndexOperation::with_snapshot_dependencies(
        LogicalCodebaseStore::new(paths.clone()),
        AggregateIndexStore::new(paths.clone()),
        CodeGraphCli::new(
            Arc::new(FakeCodeGraphCli::new(paths.clone(), project_id, lc_id)),
            FAKE_CODEGRAPH_EXECUTABLE.to_string(),
        ),
        CodeGraphExcludeGenerator,
        AggregateIndexSnapshotCollector::for_paths(paths.clone()),
    ))
}

/// 夹具前置：经生产聚合索引操作（假 CLI）发布一条 active 聚合索引世代。
///
/// 返回落盘的 active 世代记录（成员快照取自真实 git ⇒ 后续读取方的
/// freshness 判定为 active，不会触发真实 codegraph 同步）。
pub(crate) fn seed_active_aggregate_index(
    paths: &ProductAppPaths,
    project_id: &str,
    lc_id: &str,
) -> AggregateIndexRecord {
    let operation = fake_index_operation(paths, project_id, lc_id).for_lc(lc_id.to_string());
    let record = operation
        .rebuild(project_id)
        .expect("rebuild aggregate index with fake codegraph cli");
    assert_eq!(
        record.status,
        AggregateIndexStatus::Active,
        "fake rebuild must publish an active aggregate-index generation"
    );
    record
}
