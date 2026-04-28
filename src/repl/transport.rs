use std::path::Path;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;

use crate::protocol::repl_wire::{RequestEnvelope, ResponseEnvelope};

pub struct UnixJsonTransport {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl UnixJsonTransport {
    pub async fn connect(socket_path: &Path) -> anyhow::Result<Self> {
        let stream = UnixStream::connect(socket_path).await?;
        let (read_half, write_half) = stream.into_split();
        Ok(Self {
            reader: BufReader::new(read_half),
            writer: write_half,
        })
    }

    pub async fn send_request(
        &mut self,
        request: RequestEnvelope,
    ) -> anyhow::Result<ResponseEnvelope> {
        let mut bytes = serde_json::to_vec(&request)?;
        bytes.push(b'\n');
        self.writer.write_all(&bytes).await?;
        self.writer.flush().await?;

        let mut line = String::new();
        self.reader.read_line(&mut line).await?;
        Ok(serde_json::from_str(&line)?)
    }
}
