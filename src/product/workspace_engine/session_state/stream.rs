use super::*;

impl WorkspaceEngine {
    pub async fn buffer_stream_chunk(
        &mut self,
        node_id: &str,
        content: String,
    ) -> Result<(), String> {
        let should_flush = {
            let buffer = self.stream_buffers.entry(node_id.to_string()).or_default();
            buffer.content.push_str(&content);
            buffer.content.len() >= 4096
                || buffer.last_flush_at.elapsed() >= Duration::from_millis(200)
        };

        if should_flush {
            self.flush_stream_buffer(node_id).await?;
        }
        Ok(())
    }

    pub async fn flush_stream_buffer(&mut self, node_id: &str) -> Result<(), String> {
        let Some(buffer) = self.stream_buffers.remove(node_id) else {
            return Ok(());
        };
        if buffer.content.is_empty() {
            return Ok(());
        }

        self.update_node_detail(node_id, |detail| {
            detail.streaming_content.push_str(&buffer.content);
        })
        .await
    }

    pub async fn append_active_run_stream(
        &mut self,
        role: &str,
        content: impl Into<String>,
    ) -> Result<(), String> {
        let content = content.into();
        let node_id = self.active_node_id.clone();
        let persist_result = if let Some(node_id) = node_id.as_deref() {
            match self.buffer_stream_chunk(node_id, content.clone()).await {
                Ok(()) => self.flush_stream_buffer(node_id).await,
                Err(error) => Err(error),
            }
        } else {
            Ok(())
        };
        let _ = self
            .event_tx
            .send(EngineEvent::StreamChunk {
                role: role.to_string(),
                content,
                node_id,
            })
            .await;
        persist_result
    }

    pub async fn persist_permission_request(
        &mut self,
        node_id: &str,
        request_id: String,
        request: serde_json::Value,
    ) -> Result<(), String> {
        self.update_node_detail(node_id, |detail| {
            if let Some(event) = detail
                .permission_events
                .iter_mut()
                .find(|event| event.request_id == request_id)
            {
                event.request = request;
                return;
            }

            detail.permission_events.push(PermissionEvent {
                request_id,
                request,
                response: None,
                ts: chrono::Utc::now().to_rfc3339(),
            });
        })
        .await
    }

    pub async fn persist_permission_response(
        &mut self,
        node_id: &str,
        request_id: String,
        response: serde_json::Value,
    ) -> Result<(), String> {
        self.update_node_detail(node_id, |detail| {
            if let Some(event) = detail
                .permission_events
                .iter_mut()
                .find(|event| event.request_id == request_id)
            {
                event.response = Some(response);
            }
        })
        .await
    }

    pub async fn persist_permission_timeout(
        &mut self,
        node_id: &str,
        request_id: String,
    ) -> Result<(), String> {
        self.persist_permission_response(
            node_id,
            request_id,
            serde_json::json!({ "status": "timeout" }),
        )
        .await
    }

    pub async fn persist_review_verdict(
        &mut self,
        node_id: &str,
        verdict: serde_json::Value,
    ) -> Result<(), String> {
        self.update_node_detail(node_id, |detail| {
            detail.verdict = Some(verdict);
        })
        .await
    }

    pub async fn persist_artifact_ref(
        &mut self,
        node_id: &str,
        artifact_ref: ArtifactRef,
    ) -> Result<(), String> {
        self.update_node_detail(node_id, |detail| {
            detail.artifact_ref = Some(artifact_ref);
        })
        .await
    }
}
