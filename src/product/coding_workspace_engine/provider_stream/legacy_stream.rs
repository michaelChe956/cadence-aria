//! 从 provider_stream.rs 拆出以满足 large_file_guard 的 1200 行上限
//! （纯移动，无行为变化）：legacy fallback stream 路径。

use super::*;

impl CodingWorkspaceEngine {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn run_legacy_stream_to_completion(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        role_run: Option<&CodingRoleRun>,
        provider: &dyn StreamingProviderAdapter,
        input: &AdapterInput,
        provider_name: &ProviderName,
        provider_role: CodingProviderRole,
        suppress_failure_side_effects: bool,
        partial_output_observer: Option<Arc<Mutex<String>>>,
    ) -> Result<ProviderStreamOutcome, CodingWorkspaceEngineError> {
        let cancel = self.cancellation.child_token();
        // legacy fallback 仅对未实现 `start` 的 adapter（测试替身）生效；真实
        // adapter 均实现 `start` 并走上方已接线 durable 审计的现代路径。默认
        // bridge 按角色矩阵派生策略（GC13「接受双向守卫」），denylist argv 照常
        // 注入；durable 审计由现代 start 路径承载，本路径保持既有
        // role-run/execution_event 审计不变。
        let mut stream = tokio::select! {
            biased;
            // P2-1：策略角色的 legacy 直连由 engine 注入 run-bound sink（不得
            // policy+缺 sink 运行时 fail-closed）；非策略角色原样透传。
            result = self.run_legacy_provider_stream(
                provider,
                input,
                &attempt.id,
                cancel.clone(),
            ) => result?,
            _ = self.cancellation.cancelled() => {
                cancel.cancel();
                warn_cancellation_site(
                    attempt,
                    role_run,
                    "engine_cancellation",
                    "legacy_provider_start",
                );
                self.persist_provider_cancellation(attempt, role_run, "legacy_provider_start")?;
                return Err(CodingWorkspaceEngineError::Aborted);
            }
        };
        if let Err(error) = self.record_provider_start_required(
            attempt,
            role_run,
            json!({
                "provider": provider_name,
                "role": format!("{provider_role:?}"),
                "mode": "legacy_stream"
            }),
        ) {
            let message = error.to_string();
            cancel.cancel();
            warn_cancellation_site(
                attempt,
                role_run,
                "provider_start_persistence_failure",
                "legacy_provider_start",
            );
            drop(stream);
            return self
                .fail_provider_stream_with_ownership(
                    attempt,
                    node_id,
                    suppress_failure_side_effects,
                    message,
                )
                .await;
        }
        let mut full_output = String::new();
        loop {
            let chunk = tokio::select! {
                biased;
                _ = self.cancellation.cancelled() => {
                    cancel.cancel();
                    warn_cancellation_site(
                        attempt,
                        role_run,
                        "engine_cancellation",
                        "legacy_provider_stream",
                    );
                    self.persist_provider_cancellation(
                        attempt,
                        role_run,
                        "legacy_provider_stream",
                    )?;
                    return Err(CodingWorkspaceEngineError::Aborted);
                }
                chunk = stream.recv() => chunk,
            };
            let Some(chunk) = chunk else {
                break;
            };
            match chunk {
                StreamChunk::Text(content) => {
                    let content_for_event = content.clone();
                    full_output.push_str(&content);
                    append_partial_output(partial_output_observer.as_ref(), &content_for_event);
                    let _ = self
                        .event_tx
                        .send(CodingWsOutMessage::CodingStreamChunk {
                            content,
                            node_id: Some(node_id.to_string()),
                        })
                        .await;
                    self.record_role_run_event(
                        attempt,
                        role_run,
                        CodingRoleRunEventType::TextDelta,
                        json!({
                            "content": content_for_event
                        }),
                    );
                }
                StreamChunk::Done {
                    full_output: completed_output,
                } => {
                    let output = if completed_output.trim().is_empty() {
                        full_output
                    } else {
                        completed_output
                    };
                    let output_bytes = output.len();
                    let _ = self
                        .event_tx
                        .send(CodingWsOutMessage::CodingMessageComplete {
                            node_id: Some(node_id.to_string()),
                        })
                        .await;
                    self.record_role_run_event(
                        attempt,
                        role_run,
                        CodingRoleRunEventType::MessageComplete,
                        json!({
                            "provider_session_id": null,
                            "output_bytes": output_bytes
                        }),
                    );
                    return Ok(ProviderStreamOutcome {
                        full_output: output,
                        structured_output: StructuredOutputState::NotRequested,
                    });
                }
                StreamChunk::Error(message) => {
                    self.record_role_run_event(
                        attempt,
                        role_run,
                        CodingRoleRunEventType::ProviderFailed,
                        json!({
                            "message": message.clone()
                        }),
                    );
                    return self
                        .fail_provider_stream_with_ownership(
                            attempt,
                            node_id,
                            suppress_failure_side_effects,
                            message,
                        )
                        .await;
                }
            }
        }

        self.fail_provider_stream_ended_with_ownership(
            attempt,
            node_id,
            suppress_failure_side_effects,
        )
        .await
    }
}
