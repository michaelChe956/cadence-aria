//! engine.rs 的外置测试模块：经 engine.rs 内 `#[path = "engine_tests.rs"]` 引入，
//! `use super::*` 语义与原内嵌 `mod tests` 完全一致（纯移动，逐字外迁）。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use base64::engine::general_purpose;
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::cross_cutting::image_client::{ImageClientError, ImageGenOutcome};
use crate::cross_cutting::streaming_provider::{
    FakeStreamingProvider, ProviderCommand, ProviderCompletion, ProviderEvent, ProviderSession,
    StreamingProviderAdapter, StreamingProviderInput,
};
use crate::product::image_create::models::{
    ApiKeyAction, CreateSessionRequest, DeleteLease, ImageBackground, ImageCreateSession,
    ImageCreateSettings, ImageOutputFormat, ImageQuality, ImageSize, MaskedSettings,
    PresetTemplate, SessionSummary, SettingsUpdate, SettingsUpdateRequest, TemplateChoice,
};
use crate::product::models::ProviderName;
use crate::protocol::contracts::ProviderType;

#[derive(Default)]
struct FakeSessionStore {
    record: AsyncMutex<Option<SessionRecord>>,
    operations: AsyncMutex<Vec<String>>,
    fail_generation_writes: AtomicUsize,
    finish_called: AtomicBool,
}

impl FakeSessionStore {
    fn with_record() -> Self {
        Self {
            record: AsyncMutex::new(Some(test_record())),
            ..Self::default()
        }
    }

    fn with_record_for(provider_name: ProviderName) -> Self {
        let mut record = test_record();
        record.session.provider_name = provider_name;
        Self {
            record: AsyncMutex::new(Some(record)),
            ..Self::default()
        }
    }

    fn fail_generation_writes(count: usize) -> Self {
        Self {
            record: AsyncMutex::new(Some(test_record())),
            fail_generation_writes: AtomicUsize::new(count),
            ..Self::default()
        }
    }
}

#[async_trait]
impl SessionStoreApi for FakeSessionStore {
    async fn create(
        &self,
        _req: CreateSessionRequest,
    ) -> Result<ImageCreateSession, ImageCreateError> {
        unimplemented!()
    }

    async fn list(&self) -> Result<Vec<SessionSummary>, ImageCreateError> {
        unimplemented!()
    }

    async fn get(&self, _id: &str) -> Result<Option<SessionRecord>, ImageCreateError> {
        Ok(self.record.lock().await.clone())
    }

    async fn append_message(&self, _id: &str, msg: ChatMessage) -> Result<(), ImageCreateError> {
        self.operations.lock().await.push("message".to_string());
        self.record
            .lock()
            .await
            .as_mut()
            .ok_or(ImageCreateError::SessionNotFound)?
            .messages
            .push(msg);
        Ok(())
    }

    async fn append_prompt_block(
        &self,
        _id: &str,
        block: PromptBlock,
    ) -> Result<(), ImageCreateError> {
        self.operations.lock().await.push("prompt".to_string());
        self.record
            .lock()
            .await
            .as_mut()
            .ok_or(ImageCreateError::SessionNotFound)?
            .prompt_blocks
            .push(block);
        Ok(())
    }

    async fn append_generation_result(
        &self,
        _id: &str,
        result: GenerationResult,
    ) -> Result<(), ImageCreateError> {
        let remaining = self.fail_generation_writes.load(Ordering::SeqCst);
        if remaining > 0 {
            self.fail_generation_writes.fetch_sub(1, Ordering::SeqCst);
            return Err(ImageCreateError::Store("write failed".to_string()));
        }
        self.record
            .lock()
            .await
            .as_mut()
            .ok_or(ImageCreateError::SessionNotFound)?
            .generation_results
            .push(result);
        Ok(())
    }

    async fn append_event(&self, _id: &str, event: SessionEvent) -> Result<(), ImageCreateError> {
        self.record
            .lock()
            .await
            .as_mut()
            .ok_or(ImageCreateError::SessionNotFound)?
            .events
            .push(event);
        Ok(())
    }

    async fn update_session_meta(
        &self,
        _id: &str,
        current_prompt: Option<String>,
        last_provider_session_id: Option<String>,
    ) -> Result<(), ImageCreateError> {
        self.operations.lock().await.push("meta".to_string());
        let mut record = self.record.lock().await;
        let session = &mut record
            .as_mut()
            .ok_or(ImageCreateError::SessionNotFound)?
            .session;
        if current_prompt.is_some() {
            session.current_prompt = current_prompt;
        }
        if last_provider_session_id.is_some() {
            session.last_provider_session_id = last_provider_session_id;
        }
        Ok(())
    }

    async fn begin_delete(&self, id: &str) -> Result<Option<DeleteLease>, ImageCreateError> {
        let mut record = self.record.lock().await;
        let Some(record) = record.as_mut() else {
            return Ok(None);
        };
        record.session.status = SessionStatus::Deleting;
        record.generation += 1;
        Ok(Some(DeleteLease {
            id: id.to_string(),
            token: record.generation,
        }))
    }

    async fn finish_delete(&self, _lease: DeleteLease) -> Result<(), ImageCreateError> {
        *self.record.lock().await = None;
        self.finish_called.store(true, Ordering::SeqCst);
        Ok(())
    }
}

struct FakeSettingsStore(ImageCreateSettings);

#[async_trait]
impl SettingsStoreApi for FakeSettingsStore {
    async fn load(&self) -> ImageCreateSettings {
        self.0.clone()
    }
    async fn save(&self, _settings: &ImageCreateSettings) -> Result<(), ImageCreateError> {
        unimplemented!()
    }
    async fn to_masked(&self, _settings: &ImageCreateSettings) -> MaskedSettings {
        unimplemented!()
    }
    async fn validate_base_url(&self, _url: &str) -> Result<(), ImageCreateError> {
        unimplemented!()
    }
    async fn apply_update(
        &self,
        _current: &ImageCreateSettings,
        _update: SettingsUpdate,
    ) -> ImageCreateSettings {
        unimplemented!()
    }
    async fn from_request(&self, _req: SettingsUpdateRequest) -> SettingsUpdate {
        unimplemented!()
    }
}

struct FakeImageClient {
    references: AsyncMutex<Vec<Option<ImageRefImage>>>,
    outcomes: AsyncMutex<VecDeque<Result<ImageGenOutcome, ImageClientError>>>,
}

impl FakeImageClient {
    fn success() -> Self {
        Self {
            references: AsyncMutex::new(Vec::new()),
            outcomes: AsyncMutex::new(
                [Ok(ImageGenOutcome {
                    media_type: "image/png".to_string(),
                    b64: "AAAA".to_string(),
                })]
                .into(),
            ),
        }
    }
}

#[async_trait]
impl ImageClientApi for FakeImageClient {
    async fn generate(
        &self,
        _settings: &ImageCreateSettings,
        _req: &ImageGenRequest,
        reference: Option<ImageRefImage>,
    ) -> Result<ImageGenOutcome, ImageClientError> {
        self.references.lock().await.push(reference);
        self.outcomes
            .lock()
            .await
            .pop_front()
            .expect("scripted outcome")
    }
}

struct ScriptedIterationProvider {
    suggested_prompt: Option<String>,
}

#[async_trait]
impl StreamingProviderAdapter for ScriptedIterationProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, crate::cross_cutting::provider_adapter::ProviderAdapterError> {
        let contract = input.structured_output_contract.as_ref().expect("contract");
        let full_output = match &self.suggested_prompt {
            Some(prompt) => format!(
                "readable\n<ARIA_STRUCTURED_OUTPUT nonce=\"{}\">\n{{\"nonce\":\"{}\",\"suggested_prompt\":\"{}\"}}\n</ARIA_STRUCTURED_OUTPUT>",
                contract.nonce, contract.nonce, prompt
            ),
            None => "readable".to_string(),
        };
        let completion = ProviderCompletion::from_output(
            full_output.clone(),
            Some(contract),
            Some("provider-session".to_string()),
        );
        let (event_tx, event_rx) = mpsc::channel(2);
        let (command_tx, _command_rx) = mpsc::channel::<ProviderCommand>(1);
        tokio::spawn(async move {
            let _ = event_tx.send(ProviderEvent::Completed(completion)).await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }
}

fn test_record() -> SessionRecord {
    SessionRecord {
        session: ImageCreateSession {
            id: "session".to_string(),
            provider_name: ProviderName::Fake,
            template: TemplateChoice {
                preset: Some(PresetTemplate::PptBusinessIllustration),
                custom: None,
            },
            last_provider_session_id: None,
            current_prompt: None,
            status: SessionStatus::Active,
            created_at: Utc::now(),
        },
        messages: Vec::new(),
        prompt_blocks: Vec::new(),
        generation_results: Vec::new(),
        events: Vec::new(),
        generation: 0,
    }
}

fn valid_settings() -> ImageCreateSettings {
    ImageCreateSettings {
        base_url: "https://images.example.com".to_string(),
        api_key: "sk-test".to_string(),
        defaults: DefaultParams::default(),
    }
}

fn request(prompt: &str) -> ImageGenRequest {
    ImageGenRequest {
        prompt: prompt.to_string(),
        size: ImageSize::Square,
        quality: ImageQuality::High,
        background: ImageBackground::Opaque,
        output_format: ImageOutputFormat::Png,
        input_fidelity: None,
    }
}

fn png() -> Vec<u8> {
    let mut bytes = Vec::new();
    PngEncoder::new(&mut bytes)
        .write_image(&[0xff], 1, 1, ExtendedColorType::L8)
        .expect("encode png");
    bytes
}

fn registry(provider: Option<Arc<dyn StreamingProviderAdapter>>) -> Arc<ProviderRegistry> {
    registry_for_provider(ProviderName::Fake, provider)
}

fn registry_for_provider(
    provider_name: ProviderName,
    provider: Option<Arc<dyn StreamingProviderAdapter>>,
) -> Arc<ProviderRegistry> {
    let mut registry = ProviderRegistry::new();
    registry.register(
        provider_name,
        provider.unwrap_or_else(|| Arc::new(FakeStreamingProvider)),
    );
    Arc::new(registry)
}

fn engine(
    store: Arc<FakeSessionStore>,
    settings: ImageCreateSettings,
    client: Arc<dyn ImageClientApi>,
    provider: Option<Arc<dyn StreamingProviderAdapter>>,
) -> ImageCreateEngine {
    let root = tempfile::tempdir().expect("root").keep();
    ImageCreateEngine::new(
        AriaStatePaths::from_workspace_root(root),
        store,
        Arc::new(FakeSettingsStore(settings)),
        client,
        registry(provider),
        Arc::new(ImageCreateRunRegistry::default()),
    )
}

fn engine_for_provider(
    store: Arc<FakeSessionStore>,
    settings: ImageCreateSettings,
    client: Arc<dyn ImageClientApi>,
    provider_name: ProviderName,
    provider: Arc<dyn StreamingProviderAdapter>,
) -> ImageCreateEngine {
    let root = tempfile::tempdir().expect("root").keep();
    ImageCreateEngine::new(
        AriaStatePaths::from_workspace_root(root),
        store,
        Arc::new(FakeSettingsStore(settings)),
        client,
        registry_for_provider(provider_name, Some(provider)),
        Arc::new(ImageCreateRunRegistry::default()),
    )
}

#[tokio::test]
async fn generate_rejects_missing_config_and_invalid_prompt_without_client_call() {
    let store = Arc::new(FakeSessionStore::with_record());
    let client = Arc::new(FakeImageClient::success());
    let missing = engine(
        store.clone(),
        ImageCreateSettings::default(),
        client.clone(),
        None,
    );
    assert!(matches!(
        missing.generate("session", request("draw"), None).await,
        Err(ImageCreateError::MissingConfig)
    ));

    let invalid = engine(store, valid_settings(), client.clone(), None);
    assert!(matches!(
        invalid.generate("session", request("   "), None).await,
        Err(ImageCreateError::InvalidParam(_))
    ));
    assert!(client.references.lock().await.is_empty());
}

#[tokio::test]
async fn generate_routes_text_and_validated_reference_and_persists_success() {
    for reference in [
        None,
        Some(ImageRefImage {
            bytes: png(),
            declared_mime: "image/png".to_string(),
        }),
    ] {
        let store = Arc::new(FakeSessionStore::with_record());
        let client = Arc::new(FakeImageClient::success());
        let engine = engine(store.clone(), valid_settings(), client.clone(), None);
        let result = engine
            .generate("session", request("draw"), reference.clone())
            .await
            .expect("generate");
        assert!(result.image_id.is_some());
        assert!(result.b64.is_none());
        assert_eq!(
            client.references.lock().await[0].is_some(),
            reference.is_some()
        );
        assert_eq!(
            store
                .record
                .lock()
                .await
                .as_ref()
                .expect("record")
                .generation_results
                .len(),
            1
        );
    }
}

#[tokio::test]
async fn finish_generation_writes_image_file_before_appending_reference() {
    let store = Arc::new(FakeSessionStore::with_record());
    let engine = engine(
        store.clone(),
        valid_settings(),
        Arc::new(FakeImageClient::success()),
        None,
    );
    let outcome = Ok(ImageGenOutcome {
        media_type: "image/png".to_string(),
        b64: general_purpose::STANDARD.encode("img-bytes"),
    });

    let result = engine
        .finish_generation("session", &request("draw"), outcome)
        .await
        .expect("finish");
    let image_id = result.image_id.as_deref().expect("image id assigned");
    assert!(result.b64.is_none());
    let file = engine
        .paths
        .image_create_image_file(image_id, "image/png")
        .expect("path");
    assert_eq!(
        tokio::fs::read(&file).await.expect("file written"),
        b"img-bytes"
    );

    let record = store.record.lock().await;
    let record = record.as_ref().expect("record");
    assert_eq!(record.generation_results.len(), 1);
    assert_eq!(
        record.generation_results[0].image_id.as_deref(),
        Some(image_id)
    );
    assert!(record.generation_results[0].b64.is_none());
}

#[tokio::test]
async fn finish_generation_image_write_failure_keeps_record_clean() {
    let store = Arc::new(FakeSessionStore::with_record());
    let engine = engine(
        store.clone(),
        valid_settings(),
        Arc::new(FakeImageClient::success()),
        None,
    );
    std::fs::create_dir_all(
        engine
            .paths
            .image_create_images_dir()
            .parent()
            .expect("image create directory"),
    )
    .expect("create image create directory");
    std::fs::write(engine.paths.image_create_images_dir(), "not a directory")
        .expect("block images directory");
    let outcome = Ok(ImageGenOutcome {
        media_type: "image/png".to_string(),
        b64: general_purpose::STANDARD.encode("img-bytes"),
    });

    let error = engine
        .finish_generation("session", &request("draw"), outcome)
        .await
        .expect_err("must fail");
    assert!(matches!(error, ImageCreateError::Store(_)));
    let record = store.record.lock().await;
    let record = record.as_ref().expect("record");
    assert!(record.generation_results.is_empty(), "失败不得写入成功结果");
    assert!(
        record
            .events
            .iter()
            .any(|event| event.kind == "generation_error")
    );
}

#[tokio::test]
async fn generation_write_failure_returns_error_and_is_flushed_on_next_operation() {
    let store = Arc::new(FakeSessionStore::fail_generation_writes(1));
    let engine = engine(
        store.clone(),
        valid_settings(),
        Arc::new(FakeImageClient::success()),
        None,
    );
    assert!(matches!(
        engine.generate("session", request("draw"), None).await,
        Err(ImageCreateError::Store(_))
    ));
    assert!(
        store
            .record
            .lock()
            .await
            .as_ref()
            .expect("record")
            .generation_results
            .is_empty()
    );
    let image_id = engine
        .pending_results
        .lock()
        .await
        .get("session")
        .and_then(|pending| pending.first())
        .and_then(|pending| pending.result.image_id.as_deref())
        .expect("pending image id")
        .to_string();
    assert!(
        engine
            .paths
            .image_create_image_file(&image_id, "image/png")
            .expect("pending image path")
            .exists()
    );

    assert!(matches!(
        engine.generate("session", request("   "), None).await,
        Err(ImageCreateError::InvalidParam(_))
    ));
    assert_eq!(
        store
            .record
            .lock()
            .await
            .as_ref()
            .expect("record")
            .generation_results
            .len(),
        1
    );
    assert_eq!(
        store
            .record
            .lock()
            .await
            .as_ref()
            .expect("record")
            .generation_results[0]
            .image_id
            .as_deref(),
        Some(image_id.as_str())
    );
}

#[tokio::test]
async fn image_create_runs_with_kimi_provider() {
    let store = Arc::new(FakeSessionStore::with_record_for(ProviderName::KimiCode));
    let provider = Arc::new(ScriptedIterationProvider {
        suggested_prompt: Some("Kimi 图片 prompt".to_string()),
    });
    let engine = engine_for_provider(
        store.clone(),
        valid_settings(),
        Arc::new(FakeImageClient::success()),
        ProviderName::KimiCode,
        provider,
    );

    let mut events = engine
        .start_iteration("session", "用 Kimi 完善图片提示词".to_string())
        .await
        .expect("start Kimi image iteration");
    let mut done_session_id = None;
    while let Some(event) = events.recv().await {
        if event.kind == "done" {
            done_session_id = event.provider_session_id;
        }
    }

    assert_eq!(done_session_id.as_deref(), Some("provider-session"));
    assert_eq!(
        store
            .record
            .lock()
            .await
            .as_ref()
            .expect("record")
            .session
            .last_provider_session_id
            .as_deref(),
        Some("provider-session")
    );
}

#[tokio::test]
async fn iteration_persists_message_optional_prompt_and_meta_in_order() {
    for suggested_prompt in [Some("final prompt".to_string()), None] {
        let store = Arc::new(FakeSessionStore::with_record());
        let provider = Arc::new(ScriptedIterationProvider {
            suggested_prompt: suggested_prompt.clone(),
        });
        let engine = engine(
            store.clone(),
            valid_settings(),
            Arc::new(FakeImageClient::success()),
            Some(provider),
        );
        let mut events = engine
            .start_iteration("session", "make it blue".to_string())
            .await
            .expect("start iteration");
        while events.recv().await.is_some() {}

        let operations = store.operations.lock().await.clone();
        if suggested_prompt.is_some() {
            assert_eq!(operations, ["message", "prompt", "meta"]);
        } else {
            assert_eq!(operations, ["message", "meta"]);
        }
        let record = store.record.lock().await;
        let record = record.as_ref().expect("record");
        assert_eq!(record.messages[0].content, "make it blue");
        assert_eq!(
            record.prompt_blocks.len(),
            usize::from(suggested_prompt.is_some())
        );
        assert_eq!(
            record.session.last_provider_session_id.as_deref(),
            Some("provider-session")
        );
    }
}

#[tokio::test]
async fn delete_cancels_and_awaits_in_flight_run_before_finish() {
    let store = Arc::new(FakeSessionStore::with_record());
    let run_registry = Arc::new(ImageCreateRunRegistry::default());
    let reservation = run_registry
        .try_reserve("session", RunKind::Generate)
        .await
        .expect("reserve");
    let cancelled = Arc::new(Notify::new());
    let task_done = Arc::new(AtomicBool::new(false));
    let cancel = reservation.cancel.clone();
    let cancelled_signal = cancelled.clone();
    let task_done_signal = task_done.clone();
    let join = tokio::spawn(async move {
        cancel.cancelled().await;
        task_done_signal.store(true, Ordering::SeqCst);
        cancelled_signal.notify_one();
    });
    run_registry
        .attach_join("session", join)
        .await
        .expect("attach");

    let root = tempfile::tempdir().expect("root");
    let engine = ImageCreateEngine::new(
        AriaStatePaths::from_workspace_root(root.path()),
        store.clone(),
        Arc::new(FakeSettingsStore(valid_settings())),
        Arc::new(FakeImageClient::success()),
        registry(None),
        run_registry,
    );
    engine.delete_session("session").await.expect("delete");
    cancelled.notified().await;
    assert!(task_done.load(Ordering::SeqCst));
    assert!(store.finish_called.load(Ordering::SeqCst));
    assert!(store.record.lock().await.is_none());
}

#[test]
fn request_types_remain_enum_backed() {
    let req = request("draw");
    let _ = ApiKeyAction::Retain;
    let _: ProviderType = ProviderName::KimiCode.into();
    assert_eq!(req.size, ImageSize::Square);
}
