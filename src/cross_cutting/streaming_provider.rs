use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::protocol::contracts::AdapterInput;

#[derive(Debug, Clone)]
pub enum StreamChunk {
    Text(String),
    Done { full_output: String },
    Error(String),
}

pub struct StreamingRunHandle {
    pub receiver: mpsc::Receiver<StreamChunk>,
    pub cancel: CancellationToken,
}

#[async_trait::async_trait]
pub trait StreamingProviderAdapter: Send + Sync {
    async fn run_streaming(
        &self,
        input: &AdapterInput,
        cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError>;
}

pub struct FakeStreamingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for FakeStreamingProvider {
    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        let (tx, rx) = mpsc::channel(32);
        let prompt = input.prompt.clone();

        tokio::spawn(async move {
            let words: Vec<&str> = prompt.split_whitespace().collect();
            for (i, word) in words.iter().enumerate() {
                tokio::select! {
                    _ = _cancel.cancelled() => return,
                    _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {}
                }
                let chunk = if i == 0 {
                    word.to_string()
                } else {
                    format!(" {word}")
                };
                tokio::select! {
                    _ = _cancel.cancelled() => return,
                    send_result = tx.send(StreamChunk::Text(chunk)) => {
                        if send_result.is_err() {
                            return;
                        }
                    }
                }
            }
            if _cancel.is_cancelled() {
                return;
            }
            let _ = tx
                .send(StreamChunk::Done {
                    full_output: prompt,
                })
                .await;
        });

        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::contracts::AdapterInput;

    fn make_input(prompt: &str) -> AdapterInput {
        AdapterInput {
            prompt: prompt.to_string(),
            provider_type: crate::protocol::contracts::ProviderType::Fake,
            role: crate::protocol::contracts::AdapterRole::Orchestrator,
            worktree_path: None,
            context_files: Vec::new(),
            output_schema: String::new(),
            timeout: 60,
            max_retries: 0,
        }
    }

    #[tokio::test]
    async fn fake_streaming_provider_emits_chunks_then_done() {
        let provider = FakeStreamingProvider;
        let cancel = CancellationToken::new();
        let input = make_input("hello world foo");

        let mut rx = provider.run_streaming(&input, cancel).await.unwrap();

        let mut texts = Vec::new();
        let mut done_output = None;

        while let Some(chunk) = rx.recv().await {
            match chunk {
                StreamChunk::Text(t) => texts.push(t),
                StreamChunk::Done { full_output } => {
                    done_output = Some(full_output);
                    break;
                }
                StreamChunk::Error(_) => panic!("unexpected error"),
            }
        }

        assert_eq!(texts, vec!["hello", " world", " foo"]);
        assert_eq!(done_output.unwrap(), "hello world foo");
    }

    #[tokio::test]
    async fn fake_streaming_provider_cancel_stops_output() {
        let provider = FakeStreamingProvider;
        let cancel = CancellationToken::new();
        let input = make_input("a b c d e f g h i j");

        let mut rx = provider
            .run_streaming(&input, cancel.clone())
            .await
            .unwrap();

        let first = rx.recv().await.unwrap();
        assert!(matches!(first, StreamChunk::Text(_)));
        cancel.cancel();

        let mut post_cancel_chunks = Vec::new();
        while let Ok(Some(chunk)) =
            tokio::time::timeout(std::time::Duration::from_millis(25), rx.recv()).await
        {
            post_cancel_chunks.push(chunk);
        }

        assert!(
            post_cancel_chunks.len() < 9,
            "cancelled provider should stop before completing the full stream"
        );
        assert!(
            post_cancel_chunks
                .iter()
                .all(|chunk| !matches!(chunk, StreamChunk::Done { .. })),
            "cancelled provider should not emit a completion marker"
        );
    }
}
