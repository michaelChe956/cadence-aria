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
use crate::cross_cutting::image_client::{ImageCallTarget, ImageClientError, ImageGenOutcome};
use crate::cross_cutting::streaming_provider::{
    FakeStreamingProvider, ProviderCommand, ProviderCompletion, ProviderEvent, ProviderSession,
    StreamingProviderAdapter, StreamingProviderInput,
};
use crate::product::image_create::models::{
    ApiKeyAction, CreateSessionRequest, DeleteLease, ImageBackground, ImageCreateSession,
    ImageCreateSettings, ImageOutputFormat, ImageQuality, ImageSize, ImageVendor, MaskedSettings,
    PresetTemplate, ReferenceSource, ResolvedTarget, SessionSummary, SettingsUpdate,
    SettingsUpdateRequest, TemplateChoice,
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
    async fn validate_settings(
        &self,
        _settings: &ImageCreateSettings,
    ) -> Result<(), ImageCreateError> {
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
    targets: AsyncMutex<Vec<ImageCallTarget>>,
    references: AsyncMutex<Vec<Option<ImageRefImage>>>,
    outcomes: AsyncMutex<VecDeque<Result<ImageGenOutcome, ImageClientError>>>,
}

impl FakeImageClient {
    fn success() -> Self {
        Self::with_outcomes(vec![ok_outcome()])
    }

    fn with_outcomes(outcomes: Vec<Result<ImageGenOutcome, ImageClientError>>) -> Self {
        Self {
            targets: AsyncMutex::new(Vec::new()),
            references: AsyncMutex::new(Vec::new()),
            outcomes: AsyncMutex::new(outcomes.into()),
        }
    }

    async fn calls(&self) -> usize {
        self.targets.lock().await.len()
    }

    async fn last_target(&self) -> ImageCallTarget {
        self.targets
            .lock()
            .await
            .last()
            .expect("target recorded")
            .clone()
    }
}

#[async_trait]
impl ImageClientApi for FakeImageClient {
    async fn generate(
        &self,
        target: &ImageCallTarget,
        _req: &ImageGenRequest,
        reference: Option<ImageRefImage>,
    ) -> Result<ImageGenOutcome, ImageClientError> {
        self.targets.lock().await.push(target.clone());
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
    // 两家各配一把 key：用于断言「key 只发往本家」的隔离。
    let mut settings = ImageCreateSettings::default();
    let gpt = settings.vendors.get_mut(ImageVendor::Gpt);
    gpt.base_url = "https://gpt.example.com".to_string();
    gpt.api_key = "sk-gpt".to_string();
    let minimax = settings.vendors.get_mut(ImageVendor::Minimax);
    minimax.base_url = "https://minimax.example.com".to_string();
    minimax.api_key = "sk-minimax".to_string();
    settings
}

fn request(prompt: &str) -> ImageGenRequest {
    ImageGenRequest {
        prompt: prompt.to_string(),
        size: ImageSize::Square,
        quality: ImageQuality::High,
        background: ImageBackground::Opaque,
        output_format: ImageOutputFormat::Png,
    }
}

fn png() -> Vec<u8> {
    let mut bytes = Vec::new();
    PngEncoder::new(&mut bytes)
        .write_image(&[0xff], 1, 1, ExtendedColorType::L8)
        .expect("encode png");
    bytes
}

/// 生成结果图（与上传参考图 png() 内容不同，用于断言上传图不落盘）。
fn result_png() -> Vec<u8> {
    let mut bytes = Vec::new();
    PngEncoder::new(&mut bytes)
        .write_image(&[0x00, 0x7f, 0x7f, 0x00], 2, 2, ExtendedColorType::L8)
        .expect("encode result png");
    bytes
}

fn ok_outcome() -> Result<ImageGenOutcome, ImageClientError> {
    Ok(ImageGenOutcome {
        media_type: "image/png".to_string(),
        b64: general_purpose::STANDARD.encode(result_png()),
    })
}

const GPT_TEST_MODEL: &str = "gpt-image-2.6";
const MINIMAX_TEST_MODEL: &str = "MiniMax-H3-Image";

fn model_command(
    req: ImageGenRequest,
    model: &str,
    use_previous: bool,
    reference: Option<ImageRefImage>,
) -> GenerateCommand {
    GenerateCommand {
        request: req,
        model: model.to_string(),
        use_previous,
        reference,
    }
}

/// 默认命令：手动选 GPT 家清单内的模型。
fn gpt_command(
    req: ImageGenRequest,
    use_previous: bool,
    reference: Option<ImageRefImage>,
) -> GenerateCommand {
    model_command(req, GPT_TEST_MODEL, use_previous, reference)
}

async fn generate_ok(engine: &ImageCreateEngine, use_previous: bool) -> GenerationResult {
    engine
        .generate("session", gpt_command(request("draw"), use_previous, None))
        .await
        .expect("generate")
}

async fn generate_err(engine: &ImageCreateEngine, command: GenerateCommand) -> ImageCreateError {
    engine
        .generate("session", command)
        .await
        .expect_err("generate")
}

async fn results(store: &FakeSessionStore) -> Vec<GenerationResult> {
    store
        .record
        .lock()
        .await
        .as_ref()
        .expect("record")
        .generation_results
        .clone()
}

async fn push_result(store: &FakeSessionStore, result: GenerationResult) {
    store
        .record
        .lock()
        .await
        .as_mut()
        .expect("record")
        .generation_results
        .push(result);
}

/// 第 1 次成功生成后其结果图的落盘路径。
async fn first_image_path(
    store: &FakeSessionStore,
    engine: &ImageCreateEngine,
) -> std::path::PathBuf {
    let image_id = results(store).await[0]
        .image_id
        .clone()
        .expect("第 1 次结果落盘");
    engine
        .paths
        .image_create_image_file(&image_id, "image/png")
        .expect("第 1 张图路径")
}

fn count_image_files(engine: &ImageCreateEngine) -> usize {
    std::fs::read_dir(engine.paths.image_create_images_dir())
        .map(|entries| entries.filter_map(Result::ok).count())
        .unwrap_or(0)
}

/// 旧记录：无 image_id、无内联 b64；vendor 为 None 表示旧形态缺字段。
fn legacy_result(vendor: Option<ImageVendor>) -> GenerationResult {
    GenerationResult {
        prompt: "旧提示词".to_string(),
        params: DefaultParams::default(),
        media_type: "image/png".to_string(),
        image_id: None,
        b64: None,
        vendor,
        model: None,
        reference_source: None,
        ts: Utc::now(),
    }
}

fn resolved_target(vendor: ImageVendor) -> ResolvedTarget {
    ResolvedTarget {
        vendor,
        model: GPT_TEST_MODEL.to_string(),
    }
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
        missing
            .generate("session", gpt_command(request("draw"), false, None))
            .await,
        Err(ImageCreateError::MissingConfig(_))
    ));

    let invalid = engine(store, valid_settings(), client.clone(), None);
    assert!(matches!(
        invalid
            .generate("session", gpt_command(request("   "), false, None))
            .await,
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
            .generate(
                "session",
                gpt_command(request("draw"), false, reference.clone()),
            )
            .await
            .expect("generate");
        assert!(result.image_id.is_some());
        assert!(result.b64.is_none());
        assert_eq!(
            client.references.lock().await[0].is_some(),
            reference.is_some()
        );
        assert_eq!(results(&store).await.len(), 1);
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

    let target = resolved_target(ImageVendor::Gpt);
    let result = engine
        .finish_generation(
            "session",
            &request("draw"),
            &target,
            ReferenceSource::None,
            outcome,
        )
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

    let target = resolved_target(ImageVendor::Gpt);
    let error = engine
        .finish_generation(
            "session",
            &request("draw"),
            &target,
            ReferenceSource::None,
            outcome,
        )
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
        engine
            .generate("session", gpt_command(request("draw"), false, None))
            .await,
        Err(ImageCreateError::Store(_))
    ));
    assert!(results(&store).await.is_empty());
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
        engine
            .generate("session", gpt_command(request("   "), false, None))
            .await,
        Err(ImageCreateError::InvalidParam(_))
    ));
    assert_eq!(results(&store).await.len(), 1);
    assert_eq!(
        results(&store).await[0].image_id.as_deref(),
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

// ---------- E-02…E-16：手动选模型、key 隔离、携带上一张图、结果记录 ----------

fn engine_with_outcomes(
    outcomes: Vec<Result<ImageGenOutcome, ImageClientError>>,
) -> (
    Arc<FakeSessionStore>,
    Arc<FakeImageClient>,
    ImageCreateEngine,
) {
    let store = Arc::new(FakeSessionStore::with_record());
    let client = Arc::new(FakeImageClient::with_outcomes(outcomes));
    let engine = engine(store.clone(), valid_settings(), client.clone(), None);
    (store, client, engine)
}

fn engine_under_test(
    successes: usize,
) -> (
    Arc<FakeSessionStore>,
    Arc<FakeImageClient>,
    ImageCreateEngine,
) {
    engine_with_outcomes((0..successes).map(|_| ok_outcome()).collect())
}

/// E-02：key 隔离 —— 每次出站目标的 base_url / api_key 属于手动选定的目标家。
#[tokio::test]
async fn e02_outbound_target_carries_only_target_vendor_credentials() {
    let (_, client, engine) = engine_under_test(2);
    engine
        .generate(
            "session",
            model_command(request("draw"), GPT_TEST_MODEL, false, None),
        )
        .await
        .expect("第 1 次生成");
    engine
        .generate(
            "session",
            model_command(request("draw"), MINIMAX_TEST_MODEL, false, None),
        )
        .await
        .expect("第 2 次生成");
    let targets = client.targets.lock().await;
    assert_eq!(targets[0].base_url, "https://gpt.example.com");
    assert_eq!(targets[0].api_key, "sk-gpt");
    assert_eq!(targets[1].base_url, "https://minimax.example.com");
    assert_eq!(targets[1].api_key, "sk-minimax");
}

/// E-03：失败不重试 —— 失败那次客户端只被调用 1 次，且不落盘成功结果。
#[tokio::test]
async fn e03_failed_generation_does_not_retry() {
    let (store, client, engine) = engine_with_outcomes(vec![
        ok_outcome(),
        Err(ImageClientError::HttpStatus {
            code: 500,
            body: "boom".to_string(),
        }),
    ]);
    generate_ok(&engine, false).await;
    let error = generate_err(&engine, gpt_command(request("draw"), false, None)).await;
    assert!(matches!(error, ImageCreateError::ImageClient(_)));
    assert_eq!(
        client.calls().await,
        2,
        "失败那次客户端只被调用 1 次，无重试、不换家"
    );
    assert_eq!(results(&store).await.len(), 1, "失败不落盘成功结果");
}

/// E-06：use_previous=true 且已有 1 条结果 → reference 字节等于上一张图片文件内容。
#[tokio::test]
async fn e06_use_previous_carries_last_generated_image_file() {
    let (store, client, engine) = engine_under_test(2);
    generate_ok(&engine, false).await;
    let second = generate_ok(&engine, true).await;
    assert_eq!(second.reference_source, Some(ReferenceSource::Previous));
    let reference = client.references.lock().await[1]
        .clone()
        .expect("携带上一张图");
    assert_eq!(
        reference.bytes,
        tokio::fs::read(first_image_path(&store, &engine).await)
            .await
            .expect("上一张图文件")
    );
    assert_eq!(reference.declared_mime, "image/png");
}

/// E-07：use_previous=true 但会话无结果 → 无 reference，来源记 none。
#[tokio::test]
async fn e07_use_previous_without_results_sends_no_reference() {
    let (_, client, engine) = engine_under_test(1);
    let result = generate_ok(&engine, true).await;
    assert!(client.references.lock().await[0].is_none());
    assert_eq!(result.reference_source, Some(ReferenceSource::None));
}

/// E-08：use_previous=true 且带上传图 → 上传图优先，来源记 uploaded。
#[tokio::test]
async fn e08_uploaded_reference_takes_priority_over_previous() {
    let (_, client, engine) = engine_under_test(2);
    generate_ok(&engine, false).await;
    let command = gpt_command(
        request("draw"),
        true,
        Some(ImageRefImage {
            bytes: png(),
            declared_mime: "image/png".to_string(),
        }),
    );
    let second = engine
        .generate("session", command)
        .await
        .expect("第 2 次生成");
    assert_eq!(second.reference_source, Some(ReferenceSource::Uploaded));
    assert_eq!(
        client.references.lock().await[1]
            .as_ref()
            .expect("携带上传图")
            .bytes,
        png()
    );
}

/// E-09：use_previous=false 且已有结果 → 无 reference。
#[tokio::test]
async fn e09_use_previous_false_sends_no_reference() {
    let (_, client, engine) = engine_under_test(2);
    generate_ok(&engine, false).await;
    let second = generate_ok(&engine, false).await;
    assert!(client.references.lock().await[1].is_none());
    assert_eq!(second.reference_source, Some(ReferenceSource::None));
}

/// E-10：上一张图文件被删且无内联 b64 → PreviousImageUnavailable，零出站。
#[tokio::test]
async fn e10_missing_previous_image_fails_before_outbound() {
    let (store, client, engine) = engine_under_test(2);
    generate_ok(&engine, false).await;
    let file = first_image_path(&store, &engine).await;
    tokio::fs::remove_file(&file)
        .await
        .expect("删除上一张图文件");
    let error = generate_err(&engine, gpt_command(request("draw"), true, None)).await;
    assert!(matches!(
        error,
        ImageCreateError::PreviousImageUnavailable(_)
    ));
    assert!(error.to_string().contains("图片文件缺失或无法读取"));
    assert_eq!(client.calls().await, 1, "出站前失败，客户端只被第 1 次调用");
    assert_eq!(results(&store).await.len(), 1);
    let record = store.record.lock().await;
    let record = record.as_ref().expect("record");
    assert!(
        !record
            .events
            .iter()
            .any(|event| event.kind == "generation_error"),
        "出站前错误不写 generation_error 事件"
    );
}

/// E-11：上一张为旧记录内联 b64 → reference 字节等于解码结果。
#[tokio::test]
async fn e11_previous_image_falls_back_to_inline_b64() {
    let (store, client, engine) = engine_under_test(1);
    let mut legacy = legacy_result(Some(ImageVendor::Gpt));
    legacy.b64 = Some(general_purpose::STANDARD.encode(png()));
    push_result(&store, legacy).await;
    let result = generate_ok(&engine, true).await;
    assert_eq!(result.reference_source, Some(ReferenceSource::Previous));
    assert_eq!(
        client.references.lock().await[0]
            .as_ref()
            .expect("携带上一张图")
            .bytes,
        png()
    );
}

/// E-12：显式模型不在清单 → InvalidParam，零出站。
#[tokio::test]
async fn e12_explicit_model_not_in_catalog_fails_before_outbound() {
    let (_, client, engine) = engine_under_test(1);
    let command = model_command(request("draw"), "no-such-model-anywhere", false, None);
    let error = generate_err(&engine, command).await;
    assert!(matches!(error, ImageCreateError::InvalidParam(_)));
    assert!(error.to_string().contains("no-such-model-anywhere"));
    assert_eq!(client.calls().await, 0, "出站前失败，客户端零调用");
}

/// E-13：携带链跨家 —— 第 1 次选 GPT 模型，第 2 次选 MiniMax 模型 + use_previous
/// → 第 2 次出站 MiniMax 且携带第 1 张图。
#[tokio::test]
async fn e13_previous_image_carries_across_vendor_switch() {
    let (store, client, engine) = engine_under_test(2);
    generate_ok(&engine, false).await;
    let file_bytes = tokio::fs::read(first_image_path(&store, &engine).await)
        .await
        .expect("第 1 张图");
    let second = engine
        .generate(
            "session",
            model_command(request("draw"), MINIMAX_TEST_MODEL, true, None),
        )
        .await
        .expect("第 2 次生成");
    assert_eq!(second.vendor, Some(ImageVendor::Minimax));
    assert_eq!(client.last_target().await.vendor, ImageVendor::Minimax);
    assert_eq!(
        client.references.lock().await[1]
            .as_ref()
            .expect("携带第 1 张图")
            .bytes,
        file_bytes
    );
}

/// E-14：上传参考图不落盘 —— 带上传图生成后图片目录只新增 1 个结果文件。
#[tokio::test]
async fn e14_uploaded_reference_is_not_persisted() {
    let (_, _client, engine) = engine_under_test(1);
    let before = count_image_files(&engine);
    let command = gpt_command(
        request("draw"),
        false,
        Some(ImageRefImage {
            bytes: png(),
            declared_mime: "image/png".to_string(),
        }),
    );
    let result = engine.generate("session", command).await.expect("generate");
    assert_eq!(result.reference_source, Some(ReferenceSource::Uploaded));
    assert_eq!(
        count_image_files(&engine),
        before + 1,
        "只新增 1 个结果文件"
    );
    let image_id = result.image_id.expect("结果图已落盘");
    let file = engine
        .paths
        .image_create_image_file(&image_id, "image/png")
        .expect("结果图路径");
    assert_ne!(
        tokio::fs::read(&file).await.expect("结果文件"),
        png(),
        "结果文件不是上传图"
    );
}

/// E-15：上一张图超过校验上限 → 出站前失败，原因含校验具体原因，零出站。
#[tokio::test]
async fn e15_previous_image_over_validation_limit_fails_before_outbound() {
    let (store, client, engine) = engine_under_test(1);
    let image_id = uuid::Uuid::new_v4().to_string();
    let file = engine
        .paths
        .image_create_image_file(&image_id, "image/png")
        .expect("路径");
    std::fs::create_dir_all(file.parent().expect("父目录")).expect("建目录");
    std::fs::write(&file, vec![0u8; 10 * 1024 * 1024 + 1]).expect("写超限图");
    let mut previous = legacy_result(Some(ImageVendor::Gpt));
    previous.image_id = Some(image_id);
    push_result(&store, previous).await;
    let error = generate_err(&engine, gpt_command(request("draw"), true, None)).await;
    assert!(matches!(
        error,
        ImageCreateError::PreviousImageUnavailable(_)
    ));
    assert!(
        error.to_string().contains("exceeds the 10 MiB size limit"),
        "原因含校验具体原因：{error}"
    );
    assert_eq!(client.calls().await, 0, "出站前失败，客户端零调用");
}

/// E-16：客户端错误的可观测性 —— 错误文本与 generation_error 事件文本同带「[家 · 模型] 」前缀。
#[tokio::test]
async fn e16_client_error_is_prefixed_with_vendor_and_model() {
    let (store, _client, engine) = engine_with_outcomes(vec![
        ok_outcome(),
        Err(ImageClientError::HttpStatus {
            code: 502,
            body: "upstream unavailable".to_string(),
        }),
    ]);
    generate_ok(&engine, false).await;
    let error = generate_err(
        &engine,
        model_command(request("draw"), MINIMAX_TEST_MODEL, false, None),
    )
    .await;
    let ImageCreateError::ImageClient(message) = &error else {
        panic!("客户端错误应包装为 ImageClient：{error:?}")
    };
    assert!(
        message.starts_with(&format!("[MiniMax · {MINIMAX_TEST_MODEL}] ")),
        "前缀缺失：{message}"
    );
    let record = store.record.lock().await;
    let record = record.as_ref().expect("record");
    assert_eq!(
        record
            .events
            .iter()
            .find(|event| event.kind == "generation_error")
            .expect("generation_error 事件")
            .message,
        *message,
        "事件文本与错误文本一致"
    );
}

#[path = "engine_hardening_tests.rs"]
mod engine_hardening_tests;
