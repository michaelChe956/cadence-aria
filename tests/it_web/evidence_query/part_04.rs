// WP6 Task 2：50 成员预算合成 case。
//
// fixture 只通过 LogicalCodebaseStore / AggregateIndexStore 直接落盘 manifest、members、
// checkouts 与 index record；成员目录是普通临时目录，不创建 50 个真实 git 仓库。
// evidence query 仍走真实 HTTP handler + 已有 CodeGraph 索引 fixture，不调用真实 Provider。

use std::collections::HashSet;
use std::time::{Duration, Instant};

const MEMBER_COUNT: usize = 50;
const BUDGET_QUERY: &str = "budget_boundary_symbol";

struct FiftyMemberFixture {
    _root: TempDir,
    app_paths: ProductAppPaths,
    member_ids: Vec<LogicalRepositoryId>,
}

impl FiftyMemberFixture {
    fn new() -> Self {
        Self::with_role_padding(0)
    }

    fn with_role_padding(role_padding: usize) -> Self {
        let root = tempfile::tempdir().expect("50 member fixture root");
        let app_paths = ProductAppPaths::new(root.path().join(".aria"));
        let aggregate_root = root.path().join("aggregate-root");
        fs::create_dir_all(&aggregate_root).expect("aggregate root");

        let member_ids: Vec<_> = (0..MEMBER_COUNT)
            .map(|index| LogicalRepositoryId(uuid::Uuid::from_u128((index + 1) as u128)))
            .collect();
        let checkout_ids: Vec<_> = (0..MEMBER_COUNT)
            .map(|index| RepositoryCheckoutId(uuid::Uuid::from_u128((index + 101) as u128)))
            .collect();

        let manifest = LogicalCodebaseManifest {
            schema_version: 1,
            project_id: PROJECT_ID.to_string(),
            logical_codebase_id: uuid::Uuid::from_u128(0x5050),
            provider_context_root: aggregate_root.clone(),
            layout: LogicalCodebaseLayout::CommonNonGitParent,
            membership_revision: 1,
            member_ids: member_ids.clone(),
            active_aggregate_index_id: Some("aggregate_index_50_members".to_string()),
            context_policy_digest: String::new(),
            created_at: NOW.to_string(),
            updated_at: NOW.to_string(),
        };
        let logical = LogicalCodebaseStore::new(app_paths.clone());
        logical
            .save_manifest(PROJECT_ID, &manifest)
            .expect("save 50 member manifest");

        let mut snapshots = Vec::with_capacity(MEMBER_COUNT);
        for index in 0..MEMBER_COUNT {
            let alias = format!("member-{index:02}");
            let member_root = aggregate_root.join(&alias);
            fs::create_dir_all(&member_root).expect("synthetic member directory");
            // The source is intentionally ordinary files: no real repository checkout or git
            // command is needed for this budget/evidence correctness case.
            fs::write(
                member_root.join("symbols.ts"),
                format!("export function {BUDGET_QUERY}_{index:02}() {{ return {index}; }}\n"),
            )
            .expect("synthetic member source");

            let logical_id = member_ids[index];
            let checkout_id = checkout_ids[index];
            let path = member_root.clone();
            logical
                .save_member(
                    PROJECT_ID,
                    &CodebaseMemberRecord {
                        logical_repository_id: logical_id,
                        physical_repository_id: format!("physical-{alias}"),
                        alias: alias.clone(),
                        role: if role_padding == 0 {
                            "service".to_string()
                        } else {
                            format!("service-{}", "r".repeat(role_padding))
                        },
                        ordinal: index as u32,
                        source_identity: RepositorySourceIdentity {
                            scheme: "synthetic_store_fixture_v1".to_string(),
                            key_digest: format!("sha256:{alias}"),
                            canonical_git_dir: path.join(".git"),
                            canonical_origin: None,
                            first_seen_path_hash: format!("hash:{alias}"),
                        },
                        repo_type: RepositoryType::Unknown,
                        tech_stack: vec!["synthetic".to_string()],
                        owner: None,
                        tags: Vec::new(),
                        default_ref: Some("main".to_string()),
                        checkout_ids: vec![checkout_id],
                        status: MemberStatus::Active,
                        created_at: NOW.to_string(),
                        updated_at: NOW.to_string(),
                    },
                )
                .expect("save synthetic member");
            logical
                .save_checkout(
                    PROJECT_ID,
                    &RepositoryCheckoutRecord {
                        checkout_id,
                        logical_repository_id: logical_id,
                        physical_repository_id: format!("physical-{alias}"),
                        kind: CheckoutKind::Main,
                        canonical_path: path,
                        checkout_path_hash: format!("sha256:checkout-{alias}"),
                        git_dir_identity: format!("synthetic-git-dir-{alias}"),
                        revision: Some(format!("revision-{index:02}")),
                        availability: CheckoutAvailability::Available,
                        observed_at: NOW.to_string(),
                        created_at: NOW.to_string(),
                        updated_at: NOW.to_string(),
                    },
                )
                .expect("save synthetic checkout");
            snapshots.push(AggregateIndexMemberSnapshot::indexed(
                logical_id,
                checkout_id,
                format!("revision-{index:02}"),
                false,
                NOW.to_string(),
            ));
        }

        let index_store = AggregateIndexStore::new(app_paths.clone());
        index_store
            .create(
                PROJECT_ID,
                AggregateIndexRecord {
                    aggregate_index_id: "aggregate_index_50_members".to_string(),
                    project_id: PROJECT_ID.to_string(),
                    membership_revision: 1,
                    // This store-only fixture intentionally has ordinary source directories,
                    // not Git checkout metadata. Its index is a readable last-known-good
                    // fixture, so the production fresh entrypoint must retain it without
                    // attempting a Git/CodeGraph rebuild.
                    status: AggregateIndexStatus::Degraded,
                    member_snapshots: snapshots,
                    observed_after_member_snapshots: Vec::new(),
                    codegraph_version: "synthetic-test".to_string(),
                    codegraph_root: aggregate_root.clone(),
                    config_digest: String::new(),
                    created_at: NOW.to_string(),
                    updated_at: NOW.to_string(),
                    supersedes_aggregate_index_id: None,
                    warning: None,
                },
            )
            .expect("save synthetic aggregate index");

        Self {
            _root: root,
            app_paths,
            member_ids,
        }
    }

    fn app(&self) -> axum::Router {
        build_web_router_with_evidence(
            WebAppState::new(
                self._root.path().to_path_buf(),
                WebRuntime::new_fake(self._root.path().to_path_buf()),
            ),
            true,
        )
    }
}

/// 在已有的认证 HTTP evidence fixture 上追加 48 个 store-only 成员。保留它原有的
/// api/web 两个小型 git 仓，仅供一次真实 CodeGraph 查询；新增成员都只是普通目录。
fn append_synthetic_members_to_evidence_fixture(fx: &EvidenceFixture) {
    let logical = LogicalCodebaseStore::new(fx.paths.clone());
    let mut manifest = logical
        .load_manifest(PROJECT_ID)
        .expect("load base manifest")
        .expect("base manifest");
    assert_eq!(manifest.member_ids.len(), 2, "base fixture member count");

    let mut index = AggregateIndexStore::new(fx.paths.clone())
        .get(PROJECT_ID, &fx.aggregate_index_id)
        .expect("load aggregate index")
        .expect("base aggregate index");
    for index_number in 2..MEMBER_COUNT {
        let alias = format!("member-{index_number:02}");
        let logical_id = LogicalRepositoryId(uuid::Uuid::from_u128((index_number + 1_000) as u128));
        let checkout_id =
            RepositoryCheckoutId(uuid::Uuid::from_u128((index_number + 2_000) as u128));
        let member_root = fx.aggregate_root.join(&alias);
        fs::create_dir_all(&member_root).expect("synthetic evidence member directory");
        fs::write(
            member_root.join("symbols.ts"),
            format!("export const {BUDGET_QUERY}_{index_number:02} = {index_number};\n"),
        )
        .expect("synthetic evidence member source");

        logical
            .save_member(
                PROJECT_ID,
                &CodebaseMemberRecord {
                    logical_repository_id: logical_id,
                    physical_repository_id: format!("physical-{alias}"),
                    alias: alias.clone(),
                    role: "service".to_string(),
                    ordinal: index_number as u32,
                    source_identity: RepositorySourceIdentity {
                        scheme: "synthetic_store_fixture_v1".to_string(),
                        key_digest: format!("sha256:{alias}"),
                        canonical_git_dir: member_root.join(".git"),
                        canonical_origin: None,
                        first_seen_path_hash: format!("hash:{alias}"),
                    },
                    repo_type: RepositoryType::Unknown,
                    tech_stack: vec!["synthetic".to_string()],
                    owner: None,
                    tags: Vec::new(),
                    default_ref: Some("main".to_string()),
                    checkout_ids: vec![checkout_id],
                    status: MemberStatus::Active,
                    created_at: NOW.to_string(),
                    updated_at: NOW.to_string(),
                },
            )
            .expect("save synthetic evidence member");
        logical
            .save_checkout(
                PROJECT_ID,
                &RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: logical_id,
                    physical_repository_id: format!("physical-{alias}"),
                    kind: CheckoutKind::Main,
                    canonical_path: member_root,
                    checkout_path_hash: format!("sha256:checkout-{alias}"),
                    git_dir_identity: format!("synthetic-git-dir-{alias}"),
                    revision: Some(format!("revision-{index_number:02}")),
                    availability: CheckoutAvailability::Available,
                    observed_at: NOW.to_string(),
                    created_at: NOW.to_string(),
                    updated_at: NOW.to_string(),
                },
            )
            .expect("save synthetic evidence checkout");
        manifest.member_ids.push(logical_id);
        index.member_snapshots.push(AggregateIndexMemberSnapshot::indexed(
            logical_id,
            checkout_id,
            format!("revision-{index_number:02}"),
            false,
            NOW.to_string(),
        ));
    }
    manifest.membership_revision = 2;
    logical
        .save_manifest(PROJECT_ID, &manifest)
        .expect("advance manifest to 50 members");

    index.membership_revision = 2;
    AggregateIndexStore::new(fx.paths.clone())
        .replace_active(PROJECT_ID, index)
        .expect("advance aggregate index to 50 members");

    let mut attempt = fx.attempt.clone();
    attempt
        .target_snapshot
        .as_mut()
        .expect("base attempt target snapshot")
        .membership_revision = 2;
    write_json(&attempt_record_path(&fx.paths, ATTEMPT_ID), &attempt)
        .expect("advance attempt target snapshot membership revision");
}

#[tokio::test]
async fn fifty_member_planning_query_returns_all_unique_member_ids_and_aliases() {
    let fx = FiftyMemberFixture::new();
    let logical = LogicalCodebaseStore::new(fx.app_paths.clone());
    let manifest = logical
        .load_manifest(PROJECT_ID)
        .expect("load 50 member manifest")
        .expect("manifest exists");
    let members = logical.list_members(PROJECT_ID).expect("list members");
    let checkouts = logical.list_checkouts(PROJECT_ID).expect("list checkouts");
    assert_eq!(manifest.member_ids.len(), MEMBER_COUNT);
    assert_eq!(members.len(), MEMBER_COUNT);
    assert_eq!(checkouts.len(), MEMBER_COUNT);
    let ids: HashSet<_> = members.iter().map(|member| member.logical_repository_id).collect();
    let aliases: HashSet<_> = members.iter().map(|member| member.alias.as_str()).collect();
    assert_eq!(ids.len(), MEMBER_COUNT, "logical_repository_id must be unique");
    assert_eq!(aliases.len(), MEMBER_COUNT, "alias must be unique");
    assert_eq!(ids, manifest.member_ids.iter().copied().collect());

    // There is no standalone logical-member list route. The real Story generation endpoint is
    // the Web query path that consumes the authoritative member listing and returns it in its
    // planning context. It must expose all 50 (not an arbitrarily truncated subset) here.
    let app = fx.app();
    let (status, project) = crate::web_coding_attempt_api::request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name": "50 member planning", "description": null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create project: {project}");
    cadence_aria::product::issue_store::IssueStore::new(fx.app_paths.clone())
        .create(cadence_aria::product::issue_store::CreateProductIssueInput {
            project_id: PROJECT_ID.to_string(),
            repo_id: None,
            title: "50 member planning query".to_string(),
            description: None,
            change_id: None,
        })
        .expect("create logical issue");
    cadence_aria::product::logical_codebase::IssueCodebaseSelectionStore::new(fx.app_paths.clone())
        .save(&cadence_aria::product::logical_codebase::IssueCodebaseSelection::all_members(
            PROJECT_ID,
            ISSUE_ID,
            None,
        ))
        .expect("save all-member selection");
    cadence_aria::product::logical_codebase::policy::AggregatePolicyArtifactStore::new(
        fx.app_paths.clone(),
    )
    .ensure_bootstrap(&manifest)
    .expect("bootstrap aggregate policy");

    let (status, body) = crate::web_coding_attempt_api::request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({
            "title": "50 member Story",
            "author_provider": "fake",
            "reviewer_provider": "codex",
            "review_rounds": 1,
            "superpowers_enabled": false,
            "openspec_enabled": false
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "50-member planning response: {body}");
    let messages = body["workspace_session"]["messages"]
        .as_array()
        .expect("planning messages");
    let context = messages
        .iter()
        .find_map(|message| message["content"].as_str())
        .expect("planning context message");
    for index in 0..MEMBER_COUNT {
        let member = members
            .iter()
            .find(|member| member.alias == format!("member-{index:02}"))
            .expect("fixture member");
        assert!(
            context.contains(&member.logical_repository_id.0.to_string()),
            "missing member id for {}: {context}",
            member.alias
        );
        assert!(context.contains(&member.alias), "missing alias {}: {context}", member.alias);
    }
}

#[tokio::test]
async fn fifty_member_http_evidence_query_filters_directory_scope_under_five_seconds() {
    let fx = seed_evidence_fixture();
    append_synthetic_members_to_evidence_fixture(&fx);
    let logical = LogicalCodebaseStore::new(fx.paths.clone());
    assert_eq!(
        logical.list_members(PROJECT_ID).expect("list expanded members").len(),
        MEMBER_COUNT
    );

    // The 5s ceiling is deliberately loose to avoid CI scheduling/filesystem jitter; this is
    // not a performance benchmark, only a regression guard against accidentally unbounded scans.
    let started = Instant::now();
    let (status, body) = evidence_query(fx.app(), &fx.token, "coder", "cross_repo_symbol").await;
    let elapsed = started.elapsed();

    assert_eq!(status, StatusCode::OK, "50-member evidence response: {body}");
    assert!(elapsed < Duration::from_secs(5), "HTTP evidence query exceeded 5s: {elapsed:?}");
    let text = body["text"].as_str().expect("evidence response text");
    assert!(text.contains("web/src/app.ts"), "cross-member hit missing: {text}");
    assert!(!text.contains("api/"), "target directory leaked: {text}");
    assert!(!text.contains("other/"), "non-member directory leaked: {text}");
    for index in 2..MEMBER_COUNT {
        assert!(
            !text.contains(&format!("member-{index:02}/")),
            "non-query synthetic member leaked into filtered response: {text}"
        );
    }
}

#[tokio::test]
async fn fifty_member_planning_budget_injection_truncates_and_keeps_target() {
    // Long profile metadata puts the same store-backed 50-member data beyond the soft budget.
    // This follows the production PlanningContextResolver path rather than reconstructing an
    // inventory in the test, and still requires no checkout repositories or Provider call.
    let fx = FiftyMemberFixture::with_role_padding(120);
    let logical = LogicalCodebaseStore::new(fx.app_paths.clone());
    let manifest = logical.load_manifest(PROJECT_ID).unwrap().unwrap();
    let target = fx.member_ids[0];
    cadence_aria::product::logical_codebase::IssueCodebaseSelectionStore::new(fx.app_paths.clone())
        .save(&cadence_aria::product::logical_codebase::IssueCodebaseSelection::all_members(
            PROJECT_ID,
            ISSUE_ID,
            None,
        ))
        .expect("save all-member selection");
    cadence_aria::product::logical_codebase::policy::AggregatePolicyArtifactStore::new(
        fx.app_paths.clone(),
    )
    .ensure_bootstrap(&manifest)
    .expect("bootstrap aggregate policy");

    let context = cadence_aria::product::logical_codebase::PlanningContextResolver::new(
        fx.app_paths.clone(),
    )
    .build_with_fresh_index(PROJECT_ID, ISSUE_ID, &[target])
    .await
    .expect("resolve store-backed 50-member planning context");
    let injection = context.inventory_injection;
    assert!(injection.truncated, "50-member inventory must report truncation");
    assert_eq!(injection.omitted_member_ids.len(), MEMBER_COUNT - 1);
    assert!(injection.rendered.contains(&target.0.to_string()));
    assert!(injection.rendered.contains("omitted_member_count=49"));
    assert!(
        injection.rendered.len() <= injection.budget.hard_bytes,
        "budget injection exceeded hard limit: {} > {}",
        injection.rendered.len(),
        injection.budget.hard_bytes
    );
}
