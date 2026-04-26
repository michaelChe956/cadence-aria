pub mod checkpoint;
pub mod discovery;
pub mod recovery;
pub mod runner;
pub mod state_machine;
pub mod task_registry;

use std::path::Path;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::protocol::repl_wire::{Command, RequestEnvelope, ResponseEnvelope};

pub async fn serve_one_connection(workspace_root: &Path, socket_path: &Path) -> anyhow::Result<()> {
    if socket_path.exists() {
        std::fs::remove_file(socket_path)?;
    }
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(socket_path)?;
    let (stream, _) = listener.accept().await?;
    let mut state = state_machine::DaemonState::bootstrap(workspace_root)?;
    handle_stream(stream, &mut state).await?;

    let _ = std::fs::remove_file(socket_path);
    Ok(())
}

pub async fn handle_stream(
    stream: UnixStream,
    state: &mut state_machine::DaemonState,
) -> anyhow::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    loop {
        let mut line = String::new();
        let bytes_read = reader.read_line(&mut line).await?;
        if bytes_read == 0 {
            break;
        }

        let request: RequestEnvelope = serde_json::from_str(&line)?;
        let should_detach = request.command == Command::Detach;
        let response = match state.handle_request(request) {
            Ok(response) => response,
            Err(error) => ResponseEnvelope::failure("unknown_request", Command::Detach, error),
        };
        let mut bytes = serde_json::to_vec(&response)?;
        bytes.push(b'\n');
        write_half.write_all(&bytes).await?;
        write_half.flush().await?;

        if should_detach {
            break;
        }
    }

    Ok(())
}
