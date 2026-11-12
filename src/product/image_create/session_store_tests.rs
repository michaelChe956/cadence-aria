//! session_store.rs 的外置测试模块：经 session_store.rs 内
//! `#[path = "session_store_tests.rs"]` 引入，`use super::*` 语义与原内嵌
//! `mod tests` 完全一致（纯移动，满足 large_file_guard 1200 行上限，Task 20 线 A）。

use chrono::Utc;
use tempfile::TempDir;

use crate::product::image_create::models::{DefaultParams, TemplateChoice};
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

/// 统一的 GenerationResult 测试辅助：Task 1 新增的 4 个可选字段在此统一置 None，
/// 以一个构造点替换原先散落各处的字面量，避免逐个补字段撑高行数。
fn generation_result(
    prompt: &str,
    media_type: &str,
    image_id: Option<String>,
    b64: Option<&str>,
) -> GenerationResult {
    GenerationResult {
        prompt: prompt.to_string(),
        params: DefaultParams::default(),
        media_type: media_type.to_string(),
        image_id,
        b64: b64.map(str::to_string),
        vendor: None,
        model: None,
        reference_source: None,
        ts: Utc::now(),
    }
}

async fn inject_legacy_result(fixture: &Fixture, session_id: &str, b64: &str, media_type: &str) {
    let record = fixture.store.get(session_id).await.unwrap().unwrap();
    let result = generation_result("legacy prompt", media_type, None, Some(b64));
    let mut value = serde_json::to_value(&record).unwrap();
    let mut result_value = serde_json::to_value(result).unwrap();
    result_value.as_object_mut().unwrap().remove("image_id");
    value["generation_results"] = serde_json::json!([result_value]);
    tokio::fs::write(
        fixture.paths.image_create_session_file(session_id),
        serde_json::to_vec_pretty(&value).unwrap(),
    )
    .await
    .unwrap();
}

mod migration {
    use super::*;

    #[tokio::test]
    async fn get_migrates_legacy_inline_b64_to_image_file() {
        let fixture = Fixture::new();
        let session = fixture.create().await;
        inject_legacy_result(&fixture, &session.id, "aGVsbG8=", "image/png").await;

        let record = fixture.store.get(&session.id).await.unwrap().unwrap();
        let migrated = &record.generation_results[0];
        assert!(migrated.b64.is_none());
        let id = migrated.image_id.as_deref().expect("id assigned");
        let file = fixture
            .paths
            .image_create_image_file(id, "image/png")
            .expect("image path");
        assert_eq!(tokio::fs::read(file).await.unwrap(), b"hello");
        assert_eq!(record.generation, 0);

        let persisted = fixture.store.get(&session.id).await.unwrap().unwrap();
        assert_eq!(persisted, record);
    }

    #[tokio::test]
    async fn get_migration_reuses_existing_image_id_and_stops_after_failure() {
        let fixture = Fixture::new();
        let session = fixture.create().await;
        let mut record = fixture.store.get(&session.id).await.unwrap().unwrap();
        let existing_id = "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0".to_string();
        record.generation_results = vec![
            generation_result(
                "first",
                "image/png",
                Some(existing_id.clone()),
                Some("Zmlyc3Q="),
            ),
            generation_result("invalid", "image/png", None, Some("not-base64")),
            generation_result("third", "image/png", None, Some("dGhpcmQ=")),
        ];
        fixture.store.write_record(&record).await.unwrap();

        let migrated = fixture.store.get(&session.id).await.unwrap().unwrap();
        assert_eq!(
            migrated.generation_results[0].image_id.as_deref(),
            Some(existing_id.as_str())
        );
        assert!(migrated.generation_results[0].b64.is_none());
        assert!(migrated.generation_results[1].b64.is_some());
        assert!(migrated.generation_results[1].image_id.is_some());
        assert!(migrated.generation_results[2].b64.is_some());
        assert_eq!(
            tokio::fs::read(
                fixture
                    .paths
                    .image_create_image_file(&existing_id, "image/png")
                    .unwrap()
            )
            .await
            .unwrap(),
            b"first"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn get_migration_write_failure_returns_disk_record_and_is_retryable() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = Fixture::new();
        let session = fixture.create().await;
        inject_legacy_result(&fixture, &session.id, "aGVsbG8=", "image/png").await;
        let original = fixture
            .store
            .read_record(&session.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            original.generation_results[0].b64.as_deref(),
            Some("aGVsbG8=")
        );

        let sessions_dir = fixture.paths.image_create_sessions_dir();
        let original_permissions = std::fs::metadata(&sessions_dir).unwrap().permissions();
        let mut read_only = original_permissions.clone();
        read_only.set_mode(0o555);
        std::fs::set_permissions(&sessions_dir, read_only).unwrap();
        let failed = fixture.store.get(&session.id).await.unwrap().unwrap();
        std::fs::set_permissions(&sessions_dir, original_permissions).unwrap();

        assert_eq!(failed, original);
        assert!(failed.generation_results[0].image_id.is_none());
        assert!(failed.generation_results[0].b64.is_some());

        let retried = fixture.store.get(&session.id).await.unwrap().unwrap();
        assert!(retried.generation_results[0].image_id.is_some());
        assert!(retried.generation_results[0].b64.is_none());
        let id = retried.generation_results[0].image_id.as_deref().unwrap();
        assert_eq!(
            tokio::fs::read(
                fixture
                    .paths
                    .image_create_image_file(id, "image/png")
                    .unwrap()
            )
            .await
            .unwrap(),
            b"hello"
        );
    }

    #[tokio::test]
    async fn get_skips_migration_for_deleting_session() {
        let fixture = Fixture::new();
        let session = fixture.create().await;
        fixture
            .store
            .append_generation_result(
                &session.id,
                generation_result("legacy", "image/png", None, Some("aW1hZ2U=")),
            )
            .await
            .unwrap();
        let lease = fixture
            .store
            .begin_delete(&session.id)
            .await
            .unwrap()
            .unwrap();

        let record = fixture.store.get(&session.id).await.unwrap().unwrap();
        assert_eq!(
            record.generation_results[0].b64.as_deref(),
            Some("aW1hZ2U=")
        );
        assert!(record.generation_results[0].image_id.is_none());
        assert_eq!(record.generation, lease.token);
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
                    ts: None,
                },
            )
            .await
            .expect("append prompt");
        fixture
            .store
            .append_generation_result(
                &session.id,
                generation_result("prompt v1", "image/png", None, Some("aW1hZ2U=")),
            )
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
                        ts: None,
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

    /// Task 20 线 A：append_prompt_block 写入路径保留 ts 并随记录落盘。
    #[tokio::test]
    async fn append_prompt_block_persists_ts_to_disk() {
        let fixture = Fixture::new();
        let session = fixture.create().await;
        let ts = Utc::now();
        fixture
            .store
            .append_prompt_block(
                &session.id,
                PromptBlock {
                    content: "prompt v1".to_string(),
                    version: 0,
                    ts: Some(ts),
                },
            )
            .await
            .expect("append prompt");

        let record = fixture.store.get(&session.id).await.unwrap().unwrap();
        assert_eq!(record.prompt_blocks[0].ts, Some(ts));
    }

    /// Task 20 线 A：旧版本落盘的会话文件没有 ts 字段，读回必须成功且 ts 为 None。
    #[tokio::test]
    async fn legacy_prompt_block_without_ts_reads_as_none() {
        let fixture = Fixture::new();
        let session = fixture.create().await;
        fixture
            .store
            .append_prompt_block(
                &session.id,
                PromptBlock {
                    content: "old prompt".to_string(),
                    version: 0,
                    ts: Some(Utc::now()),
                },
            )
            .await
            .expect("append prompt");

        // 手工改写会话文件，抹掉 ts 字段，等价于旧版本代码落盘的记录。
        let record = fixture.store.get(&session.id).await.unwrap().unwrap();
        let mut value = serde_json::to_value(record).unwrap();
        for block in value["prompt_blocks"].as_array_mut().unwrap() {
            block.as_object_mut().unwrap().remove("ts");
        }
        tokio::fs::write(
            fixture.paths.image_create_session_file(&session.id),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .await
        .unwrap();

        let record = fixture.store.get(&session.id).await.unwrap().unwrap();
        assert_eq!(record.prompt_blocks.len(), 1);
        assert_eq!(record.prompt_blocks[0].content, "old prompt");
        assert_eq!(record.prompt_blocks[0].ts, None);
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

/// 线性化删除用例外置（large_file_guard 1200 行上限，Task 20 线 A）：
/// `#[path]` 挂载后与原内嵌 `mod linearized_delete` 同一模块域（纯移动）。
#[cfg(test)]
#[path = "session_store_linearized_delete_tests.rs"]
mod linearized_delete;
