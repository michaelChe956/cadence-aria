use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::cross_cutting::provider_adapter::ProviderAdapterError;

type PendingResponses = Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>;

#[derive(Clone)]
pub struct JsonRpcPeer<W> {
    writer: Arc<Mutex<W>>,
    pending: PendingResponses,
    incoming_rx: Arc<Mutex<mpsc::Receiver<Value>>>,
    next_id: Arc<AtomicU64>,
}

impl<W> JsonRpcPeer<W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    pub fn new<R>(reader: R, writer: W) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
    {
        let pending: PendingResponses = Arc::new(Mutex::new(HashMap::new()));
        let (incoming_tx, incoming_rx) = mpsc::channel(32);

        tokio::spawn(read_json_rpc_lines(
            reader,
            Arc::clone(&pending),
            incoming_tx,
        ));

        Self {
            writer: Arc::new(Mutex::new(writer)),
            pending,
            incoming_rx: Arc::new(Mutex::new(incoming_rx)),
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }
}

impl<W> JsonRpcPeer<W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    pub async fn request(&self, mut payload: Value) -> Result<Value, ProviderAdapterError> {
        let id = ensure_request_id(&mut payload, &self.next_id)?;
        let (response_tx, response_rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), response_tx);

        if let Err(error) = self.send(payload).await {
            self.pending.lock().await.remove(&id);
            return Err(error);
        }

        response_rx.await.map_err(|_| {
            ProviderAdapterError::execution_failed(
                None,
                String::new(),
                "JSON-RPC response channel closed",
                0,
            )
        })
    }

    pub async fn send(&self, payload: Value) -> Result<(), ProviderAdapterError> {
        let mut writer = self.writer.lock().await;
        let line = serde_json::to_string(&payload).map_err(|error| {
            ProviderAdapterError::parse_error(
                format!("invalid JSON-RPC payload: {error}"),
                String::new(),
                String::new(),
            )
        })?;
        writer.write_all(line.as_bytes()).await.map_err(io_error)?;
        writer.write_all(b"\n").await.map_err(io_error)?;
        writer.flush().await.map_err(io_error)
    }

    pub async fn next_incoming(&self) -> Option<Value> {
        self.incoming_rx.lock().await.recv().await
    }
}

async fn read_json_rpc_lines<R>(
    reader: R,
    pending: PendingResponses,
    incoming_tx: mpsc::Sender<Value>,
) where
    R: AsyncRead + Unpin,
{
    let mut lines = BufReader::new(reader).lines();
    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) | Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };

        let pending_response =
            if let (true, Some(id)) = (is_response(&value), value.get("id").and_then(id_key)) {
                pending.lock().await.remove(&id)
            } else {
                None
            };
        if let Some(response_tx) = pending_response {
            let response = value
                .get("result")
                .cloned()
                .or_else(|| value.get("response").cloned())
                .or_else(|| value.get("error").cloned())
                .unwrap_or(Value::Null);
            let _ = response_tx.send(response);
            continue;
        }

        if incoming_tx.send(value).await.is_err() {
            break;
        }
    }
    pending.lock().await.clear();
}

fn ensure_request_id(
    payload: &mut Value,
    next_id: &AtomicU64,
) -> Result<String, ProviderAdapterError> {
    if let Some(id) = payload.get("id").and_then(id_key) {
        return Ok(id);
    }

    let id = next_id.fetch_add(1, Ordering::Relaxed);
    let Some(object) = payload.as_object_mut() else {
        return Err(ProviderAdapterError::parse_error(
            "JSON-RPC request payload must be an object",
            payload.to_string(),
            String::new(),
        ));
    };
    object.insert("id".to_string(), Value::from(id));
    Ok(id.to_string())
}

fn is_response(value: &Value) -> bool {
    value.get("id").is_some()
        && (value.get("result").is_some()
            || value.get("response").is_some()
            || value.get("error").is_some())
}

fn id_key(value: &Value) -> Option<String> {
    value
        .as_u64()
        .map(|id| id.to_string())
        .or_else(|| value.as_str().map(ToString::to_string))
}

fn io_error(error: std::io::Error) -> ProviderAdapterError {
    ProviderAdapterError::execution_failed(None, String::new(), error.to_string(), 0)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    use super::JsonRpcPeer;

    #[tokio::test]
    async fn json_rpc_peer_matches_response_by_id() {
        let (client_io, server_io) = tokio::io::duplex(4096);
        let (reader, writer) = tokio::io::split(client_io);
        let peer = JsonRpcPeer::new(reader, writer);

        tokio::spawn(async move {
            let (server_reader, mut server_writer) = tokio::io::split(server_io);
            let mut line = String::new();
            let mut reader = tokio::io::BufReader::new(server_reader);
            reader.read_line(&mut line).await.unwrap();
            server_writer
                .write_all(br#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#)
                .await
                .unwrap();
            server_writer.write_all(b"\n").await.unwrap();
        });

        let value = peer
            .request(serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
            .await
            .unwrap();

        assert_eq!(value["ok"], true);
    }

    #[tokio::test]
    async fn json_rpc_peer_closes_pending_request_when_reader_ends() {
        let (client_io, server_io) = tokio::io::duplex(4096);
        let (reader, writer) = tokio::io::split(client_io);
        let peer = JsonRpcPeer::new(reader, writer);

        tokio::spawn(async move {
            let (server_reader, _server_writer) = tokio::io::split(server_io);
            let mut line = String::new();
            let mut reader = tokio::io::BufReader::new(server_reader);
            reader.read_line(&mut line).await.unwrap();
        });

        let result = tokio::time::timeout(
            Duration::from_secs(1),
            peer.request(serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {},
            })),
        )
        .await
        .expect("pending request should close when reader ends");

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn json_rpc_peer_matches_app_server_response_shape() {
        let (client_io, server_io) = tokio::io::duplex(4096);
        let (reader, writer) = tokio::io::split(client_io);
        let peer = JsonRpcPeer::new(reader, writer);

        tokio::spawn(async move {
            let (server_reader, mut server_writer) = tokio::io::split(server_io);
            let mut line = String::new();
            let mut reader = tokio::io::BufReader::new(server_reader);
            reader.read_line(&mut line).await.unwrap();
            server_writer
                .write_all(br#"{"id":1,"method":"thread/start","response":{"ok":true}}"#)
                .await
                .unwrap();
            server_writer.write_all(b"\n").await.unwrap();
        });

        let value = tokio::time::timeout(
            Duration::from_secs(1),
            peer.request(serde_json::json!({
                "id": 1,
                "method": "thread/start",
                "params": {},
            })),
        )
        .await
        .expect("app-server response should match pending request")
        .unwrap();

        assert_eq!(value["ok"], true);
    }
}
