use crate::identity::{NodeId, Platform};
use bytes::{Buf, BufMut, BytesMut};
use serde::{Deserialize, Serialize};
use std::io;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAGIC_BYTES: &[u8; 4] = b"TUSH";
pub const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024; // 16 MB max frame

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum Message {
    Hello {
        version: u32,
        node_id: NodeId,
        node_name: String,
        platform: Platform,
        public_key_hex: String,
        listen_port: u16,
    },
    HelloAck {
        version: u32,
        node_id: NodeId,
        node_name: String,
        platform: Platform,
        public_key_hex: String,
        accepted: bool,
        reason: Option<String>,
    },
    PairRequest {
        node_id: NodeId,
        ephemeral_token: String,
        signature_hex: String,
    },
    PairResponse {
        node_id: NodeId,
        signature_hex: String,
        sas_pin: String,
    },
    PairConfirm {
        node_id: NodeId,
        accepted: bool,
    },
    Heartbeat {
        sequence: u64,
        timestamp_ms: i64,
    },
    HeartbeatAck {
        sequence: u64,
        timestamp_ms: i64,
    },
    Ping {
        sequence: u64,
        timestamp_ms: i64,
    },
    Pong {
        sequence: u64,
        timestamp_ms: i64,
    },
    Disconnect {
        reason: String,
    },
    TransferInit {
        transfer_id: String,
        #[serde(default)]
        folder_id: Option<String>,
        file_name: String,
        file_size: u64,
        content_hash: String,
        chunk_size: u32,
        total_chunks: u32,
    },
    TransferInitAck {
        transfer_id: String,
        accepted: bool,
        existing_chunks: Vec<u32>,
        reason: Option<String>,
    },
    TransferChunk {
        transfer_id: String,
        chunk_index: u32,
        offset: u64,
        data: Vec<u8>,
        chunk_hash: String,
    },
    TransferChunkAck {
        transfer_id: String,
        chunk_index: u32,
        accepted: bool,
    },
    TransferComplete {
        transfer_id: String,
        content_hash: String,
    },
    TransferCompleteAck {
        transfer_id: String,
        verified: bool,
        error: Option<String>,
    },
    ManifestReq {
        folder_id: String,
        since_event_seq: u64,
    },
    ManifestResp {
        folder_id: String,
        events: Vec<SyncEvent>,
        latest_event_seq: u64,
    },
    FolderListReq,
    FolderListResp {
        folders: Vec<SharedFolderInfo>,
    },
    SyncNotify {
        folder_id: String,
        latest_event_seq: u64,
    },
    FilePullReq {
        folder_id: String,
        file_id: String,
        relative_path: String,
        #[serde(default)]
        dest_path: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncEventType {
    Upsert,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncEvent {
    pub event_seq: u64,
    pub folder_id: String,
    pub file_id: String,
    pub relative_path: String,
    pub event_type: SyncEventType,
    pub version: u64,
    pub size_bytes: u64,
    pub content_hash: String,
    pub origin_node_id: NodeId,
    pub modified_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedFolderInfo {
    pub folder_id: String,
    pub name: String,
    pub created_at: i64,
}

#[derive(Debug, Default)]
pub struct MessageCodec;

impl tokio_util::codec::Decoder for MessageCodec {
    type Item = Message;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // Need at least 8 bytes: 4 bytes MAGIC + 4 bytes length
        if src.len() < 8 {
            return Ok(None);
        }

        // Check magic bytes
        if &src[0..4] != MAGIC_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid protocol magic header",
            ));
        }

        let length = u32::from_be_bytes([src[4], src[5], src[6], src[7]]) as usize;
        if length > MAX_MESSAGE_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Message length {} exceeds max limit {}", length, MAX_MESSAGE_SIZE),
            ));
        }

        if src.len() < 8 + length {
            // Wait for remaining frame bytes
            src.reserve(8 + length - src.len());
            return Ok(None);
        }

        // Advance past header
        src.advance(8);
        let payload = src.split_to(length);

        let msg: Message = serde_json::from_slice(&payload).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("Failed to parse message: {}", e))
        })?;

        Ok(Some(msg))
    }
}

impl tokio_util::codec::Encoder<Message> for MessageCodec {
    type Error = io::Error;

    fn encode(&mut self, item: Message, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let payload = serde_json::to_vec(&item).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("Failed to serialize message: {}", e))
        })?;

        let length = payload.len() as u32;
        dst.reserve(8 + payload.len());
        dst.put_slice(MAGIC_BYTES);
        dst.put_u32(length);
        dst.put_slice(&payload);
        Ok(())
    }
}
