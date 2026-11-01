use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::logical_codebase::RegistrationCandidate;

const SNAPSHOT_TTL: Duration = Duration::hours(24);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistrationPreflightSnapshot {
    pub preflight_id: String,
    pub project_id: String,
    pub aggregate_root: PathBuf,
    pub candidates: Vec<RegistrationCandidate>,
    pub created_at: String,
}

impl RegistrationPreflightSnapshot {
    #[cfg(test)]
    fn for_test(
        preflight_id: &str,
        project_id: &str,
        aggregate_root: impl Into<PathBuf>,
        candidates: Vec<RegistrationCandidate>,
    ) -> Self {
        Self {
            preflight_id: preflight_id.to_string(),
            project_id: project_id.to_string(),
            aggregate_root: aggregate_root.into(),
            candidates,
            created_at: "2026-08-18T00:00:00Z".to_string(),
        }
    }
}

pub struct RegistrationPreflightSnapshotStore {
    paths: ProductAppPaths,
    lc_id: Option<String>,
}

impl RegistrationPreflightSnapshotStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths, lc_id: None }
    }

    /// Scopes snapshots to one logical codebase subtree (`preflights/` under
    /// the v1.3 per-LC layout; the legacy alias keeps the legacy root).
    pub fn for_lc(paths: ProductAppPaths, lc_id: impl Into<String>) -> Self {
        Self {
            paths,
            lc_id: Some(lc_id.into()),
        }
    }

    pub fn save(&self, snapshot: &RegistrationPreflightSnapshot) -> Result<(), ProductStoreError> {
        validate_relative_id(&snapshot.project_id)?;
        validate_relative_id(&snapshot.preflight_id)?;
        write_json(
            &self.path(&snapshot.project_id, &snapshot.preflight_id)?,
            snapshot,
        )
    }

    pub fn load_unexpired(
        &self,
        project_id: &str,
        preflight_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<RegistrationPreflightSnapshot>, ProductStoreError> {
        let path = self.path(project_id, preflight_id)?;
        let snapshot: RegistrationPreflightSnapshot = match read_json(&path) {
            Ok(snapshot) => snapshot,
            Err(error) => match std::fs::metadata(&path) {
                Err(metadata_error) if metadata_error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(None);
                }
                _ => return Err(error),
            },
        };

        if snapshot.project_id != project_id || snapshot.preflight_id != preflight_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "registration_preflight_snapshot",
                id: format!("{project_id}/{preflight_id}"),
            });
        }

        let created_at = DateTime::parse_from_rfc3339(&snapshot.created_at)
            .map_err(|error| ProductStoreError::InvalidRecord {
                kind: "registration_preflight_snapshot",
                reason: format!("invalid created_at: {error}"),
            })?
            .with_timezone(&Utc);
        if created_at + SNAPSHOT_TTL < now {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(ProductStoreError::Io(format!(
                        "remove {}: {error}",
                        path.display()
                    )));
                }
            }
            return Ok(None);
        }

        Ok(Some(snapshot))
    }

    fn path(&self, project_id: &str, preflight_id: &str) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(preflight_id)?;
        Ok(
            crate::product::logical_codebase::lc_scope_root(&self.paths, project_id, &self.lc_id)?
                .join("preflights")
                .join(format!("{preflight_id}.json")),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::{DateTime, Utc};

    use super::{RegistrationPreflightSnapshot, RegistrationPreflightSnapshotStore};
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::{
        RegistrationCandidate, RegistrationCandidateState, RepositorySourceIdentity,
    };

    #[test]
    fn snapshot_store_survives_restart_and_lazily_expires_after_24_hours() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let snapshot = RegistrationPreflightSnapshot::for_test(
            "preflight_0001",
            "project_0001",
            "/tmp/root",
            vec![candidate()],
        );
        RegistrationPreflightSnapshotStore::new(paths.clone())
            .save(&snapshot)
            .unwrap();
        let persisted = std::fs::read_to_string(
            paths
                .registration_preflights_root("project_0001")
                .join("preflight_0001.json"),
        )
        .unwrap();
        for field in [
            "canonical_path",
            "git_root",
            "source_identity",
            "preflight_revision",
        ] {
            assert!(persisted.contains(field), "missing persisted field {field}");
        }

        let reloaded = RegistrationPreflightSnapshotStore::new(paths.clone())
            .load_unexpired(
                "project_0001",
                "preflight_0001",
                parse("2026-08-19T00:00:00Z"),
            )
            .unwrap();
        assert_eq!(reloaded, Some(snapshot));

        assert_eq!(
            RegistrationPreflightSnapshotStore::new(paths.clone())
                .load_unexpired(
                    "project_0001",
                    "preflight_0001",
                    parse("2026-08-19T00:00:01Z"),
                )
                .unwrap(),
            None
        );
        assert!(
            !paths
                .registration_preflights_root("project_0001")
                .join("preflight_0001.json")
                .exists()
        );
    }

    #[test]
    fn snapshot_store_keeps_snapshot_at_exactly_24_hours() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let snapshot = RegistrationPreflightSnapshot::for_test(
            "preflight_0001",
            "project_0001",
            "/tmp/root",
            vec![candidate()],
        );
        let store = RegistrationPreflightSnapshotStore::new(paths);
        store.save(&snapshot).unwrap();

        assert_eq!(
            store
                .load_unexpired(
                    "project_0001",
                    "preflight_0001",
                    parse("2026-08-19T00:00:00Z"),
                )
                .unwrap(),
            Some(snapshot)
        );
    }

    #[test]
    fn snapshot_store_rejects_path_escaping_ids() {
        let store = RegistrationPreflightSnapshotStore::new(ProductAppPaths::new("/tmp/aria"));

        assert!(
            store
                .load_unexpired(
                    "../project",
                    "preflight_0001",
                    parse("2026-08-19T00:00:00Z")
                )
                .is_err()
        );
        assert!(
            store
                .load_unexpired(
                    "project_0001",
                    "../preflight",
                    parse("2026-08-19T00:00:00Z")
                )
                .is_err()
        );
    }

    fn candidate() -> RegistrationCandidate {
        RegistrationCandidate {
            submitted_path: PathBuf::from("/tmp/root/member"),
            canonical_path: Some(PathBuf::from("/tmp/root/member")),
            git_root: Some(PathBuf::from("/tmp/root/member")),
            source_identity: Some(RepositorySourceIdentity::from_git_parts(
                std::path::Path::new("/tmp/root/member"),
                PathBuf::from("/tmp/root/member/.git"),
                Some("ssh://git@example.test/acme/member.git".to_string()),
            )),
            state: RegistrationCandidateState::Eligible,
            reason: "eligible".to_string(),
            preflight_revision: "sha256:preflight".to_string(),
        }
    }

    fn parse(value: &str) -> DateTime<Utc> {
        value.parse().unwrap()
    }
}
