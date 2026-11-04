//! `it_provider` 集成测试的 LC 权威夹具（只用公开 API）。
//!
//! 背景：块3（`fd0b9a8`）删掉了 `IssueSharedWorktreeRoute::Legacy` 臂与
//! `RepositoryRouting::Legacy` 变体 ⇒ **没有 LC 归属的 issue 一律 fail-closed**。
//! 本模块按 `tests/it_product/context_builder.rs` 已验证的同款做法，为集成测试
//! 建一条**真多仓 LC 权威**：LC record → manifest → member → 主 checkout →
//! issue 选型绑定（issue → 成员 logical repo），使 work item 有 logical codebase 归属。
//!
//! 三条形态纪律（踩过就红）：
//! ① 成员 checkout 必须是**真实 git 仓**（跨目标基线采样按真仓读 HEAD），
//!    且是聚合根的**直接子目录**（`scope_member_prefix` 的 strip 前提）。
//! ② 权威必须**先于**依赖它的记录建立（project id 按 `projects/` 目录数分配）。
//! ③ manifest / policy / index 必须落**同一个 `for_lc(lc_id)` 子树**
//!    （用 project 级 `new()` 会双份权威 ⇒ `repository_routing_legacy_conflict`）。

use std::path::Path;

use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::logical_codebase::{
    AggregatePolicyArtifactStore, CheckoutAvailability, CheckoutKind, CodebaseMemberRecord,
    IssueCodebaseSelection, IssueCodebaseSelectionStore, LogicalCodebaseCreateInput,
    LogicalCodebaseManifest, LogicalCodebaseStore, LogicalRepositoryId, MemberStatus,
    RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
};

/// 一个 issue 的 LC 权威现场。
pub(crate) struct LogicalAuthority {
    /// issue 持久归属的 LC id（真多仓子树，非 legacy 别名）。
    pub(crate) logical_codebase_id: String,
}

/// 建一条真多仓 LC 权威，并把 `issue_id` 绑到该 LC 的唯一成员上。
///
/// `member_path` 必须是真实 git 仓，且是 `aggregate_root` 的直接子目录。
#[allow(clippy::too_many_arguments)]
pub(crate) fn seed_logical_authority(
    paths: &ProductAppPaths,
    project_id: &str,
    member_path: &Path,
    aggregate_root: &Path,
    physical_repository_id: &str,
    revision: &str,
) -> LogicalAuthority {
    // 真多仓 LC（非 legacy 别名）：`create` 落 `logical-codebases/{lc_id}/record.json`
    // 与子树目录，authority resolver 直读该 record（不经 `migrate_legacy`）。
    let lc_record = LogicalCodebaseStore::new(paths.clone())
        .create(
            project_id,
            LogicalCodebaseCreateInput {
                name: "it-provider-lc".to_string(),
                aggregate_root: aggregate_root.to_path_buf(),
            },
        )
        .expect("fixture logical codebase");
    let logical_codebase_id = lc_record.id;

    let member_id = LogicalRepositoryId(uuid::Uuid::new_v4());
    let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
    let source_identity =
        RepositorySourceIdentity::from_git_parts(member_path, member_path.join(".git"), None);

    let authority = LogicalCodebaseStore::for_lc(paths.clone(), &logical_codebase_id);
    let manifest = LogicalCodebaseManifest::new(project_id, aggregate_root.to_path_buf(), vec![member_id]);
    authority
        .save_manifest(project_id, &manifest)
        .expect("fixture manifest");
    authority
        .save_member(
            project_id,
            &CodebaseMemberRecord {
                logical_repository_id: member_id,
                physical_repository_id: physical_repository_id.to_string(),
                alias: physical_repository_id.to_string(),
                role: "repository".to_string(),
                ordinal: 1,
                source_identity: source_identity.clone(),
                repo_type: RepositoryType::Unknown,
                tech_stack: Vec::new(),
                owner: None,
                tags: Vec::new(),
                default_ref: None,
                checkout_ids: vec![checkout_id],
                status: MemberStatus::Active,
                created_at: "2026-08-11T00:00:00Z".to_string(),
                updated_at: "2026-08-11T00:00:00Z".to_string(),
            },
        )
        .expect("fixture member");
    authority
        .save_checkout(
            project_id,
            &RepositoryCheckoutRecord {
                checkout_id,
                logical_repository_id: member_id,
                physical_repository_id: physical_repository_id.to_string(),
                kind: CheckoutKind::Main,
                canonical_path: member_path.to_path_buf(),
                checkout_path_hash: "sha256:checkout".to_string(),
                git_dir_identity: source_identity.git_dir_identity(),
                revision: Some(revision.to_string()),
                availability: CheckoutAvailability::Available,
                observed_at: "2026-08-11T00:00:00Z".to_string(),
                created_at: "2026-08-11T00:00:00Z".to_string(),
                updated_at: "2026-08-11T00:00:00Z".to_string(),
            },
        )
        .expect("fixture checkout");
    AggregatePolicyArtifactStore::for_lc(paths.clone(), &logical_codebase_id)
        .ensure_bootstrap(&manifest)
        .expect("fixture aggregate policy");

    LogicalAuthority {
        logical_codebase_id,
    }
}

/// 把 issue 绑到该 LC 的全部成员（issue → logical repo 选型）。
///
/// 必须在 issue 记录落盘【之后】调用 —— 绑定需要真实的 issue id。
pub(crate) fn bind_issue_selection(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    logical_codebase_id: &str,
) {
    IssueCodebaseSelectionStore::for_lc(paths.clone(), logical_codebase_id)
        .save(
            &IssueCodebaseSelection::all_members(project_id, issue_id, None)
                .for_logical_codebase(logical_codebase_id),
        )
        .expect("fixture issue selection");
}

/// 把给定目录初始化成一个真实 git 仓（首次提交），返回 HEAD 的 revision。
pub(crate) fn init_member_git_repo(path: &Path) -> String {
    std::fs::create_dir_all(path).expect("fixture member dir");
    if !path.join(".git").exists() {
        for args in [
            ["init", "--quiet"].as_slice(),
            ["config", "user.email", "fixture@example.test"].as_slice(),
            ["config", "user.name", "Fixture"].as_slice(),
        ] {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(path)
                .status()
                .expect("fixture git command");
            assert!(status.success(), "git {args:?} failed");
        }
        std::fs::write(path.join("README.md"), "fixture\n").expect("fixture readme");
        for args in [
            ["add", "README.md"].as_slice(),
            ["commit", "--quiet", "-m", "fixture"].as_slice(),
        ] {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(path)
                .status()
                .expect("fixture git command");
            assert!(status.success(), "git {args:?} failed");
        }
    }
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(path)
        .output()
        .expect("fixture git rev-parse");
    String::from_utf8(output.stdout)
        .expect("fixture revision utf8")
        .trim()
        .to_string()
}
