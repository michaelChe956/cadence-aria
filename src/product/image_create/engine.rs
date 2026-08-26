use std::collections::HashMap;
use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose};
use chrono::Utc;
use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot};

use crate::cross_cutting::aria_state_paths::AriaStatePaths;
use crate::cross_cutting::image_client::{
    ImageClientApi, ImageGenOutcome, ImageGenRequest, ImageRefImage,
};
use crate::cross_cutting::image_reference_validation::validate_reference_image;
use crate::cross_cutting::provider_registry::ProviderRegistry;

use super::models::{
    ChatMessage, DefaultParams, GenerationResult, ImageCreateError, IterationEvent, PromptBlock,
    RunKind, SessionEvent, SessionRecord, SessionStatus, SessionStoreApi, SettingsStoreApi,
};
use super::prompt_iteration::PromptIterationEngine;
use super::run_registry::ImageCreateRunRegistry;

#[derive(Debug, Clone)]
struct PendingResult {
    result: GenerationResult,
}

#[derive(Clone)]
pub struct ImageCreateEngine {
    paths: AriaStatePaths,
    session_store: Arc<dyn SessionStoreApi>,
    settings_store: Arc<dyn SettingsStoreApi>,
    image_client: Arc<dyn ImageClientApi>,
    iteration_registry: Arc<ProviderRegistry>,
    run_registry: Arc<ImageCreateRunRegistry>,
    pending_results: Arc<AsyncMutex<HashMap<String, Vec<PendingResult>>>>,
}

impl ImageCreateEngine {
    pub fn new(
        paths: AriaStatePaths,
        session_store: Arc<dyn SessionStoreApi>,
        settings_store: Arc<dyn SettingsStoreApi>,
        image_client: Arc<dyn ImageClientApi>,
        iteration_registry: Arc<ProviderRegistry>,
        run_registry: Arc<ImageCreateRunRegistry>,
    ) -> Self {
        Self {
            paths,
            session_store,
            settings_store,
            image_client,
            iteration_registry,
            run_registry,
            pending_results: Arc::new(AsyncMutex::new(HashMap::new())),
        }
    }

    pub fn iteration_registry(&self) -> &Arc<ProviderRegistry> {
        &self.iteration_registry
    }

    pub async fn start_iteration(
        &self,
        session_id: &str,
        message: String,
    ) -> Result<mpsc::Receiver<IterationEvent>, ImageCreateError> {
        self.active_record(session_id).await?;
        self.flush_pending(session_id).await?;
        let reservation = self
            .run_registry
            .try_reserve(session_id, RunKind::Iteration)
            .await
            .ok_or(ImageCreateError::SessionBusy)?;

        let (event_tx, event_rx) = mpsc::channel(8);
        let (registered_tx, registered_rx) = oneshot::channel();
        let engine = self.clone();
        let id = session_id.to_string();
        let cancel = reservation.cancel.clone();
        let handle = tokio::spawn(async move {
            let registered = tokio::select! {
                _ = registered_rx => true,
                _ = cancel.cancelled() => false,
            };
            if !registered {
                engine.run_registry.release(&id).await;
                return;
            }
            let result = tokio::select! {
                biased;
                _ = cancel.cancelled() => Err(ImageCreateError::SessionClosing),
                active = engine.active_record(&id) => match active {
                    Ok(record) => PromptIterationEngine::new(engine.iteration_registry.clone())
                        .iterate(&record, &message, &engine.paths, cancel.clone())
                        .await,
                    Err(error) => Err(error),
                }
            };

            match result {
                Ok(outcome) => {
                    let persisted = engine
                        .persist_iteration(
                            &id,
                            message,
                            outcome.suggested_prompt.clone(),
                            outcome.provider_session_id.clone(),
                        )
                        .await;
                    match persisted {
                        Ok(()) => {
                            // 有 suggested_prompt 时只推 prompt（provider 的解释文本对用户无价值，不展示）；
                            // 无 suggested_prompt 时才推 text 作为 fallback（让用户知道发生了什么）。
                            if outcome.suggested_prompt.is_none() {
                                let _ = event_tx
                                    .send(IterationEvent {
                                        kind: "text".to_string(),
                                        text: Some(outcome.readable_text),
                                        suggested_prompt: None,
                                        provider_session_id: None,
                                        error: None,
                                    })
                                    .await;
                            }
                            if let Some(prompt) = outcome.suggested_prompt {
                                let _ = event_tx
                                    .send(IterationEvent {
                                        kind: "prompt".to_string(),
                                        text: None,
                                        suggested_prompt: Some(prompt),
                                        provider_session_id: None,
                                        error: None,
                                    })
                                    .await;
                            }
                            let _ = event_tx
                                .send(IterationEvent {
                                    kind: "done".to_string(),
                                    text: None,
                                    suggested_prompt: None,
                                    provider_session_id: outcome.provider_session_id,
                                    error: None,
                                })
                                .await;
                        }
                        Err(error) => {
                            let _ = send_iteration_error(&event_tx, error).await;
                        }
                    }
                }
                Err(error) => {
                    let _ = send_iteration_error(&event_tx, error).await;
                }
            }
            engine.run_registry.release(&id).await;
        });

        self.run_registry.attach_join(session_id, handle).await?;
        let _ = registered_tx.send(());
        Ok(event_rx)
    }

    pub async fn generate(
        &self,
        session_id: &str,
        req: ImageGenRequest,
        reference: Option<ImageRefImage>,
    ) -> Result<GenerationResult, ImageCreateError> {
        self.active_record(session_id).await?;
        self.flush_pending(session_id).await?;
        let reservation = self
            .run_registry
            .try_reserve(session_id, RunKind::Generate)
            .await
            .ok_or(ImageCreateError::SessionBusy)?;

        let (result_tx, result_rx) = oneshot::channel();
        let (registered_tx, registered_rx) = oneshot::channel();
        let engine = self.clone();
        let id = session_id.to_string();
        let cancel = reservation.cancel.clone();
        let handle = tokio::spawn(async move {
            let registered = tokio::select! {
                _ = registered_rx => true,
                _ = cancel.cancelled() => false,
            };
            let result = if registered {
                engine.run_generation(&id, req, reference, cancel).await
            } else {
                Err(ImageCreateError::SessionClosing)
            };
            let _ = result_tx.send(result);
            engine.run_registry.release(&id).await;
        });

        self.run_registry.attach_join(session_id, handle).await?;
        let _ = registered_tx.send(());
        result_rx
            .await
            .map_err(|_| ImageCreateError::ImageClient("generation task stopped".to_string()))?
    }

    pub async fn delete_session(&self, session_id: &str) -> Result<(), ImageCreateError> {
        let lease = self
            .session_store
            .begin_delete(session_id)
            .await?
            .ok_or(ImageCreateError::SessionNotFound)?;

        if let Some(active) = self.run_registry.take(session_id).await {
            active.cancel.cancel();
            if let Some(join) = active.join {
                let _ = join.await;
            }
        }
        self.pending_results.lock().await.remove(session_id);
        self.session_store.finish_delete(lease).await
    }

    async fn active_record(&self, session_id: &str) -> Result<SessionRecord, ImageCreateError> {
        let record = self
            .session_store
            .get(session_id)
            .await?
            .ok_or(ImageCreateError::SessionNotFound)?;
        if record.session.status == SessionStatus::Deleting {
            return Err(ImageCreateError::SessionClosing);
        }
        Ok(record)
    }

    async fn flush_pending(&self, session_id: &str) -> Result<(), ImageCreateError> {
        let pending = self.pending_results.lock().await.remove(session_id);
        let Some(pending) = pending else {
            return Ok(());
        };

        let mut iter = pending.into_iter();
        while let Some(item) = iter.next() {
            if let Err(error) = self
                .session_store
                .append_generation_result(session_id, item.result.clone())
                .await
            {
                let mut remaining = vec![item];
                remaining.extend(iter);
                self.pending_results
                    .lock()
                    .await
                    .entry(session_id.to_string())
                    .or_default()
                    .extend(remaining);
                return Err(error);
            }
        }
        Ok(())
    }

    async fn persist_iteration(
        &self,
        session_id: &str,
        message: String,
        suggested_prompt: Option<String>,
        provider_session_id: Option<String>,
    ) -> Result<(), ImageCreateError> {
        let writes = async {
            self.session_store
                .append_message(
                    session_id,
                    ChatMessage {
                        role: "user".to_string(),
                        content: message,
                        ts: Utc::now(),
                    },
                )
                .await?;
            if let Some(prompt) = suggested_prompt.clone() {
                self.session_store
                    .append_prompt_block(
                        session_id,
                        PromptBlock {
                            content: prompt,
                            version: 0,
                        },
                    )
                    .await?;
            }
            self.session_store
                .update_session_meta(session_id, suggested_prompt, provider_session_id)
                .await
        }
        .await;

        match writes {
            Err(ImageCreateError::SessionClosing | ImageCreateError::SessionNotFound) => Ok(()),
            other => other,
        }
    }

    async fn run_generation(
        &self,
        session_id: &str,
        req: ImageGenRequest,
        reference: Option<ImageRefImage>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<GenerationResult, ImageCreateError> {
        let settings = self.settings_store.load().await;
        if settings.base_url.trim().is_empty() || settings.api_key.trim().is_empty() {
            return Err(ImageCreateError::MissingConfig);
        }
        if req.prompt.trim().is_empty() {
            return Err(ImageCreateError::InvalidParam(
                "prompt must not be empty".to_string(),
            ));
        }
        let reference = match reference {
            Some(reference) => {
                validate_reference_image(&reference.bytes, &reference.declared_mime)
                    .map_err(|error| ImageCreateError::RefImage(error.to_string()))?;
                Some(reference)
            }
            None => None,
        };

        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(ImageCreateError::SessionClosing),
            outcome = self.image_client.generate(&settings, &req, reference) => {
                self.finish_generation(session_id, &req, outcome).await
            }
        }
    }

    async fn finish_generation(
        &self,
        session_id: &str,
        req: &ImageGenRequest,
        outcome: Result<ImageGenOutcome, crate::cross_cutting::image_client::ImageClientError>,
    ) -> Result<GenerationResult, ImageCreateError> {
        match outcome {
            Ok(outcome) => {
                let image_id = uuid::Uuid::new_v4().to_string();
                let bytes = general_purpose::STANDARD
                    .decode(outcome.b64.as_bytes())
                    .map_err(|error| {
                        ImageCreateError::ImageClient(format!("invalid b64 from gateway: {error}"))
                    })?;
                let media_type = outcome.media_type;
                if let Err(error) = crate::product::image_create::image_files::write_image_atomic(
                    &self.paths,
                    &image_id,
                    &media_type,
                    &bytes,
                )
                .await
                {
                    let _ = self
                        .session_store
                        .append_event(
                            session_id,
                            SessionEvent {
                                kind: "generation_error".to_string(),
                                message: error.to_string(),
                                ts: Utc::now(),
                            },
                        )
                        .await;
                    return Err(error);
                }

                let result = GenerationResult {
                    prompt: req.prompt.clone(),
                    params: DefaultParams {
                        size: req.size,
                        quality: req.quality,
                        background: req.background,
                        output_format: req.output_format,
                    },
                    media_type,
                    image_id: Some(image_id),
                    b64: None,
                    ts: Utc::now(),
                };
                if let Err(error) = self
                    .session_store
                    .append_generation_result(session_id, result.clone())
                    .await
                {
                    self.pending_results
                        .lock()
                        .await
                        .entry(session_id.to_string())
                        .or_default()
                        .push(PendingResult { result });
                    return Err(error);
                }
                Ok(result)
            }
            Err(error) => {
                let message = error.to_string();
                let _ = self
                    .session_store
                    .append_event(
                        session_id,
                        SessionEvent {
                            kind: "generation_error".to_string(),
                            message: message.clone(),
                            ts: Utc::now(),
                        },
                    )
                    .await;
                Err(ImageCreateError::ImageClient(message))
            }
        }
    }
}

async fn send_iteration_error(
    sender: &mpsc::Sender<IterationEvent>,
    error: ImageCreateError,
) -> Result<(), mpsc::error::SendError<IterationEvent>> {
    sender
        .send(IterationEvent {
            kind: "error".to_string(),
            text: None,
            suggested_prompt: None,
            provider_session_id: None,
            error: Some(error.to_string()),
        })
        .await
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use async_trait::async_trait;
    use base64::{Engine as _, engine::general_purpose};
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

        async fn append_message(
            &self,
            _id: &str,
            msg: ChatMessage,
        ) -> Result<(), ImageCreateError> {
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

        async fn append_event(
            &self,
            _id: &str,
            event: SessionEvent,
        ) -> Result<(), ImageCreateError> {
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
        ) -> Result<ProviderSession, crate::cross_cutting::provider_adapter::ProviderAdapterError>
        {
            let contract = input.structured_output_contract.as_ref().expect("contract");
            let full_output = match &self.suggested_prompt {
                Some(prompt) => format!(
                    "readable\n<ARIA_STRUCTURED_OUTPUT nonce=\"{}\">\n{{\"suggested_prompt\":\"{}\"}}\n</ARIA_STRUCTURED_OUTPUT nonce=\"{}\">",
                    contract.nonce, prompt, contract.nonce
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
}
