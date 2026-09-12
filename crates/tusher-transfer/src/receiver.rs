use crate::hash::{hash_bytes, hash_file};
use crate::state::TransferState;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncSeekExt, AsyncWriteExt, SeekFrom};
use tracing::{debug, error, info, warn};
use tusher_core::protocol::Message;

#[derive(Debug, Clone)]
pub struct TransferCompletedInfo {
    pub transfer_id: String,
    pub folder_id: Option<String>,
    pub file_name: String,
    pub file_size: u64,
    pub content_hash: String,
    pub dest_path: PathBuf,
}

pub struct FileReceiver {
    staging_dir: PathBuf,
    destination_dir: PathBuf,
    folder_destinations: std::collections::HashMap<String, PathBuf>,
    active_transfers: std::collections::HashMap<String, TransferState>,
    completion_tx: Option<tokio::sync::mpsc::Sender<TransferCompletedInfo>>,
}

impl FileReceiver {
    pub async fn new<P1: AsRef<Path>, P2: AsRef<Path>>(
        staging_dir: P1,
        destination_dir: P2,
    ) -> anyhow::Result<Self> {
        let s_dir = staging_dir.as_ref().to_path_buf();
        let d_dir = destination_dir.as_ref().to_path_buf();
        tokio::fs::create_dir_all(&s_dir).await?;
        tokio::fs::create_dir_all(&d_dir).await?;

        Ok(Self {
            staging_dir: s_dir,
            destination_dir: d_dir,
            folder_destinations: std::collections::HashMap::new(),
            active_transfers: std::collections::HashMap::new(),
            completion_tx: None,
        })
    }

    pub fn set_completion_channel(&mut self, tx: tokio::sync::mpsc::Sender<TransferCompletedInfo>) {
        self.completion_tx = Some(tx);
    }

    pub fn register_folder_destination(&mut self, folder_id: String, path: PathBuf) {
        self.folder_destinations.insert(folder_id, path);
    }

    /// Handles TransferInit message. Checks for existing state for resumable transfer.
    pub async fn handle_init(
        &mut self,
        transfer_id: String,
        folder_id: Option<String>,
        file_name: String,
        file_size: u64,
        content_hash: String,
        chunk_size: u32,
        total_chunks: u32,
    ) -> anyhow::Result<Message> {
        // Prevent path traversal while supporting nested relative paths
        let clean_file_name = {
            let p = Path::new(&file_name);
            let mut safe = PathBuf::new();
            for comp in p.components() {
                match comp {
                    std::path::Component::Normal(c) => safe.push(c),
                    _ => {} // Strip RootDir, Prefix, ParentDir (..), CurDir (.) to prevent traversal
                }
            }
            if safe.as_os_str().is_empty() {
                anyhow::bail!("Invalid file path: {}", file_name);
            }
            safe.to_string_lossy().replace('\\', "/")
        };

        // Check if there is already a resume state
        let state = if let Ok(existing) = TransferState::load(&self.staging_dir, &transfer_id).await {
            if existing.content_hash == content_hash && existing.file_size == file_size {
                info!(
                    "Resuming transfer {}: already have {}/{} chunks",
                    transfer_id,
                    existing.completed_chunks.len(),
                    existing.total_chunks
                );
                existing
            } else {
                warn!("Existing state for {} invalid or outdated, restarting", transfer_id);
                TransferState::new(
                    transfer_id.clone(),
                    folder_id,
                    clean_file_name,
                    file_size,
                    content_hash,
                    chunk_size,
                    total_chunks,
                )
            }
        } else {
            TransferState::new(
                transfer_id.clone(),
                folder_id,
                clean_file_name,
                file_size,
                content_hash,
                chunk_size,
                total_chunks,
            )
        };

        // Ensure partial file exists with pre-allocated or empty state
        let partial_path = TransferState::partial_file_path(&self.staging_dir, &transfer_id);
        if !partial_path.exists() {
            let f = tokio::fs::File::create(&partial_path).await?;
            f.set_len(file_size).await?; // Pre-allocate file size
        }

        state.save(&self.staging_dir).await?;
        let existing_chunks: Vec<u32> = state.completed_chunks.iter().cloned().collect();
        self.active_transfers.insert(transfer_id.clone(), state);

        Ok(Message::TransferInitAck {
            transfer_id,
            accepted: true,
            existing_chunks,
            reason: None,
        })
    }

    /// Handles incoming data chunk. Validates chunk hash, writes at offset, checkpoints state.
    pub async fn handle_chunk(
        &mut self,
        transfer_id: String,
        chunk_index: u32,
        offset: u64,
        data: Vec<u8>,
        chunk_hash: String,
    ) -> anyhow::Result<Message> {
        // 1. Verify chunk hash
        let computed_chunk_hash = hash_bytes(&data);
        if computed_chunk_hash != chunk_hash {
            error!(
                "Chunk {} hash mismatch! Expected {}, got {}",
                chunk_index, chunk_hash, computed_chunk_hash
            );
            return Ok(Message::TransferChunkAck {
                transfer_id,
                chunk_index,
                accepted: false,
            });
        }

        // 2. Write to partial file
        let partial_path = TransferState::partial_file_path(&self.staging_dir, &transfer_id);
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .open(&partial_path)
            .await?;

        file.seek(SeekFrom::Start(offset)).await?;
        file.write_all(&data).await?;
        file.flush().await?;

        // 3. Update & persist state
        if let Some(state) = self.active_transfers.get_mut(&transfer_id) {
            state.completed_chunks.insert(chunk_index);
            state.save(&self.staging_dir).await?;
        }

        debug!("Saved chunk {} for transfer {}", chunk_index, transfer_id);

        Ok(Message::TransferChunkAck {
            transfer_id,
            chunk_index,
            accepted: true,
        })
    }

    /// Handles TransferComplete. Validates full SHA-256 and atomically moves file to final destination.
    pub async fn handle_complete(
        &mut self,
        transfer_id: String,
        expected_hash: String,
    ) -> anyhow::Result<Message> {
        let state = match self.active_transfers.remove(&transfer_id) {
            Some(s) => s,
            None => TransferState::load(&self.staging_dir, &transfer_id).await?,
        };

        let partial_path = TransferState::partial_file_path(&self.staging_dir, &transfer_id);

        // Verify full file SHA-256
        let computed_file_hash = hash_file(&partial_path).await?;
        if computed_file_hash != expected_hash || computed_file_hash != state.content_hash {
            error!(
                "Final transfer integrity failed for {}! Expected {}, computed {}",
                transfer_id, expected_hash, computed_file_hash
            );
            return Ok(Message::TransferCompleteAck {
                transfer_id,
                verified: false,
                error: Some(format!(
                    "Integrity check failed: hash mismatch (computed {})",
                    computed_file_hash
                )),
            });
        }

        // Atomic placement: move partial file to final destination
        let dest_path = if let Some(ref fid) = state.folder_id {
            if let Some(folder_path) = self.folder_destinations.get(fid) {
                folder_path.join(&state.file_name)
            } else {
                self.destination_dir.join(&state.file_name)
            }
        } else {
            self.destination_dir.join(&state.file_name)
        };
        if let Some(parent) = dest_path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        tokio::fs::rename(&partial_path, &dest_path).await?;

        // Cleanup state file
        let state_path = TransferState::state_file_path(&self.staging_dir, &transfer_id);
        let _ = tokio::fs::remove_file(state_path).await;

        info!(
            "Transfer {} completed & verified! File placed at {}",
            transfer_id,
            dest_path.display()
        );

        if let Some(tx) = &self.completion_tx {
            let _ = tx.try_send(TransferCompletedInfo {
                transfer_id: transfer_id.clone(),
                folder_id: state.folder_id.clone(),
                file_name: state.file_name.clone(),
                file_size: state.file_size,
                content_hash: state.content_hash.clone(),
                dest_path: dest_path.clone(),
            });
        }

        Ok(Message::TransferCompleteAck {
            transfer_id,
            verified: true,
            error: None,
        })
    }
}
