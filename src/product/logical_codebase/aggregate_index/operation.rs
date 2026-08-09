use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde_json::Value;
use uuid::Uuid;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::validate_relative_id;
use crate::product::logical_codebase::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalCodebaseManifest,
    LogicalCodebaseStore, MemberStatus, RepositoryCheckoutRecord,
};

use super::{
    AggregateIndexError, AggregateIndexRecord, AggregateIndexSnapshotCollector,
    AggregateIndexStatus, AggregateIndexStore, CodeGraphCli, CodeGraphExcludeGenerator,
};

const REPRESENTATIVE_QUERY: &str = "crossRepoGreeting";
const EXCLUDED_QUERIES: [&str; 4] = [
    "SHOULD_NOT_INDEX_NONMEMBER",
    "SHOULD_NOT_INDEX_WORKTREE",
    "SHOULD_NOT_INDEX_ARIA",
    "SHOULD_NOT_INDEX_BUILD",
];

/// Evidence produced only after every CodeGraph scope assertion succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregateIndexAcceptance {
    pub member_files: BTreeMap<String, Vec<PathBuf>>,
    pub representative_query: Value,
    pub excluded_queries: BTreeMap<String, Vec<PathBuf>>,
}

impl AggregateIndexAcceptance {
    fn verify(
        cli: &CodeGraphCli,
        root: &Path,
        member_names: &[String],
    ) -> Result<Self, AggregateIndexError> {
        let files = cli.files(root)?;
        let member_files = verify_member_coverage(&files, member_names)?;
        let representative_query = cli.query_json(root, REPRESENTATIVE_QUERY)?;
        verify_cross_member_hit(&representative_query, member_names)?;

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
            member_files,
            representative_query,
            excluded_queries,
        })
    }

    fn soft_warning(&self) -> Option<String> {
        None
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
        }
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

        self.cli.verify_v1_5_0()?;
        let members = self.logical.list_members(project_id)?;
        let checkouts = self.logical.list_checkouts(project_id)?;
        let included = included_main_checkouts(&manifest, &members, &checkouts)?;
        let snapshots = self.snapshots.capture_included(project_id, &manifest)?;
        let member_names = included
            .iter()
            .map(|(_, checkout)| checkout_root_name(&manifest.provider_context_root, checkout))
            .collect::<Result<Vec<_>, _>>()?;

        let config = self.excludes.generate(&manifest, &members, &checkouts)?;
        let config_digest = self
            .excludes
            .write_atomically(&manifest.provider_context_root, &config)?;
        self.cli.init(&manifest.provider_context_root)?;
        let acceptance = AggregateIndexAcceptance::verify(
            &self.cli,
            &manifest.provider_context_root,
            &member_names,
        )?;

        let now = Utc::now().to_rfc3339();
        let mut record = AggregateIndexRecord::building(
            new_index_id(),
            project_id.to_string(),
            manifest.membership_revision,
            snapshots,
            now.clone(),
        );
        record.status = AggregateIndexStatus::Active;
        record.codegraph_root = manifest.provider_context_root;
        record.config_digest = config_digest;
        record.warning = acceptance.soft_warning();
        record.updated_at = now;
        self.store.replace_active(project_id, record)
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
) -> Result<BTreeMap<String, Vec<PathBuf>>, AggregateIndexError> {
    let mut result = BTreeMap::new();
    for member_name in member_names {
        validate_relative_id(member_name)?;
        let prefix = Path::new(member_name);
        let covered = files
            .iter()
            .filter(|file| file.starts_with(prefix))
            .cloned()
            .collect::<Vec<_>>();
        if covered.is_empty() {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_member_coverage_failed",
                message: format!(
                    "codegraph files has no indexed file under included member {member_name}"
                ),
            });
        }
        result.insert(member_name.clone(), covered);
    }
    Ok(result)
}

fn verify_cross_member_hit(
    result: &Value,
    member_names: &[String],
) -> Result<(), AggregateIndexError> {
    let paths = result_paths(result);
    let hit_members = paths
        .iter()
        .filter_map(|path| first_path_component(path))
        .filter(|member| member_names.iter().any(|name| name == member))
        .collect::<BTreeSet<_>>();
    if hit_members.len() < 2 {
        return Err(AggregateIndexError::Failed {
            code: "aggregate_index_cross_member_query_failed",
            message: format!(
                "representative query {REPRESENTATIVE_QUERY} must hit two included members; paths: {}",
                format_paths(&paths)
            ),
        });
    }
    Ok(())
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

fn first_path_component(path: &Path) -> Option<&str> {
    path.components().next()?.as_os_str().to_str()
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
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use sha2::Digest;

    use crate::cross_cutting::bounded_command_runner::{
        BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    };
    use crate::product::logical_codebase::{
        CheckoutAvailability, LogicalRepositoryId, RepositoryCheckoutId, RepositorySourceIdentity,
        RepositoryType,
    };

    use super::*;

    #[test]
    fn build_requires_member_coverage_cross_member_hit_and_negative_exclusion() {
        let fixture = aggregate_index_fixture();
        fixture.cli.files_return(["api/src/A.java", "web/src/B.ts"]);
        fixture.cli.query_returns(
            "crossRepoGreeting",
            serde_json::json!([{"file":"api/src/A.java"}, {"file":"web/src/B.ts"}]),
        );
        fixture
            .cli
            .query_returns("SHOULD_NOT_INDEX_WORKTREE", serde_json::json!([]));
        fixture
            .cli
            .query_returns("SHOULD_NOT_INDEX_ARIA", serde_json::json!([]));
        fixture
            .cli
            .query_returns("SHOULD_NOT_INDEX_BUILD", serde_json::json!([]));
        fixture
            .cli
            .query_returns("SHOULD_NOT_INDEX_NONMEMBER", serde_json::json!([]));

        let record = fixture.operation().build("project_0001", 3).unwrap();
        assert_eq!(record.status, AggregateIndexStatus::Active);
        assert_eq!(record.membership_revision, 3);
        assert_eq!(record.config_digest, fixture.config_digest());
        assert_eq!(record.member_snapshots.len(), 2);
        assert!(
            record
                .member_snapshots
                .iter()
                .all(|snapshot| !snapshot.dirty)
        );
        assert_eq!(
            fixture.store.active("project_0001").unwrap().unwrap(),
            record
        );

        fixture.cli.files_return(["api/src/A.java", "web/src/B.ts"]);
        fixture.cli.query_returns(
            "crossRepoGreeting",
            serde_json::json!([{"file":"api/src/A.java"}, {"file":"web/src/B.ts"}]),
        );
        fixture
            .cli
            .query_returns("SHOULD_NOT_INDEX_WORKTREE", serde_json::json!([]));
        fixture
            .cli
            .query_returns("SHOULD_NOT_INDEX_ARIA", serde_json::json!([]));
        fixture
            .cli
            .query_returns("SHOULD_NOT_INDEX_BUILD", serde_json::json!([]));
        fixture.cli.query_returns(
            "SHOULD_NOT_INDEX_NONMEMBER",
            serde_json::json!([{"file":"not-a-repo/src/Leak.java"}]),
        );
        assert!(matches!(
            fixture.operation().build("project_0001", 3),
            Err(AggregateIndexError::Failed { code, .. }) if code == "aggregate_index_exclusion_failed"
        ));
    }

    #[test]
    fn build_rejects_missing_member_coverage_and_cross_member_miss() {
        let fixture = aggregate_index_fixture();
        fixture.cli.files_return(["api/src/A.java"]);
        assert!(matches!(
            fixture.operation().build("project_0001", 3),
            Err(AggregateIndexError::Failed { code, .. }) if code == "aggregate_index_member_coverage_failed"
        ));

        fixture.cli.files_return(["api/src/A.java", "web/src/B.ts"]);
        fixture.cli.query_returns(
            "crossRepoGreeting",
            serde_json::json!([{"file":"api/src/A.java"}]),
        );
        assert!(matches!(
            fixture.operation().build("project_0001", 3),
            Err(AggregateIndexError::Failed { code, .. }) if code == "aggregate_index_cross_member_query_failed"
        ));
    }

    #[test]
    fn build_rejects_invalid_project_id_and_membership_revision_before_cli() {
        let fixture = aggregate_index_fixture();
        assert!(matches!(
            fixture.operation().build("../project_0001", 3),
            Err(AggregateIndexError::Failed { code, .. }) if code == "aggregate_index_store_error"
        ));
        assert!(matches!(
            fixture.operation().build("project_0001", 2),
            Err(AggregateIndexError::Failed { code, .. }) if code == "aggregate_index_membership_revision_mismatch"
        ));
        assert!(fixture.cli.requests().is_empty());
    }

    #[test]
    fn build_requires_an_available_main_checkout() {
        let fixture = aggregate_index_fixture();
        let mut checkout = fixture
            .logical
            .list_checkouts("project_0001")
            .unwrap()
            .remove(0);
        checkout.availability = CheckoutAvailability::Missing;
        fixture
            .logical
            .save_checkout("project_0001", &checkout)
            .unwrap();

        assert!(matches!(
            fixture.operation().build("project_0001", 3),
            Err(AggregateIndexError::Failed { code, .. }) if code == "aggregate_index_member_invalid"
        ));
        let requests = fixture.cli.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].argv, ["--version"]);
    }

    struct Fixture {
        _temp: tempfile::TempDir,
        logical: LogicalCodebaseStore,
        store: AggregateIndexStore,
        cli: Arc<FakeCodeGraphRunner>,
    }

    impl Fixture {
        fn operation(&self) -> AggregateIndexOperation {
            AggregateIndexOperation::with_snapshot_dependencies(
                self.logical.clone(),
                self.store.clone(),
                CodeGraphCli::new(self.cli.clone(), "codegraph".to_string()),
                CodeGraphExcludeGenerator,
                AggregateIndexSnapshotCollector::with_dependencies(
                    self.logical.clone(),
                    self.cli.clone(),
                ),
            )
        }

        fn config_digest(&self) -> String {
            let config: super::super::CodeGraphConfig = serde_json::from_slice(
                &std::fs::read(
                    self.logical
                        .load_manifest("project_0001")
                        .unwrap()
                        .unwrap()
                        .provider_context_root
                        .join("codegraph.json"),
                )
                .unwrap(),
            )
            .unwrap();
            format!(
                "sha256:{:x}",
                sha2::Sha256::digest(serde_json::to_vec_pretty(&config).unwrap())
            )
        }
    }

    fn aggregate_index_fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("aggregate");
        std::fs::create_dir_all(root.join("api/src")).unwrap();
        std::fs::create_dir_all(root.join("web/src")).unwrap();
        std::fs::create_dir_all(root.join("not-a-repo/src")).unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let logical = LogicalCodebaseStore::new(paths.clone());
        let store = AggregateIndexStore::new(paths.clone());
        let api = member_with_checkout(&root, "api", "a".repeat(40));
        let web = member_with_checkout(&root, "web", "b".repeat(40));
        let mut manifest = LogicalCodebaseManifest::new(
            "project_0001",
            root,
            vec![api.0.logical_repository_id, web.0.logical_repository_id],
        );
        manifest.membership_revision = 3;
        logical.save_manifest("project_0001", &manifest).unwrap();
        logical.save_member("project_0001", &api.0).unwrap();
        logical.save_member("project_0001", &web.0).unwrap();
        logical.save_checkout("project_0001", &api.1).unwrap();
        logical.save_checkout("project_0001", &web.1).unwrap();

        Fixture {
            _temp: temp,
            logical,
            store,
            cli: Arc::new(FakeCodeGraphRunner::default()),
        }
    }

    fn member_with_checkout(
        root: &Path,
        name: &str,
        revision: String,
    ) -> (CodebaseMemberRecord, RepositoryCheckoutRecord) {
        let logical_repository_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let canonical_path = root.join(name);
        let now = "2026-08-09T00:00:00Z".to_string();
        let source_identity = RepositorySourceIdentity {
            scheme: "test".to_string(),
            key_digest: format!("sha256:source-{name}"),
            canonical_git_dir: canonical_path.join(".git"),
            canonical_origin: None,
            first_seen_path_hash: format!("sha256:path-{name}"),
        };
        let member = CodebaseMemberRecord {
            logical_repository_id,
            physical_repository_id: format!("repository_{name}"),
            alias: name.to_string(),
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
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        let checkout = RepositoryCheckoutRecord {
            checkout_id,
            logical_repository_id,
            physical_repository_id: member.physical_repository_id.clone(),
            kind: CheckoutKind::Main,
            canonical_path,
            checkout_path_hash: format!("sha256:checkout-{name}"),
            git_dir_identity: source_identity.git_dir_identity(),
            revision: Some(revision),
            availability: CheckoutAvailability::Available,
            observed_at: now.clone(),
            created_at: now.clone(),
            updated_at: now,
        };
        (member, checkout)
    }

    #[derive(Default)]
    struct FakeCodeGraphRunner {
        state: Mutex<FakeCodeGraphState>,
    }

    #[derive(Default)]
    struct FakeCodeGraphState {
        files: Vec<PathBuf>,
        queries: BTreeMap<String, Value>,
        requests: Vec<BoundedCommandRequest>,
    }

    impl FakeCodeGraphRunner {
        fn files_return<I, P>(&self, paths: I)
        where
            I: IntoIterator<Item = P>,
            P: AsRef<Path>,
        {
            self.state.lock().unwrap().files = paths
                .into_iter()
                .map(|path| path.as_ref().to_path_buf())
                .collect();
        }

        fn query_returns(&self, query: &str, value: Value) {
            self.state
                .lock()
                .unwrap()
                .queries
                .insert(query.to_string(), value);
        }

        fn requests(&self) -> Vec<BoundedCommandRequest> {
            std::mem::take(&mut self.state.lock().unwrap().requests)
        }
    }

    #[async_trait::async_trait]
    impl BoundedCommandRunner for FakeCodeGraphRunner {
        async fn run(
            &self,
            request: BoundedCommandRequest,
        ) -> Result<BoundedCommandResult, BoundedCommandError> {
            let stdout = {
                let mut state = self.state.lock().unwrap();
                state.requests.push(request.clone());
                match request.argv.as_slice() {
                    [version] if version == "--version" => "1.5.0\n".to_string(),
                    [rev_parse, head] if rev_parse == "rev-parse" && head == "HEAD" => {
                        match request
                            .working_dir
                            .file_name()
                            .and_then(|name| name.to_str())
                        {
                            Some("api") => "a".repeat(40),
                            Some("web") => "b".repeat(40),
                            name => panic!("unexpected fake Git checkout: {name:?}"),
                        }
                    }
                    [status, porcelain] if status == "status" && porcelain == "--porcelain=v1" => {
                        String::new()
                    }
                    [init, dot] if init == "init" && dot == "." => "Indexed 2 files\n".to_string(),
                    [files, json] if files == "files" && json == "--json" => serde_json::to_string(
                        &state
                            .files
                            .iter()
                            .map(|path| serde_json::json!({"path": path}))
                            .collect::<Vec<_>>(),
                    )
                    .unwrap(),
                    [query, symbol, json] if query == "query" && json == "--json" => state
                        .queries
                        .get(symbol)
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!([]))
                        .to_string(),
                    argv => panic!("unexpected fake CodeGraph argv: {argv:?}"),
                }
            };
            Ok(BoundedCommandResult {
                exit_code: Some(0),
                stdout,
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                duration_ms: 1,
            })
        }
    }
}
