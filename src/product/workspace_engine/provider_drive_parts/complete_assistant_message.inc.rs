// assistant 消息完成与 artifact 落盘收口（从 provider_drive.rs 整块搬移，
// 逐字未改；主模块超出登记上限，按职责搬移不压行）。
//
// `include!` 复用仓库既有搬移惯例（同 `run/gateway_start_parts/*.inc.rs`）：
// 展开于同一模块，签名/可见性零变化。
impl WorkspaceEngine {
    pub(crate) async fn complete_assistant_message(
        &mut self,
        assistant_msg_id: String,
        full_content: String,
        artifact_retry_attempted: bool,
    ) {
        if self.cancel.is_cancelled() {
            // 诊断打点：assistant 消息完成前的取消后置检查（complete_assistant_message，
            // 无角色上下文——由调用方 drive 循环的位点打点补充角色）。
            eprintln!(
                "[aria-cancellation] workspace provider_drive complete_cancelled trigger=engine_cancelled_observed session_id={} role=author",
                self.session.session_id
            );
            self.finish_aborted_run().await;
            return;
        }

        if full_content.is_empty() {
            self.finish_empty_assistant_output().await;
            return;
        }

        let assistant_msg = SessionMessage {
            id: assistant_msg_id.clone(),
            role: "assistant".to_string(),
            content: full_content.clone(),
            checkpoint_id: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        self.session.messages.push(assistant_msg);
        if let Some(store) = &self.lifecycle_store {
            let _ = store.append_workspace_message(
                &self.session.session_id,
                "assistant".to_string(),
                full_content.clone(),
            );
        }

        if let Some(choice) =
            detect_author_choice_request(&full_content, &self.session.workspace_type).map(
                |(prompt, options)| PendingAuthorChoice {
                    id: format!("author_choice_{}", assistant_msg_id),
                    prompt,
                    options,
                    source_node_id: self.active_node_id.clone(),
                },
            )
        {
            if let Some(node_id) = choice.source_node_id.as_deref() {
                self.update_timeline_node(
                    node_id,
                    TimelineNodeStatus::Paused,
                    Some("等待用户选择".to_string()),
                )
                .await;
            }
            self.pending_author_choice = Some(choice.clone());
            let _ = self
                .event_tx
                .send(EngineEvent::ChoiceRequest {
                    id: choice.id,
                    prompt: choice.prompt.clone(),
                    options: choice.options.clone(),
                    allow_multiple: false,
                    allow_free_text: true,
                    questions: vec![ChoiceQuestionData {
                        id: "default".to_string(),
                        prompt: choice.prompt,
                        options: choice.options,
                        allow_multiple: false,
                        allow_free_text: true,
                    }],
                    source: ChoiceRequestSource::TextFallback,
                })
                .await;
            return;
        }

        self.pending_author_choice = None;
        // REQ-ACS-01：唯一 gate-passing 候选才作为产物；零通过/歧义走既有失败分支
        // （retry 判断由调用方在同一 raw 源上先行完成，此处不重新选源）。
        let selection = workspace_artifact_selection(&full_content, &self.session.workspace_type);
        // REQ-ACS-02：候选选择发生/失败一律落一条有界诊断（durable-only，原文零复制）；
        // 诊断持久化失败只作为失败摘要里的有界提示，不放宽 gate、不额外启动 provider。
        let diagnostic_node_id = self.active_node_id.clone();
        let diagnostic_error = self
            .record_artifact_selection_diagnostic(diagnostic_node_id.as_deref(), &selection)
            .await
            .err();
        let Some(artifact_markdown) = selected_artifact_markdown(&selection) else {
            let blocking_reasons =
                artifact_failure_reasons_with_diagnostic(&selection, diagnostic_error.as_deref());
            if artifact_retry_attempted {
                self.finish_invalid_workspace_artifact_after_retry(&blocking_reasons)
                    .await;
            } else {
                self.finish_invalid_workspace_artifact(&blocking_reasons)
                    .await;
            }
            return;
        };
        let aggregate_write_back_diagnostic = if let Some(store) = &self.lifecycle_store
            && matches!(
                self.session.workspace_type,
                WorkspaceType::Story | WorkspaceType::Design
            ) {
            // 方案X阶段2：AI run 后解析 structured output，回写 involved/change_order
            // 到 Spec record（复用 Task 3 parse_* 与 Task 2 update_*）。
            // 缺陷 #4（2026-10-02 E2E）：prompt 要求 sentinel 标签输出在 artifact
            // 围栏**之外**（「artifact 之外必须额外输出」），而产物选择只取围栏内
            // 正文——解析源必须是 provider 全量输出 full_content，否则标签永远
            // 不可见（MissingStructuredOutput），involved 恒空、终确认恒 4xx。
            // 约束4：回写失败/缺 tag 不沿用 `let _ =` 静默吞，转为可见诊断；
            // 单仓（logical_codebase_ref 为 None）回写跳过，append_version 行为不变。
            let diagnostic = match self.write_back_aggregate_output(store, &full_content) {
                Ok(Some(message)) => Some(message),
                Ok(None) => None,
                Err(error) => Some(format!("聚合代码库 involved 回写失败：{error}")),
            };
            let _ = store.append_version(AppendSpecVersionInput {
                project_id: self.session.project_id.clone(),
                issue_id: self.session.issue_id.clone(),
                entity_id: self.session.entity_id.clone(),
                markdown: artifact_markdown.clone(),
                provider_run_refs: Vec::new(),
                review_refs: Vec::new(),
                confirmed_by: None,
            });
            diagnostic
        } else {
            None
        };
        // artifact 落盘失败必须显式暴露：Error 事件 + Failed 节点，并在 checkpoint /
        // MessageComplete / AuthorConfirm 之前返回（不假装产物已耐久）。
        if let Err(message) = self
            .update_artifact(ArtifactPayload::Markdown {
                markdown: artifact_markdown.clone(),
                diff: None,
            })
            .await
        {
            let _ = self
                .event_tx
                .send(EngineEvent::Error {
                    message: message.clone(),
                })
                .await;
            self.finish_active_run_with_failed_node(message).await;
            return;
        }

        let message_index = self.session.messages.len() as u32;
        let artifact_snapshot = self.session.artifact.as_ref();
        // D1.3b（fix-cascade-deletion plan Interfaces）：persistent engine
        // checkpoint 的 scope guard 包实际 write——purging/deleted 会话拒绝且
        // checkpoints 目录不复活；非 persistent（无 lifecycle_store）保持原路径。
        let checkpoint = match &self.lifecycle_store {
            Some(store) => {
                store.with_workspace_session_live_write(&self.session.session_id, || {
                    self.checkpoint_store.create_checkpoint(
                        &self.session.session_id,
                        message_index,
                        artifact_snapshot,
                        WorkspaceStage::AuthorConfirm.as_str(),
                    )
                })
            }
            None => self.checkpoint_store.create_checkpoint(
                &self.session.session_id,
                message_index,
                artifact_snapshot,
                WorkspaceStage::AuthorConfirm.as_str(),
            ),
        };

        let checkpoint_id = match checkpoint {
            Ok(cp) => {
                if let Some(last) = self.session.messages.last_mut() {
                    last.checkpoint_id = Some(cp.id.clone());
                }
                cp.id
            }
            Err(e) => {
                let _ = self
                    .event_tx
                    .send(EngineEvent::Error {
                        message: format!("checkpoint error: {e}"),
                    })
                    .await;
                return;
            }
        };

        // 约束4：聚合回写诊断在 checkpoint 之后追加，避免干扰 checkpoint 对最后一条
        // assistant 消息的 checkpoint_id 绑定。
        if let Some(diagnostic) = aggregate_write_back_diagnostic {
            self.append_aggregate_write_back_diagnostic(&diagnostic);
        }

        let node_id = self.active_node_id.clone();
        let _ = self
            .event_tx
            .send(EngineEvent::MessageComplete {
                message_id: assistant_msg_id,
                checkpoint_id,
                node_id,
            })
            .await;
        // spec-design-dialog-revision T4：author 反馈修订完成后回 AuthorConfirm，summary 携带改动摘要
        // （brief 未覆盖提取机制，按现有 Revision 完成路径最小适配：从产物「## 改动摘要」小节提取）。
        // T5/M-1：谓词提取为共享 helper is_author_feedback_revision（decisions.rs）。
        let confirm_summary = if self.is_author_feedback_revision() {
            extract_changelog_summary(&artifact_markdown)
                .map(|changelog| format!("修订完成。\n\n## 改动摘要\n{changelog}"))
                .unwrap_or_else(|| "等待用户确认 author 结果".to_string())
        } else {
            "等待用户确认 author 结果".to_string()
        };
        self.complete_active_node(Some("生成完成".to_string()))
            .await;
        self.enter_author_confirm(Some(confirm_summary)).await;
    }
}
