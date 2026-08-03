use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex as AsyncMutex;

use crate::cross_cutting::aria_state_paths::AriaStatePaths;

use super::models::{
    ChatMessage, CreateSessionRequest, DeleteLease, GenerationResult, ImageCreateError,
    ImageCreateSession, PromptBlock, SessionEvent, SessionRecord, SessionStatus, SessionStoreApi,
    SessionSummary, validate_session_id,
};

#[derive(Debug, Clone)]
pub struct SessionStore {
    paths: AriaStatePaths,
    locks: Arc<AsyncMutex<HashMap<String, Arc<AsyncMutex<()>>>>>,
}

impl SessionStore {
    pub fn new(paths: AriaStatePaths) -> Self {
        Self {
            paths,
            locks: Arc::new(AsyncMutex::new(HashMap::new())),
        }
    }

    async fn session_lock(&self, id: &str) -> Arc<AsyncMutex<()>> {
        let mut locks = self.locks.lock().await;
        locks
            .entry(id.to_string())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    async fn read_record(&self, id: &str) -> Result<Option<SessionRecord>, ImageCreateError> {
        read_json_optional(&self.paths.image_create_session_file(id)).await
    }

    async fn required_record(&self, id: &str) -> Result<SessionRecord, ImageCreateError> {
        self.read_record(id)
            .await?
            .ok_or(ImageCreateError::SessionNotFound)
    }

    async fn write_record(&self, record: &SessionRecord) -> Result<(), ImageCreateError> {
        write_json_atomic(
            &self.paths.image_create_session_file(&record.session.id),
            record,
        )
        .await
    }

    fn ensure_active(record: &SessionRecord) -> Result<(), ImageCreateError> {
        if record.session.status == SessionStatus::Deleting {
            Err(ImageCreateError::SessionClosing)
        } else {
            Ok(())
        }
    }
}

#[async_trait]
impl SessionStoreApi for SessionStore {
    async fn create(
        &self,
        req: CreateSessionRequest,
    ) -> Result<ImageCreateSession, ImageCreateError> {
        let id = uuid::Uuid::new_v4().to_string();
        validate_session_id(&id)?;

        let session = ImageCreateSession {
            id: id.clone(),
            provider_name: req.provider_name,
            template: req.template,
            last_provider_session_id: None,
            current_prompt: None,
            status: SessionStatus::Active,
            created_at: Utc::now(),
        };
        let record = SessionRecord {
            session: session.clone(),
            messages: Vec::new(),
            prompt_blocks: Vec::new(),
            generation_results: Vec::new(),
            events: Vec::new(),
            generation: 0,
        };
        let scratch_dir = self.paths.image_create_session_scratch_dir(&id);
        tokio::fs::create_dir_all(&scratch_dir)
            .await
            .map_err(|error| store_io("create", &scratch_dir, error))?;
        if let Err(error) = self.write_record(&record).await {
            let _ = tokio::fs::remove_dir_all(&scratch_dir).await;
            return Err(error);
        }

        Ok(session)
    }

    async fn list(&self) -> Result<Vec<SessionSummary>, ImageCreateError> {
        let sessions_dir = self.paths.image_create_sessions_dir();
        let mut entries = match tokio::fs::read_dir(&sessions_dir).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(store_io("read", &sessions_dir, error)),
        };
        let mut summaries = Vec::new();

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|error| store_io("read", &sessions_dir, error))?
        {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }

            let record: SessionRecord = read_json(&path).await?;
            let metadata = entry
                .metadata()
                .await
                .map_err(|error| store_io("stat", &path, error))?;
            let updated_at = metadata
                .modified()
                .map(DateTime::<Utc>::from)
                .map_err(|error| {
                    ImageCreateError::Store(format!("stat {}: {error}", path.display()))
                })?;
            summaries.push(SessionSummary {
                id: record.session.id,
                provider_name: record.session.provider_name,
                template: record.session.template,
                status: record.session.status,
                created_at: record.session.created_at,
                updated_at,
            });
        }

        summaries.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(summaries)
    }

    async fn get(&self, id: &str) -> Result<Option<SessionRecord>, ImageCreateError> {
        validate_session_id(id)?;
        self.read_record(id).await
    }

    async fn append_message(&self, id: &str, msg: ChatMessage) -> Result<(), ImageCreateError> {
        validate_session_id(id)?;
        let session_lock = self.session_lock(id).await;
        let _guard = session_lock.lock().await;
        let mut record = self.required_record(id).await?;
        Self::ensure_active(&record)?;
        record.messages.push(msg);
        record.generation += 1;
        self.write_record(&record).await
    }

    async fn append_prompt_block(
        &self,
        id: &str,
        mut block: PromptBlock,
    ) -> Result<(), ImageCreateError> {
        validate_session_id(id)?;
        let session_lock = self.session_lock(id).await;
        let _guard = session_lock.lock().await;
        let mut record = self.required_record(id).await?;
        Self::ensure_active(&record)?;
        block.version = record
            .prompt_blocks
            .iter()
            .map(|existing| existing.version)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| ImageCreateError::Store("prompt block version overflow".to_string()))?;
        record.prompt_blocks.push(block);
        record.generation += 1;
        self.write_record(&record).await
    }

    async fn append_generation_result(
        &self,
        id: &str,
        result: GenerationResult,
    ) -> Result<(), ImageCreateError> {
        validate_session_id(id)?;
        let session_lock = self.session_lock(id).await;
        let _guard = session_lock.lock().await;
        let mut record = self.required_record(id).await?;
        Self::ensure_active(&record)?;
        record.generation_results.push(result);
        record.generation += 1;
        self.write_record(&record).await
    }

    async fn append_event(&self, id: &str, event: SessionEvent) -> Result<(), ImageCreateError> {
        validate_session_id(id)?;
        let session_lock = self.session_lock(id).await;
        let _guard = session_lock.lock().await;
        let mut record = self.required_record(id).await?;
        Self::ensure_active(&record)?;
        record.events.push(event);
        record.generation += 1;
        self.write_record(&record).await
    }

    async fn update_session_meta(
        &self,
        id: &str,
        current_prompt: Option<String>,
        last_provider_session_id: Option<String>,
    ) -> Result<(), ImageCreateError> {
        validate_session_id(id)?;
        let session_lock = self.session_lock(id).await;
        let _guard = session_lock.lock().await;
        let mut record = self.required_record(id).await?;
        Self::ensure_active(&record)?;
        if let Some(current_prompt) = current_prompt {
            record.session.current_prompt = Some(current_prompt);
        }
        if let Some(last_provider_session_id) = last_provider_session_id {
            record.session.last_provider_session_id = Some(last_provider_session_id);
        }
        record.generation += 1;
        self.write_record(&record).await
    }

    async fn begin_delete(&self, id: &str) -> Result<Option<DeleteLease>, ImageCreateError> {
        validate_session_id(id)?;
        let session_lock = self.session_lock(id).await;
        let _guard = session_lock.lock().await;
        let Some(mut record) = self.read_record(id).await? else {
            return Ok(None);
        };
        if record.session.status == SessionStatus::Deleting {
            return Ok(None);
        }

        record.session.status = SessionStatus::Deleting;
        record.generation += 1;
        let token = record.generation;
        self.write_record(&record).await?;
        Ok(Some(DeleteLease {
            id: id.to_string(),
            token,
        }))
    }

    async fn finish_delete(&self, lease: DeleteLease) -> Result<(), ImageCreateError> {
        validate_session_id(&lease.id)?;
        let session_lock = self.session_lock(&lease.id).await;
        let _guard = session_lock.lock().await;
        let record = self.required_record(&lease.id).await?;
        if record.generation != lease.token {
            return Err(ImageCreateError::Store(
                "concurrent modification while finishing session deletion".to_string(),
            ));
        }
        if record.session.status != SessionStatus::Deleting {
            return Err(ImageCreateError::Store(
                "session is not marked for deletion".to_string(),
            ));
        }

        let scratch_dir = self.paths.image_create_session_scratch_dir(&lease.id);
        match tokio::fs::remove_dir_all(&scratch_dir).await {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(store_io("remove", &scratch_dir, error)),
        }
        let record_path = self.paths.image_create_session_file(&lease.id);
        tokio::fs::remove_file(&record_path)
            .await
            .map_err(|error| store_io("remove", &record_path, error))
    }
}

async fn read_json_optional<T: DeserializeOwned>(
    path: &Path,
) -> Result<Option<T>, ImageCreateError> {
    match tokio::fs::read(path).await {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| ImageCreateError::Store(format!("read {}: {error}", path.display()))),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(store_io("read", path, error)),
    }
}

async fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, ImageCreateError> {
    read_json_optional(path).await?.ok_or_else(|| {
        ImageCreateError::Store(format!("read {}: file disappeared", path.display()))
    })
}

async fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), ImageCreateError> {
    let parent = path
        .parent()
        .ok_or_else(|| ImageCreateError::Store(format!("{} has no parent", path.display())))?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| store_io("create", parent, error))?;

    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("session.json");
    let temp_path = parent.join(format!(
        ".{file_name}.{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| {
        ImageCreateError::Store(format!("serialize {}: {error}", path.display()))
    })?;
    let result = async {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .await
            .map_err(|error| store_io("create", &temp_path, error))?;
        file.write_all(&bytes)
            .await
            .map_err(|error| store_io("write", &temp_path, error))?;
        file.sync_all()
            .await
            .map_err(|error| store_io("sync", &temp_path, error))?;
        drop(file);
        tokio::fs::rename(&temp_path, path)
            .await
            .map_err(|error| store_io("rename", path, error))
    }
    .await;

    if result.is_err() {
        let _ = tokio::fs::remove_file(&temp_path).await;
    }
    result
}

fn store_io(action: &str, path: &Path, error: std::io::Error) -> ImageCreateError {
    ImageCreateError::Store(format!("{action} {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use tempfile::TempDir;

    use crate::product::image_create::models::{
        DefaultParams, ImageBackground, ImageOutputFormat, ImageQuality, ImageSize, TemplateChoice,
    };
    use crate::product::models::ProviderName;

    use super::*;

    struct Fixture {
        _root: TempDir,
        paths: AriaStatePaths,
        store: SessionStore,
    }

    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().expect("temp dir");
            let paths = AriaStatePaths::from_workspace_root(root.path());
            let store = SessionStore::new(paths.clone());
            Self {
                _root: root,
                paths,
                store,
            }
        }

        async fn create(&self) -> ImageCreateSession {
            self.store
                .create(CreateSessionRequest {
                    template: TemplateChoice {
                        preset: None,
                        custom: Some("technical diagram".to_string()),
                    },
                    provider_name: ProviderName::Fake,
                })
                .await
                .expect("create session")
        }
    }

    fn generation_result(prompt: &str) -> GenerationResult {
        GenerationResult {
            prompt: prompt.to_string(),
            params: DefaultParams {
                size: ImageSize::Landscape,
                quality: ImageQuality::High,
                background: ImageBackground::Opaque,
                output_format: ImageOutputFormat::Png,
            },
            media_type: "image/png".to_string(),
            b64: "aW1hZ2U=".to_string(),
            ts: Utc::now(),
        }
    }

    mod basic {
        use super::*;

        #[tokio::test]
        async fn create_list_get_and_append_round_trip_full_history() {
            let fixture = Fixture::new();
            assert!(fixture.store.list().await.expect("empty list").is_empty());

            let session = fixture.create().await;
            assert!(
                fixture
                    .paths
                    .image_create_session_file(&session.id)
                    .exists()
            );
            assert!(
                fixture
                    .paths
                    .image_create_session_scratch_dir(&session.id)
                    .is_dir()
            );

            let message = ChatMessage {
                role: "user".to_string(),
                content: "draw it".to_string(),
                ts: Utc::now(),
            };
            let event = SessionEvent {
                kind: "iteration".to_string(),
                message: "started".to_string(),
                ts: Utc::now(),
            };
            fixture
                .store
                .append_message(&session.id, message.clone())
                .await
                .expect("append message");
            fixture
                .store
                .append_prompt_block(
                    &session.id,
                    PromptBlock {
                        content: "prompt v1".to_string(),
                        version: 99,
                    },
                )
                .await
                .expect("append prompt");
            fixture
                .store
                .append_generation_result(&session.id, generation_result("prompt v1"))
                .await
                .expect("append result");
            fixture
                .store
                .append_event(&session.id, event.clone())
                .await
                .expect("append event");

            let record = fixture
                .store
                .get(&session.id)
                .await
                .expect("get")
                .expect("record");
            assert_eq!(record.messages, vec![message]);
            assert_eq!(record.prompt_blocks[0].version, 1);
            assert_eq!(record.generation_results.len(), 1);
            assert_eq!(record.events, vec![event]);
            assert_eq!(record.generation, 4);

            let summaries = fixture.store.list().await.expect("list");
            assert_eq!(summaries.len(), 1);
            assert_eq!(summaries[0].id, session.id);
            assert_eq!(summaries[0].status, SessionStatus::Active);
            assert!(summaries[0].updated_at >= summaries[0].created_at);
        }

        #[tokio::test]
        async fn prompt_versions_increment_from_existing_maximum() {
            let fixture = Fixture::new();
            let session = fixture.create().await;
            for content in ["first", "second", "third"] {
                fixture
                    .store
                    .append_prompt_block(
                        &session.id,
                        PromptBlock {
                            content: content.to_string(),
                            version: 0,
                        },
                    )
                    .await
                    .expect("append prompt block");
            }

            let record = fixture.store.get(&session.id).await.unwrap().unwrap();
            let versions = record
                .prompt_blocks
                .iter()
                .map(|block| block.version)
                .collect::<Vec<_>>();
            assert_eq!(versions, vec![1, 2, 3]);
        }

        #[tokio::test]
        async fn update_meta_none_preserves_fields_and_some_updates_them() {
            let fixture = Fixture::new();
            let session = fixture.create().await;
            fixture
                .store
                .update_session_meta(
                    &session.id,
                    Some("stable prompt".to_string()),
                    Some("provider-session-1".to_string()),
                )
                .await
                .expect("initial update");
            fixture
                .store
                .update_session_meta(&session.id, None, Some("provider-session-2".to_string()))
                .await
                .expect("partial update");

            let record = fixture.store.get(&session.id).await.unwrap().unwrap();
            assert_eq!(
                record.session.current_prompt.as_deref(),
                Some("stable prompt")
            );
            assert_eq!(
                record.session.last_provider_session_id.as_deref(),
                Some("provider-session-2")
            );
            assert_eq!(record.generation, 2);
        }

        #[tokio::test]
        async fn public_id_methods_reject_unsafe_ids() {
            let fixture = Fixture::new();
            assert!(matches!(
                fixture.store.get("../escape").await,
                Err(ImageCreateError::InvalidSessionId(_))
            ));
            assert!(matches!(
                fixture
                    .store
                    .append_event(
                        "bad/id",
                        SessionEvent {
                            kind: "x".to_string(),
                            message: "x".to_string(),
                            ts: Utc::now(),
                        },
                    )
                    .await,
                Err(ImageCreateError::InvalidSessionId(_))
            ));
            assert!(matches!(
                fixture.store.begin_delete("/absolute").await,
                Err(ImageCreateError::InvalidSessionId(_))
            ));
            assert!(matches!(
                fixture
                    .store
                    .finish_delete(DeleteLease {
                        id: "a\\b".to_string(),
                        token: 1,
                    })
                    .await,
                Err(ImageCreateError::InvalidSessionId(_))
            ));
        }
    }

    mod linearized_delete {
        use super::*;

        #[tokio::test]
        async fn tombstone_rejects_all_mutations_without_changing_record() {
            let fixture = Fixture::new();
            let session = fixture.create().await;
            let lease = fixture
                .store
                .begin_delete(&session.id)
                .await
                .expect("begin delete")
                .expect("lease");
            let tombstone = fixture.store.get(&session.id).await.unwrap().unwrap();
            assert_eq!(tombstone.session.status, SessionStatus::Deleting);
            assert_eq!(tombstone.generation, lease.token);

            assert!(matches!(
                fixture
                    .store
                    .append_message(
                        &session.id,
                        ChatMessage {
                            role: "assistant".to_string(),
                            content: "late".to_string(),
                            ts: Utc::now(),
                        },
                    )
                    .await,
                Err(ImageCreateError::SessionClosing)
            ));
            assert!(matches!(
                fixture
                    .store
                    .append_prompt_block(
                        &session.id,
                        PromptBlock {
                            content: "late".to_string(),
                            version: 0,
                        },
                    )
                    .await,
                Err(ImageCreateError::SessionClosing)
            ));
            assert!(matches!(
                fixture
                    .store
                    .append_generation_result(&session.id, generation_result("late"))
                    .await,
                Err(ImageCreateError::SessionClosing)
            ));
            assert!(matches!(
                fixture
                    .store
                    .append_event(
                        &session.id,
                        SessionEvent {
                            kind: "late".to_string(),
                            message: "late".to_string(),
                            ts: Utc::now(),
                        },
                    )
                    .await,
                Err(ImageCreateError::SessionClosing)
            ));
            assert!(matches!(
                fixture
                    .store
                    .update_session_meta(&session.id, Some("late".to_string()), None)
                    .await,
                Err(ImageCreateError::SessionClosing)
            ));

            assert_eq!(
                fixture.store.get(&session.id).await.unwrap().unwrap(),
                tombstone
            );
        }

        #[tokio::test]
        async fn concurrent_begin_delete_returns_exactly_one_lease() {
            let fixture = Fixture::new();
            let session = fixture.create().await;

            let (left, right) = tokio::join!(
                fixture.store.begin_delete(&session.id),
                fixture.store.begin_delete(&session.id)
            );
            let leases = [left.expect("left"), right.expect("right")];
            assert_eq!(leases.iter().filter(|lease| lease.is_some()).count(), 1);
        }

        #[tokio::test]
        async fn finish_delete_removes_record_and_scratch_directory() {
            let fixture = Fixture::new();
            let session = fixture.create().await;
            let lease = fixture
                .store
                .begin_delete(&session.id)
                .await
                .unwrap()
                .unwrap();

            fixture
                .store
                .finish_delete(lease)
                .await
                .expect("finish delete");

            assert!(fixture.store.get(&session.id).await.unwrap().is_none());
            assert!(
                !fixture
                    .paths
                    .image_create_session_file(&session.id)
                    .exists()
            );
            assert!(
                !fixture
                    .paths
                    .image_create_session_scratch_dir(&session.id)
                    .exists()
            );
        }

        #[tokio::test]
        async fn generation_token_detects_concurrent_record_modification() {
            let fixture = Fixture::new();
            let session = fixture.create().await;
            let lease = fixture
                .store
                .begin_delete(&session.id)
                .await
                .unwrap()
                .unwrap();
            let mut record = fixture.store.get(&session.id).await.unwrap().unwrap();
            record.generation += 1;
            fixture.store.write_record(&record).await.unwrap();

            assert!(matches!(
                fixture.store.finish_delete(lease).await,
                Err(ImageCreateError::Store(message)) if message.contains("concurrent modification")
            ));
            assert!(
                fixture
                    .paths
                    .image_create_session_file(&session.id)
                    .exists()
            );
            assert!(
                fixture
                    .paths
                    .image_create_session_scratch_dir(&session.id)
                    .exists()
            );
        }

        #[tokio::test]
        async fn asynchronous_generation_completion_cannot_write_after_delete_begins() {
            let fixture = Fixture::new();
            let session = fixture.create().await;
            let lease = fixture
                .store
                .begin_delete(&session.id)
                .await
                .unwrap()
                .unwrap();

            assert!(matches!(
                fixture
                    .store
                    .append_generation_result(&session.id, generation_result("late completion"))
                    .await,
                Err(ImageCreateError::SessionClosing)
            ));
            fixture.store.finish_delete(lease).await.unwrap();
            assert!(fixture.store.get(&session.id).await.unwrap().is_none());
        }
    }
}
