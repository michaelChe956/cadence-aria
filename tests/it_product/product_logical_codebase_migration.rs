use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::{CodingAttemptStore, IssueDeliveryOverall};
use cadence_aria::product::issue_store::{CreateProductIssueInput, IssueStore};
use cadence_aria::product::json_store::{ProductStoreError, read_json, write_json};
use cadence_aria::product::lifecycle_store::{
    CreateDesignSpecInput, CreateStorySpecInput, CreateWorkItemInput, LifecycleStore,
};
use cadence_aria::product::logical_codebase::{
    IdentityMigrationJournalStore, LogicalCodebaseFeature, LogicalCodebaseStore, MemberStatus,
    SelectionPolicy,
};
use cadence_aria::product::models::{RepositoryRecord, WorkItemPlanStatus};
use cadence_aria::product::project_store::{CreateProjectInput, ProjectStore};
use cadence_aria::product::repository_store::{
    CreateRepositoryInput, DeleteRepositoryCommand, RepositoryStore,
};
use serde_json::Value;
use tempfile::TempDir;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";
const LEGACY_REPOSITORY_ID: &str = "repository_0001";
const LEGACY_REPOSITORY_NAME: &str = "legacy-api";

///真实单仓旧数据 fixture：先经 feature disabled 的公开 store 写入历史物理
///`repo_id`，再删除新 identity 字段，确保迁移读取的是 serde 默认兼容的旧 JSON。
struct LegacySingleRepositoryFixture {
    _root: TempDir,
    paths: ProductAppPaths,
    repository_path: PathBuf,
}

impl LegacySingleRepositoryFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("temporary root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "legacy project".to_string(),
                description: Some("single-repository project before logical-codebase".to_string()),
                multi_repo: false,
            })
            .expect("create project");

        let repository_path = root.path().join(LEGACY_REPOSITORY_NAME);
        fs::create_dir_all(&repository_path).expect("create legacy repository directory");
        run_git(&repository_path, &["init", "--quiet"]);
        run_git(
            &repository_path,
            &[
                "remote",
                "add",
                "origin",
                "ssh://git@example.test/acme/legacy-api.git",
            ],
        );

        let repository = RepositoryStore::new(paths.clone())
            .create(CreateRepositoryInput {
                project_id: PROJECT_ID.to_string(),
                name: LEGACY_REPOSITORY_NAME.to_string(),
                path: repository_path.clone(),
                default_policy_preset: None,
                default_provider_mode: None,
                idempotency_key: "legacy_repository_registration".to_string(),
            })
            .expect("create legacy physical repository");
        assert_eq!(repository.id, LEGACY_REPOSITORY_ID);
        assert!(repository.logical_repository_id.is_none());

        let issue = IssueStore::new(paths.clone())
            .create(CreateProductIssueInput {
                project_id: PROJECT_ID.to_string(),
                repo_id: Some(LEGACY_REPOSITORY_ID.to_string()),
                title: "legacy issue".to_string(),
                description: Some("stored before logical codebase migration".to_string()),
                change_id: Some("legacy-change".to_string()),
            })
            .expect("create legacy issue");
        assert_eq!(issue.id, ISSUE_ID);

        let lifecycle = LifecycleStore::new(paths.clone());
        let story = lifecycle
            .create_story_spec(CreateStorySpecInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: LEGACY_REPOSITORY_ID.to_string(),
                title: "legacy story".to_string(),
                aggregate_codebase: None,
            })
            .expect("create legacy story");
        let design = lifecycle
            .create_design_spec(CreateDesignSpecInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                story_spec_ids: vec![story.id.clone()],
                title: "legacy design".to_string(),
                aggregate_codebase: None,
            })
            .expect("create legacy design");
        let work_item = lifecycle
            .create_work_item(CreateWorkItemInput {
                id: Some("work_item_0001".to_string()),
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: LEGACY_REPOSITORY_ID.to_string(),
                story_spec_ids: vec![story.id.clone()],
                design_spec_ids: vec![design.id.clone()],
                title: "legacy work item".to_string(),
                plan_status: WorkItemPlanStatus::Confirmed,
                ..Default::default()
            })
            .expect("create legacy work item");

        let fixture = Self {
            _root: root,
            paths,
            repository_path,
        };
        fixture.remove_identity_extensions(&story.id, &design.id, &work_item.id);
        fixture
    }

    fn enabled_repository_store(&self) -> RepositoryStore {
        RepositoryStore::with_logical_codebase_feature(
            self.paths.clone(),
            LogicalCodebaseFeature::enabled(),
        )
    }

    /// 通过登记读取入口触发真实 migration，而非直接伪造 authority 文件。
    fn migrate_through_repository_entry(&self) -> RepositoryRecord {
        let repositories = self
            .enabled_repository_store()
            .list(PROJECT_ID)
            .expect("feature-enabled repository entry migrates legacy project");
        assert_eq!(repositories.len(), 1);
        repositories
            .into_iter()
            .next()
            .expect("one legacy repository projection")
    }

    fn set_dual_read_window(&self) {
        let journal_store = IdentityMigrationJournalStore::new(self.paths.clone());
        let mut journal = journal_store
            .load(PROJECT_ID)
            .expect("load migration journal")
            .expect("migration journal");
        // `ensure_identity_schema` has completed all durable writes. Reopen only
        // the documented persisted dual-read marker to exercise the old physical
        // ID reader, as done by crash/recovery migration fixtures.
        journal.read_mode = Some("dual".to_string());
        journal_store
            .save(PROJECT_ID, &journal)
            .expect("persist dual-read compatibility window");
    }

    fn repos_path(&self) -> PathBuf {
        self.paths.project_root(PROJECT_ID).join("repos.json")
    }

    fn remove_identity_extensions(&self, story_id: &str, design_id: &str, work_item_id: &str) {
        remove_json_fields(
            &self.repos_path(),
            &[
                "logical_repository_id",
                "primary_checkout_id",
                "identity_schema_version",
            ],
        );
        remove_json_fields(
            &self
                .paths
                .issue_root(PROJECT_ID, ISSUE_ID)
                .join("story-specs")
                .join(format!("{story_id}.json")),
            &[
                "logical_codebase_ref",
                "involved_repository_ids",
                "focus_repository_id",
            ],
        );
        remove_json_fields(
            &self
                .paths
                .issue_root(PROJECT_ID, ISSUE_ID)
                .join("design-specs")
                .join(format!("{design_id}.json")),
            &[
                "logical_codebase_ref",
                "involved_repository_ids",
                "change_order",
            ],
        );
        remove_json_fields(
            &self
                .paths
                .issue_root(PROJECT_ID, ISSUE_ID)
                .join("work-items")
                .join(format!("{work_item_id}.json")),
            &["target_repository_id"],
        );
    }
}

#[test]
fn legacy_single_repository_project_migrates_to_one_default_member_and_repo_id_projection() {
    let fixture = LegacySingleRepositoryFixture::new();

    let projection = fixture.migrate_through_repository_entry();
    let logical_id = projection
        .logical_repository_id
        .expect("legacy repository receives logical default member identity");
    let checkout_id = projection
        .primary_checkout_id
        .expect("legacy repository receives primary checkout projection");

    let authority = LogicalCodebaseStore::new(fixture.paths.clone());
    let manifest = authority
        .load_manifest(PROJECT_ID)
        .expect("load manifest")
        .expect("default logical codebase manifest");
    let members = authority
        .list_members(PROJECT_ID)
        .expect("list default member");
    let checkout = authority
        .load_checkout(PROJECT_ID, checkout_id)
        .expect("load primary checkout")
        .expect("default member checkout");

    assert_eq!(manifest.member_ids, vec![logical_id]);
    assert_eq!(members.len(), 1, "single repository becomes one member");
    assert_eq!(members[0].logical_repository_id, logical_id);
    assert_eq!(members[0].physical_repository_id, LEGACY_REPOSITORY_ID);
    assert_eq!(members[0].alias, LEGACY_REPOSITORY_NAME);
    assert_eq!(members[0].status, MemberStatus::Active);
    assert_eq!(checkout.logical_repository_id, logical_id);
    assert_eq!(checkout.physical_repository_id, LEGACY_REPOSITORY_ID);
    assert_eq!(
        checkout.canonical_path,
        fs::canonicalize(&fixture.repository_path).unwrap()
    );

    assert_eq!(projection.id, LEGACY_REPOSITORY_ID);
    assert_eq!(projection.identity_schema_version, 1);
    assert_eq!(projection.logical_repository_id, Some(logical_id));
    assert_eq!(projection.primary_checkout_id, Some(checkout_id));

    let issue = IssueStore::new(fixture.paths.clone())
        .get(PROJECT_ID, ISSUE_ID)
        .expect("legacy issue remains readable");
    assert_eq!(issue.repo_id.as_deref(), Some(LEGACY_REPOSITORY_ID));

    let selection = cadence_aria::product::logical_codebase::IssueCodebaseSelectionStore::new(
        fixture.paths.clone(),
    )
    .load(PROJECT_ID, ISSUE_ID)
    .expect("load migrated issue selection")
    .expect("repo_id creates selection projection");
    assert_eq!(selection.selection_policy, SelectionPolicy::Explicit);
    assert_eq!(selection.included_repository_ids, vec![logical_id]);
    assert_eq!(selection.focus_repository_ids, vec![logical_id]);
    assert!(selection.excluded_repository_ids.is_empty());

    let lifecycle = LifecycleStore::new(fixture.paths.clone());
    let stories = lifecycle
        .list_story_specs(PROJECT_ID, ISSUE_ID)
        .expect("legacy story remains readable");
    let designs = lifecycle
        .list_design_specs(PROJECT_ID, ISSUE_ID)
        .expect("legacy design remains readable");
    let work_items = lifecycle
        .list_work_items(PROJECT_ID, ISSUE_ID)
        .expect("legacy work item remains readable");

    assert_eq!(stories.len(), 1);
    assert_eq!(stories[0].repository_id, LEGACY_REPOSITORY_ID);
    assert_eq!(
        stories[0].logical_codebase_ref,
        Some(manifest.logical_codebase_id)
    );
    assert_eq!(stories[0].involved_repository_ids, vec![logical_id]);
    assert_eq!(stories[0].focus_repository_id, Some(logical_id));
    assert_eq!(designs.len(), 1, "legacy design remains readable");
    assert_eq!(designs[0].story_spec_ids, vec![stories[0].id.clone()]);
    assert_eq!(work_items.len(), 1);
    assert_eq!(work_items[0].repository_id, LEGACY_REPOSITORY_ID);
    assert_eq!(work_items[0].target_repository_id, Some(logical_id));
}

#[test]
fn migrated_legacy_repo_id_is_usable_by_registration_planning_and_delivery_entries() {
    let fixture = LegacySingleRepositoryFixture::new();
    let projection = fixture.migrate_through_repository_entry();
    let logical_id = projection.logical_repository_id.expect("logical id");
    fixture.set_dual_read_window();

    // 登记/compatibility entry：旧 physical ID 在双读窗口唯一映射至 member、checkout、projection。
    let (member, checkout, resolved_projection) = fixture
        .enabled_repository_store()
        .resolve_legacy_physical_repository_if_dual(PROJECT_ID, LEGACY_REPOSITORY_ID)
        .expect("legacy repository id resolves during dual-read window");
    assert_eq!(member.logical_repository_id, logical_id);
    assert_eq!(checkout.logical_repository_id, logical_id);
    assert_eq!(resolved_projection.id, LEGACY_REPOSITORY_ID);

    // 规划 entry：旧 repo_id 保留为兼容投影，新 selection/WorkItem target 是权威 logical ID。
    let selection = cadence_aria::product::logical_codebase::IssueCodebaseSelectionStore::new(
        fixture.paths.clone(),
    )
    .load(PROJECT_ID, ISSUE_ID)
    .expect("load planning selection")
    .expect("selection created from old repo_id");
    let work_item = LifecycleStore::new(fixture.paths.clone())
        .list_work_items(PROJECT_ID, ISSUE_ID)
        .expect("read migrated planning work item")
        .into_iter()
        .next()
        .expect("legacy work item");
    assert_eq!(selection.focus_repository_ids, vec![logical_id]);
    assert_eq!(work_item.repository_id, LEGACY_REPOSITORY_ID);
    assert_eq!(work_item.target_repository_id, Some(logical_id));

    // 交付 entry：不调用 Provider；delivery projection 能通过 logical target
    // 解析回真实成员展示名，且旧 work item physical projection 不被破坏。
    let delivery = CodingAttemptStore::new(fixture.paths.clone())
        .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
        .expect("delivery projection for migrated legacy work item");
    assert_eq!(delivery.overall, IssueDeliveryOverall::Partial);
    assert_eq!(delivery.entries.len(), 1);
    assert_eq!(delivery.entries[0].work_item_id, work_item.id);
    assert_eq!(delivery.entries[0].repository_name, LEGACY_REPOSITORY_NAME);
    assert_eq!(delivery.entries[0].attempt_status, None);
}

#[test]
fn deleting_migrated_member_with_legacy_repo_id_reference_is_rejected_without_mutation() {
    let fixture = LegacySingleRepositoryFixture::new();
    let projection = fixture.migrate_through_repository_entry();
    let logical_id = projection.logical_repository_id.expect("logical id");
    let repos_before =
        fs::read_to_string(fixture.repos_path()).expect("read projection before delete");

    let error = fixture
        .enabled_repository_store()
        .delete(
            PROJECT_ID,
            LEGACY_REPOSITORY_ID,
            DeleteRepositoryCommand {
                operation_id: "delete_legacy_member_with_reference".to_string(),
                expected_updated_at: Some(projection.updated_at.clone()),
                allow_tombstone_reactivation: false,
            },
        )
        .expect_err("legacy repo_id issue reference blocks member deletion");
    assert!(matches!(
        error,
        ProductStoreError::Conflict {
            kind: "repository_references",
            ref id,
        } if id == LEGACY_REPOSITORY_ID
    ));

    assert_eq!(
        fs::read_to_string(fixture.repos_path()).expect("read projection after rejected delete"),
        repos_before,
        "rejected delete must not remove compatibility projection"
    );
    let member = LogicalCodebaseStore::new(fixture.paths.clone())
        .load_member(PROJECT_ID, logical_id)
        .expect("load member after rejected delete")
        .expect("member remains present");
    assert_eq!(member.status, MemberStatus::Active);
}

fn remove_json_fields(path: &Path, fields: &[&str]) {
    let mut value: Value = read_json(path).expect("read legacy JSON");
    let records = match &mut value {
        Value::Object(_) => std::slice::from_mut(&mut value),
        Value::Array(records) => records.as_mut_slice(),
        _ => panic!("legacy JSON must be an object or array of objects"),
    };
    for record in records {
        let object = record.as_object_mut().expect("legacy JSON object");
        for field in fields {
            object.remove(*field);
        }
    }
    write_json(path, &value).expect("write old JSON shape");
}

fn run_git(repository: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .current_dir(repository)
        .args(arguments)
        .status()
        .expect("start git");
    assert!(
        status.success(),
        "git -C {} {arguments:?}",
        repository.display()
    );
}
