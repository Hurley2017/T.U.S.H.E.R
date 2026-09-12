use crate::db::MetadataDb;
use crate::models::ReconciliationAction;
use std::path::Path;
use tusher_core::protocol::{SyncEvent, SyncEventType};

pub struct ReconciliationEngine;

impl ReconciliationEngine {
    pub fn reconcile_event(
        db: &MetadataDb,
        event: &SyncEvent,
    ) -> anyhow::Result<ReconciliationAction> {
        let local_file_opt = db.get_file(&event.folder_id, &event.relative_path)?;

        match local_file_opt {
            None => {
                // Local node does not have this file at all
                match event.event_type {
                    SyncEventType::Upsert => Ok(ReconciliationAction::DownloadNeeded {
                        folder_id: event.folder_id.clone(),
                        file_id: event.file_id.clone(),
                        relative_path: event.relative_path.clone(),
                        size_bytes: event.size_bytes,
                        content_hash: event.content_hash.clone(),
                        version: event.version,
                        origin_node_id: event.origin_node_id.clone(),
                    }),
                    SyncEventType::Delete => {
                        // Remote deleted a file that local doesn't even have; ignore
                        Ok(ReconciliationAction::Ignore)
                    }
                }
            }
            Some(local) => {
                // Check if file content hashes are already identical
                if !local.is_deleted
                    && event.event_type == SyncEventType::Upsert
                    && local.content_hash == event.content_hash
                {
                    return Ok(ReconciliationAction::UpToDate {
                        file_id: local.file_id,
                        version: std::cmp::max(local.version, event.version),
                    });
                }

                match event.event_type {
                    SyncEventType::Delete => {
                        if local.is_deleted {
                            // Already deleted locally
                            Ok(ReconciliationAction::Ignore)
                        } else if event.version >= local.version {
                            // Remote deletion occurred on or after local version -> Clean Delete
                            Ok(ReconciliationAction::DeleteLocal {
                                folder_id: event.folder_id.clone(),
                                file_id: local.file_id,
                                relative_path: event.relative_path.clone(),
                                version: event.version,
                            })
                        } else {
                            // Local file was modified AFTER remote delete -> Local modification wins
                            Ok(ReconciliationAction::Ignore)
                        }
                    }
                    SyncEventType::Upsert => {
                        if local.is_deleted {
                            // Local had deleted it, but remote updated/created with higher version
                            if event.version > local.version {
                                Ok(ReconciliationAction::DownloadNeeded {
                                    folder_id: event.folder_id.clone(),
                                    file_id: event.file_id.clone(),
                                    relative_path: event.relative_path.clone(),
                                    size_bytes: event.size_bytes,
                                    content_hash: event.content_hash.clone(),
                                    version: event.version,
                                    origin_node_id: event.origin_node_id.clone(),
                                })
                            } else {
                                Ok(ReconciliationAction::Ignore)
                            }
                        } else {
                            // Both have active files with different content hashes!
                            // Causality check:
                            if event.version > local.version && event.origin_node_id == local.origin_node_id {
                                // Linear update from same origin node -> Clean fast-forward
                                Ok(ReconciliationAction::DownloadNeeded {
                                    folder_id: event.folder_id.clone(),
                                    file_id: event.file_id.clone(),
                                    relative_path: event.relative_path.clone(),
                                    size_bytes: event.size_bytes,
                                    content_hash: event.content_hash.clone(),
                                    version: event.version,
                                    origin_node_id: event.origin_node_id.clone(),
                                })
                            } else if event.version > local.version && local.origin_node_id != event.origin_node_id && local.modified_at < event.modified_at {
                                // Peer updated newer content without concurrent local change
                                Ok(ReconciliationAction::DownloadNeeded {
                                    folder_id: event.folder_id.clone(),
                                    file_id: event.file_id.clone(),
                                    relative_path: event.relative_path.clone(),
                                    size_bytes: event.size_bytes,
                                    content_hash: event.content_hash.clone(),
                                    version: event.version,
                                    origin_node_id: event.origin_node_id.clone(),
                                })
                            } else {
                                // CONFLICT: Concurrent offline modification!
                                // Zero data loss: Keep local untouched, generate conflict file name
                                let conflict_path = generate_conflict_path(
                                    &event.relative_path,
                                    event.origin_node_id.as_str(),
                                    event.modified_at,
                                );

                                Ok(ReconciliationAction::Conflict {
                                    folder_id: event.folder_id.clone(),
                                    file_id: event.file_id.clone(),
                                    original_relative_path: event.relative_path.clone(),
                                    conflict_relative_path: conflict_path,
                                    size_bytes: event.size_bytes,
                                    content_hash: event.content_hash.clone(),
                                    version: event.version,
                                    origin_node_id: event.origin_node_id.clone(),
                                })
                            }
                        }
                    }
                }
            }
        }
    }
}

pub fn generate_conflict_path(relative_path: &str, origin_node: &str, modified_at: i64) -> String {
    let path = Path::new(relative_path);
    let parent = path.parent().and_then(|p| p.to_str()).unwrap_or("");
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = path.extension().and_then(|e| e.to_str());

    let dt = chrono::DateTime::from_timestamp(modified_at, 0)
        .unwrap_or_else(chrono::Utc::now);
    let time_str = dt.format("%Y%m%d_%H%M%S");
    let node_short = if origin_node.len() > 8 {
        &origin_node[..8]
    } else {
        origin_node
    };

    let filename = match ext {
        Some(extension) => format!("{} (Conflict - {} {}).{}", stem, node_short, time_str, extension),
        None => format!("{} (Conflict - {} {})", stem, node_short, time_str),
    };

    if parent.is_empty() {
        filename
    } else {
        format!("{}/{}", parent.replace('\\', "/"), filename)
    }
}
