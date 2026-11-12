use std::collections::HashMap;
use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose};
use chrono::Utc;
use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot};

use crate::cross_cutting::aria_state_paths::AriaStatePaths;
use crate::cross_cutting::image_client::{
    ImageCallTarget, ImageClientApi, ImageGenOutcome, ImageGenRequest, ImageRefImage,
    validate_base_url,
};
use crate::cross_cutting::image_reference_validation::validate_reference_image;
use crate::cross_cutting::provider_registry::ProviderRegistry;

use super::models::{
    ChatMessage, DefaultParams, GenerationResult, ImageCreateError, ImageCreateSettings,
    ImageVendor, IterationEvent, PromptBlock, ReferenceSource, ResolvedTarget, RunKind,
    SessionEvent, SessionRecord, SessionStatus, SessionStoreApi, SettingsStoreApi,
};
use super::prompt_iteration::PromptIterationEngine;
use super::run_registry::ImageCreateRunRegistry;

/// prompt 长度上限（Unicode 字符数）：两家官方上限均为 32000，
/// 超长在出站前拒绝，避免一次必然被上游拒收、可能计费的往返（H5）。
const MAX_PROMPT_CHARS: usize = 32000;

#[derive(Debug, Clone)]
struct PendingResult {
    result: GenerationResult,
}

/// 一次图片生成的命令（设计 5.5）：请求参数、手动选定的模型 id、
/// 是否携带上一张生成图、上传的参考图。
#[derive(Debug, Clone)]
pub struct GenerateCommand {
    pub request: ImageGenRequest,
    pub model: String,
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
                    // H2：迭代执行器的 cwd 在 .aria 之下，能读到明文 key 并写进输出；
                    // 落库与推事件前对两家 key 统一脱敏（不含 key 时逐字保留）。
                    let settings = engine.settings_store.load().await;
                    let readable_text = redact_api_keys(&outcome.readable_text, &settings);
                    let suggested_prompt = outcome
                        .suggested_prompt
                        .as_ref()
                        .map(|prompt| redact_api_keys(prompt, &settings));
                    let persisted = engine
                        .persist_iteration(
                            &id,
                            message,
                            suggested_prompt.clone(),
                            outcome.provider_session_id.clone(),
                        )
                        .await;
                    match persisted {
                        Ok(()) => {
                            // 有 suggested_prompt 时只推 prompt（provider 的解释文本对用户无价值，不展示）；
                            // 无 suggested_prompt 时才推 text 作为 fallback（让用户知道发生了什么）。
                            // text/done 都带本轮 prompt_declined：前端据此区分「模型明确拒答」
                            // 与「格式不符/无视契约」，拒答时提示排在 fallback 文本之前。
                            // 可读文本只剩空白时不推 text（前端会渲染出空的「模型说明」折叠块），只推 done。
                            if suggested_prompt.is_none() && !readable_text.trim().is_empty() {
                                let _ = event_tx
                                    .send(IterationEvent {
                                        kind: "text".to_string(),
                                        text: Some(readable_text),
                                        suggested_prompt: None,
                                        prompt_declined: outcome.prompt_declined,
                                        provider_session_id: None,
                                        error: None,
                                    })
                                    .await;
                            }
                            if let Some(prompt) = suggested_prompt {
                                let _ = event_tx
                                    .send(IterationEvent {
                                        kind: "prompt".to_string(),
                                        text: None,
                                        suggested_prompt: Some(prompt),
                                        prompt_declined: outcome.prompt_declined,
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
                                    prompt_declined: outcome.prompt_declined,
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
                            ts: Some(Utc::now()),
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
        // 出站前决策链（设计 5.7）：任一步失败都不产生出站请求，零费用。
        let settings = self.settings_store.load().await;
        if command.request.prompt.trim().is_empty() {
            return Err(ImageCreateError::InvalidParam(
                "prompt must not be empty".to_string(),
            ));
        }
        // 按字符数（非字节数）计上限，与两家官方限制口径一致（H5）。
        let prompt_chars = command.request.prompt.chars().count();
        if prompt_chars > MAX_PROMPT_CHARS {
            return Err(ImageCreateError::InvalidParam(format!(
                "prompt 长度超过 {MAX_PROMPT_CHARS} 字符上限（当前 {prompt_chars} 字符）"
            )));
        }
        // 末项在 generate() 的 flush_pending() 之后重新读取（D7）：
        // 写图成功但引用待补的结果已进入序列，上一张图以它为准。
        let record = self.active_record(session_id).await?;
        let last = record.generation_results.last().cloned();
        let target = settings.resolve_target(&command.model)?;
        // H3：出站前对本次目标家的 base_url 再校验一次（与保存时同规则）。手改设置
        // 文件后的非法值在此拦下：InvalidConfig → HTTP 400，且不调用客户端；
        // 客户端内部的同一校验保留，作为二次防线（D12.1）。
        validate_base_url(settings.vendors.get(target.vendor).base_url.trim())
            .map_err(|error| ImageCreateError::InvalidConfig(error.to_string()))?;

        // reference 决定（D7）：上传图优先 → use_previous 且有上一张 → 无。
        let (reference, reference_source) = match command.reference {
            Some(reference) => {
                validate_reference_image(&reference.bytes, &reference.declared_mime)
                    .map_err(|error| ImageCreateError::RefImage(error.to_string()))?;
                (Some(reference), ReferenceSource::Uploaded)
            }
            None if command.use_previous => match last {
                Some(last) => {
                    // 上一张图取不到（文件缺失且无内联 b64）→ 明确失败、不出站（D8）。
                    let bytes = crate::product::image_create::image_files::read_result_image(
                        &self.paths,
                        &last,
                    )
                    .await
                    .ok_or_else(|| {
                        ImageCreateError::PreviousImageUnavailable(
                            "图片文件缺失或无法读取".to_string(),
                        )
                    })?;
                    // 与上传图同一套校验；不过则以校验原因失败，同样不出站。
                    validate_reference_image(&bytes, &last.media_type).map_err(|error| {
                        ImageCreateError::PreviousImageUnavailable(error.to_string())
                    })?;
                    (
                        Some(ImageRefImage {
                            bytes,
                            declared_mime: last.media_type.clone(),
                        }),
                        ReferenceSource::Previous,
                    )
                }
                None => (None, ReferenceSource::None),
            },
            None => (None, ReferenceSource::None),
        };

        // 出站目标只含目标家的凭据：key 只发往本家 base_url（D12）。
        let vendor_settings = settings.vendors.get(target.vendor);
        let call_target = ImageCallTarget {
            vendor: target.vendor,
            base_url: vendor_settings.base_url.clone(),
            api_key: vendor_settings.api_key.clone(),
            model: target.model.clone(),
        };

        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(ImageCreateError::SessionClosing),
            outcome = self.image_client.generate(&call_target, &command.request, reference) => {
                self.finish_generation(
                    session_id,
                    &command.request,
                    &target,
                    reference_source,
                    outcome,
                )
                .await
            }
        }
    }

    async fn finish_generation(
        &self,
        session_id: &str,
        req: &ImageGenRequest,
        target: &ResolvedTarget,
        reference_source: ReferenceSource,
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
                    vendor: Some(target.vendor),
                    model: Some(target.model.clone()),
                    reference_source: Some(reference_source),
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
                // 已解析目标之后的供应方错误：加「[家 · 模型] 」前缀（D13），同一文本写事件。
                let message = format!("[{} · {}] {error}", target.vendor.label(), target.model);
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
) -> Result<(), Box<mpsc::error::SendError<IterationEvent>>> {
    sender
        .send(IterationEvent {
            kind: "error".to_string(),
            text: None,
            suggested_prompt: None,
            prompt_declined: false,
            provider_session_id: None,
            error: Some(error.to_string()),
        })
        .await
        .map_err(Box::new)
}

/// 迭代输出脱敏（H2）：输出文本中出现两家任一 `api_key` 明文即替换为 `[REDACTED]`。
/// 对齐 `image_client::sanitize_error_body` 的替换思路；输出不含 key 时原文逐字返回，
/// 不改写用户的 prompt。空 key 不参与替换（空串 replace 会在每个字符间插入）。
fn redact_api_keys(text: &str, settings: &ImageCreateSettings) -> String {
    let mut redacted = text.to_string();
    for vendor in ImageVendor::ALL {
        let key = settings.vendors.get(vendor).api_key.as_str();
        if !key.is_empty() {
            redacted = redacted.replace(key, "[REDACTED]");
        }
    }
    redacted
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod engine_tests;
