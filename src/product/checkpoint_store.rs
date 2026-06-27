use std::fs;
use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::json_store::{ProductStoreError, read_json, write_json};
// `ArtifactPayload` is a WebSocket protocol DTO currently defined in the web layer.
// product layer reuses it for checkpoint snapshots. Future iterations should move
// these shared types to `product::models` to eliminate the upward dependency.
use crate::web::workspace_ws_types::ArtifactPayload;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: String,
    pub session_id: String,
    pub message_index: u32,
    pub artifact_snapshot: Option<ArtifactPayload>,
    pub stage: String,
    pub created_at: String,
}

pub struct CheckpointStore {
    base_path: PathBuf,
}

impl CheckpointStore {
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }

    fn checkpoints_dir(&self, session_id: &str) -> PathBuf {
        self.base_path
            .join("workspace-sessions")
            .join(session_id)
            .join("checkpoints")
    }

    pub fn create_checkpoint(
        &self,
        session_id: &str,
        message_index: u32,
        artifact_snapshot: Option<&ArtifactPayload>,
        stage: &str,
    ) -> Result<Checkpoint, ProductStoreError> {
        let dir = self.checkpoints_dir(session_id);
        let existing = self.list_checkpoints(session_id)?;
        let seq = existing.len() as u32 + 1;
        let id = format!("cp_{seq:03}");

        let checkpoint = Checkpoint {
            id: id.clone(),
            session_id: session_id.to_string(),
            message_index,
            artifact_snapshot: artifact_snapshot.cloned(),
            stage: stage.to_string(),
            created_at: Utc::now().to_rfc3339(),
        };

        let file_path = dir.join(format!("{id}.json"));
        write_json(&file_path, &checkpoint)?;

        Ok(checkpoint)
    }

    pub fn list_checkpoints(&self, session_id: &str) -> Result<Vec<Checkpoint>, ProductStoreError> {
        let dir = self.checkpoints_dir(session_id);
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries: Vec<_> = fs::read_dir(&dir)
            .map_err(|e| ProductStoreError::Io(format!("read_dir {}: {e}", dir.display())))?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .collect();

        entries.sort_by_key(|e| e.file_name());

        let mut checkpoints = Vec::new();
        for entry in entries {
            let cp: Checkpoint = read_json(&entry.path())?;
            checkpoints.push(cp);
        }

        Ok(checkpoints)
    }

    pub fn get_checkpoint(
        &self,
        session_id: &str,
        checkpoint_id: &str,
    ) -> Result<Checkpoint, ProductStoreError> {
        let file_path = self
            .checkpoints_dir(session_id)
            .join(format!("{checkpoint_id}.json"));

        if !file_path.exists() {
            return Err(ProductStoreError::NotFound {
                kind: "checkpoint",
                id: checkpoint_id.to_string(),
            });
        }

        read_json(&file_path)
    }

    pub fn rollback_to(
        &self,
        session_id: &str,
        checkpoint_id: &str,
    ) -> Result<Checkpoint, ProductStoreError> {
        let target = self.get_checkpoint(session_id, checkpoint_id)?;
        let all = self.list_checkpoints(session_id)?;

        let target_idx = all
            .iter()
            .position(|cp| cp.id == checkpoint_id)
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "checkpoint",
                id: checkpoint_id.to_string(),
            })?;

        let dir = self.checkpoints_dir(session_id);
        for cp in &all[target_idx + 1..] {
            let file_path = dir.join(format!("{}.json", cp.id));
            if file_path.exists() {
                fs::remove_file(&file_path).map_err(|e| {
                    ProductStoreError::Io(format!("remove {}: {e}", file_path.display()))
                })?;
            }
        }

        Ok(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, CheckpointStore) {
        let tmp = TempDir::new().unwrap();
        let store = CheckpointStore::new(tmp.path().to_path_buf());
        (tmp, store)
    }

    fn snapshot(markdown: &str) -> Option<ArtifactPayload> {
        Some(ArtifactPayload::Markdown {
            markdown: markdown.to_string(),
            diff: None,
        })
    }

    #[test]
    fn create_and_list_checkpoints() {
        let (_tmp, store) = setup();

        let cp1 = store
            .create_checkpoint("session_001", 0, None, "prepare_context")
            .unwrap();
        assert_eq!(cp1.id, "cp_001");
        assert_eq!(cp1.message_index, 0);

        let cp2 = store
            .create_checkpoint("session_001", 1, snapshot("# Draft v1").as_ref(), "running")
            .unwrap();
        assert_eq!(cp2.id, "cp_002");

        let all = store.list_checkpoints("session_001").unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, "cp_001");
        assert_eq!(all[1].id, "cp_002");
    }

    #[test]
    fn get_checkpoint() {
        let (_tmp, store) = setup();

        store
            .create_checkpoint("session_001", 0, snapshot("snapshot").as_ref(), "running")
            .unwrap();

        let cp = store.get_checkpoint("session_001", "cp_001").unwrap();
        assert_eq!(cp.artifact_snapshot, snapshot("snapshot"));
        assert_eq!(cp.stage, "running");
    }

    #[test]
    fn get_nonexistent_checkpoint_returns_not_found() {
        let (_tmp, store) = setup();

        let result = store.get_checkpoint("session_001", "cp_999");
        assert!(matches!(result, Err(ProductStoreError::NotFound { .. })));
    }

    #[test]
    fn rollback_removes_subsequent_checkpoints() {
        let (_tmp, store) = setup();

        store
            .create_checkpoint("session_001", 0, None, "prepare_context")
            .unwrap();
        store
            .create_checkpoint("session_001", 1, snapshot("v1").as_ref(), "running")
            .unwrap();
        store
            .create_checkpoint("session_001", 2, snapshot("v2").as_ref(), "cross_review")
            .unwrap();
        store
            .create_checkpoint("session_001", 3, snapshot("v3").as_ref(), "human_confirm")
            .unwrap();

        let target = store.rollback_to("session_001", "cp_002").unwrap();
        assert_eq!(target.id, "cp_002");
        assert_eq!(target.artifact_snapshot, snapshot("v1"));

        let remaining = store.list_checkpoints("session_001").unwrap();
        assert_eq!(remaining.len(), 2);
        assert_eq!(remaining[0].id, "cp_001");
        assert_eq!(remaining[1].id, "cp_002");
    }

    #[test]
    fn rollback_to_last_checkpoint_removes_nothing() {
        let (_tmp, store) = setup();

        store
            .create_checkpoint("session_001", 0, None, "prepare_context")
            .unwrap();
        store
            .create_checkpoint("session_001", 1, snapshot("v1").as_ref(), "running")
            .unwrap();

        store.rollback_to("session_001", "cp_002").unwrap();

        let remaining = store.list_checkpoints("session_001").unwrap();
        assert_eq!(remaining.len(), 2);
    }

    #[test]
    fn list_empty_session_returns_empty_vec() {
        let (_tmp, store) = setup();

        let result = store.list_checkpoints("nonexistent").unwrap();
        assert!(result.is_empty());
    }
}
