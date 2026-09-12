use crate::hash::{hash_bytes, hash_file};
use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tracing::{debug, info};
use tusher_core::protocol::Message;
use tusher_network::connection::PeerConnection;

pub const DEFAULT_CHUNK_SIZE: u32 = 2 * 1024 * 1024; // 2 MB

#[derive(Debug, Clone)]
pub struct TransferStats {
    pub transfer_id: String,
    pub total_bytes: u64,
    pub chunks_sent: u32,
    pub chunks_skipped: u32,
    pub elapsed: std::time::Duration,
}

pub struct FileSender {
    chunk_size: u32,
}

impl Default for FileSender {
    fn default() -> Self {
        Self {
            chunk_size: DEFAULT_CHUNK_SIZE,
        }
    }
}

impl FileSender {
    pub fn new(chunk_size: u32) -> Self {
        Self { chunk_size }
    }

    /// Sends a file over an active PeerConnection, automatically resuming if receiver has partial chunks.
    pub async fn send_file<P: AsRef<Path>>(
        &self,
        conn: &PeerConnection,
        file_path: P,
    ) -> anyhow::Result<TransferStats> {
        self.send_file_to_folder(conn, None, file_path).await
    }

    /// Sends a file targeting a specific shared folder on the receiver
    pub async fn send_file_to_folder<P: AsRef<Path>>(
        &self,
        conn: &PeerConnection,
        folder_id: Option<String>,
        file_path: P,
    ) -> anyhow::Result<TransferStats> {
        let path = file_path.as_ref();
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| anyhow::anyhow!("Invalid file name"))?
            .to_string();
        self.send_file_with_target_path(conn, folder_id, file_path, file_name).await
    }

    /// Sends a file targeting a specific shared folder and destination relative path
    pub async fn send_file_with_target_path<P: AsRef<Path>>(
        &self,
        conn: &PeerConnection,
        folder_id: Option<String>,
        file_path: P,
        target_path: String,
    ) -> anyhow::Result<TransferStats> {
        let path = file_path.as_ref();
        let metadata = tokio::fs::metadata(path).await?;
        let file_size = metadata.len();

        info!("Computing SHA-256 for {} ({} bytes)...", target_path, file_size);
        let content_hash = hash_file(path).await?;

        let total_chunks = if file_size == 0 {
            1
        } else {
            ((file_size + self.chunk_size as u64 - 1) / self.chunk_size as u64) as u32
        };

        let transfer_id = format!("xfer_{}", &content_hash[..16]);
        let start_time = Instant::now();

        // 1. Send TransferInit
        let init_msg = Message::TransferInit {
            transfer_id: transfer_id.clone(),
            folder_id,
            file_name: target_path,
            file_size,
            content_hash: content_hash.clone(),
            chunk_size: self.chunk_size,
            total_chunks,
        };
        conn.send(init_msg).await?;

        // 2. Wait for TransferInitAck
        let existing_chunks = match tokio::time::timeout(std::time::Duration::from_secs(5), conn.recv_response())
            .await
            .map_err(|_| anyhow::anyhow!("Timeout awaiting TransferInitAck"))?
        {
            Some(Message::TransferInitAck {
                transfer_id: resp_id,
                accepted: true,
                existing_chunks,
                ..
            }) if resp_id == transfer_id => {
                info!(
                    "Receiver accepted transfer {}. Resumable chunks present: {}",
                    transfer_id,
                    existing_chunks.len()
                );
                existing_chunks.into_iter().collect::<HashSet<u32>>()
            }
            Some(Message::TransferInitAck {
                accepted: false,
                reason,
                ..
            }) => {
                anyhow::bail!("Receiver rejected transfer: {:?}", reason);
            }
            other => {
                anyhow::bail!("Unexpected response to TransferInit: {:?}", other);
            }
        };

        // 3. Stream Chunks
        let mut file = tokio::fs::File::open(path).await?;
        let mut chunks_sent = 0u32;
        let mut chunks_skipped = 0u32;

        for chunk_index in 0..total_chunks {
            let offset = chunk_index as u64 * self.chunk_size as u64;

            // Check if receiver already has this chunk (resumable transfer!)
            if existing_chunks.contains(&chunk_index) {
                debug!("Skipping chunk {} (already on receiver)", chunk_index);
                chunks_skipped += 1;
                continue;
            }

            let current_chunk_len = if chunk_index == total_chunks - 1 {
                (file_size - offset) as usize
            } else {
                self.chunk_size as usize
            };

            let mut chunk_buf = vec![0u8; current_chunk_len];
            file.seek(SeekFrom::Start(offset)).await?;
            file.read_exact(&mut chunk_buf).await?;

            let chunk_hash = hash_bytes(&chunk_buf);

            let chunk_msg = Message::TransferChunk {
                transfer_id: transfer_id.clone(),
                chunk_index,
                offset,
                data: chunk_buf,
                chunk_hash,
            };

            conn.send(chunk_msg).await?;

            // Await ChunkAck with timeout
            let chunk_resp = tokio::time::timeout(std::time::Duration::from_secs(3), conn.recv_response())
                .await
                .map_err(|_| anyhow::anyhow!("Timeout awaiting ChunkAck for chunk {}", chunk_index))?;

            match chunk_resp {
                Some(Message::TransferChunkAck {
                    chunk_index: ack_idx,
                    accepted: true,
                    ..
                }) if ack_idx == chunk_index => {
                    chunks_sent += 1;
                }
                Some(Message::TransferChunkAck { accepted: false, .. }) => {
                    anyhow::bail!("Chunk {} was rejected by receiver", chunk_index);
                }
                other => {
                    anyhow::bail!("Unexpected response awaiting ChunkAck: {:?}", other);
                }
            }
        }

        // 4. Send TransferComplete
        let complete_msg = Message::TransferComplete {
            transfer_id: transfer_id.clone(),
            content_hash: content_hash.clone(),
        };
        conn.send(complete_msg).await?;

        // 5. Await TransferCompleteAck with timeout
        let complete_resp = tokio::time::timeout(std::time::Duration::from_secs(10), conn.recv_response())
            .await
            .map_err(|_| anyhow::anyhow!("Timeout awaiting TransferCompleteAck"))?;

        match complete_resp {
            Some(Message::TransferCompleteAck {
                verified: true,
                ..
            }) => {
                info!(
                    "Transfer {} successfully verified by receiver in {:?}",
                    transfer_id,
                    start_time.elapsed()
                );
            }
            Some(Message::TransferCompleteAck {
                verified: false,
                error,
                ..
            }) => {
                anyhow::bail!("Receiver reported integrity failure: {:?}", error);
            }
            other => {
                anyhow::bail!("Unexpected response to TransferComplete: {:?}", other);
            }
        }

        Ok(TransferStats {
            transfer_id,
            total_bytes: file_size,
            chunks_sent,
            chunks_skipped,
            elapsed: start_time.elapsed(),
        })
    }
}
