use std::collections::HashMap;
use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose};
use chrono::Utc;
use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot};

use crate::cross_cutting::aria_state_paths::AriaStatePaths;
use crate::cross_cutting::image_client::{
    ImageCallTarget, ImageClientApi, ImageGenOutcome, ImageGenRequest, ImageRefImage,
};
use crate::cross_cutting::image_reference_validation::validate_reference_image;
use crate::cross_cutting::provider_registry::ProviderRegistry;

use super::models::{
    ChatMessage, DEFAULT_GPT_ROTATION_MODEL, DefaultParams, GenerationResult, ImageCreateError,
    ImageVendor, IterationEvent, ModelChoice, PromptBlock, RunKind, SessionEvent, SessionRecord,
    SessionStatus, SessionStoreApi, SettingsStoreApi,
};
use super::prompt_iteration::PromptIterationEngine;
use super::run_registry::ImageCreateRunRegistry;

#[derive(Debug, Clone)]
struct PendingResult {
    result: GenerationResult,
}

/// 一次图片生成的命令（设计 5.5）：请求参数、模型选择、是否携带上一张生成图、上传的参考图。
#[derive(Debug, Clone)]
pub struct GenerateCommand {
    pub request: ImageGenRequest,
    pub model: ModelChoice,
    pub use_previous: bool,
    /// 用户上传的参考图（已过 handler 的内容校验）。
    pub reference: Option<ImageRefImage>,
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
        command: GenerateCommand,
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
                engine.run_generation(&id, command, cancel).await
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
        command: GenerateCommand,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<GenerationResult, ImageCreateError> {
        // 临时过渡（仅本提交）：解构命令保持既有行为；轮转与携带上一张图随后落地。
        let GenerateCommand {
            request: req,
            model: _,
            use_previous: _,
            reference,
        } = command;
        let settings = self.settings_store.load().await;
        // 临时过渡：仍按单家（GPT 家）读取凭据；Task 6 将替换为按 resolve_target 解析出的目标。
        let gpt = settings.vendors.get(ImageVendor::Gpt);
        if gpt.base_url.trim().is_empty() || gpt.api_key.trim().is_empty() {
            return Err(ImageCreateError::MissingConfig(
                "GPT 尚未配置 Base URL 或 API Key".to_string(),
            ));
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

        // 临时过渡：构造仅含 GPT 家凭据与默认轮转模型的出站目标；
        // Task 6 将按 resolve_target 解析目标并实现双家轮转。
        let target = ImageCallTarget {
            vendor: ImageVendor::Gpt,
            base_url: gpt.base_url.clone(),
            api_key: gpt.api_key.clone(),
            model: DEFAULT_GPT_ROTATION_MODEL.to_string(),
        };

        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(ImageCreateError::SessionClosing),
            outcome = self.image_client.generate(&target, &req, reference) => {
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
                    // 供应方标注字段：Task 6 起随轮转解析填充，本 Task 先补 None 恢复编译。
                    vendor: None,
                    model: None,
                    selection: None,
                    reference_source: None,
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
#[path = "engine_tests.rs"]
mod engine_tests;
