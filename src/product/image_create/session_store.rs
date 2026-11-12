use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex as AsyncMutex;

use crate::cross_cutting::aria_state_paths::AriaStatePaths;
use crate::product::image_create::image_files::write_image_atomic;

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

    async fn migrate_legacy_results(
        &self,
        id: &str,
        mut record: SessionRecord,
    ) -> Result<SessionRecord, ImageCreateError> {
        if record.session.status == SessionStatus::Deleting {
            return Ok(record);
        }

        let mut changed = false;
        for result in &mut record.generation_results {
            let Some(b64) = result.b64.as_deref() else {
                continue;
            };
            let image_id = result
                .image_id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            result.image_id = Some(image_id.clone());
            changed = true;

            let bytes = match base64::engine::general_purpose::STANDARD.decode(b64) {
                Ok(bytes) => bytes,
                Err(error) => {
                    tracing::warn!(
                        session_id = id,
                        image_id,
                        error = %error,
                        "failed to decode legacy inline image"
                    );
                    break;
                }
            };
            if let Err(error) =
                write_image_atomic(&self.paths, &image_id, &result.media_type, &bytes).await
            {
                tracing::warn!(
                    session_id = id,
                    image_id,
                    error = %error,
                    "failed to migrate legacy inline image"
                );
                break;
            }
            result.b64 = None;
        }

        if !changed {
            return Ok(record);
        }
        if let Err(error) = self.write_record(&record).await {
            tracing::warn!(
                session_id = id,
                error = %error,
                "failed to persist legacy image migration; returning disk record"
            );
            return self
                .read_record(id)
                .await?
                .ok_or(ImageCreateError::SessionNotFound);
        }
        Ok(record)
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
        let session_lock = self.session_lock(id).await;
        let _guard = session_lock.lock().await;
        let Some(record) = self.read_record(id).await? else {
            return Ok(None);
        };
        Ok(Some(self.migrate_legacy_results(id, record).await?))
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

        for result in &record.generation_results {
            let Some(image_id) = result.image_id.as_deref() else {
                continue;
            };
            let Some(image_path) = self
                .paths
                .image_create_image_file(image_id, &result.media_type)
            else {
                tracing::warn!(
                    session_id = %lease.id,
                    image_id,
                    media_type = %result.media_type,
                    "failed to resolve session image file for deletion"
                );
                continue;
            };
            match tokio::fs::remove_file(&image_path).await {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => tracing::warn!(
                    session_id = %lease.id,
                    image_id,
                    error = %error,
                    "failed to remove session image file"
                ),
            }
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
#[path = "session_store_tests.rs"]
mod tests;
