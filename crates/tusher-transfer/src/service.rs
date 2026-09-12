use crate::receiver::FileReceiver;
use crate::sender::{FileSender, TransferStats, DEFAULT_CHUNK_SIZE};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tusher_core::protocol::Message;
use tusher_network::connection::PeerConnection;

pub struct TransferService {
    receiver: Arc<Mutex<FileReceiver>>,
    sender: FileSender,
    destination_dir: PathBuf,
}

impl TransferService {
    pub async fn new<P1: AsRef<Path>, P2: AsRef<Path>>(
        staging_dir: P1,
        destination_dir: P2,
    ) -> anyhow::Result<Self> {
        let dest = destination_dir.as_ref().to_path_buf();
        let receiver = Arc::new(Mutex::new(FileReceiver::new(staging_dir, &dest).await?));
        let sender = FileSender::new(DEFAULT_CHUNK_SIZE);

        Ok(Self {
            receiver,
            sender,
            destination_dir: dest,
        })
    }

    pub fn destination_dir(&self) -> &Path {
        &self.destination_dir
    }

    pub async fn register_folder_destination<P: AsRef<Path>>(&self, folder_id: &str, path: P) {
        let mut rx = self.receiver.lock().await;
        rx.register_folder_destination(folder_id.to_string(), path.as_ref().to_path_buf());
    }

    pub async fn set_completion_channel(&self, tx: tokio::sync::mpsc::Sender<crate::receiver::TransferCompletedInfo>) {
        let mut rx = self.receiver.lock().await;
        rx.set_completion_channel(tx);
    }

    pub async fn handle_incoming_message(&self, msg: Message) -> anyhow::Result<Option<Message>> {
        match msg {
            Message::TransferInit {
                transfer_id,
                folder_id,
                file_name,
                file_size,
                content_hash,
                chunk_size,
                total_chunks,
            } => {
                let mut rx = self.receiver.lock().await;
                let resp = rx
                    .handle_init(
                        transfer_id,
                        folder_id,
                        file_name,
                        file_size,
                        content_hash,
                        chunk_size,
                        total_chunks,
                    )
                    .await?;
                Ok(Some(resp))
            }
            Message::TransferChunk {
                transfer_id,
                chunk_index,
                offset,
                data,
                chunk_hash,
            } => {
                let mut rx = self.receiver.lock().await;
                let resp = rx
                    .handle_chunk(transfer_id, chunk_index, offset, data, chunk_hash)
                    .await?;
                Ok(Some(resp))
            }
            Message::TransferComplete {
                transfer_id,
                content_hash,
            } => {
                let mut rx = self.receiver.lock().await;
                let resp = rx.handle_complete(transfer_id, content_hash).await?;
                Ok(Some(resp))
            }
            _ => Ok(None),
        }
    }

    pub async fn send_file<P: AsRef<Path>>(
        &self,
        conn: &PeerConnection,
        file_path: P,
    ) -> anyhow::Result<TransferStats> {
        self.sender.send_file(conn, file_path).await
    }

    pub async fn send_file_to_folder<P: AsRef<Path>>(
        &self,
        conn: &PeerConnection,
        folder_id: Option<String>,
        file_path: P,
    ) -> anyhow::Result<TransferStats> {
        self.sender.send_file_to_folder(conn, folder_id, file_path).await
    }

    pub async fn send_file_with_target_path<P: AsRef<Path>>(
        &self,
        conn: &PeerConnection,
        folder_id: Option<String>,
        file_path: P,
        target_path: String,
    ) -> anyhow::Result<TransferStats> {
        self.sender.send_file_with_target_path(conn, folder_id, file_path, target_path).await
    }
}

