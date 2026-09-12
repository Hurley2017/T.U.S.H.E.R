use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferState {
    pub transfer_id: String,
    #[serde(default)]
    pub folder_id: Option<String>,
    pub file_name: String,
    pub file_size: u64,
    pub content_hash: String,
    pub chunk_size: u32,
    pub total_chunks: u32,
    pub completed_chunks: HashSet<u32>,
}

impl TransferState {
    pub fn new(
        transfer_id: String,
        folder_id: Option<String>,
        file_name: String,
        file_size: u64,
        content_hash: String,
        chunk_size: u32,
        total_chunks: u32,
    ) -> Self {
        Self {
            transfer_id,
            folder_id,
            file_name,
            file_size,
            content_hash,
            chunk_size,
            total_chunks,
            completed_chunks: HashSet::new(),
        }
    }

    pub fn state_file_path<P: AsRef<Path>>(staging_dir: P, transfer_id: &str) -> PathBuf {
        staging_dir.as_ref().join(format!("{}.state", transfer_id))
    }

    pub fn partial_file_path<P: AsRef<Path>>(staging_dir: P, transfer_id: &str) -> PathBuf {
        staging_dir.as_ref().join(format!("{}.partial", transfer_id))
    }

    pub async fn save<P: AsRef<Path>>(&self, staging_dir: P) -> anyhow::Result<()> {
        let path = Self::state_file_path(staging_dir, &self.transfer_id);
        let data = serde_json::to_vec_pretty(self)?;
        tokio::fs::write(path, data).await?;
        Ok(())
    }

    pub async fn load<P: AsRef<Path>>(staging_dir: P, transfer_id: &str) -> anyhow::Result<Self> {
        let path = Self::state_file_path(staging_dir, transfer_id);
        let data = tokio::fs::read(path).await?;
        let state: Self = serde_json::from_slice(&data)?;
        Ok(state)
    }

    pub async fn cleanup<P: AsRef<Path>>(staging_dir: P, transfer_id: &str) {
        let state_path = Self::state_file_path(&staging_dir, transfer_id);
        let _ = tokio::fs::remove_file(state_path).await;
        let partial_path = Self::partial_file_path(&staging_dir, transfer_id);
        let _ = tokio::fs::remove_file(partial_path).await;
    }
}
