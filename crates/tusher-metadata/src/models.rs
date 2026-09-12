use serde::{Deserialize, Serialize};
use tusher_core::identity::NodeId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedFolder {
    pub folder_id: String,
    pub name: String,
    pub local_path: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRecord {
    pub file_id: String,
    pub folder_id: String,
    pub relative_path: String,
    pub filename: String,
    pub size_bytes: u64,
    pub content_hash: String,
    pub version: u64,
    pub origin_node_id: NodeId,
    pub modified_at: i64,
    pub is_deleted: bool,
    pub tombstone_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReconciliationAction {
    UpToDate {
        file_id: String,
        version: u64,
    },
    DownloadNeeded {
        folder_id: String,
        file_id: String,
        relative_path: String,
        size_bytes: u64,
        content_hash: String,
        version: u64,
        origin_node_id: NodeId,
    },
    DeleteLocal {
        folder_id: String,
        file_id: String,
        relative_path: String,
        version: u64,
    },
    Conflict {
        folder_id: String,
        file_id: String,
        original_relative_path: String,
        conflict_relative_path: String,
        size_bytes: u64,
        content_hash: String,
        version: u64,
        origin_node_id: NodeId,
    },
    Ignore,
}
