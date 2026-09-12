use crate::db::MetadataDb;
use crate::models::{ReconciliationAction, SharedFolder};
use crate::reconciler::ReconciliationEngine;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use tusher_core::identity::NodeId;
use tusher_core::protocol::SyncEvent;

#[derive(Clone)]
pub struct MetadataService {
    db: Arc<Mutex<MetadataDb>>,
}

impl MetadataService {
    pub fn open<P: AsRef<Path>>(db_path: P) -> anyhow::Result<Self> {
        let db = MetadataDb::open(db_path)?;
        Ok(Self {
            db: Arc::new(Mutex::new(db)),
        })
    }

    pub fn open_in_memory() -> anyhow::Result<Self> {
        let db = MetadataDb::open_in_memory()?;
        Ok(Self {
            db: Arc::new(Mutex::new(db)),
        })
    }

    pub async fn add_folder(
        &self,
        folder_id: &str,
        name: &str,
        local_path: &str,
    ) -> anyhow::Result<SharedFolder> {
        let folder = SharedFolder {
            folder_id: folder_id.to_string(),
            name: name.to_string(),
            local_path: local_path.to_string(),
            created_at: chrono::Utc::now().timestamp(),
        };

        let mut db = self.db.lock().await;
        db.add_folder(&folder)?;
        Ok(folder)
    }

    pub async fn list_folders(&self) -> anyhow::Result<Vec<SharedFolder>> {
        let db = self.db.lock().await;
        db.get_folders()
    }

    pub async fn get_folder(&self, folder_id: &str) -> anyhow::Result<Option<SharedFolder>> {
        let db = self.db.lock().await;
        db.get_folder(folder_id)
    }

    pub async fn record_local_file(
        &self,
        folder_id: &str,
        relative_path: &str,
        size_bytes: u64,
        content_hash: &str,
        origin_node_id: &NodeId,
    ) -> anyhow::Result<SyncEvent> {
        let mut db = self.db.lock().await;
        db.record_local_upsert(
            folder_id,
            relative_path,
            size_bytes,
            content_hash,
            origin_node_id,
        )
    }

    pub async fn record_local_delete(
        &self,
        folder_id: &str,
        relative_path: &str,
        origin_node_id: &NodeId,
    ) -> anyhow::Result<Option<SyncEvent>> {
        let mut db = self.db.lock().await;
        db.record_local_delete(folder_id, relative_path, origin_node_id)
    }

    pub async fn handle_manifest_request(
        &self,
        folder_id: &str,
        since_event_seq: u64,
    ) -> anyhow::Result<(Vec<SyncEvent>, u64)> {
        let db = self.db.lock().await;
        db.get_events_since(folder_id, since_event_seq, 500)
    }

    pub async fn reconcile_remote_events(
        &self,
        events: &[SyncEvent],
    ) -> anyhow::Result<Vec<ReconciliationAction>> {
        let db = self.db.lock().await;
        let mut actions = Vec::with_capacity(events.len());

        for ev in events {
            let act = ReconciliationEngine::reconcile_event(&db, ev)?;
            actions.push(act);
        }

        Ok(actions)
    }

    pub async fn apply_reconciliation_action(
        &self,
        action: &ReconciliationAction,
    ) -> anyhow::Result<()> {
        let mut db = self.db.lock().await;
        match action {
            ReconciliationAction::DownloadNeeded {
                folder_id,
                file_id,
                relative_path,
                size_bytes,
                content_hash,
                version,
                origin_node_id,
            } => {
                let event = SyncEvent {
                    event_seq: 0,
                    folder_id: folder_id.clone(),
                    file_id: file_id.clone(),
                    relative_path: relative_path.clone(),
                    event_type: tusher_core::protocol::SyncEventType::Upsert,
                    version: *version,
                    size_bytes: *size_bytes,
                    content_hash: content_hash.clone(),
                    origin_node_id: origin_node_id.clone(),
                    modified_at: chrono::Utc::now().timestamp(),
                };
                db.apply_remote_upsert(&event)?;
            }
            ReconciliationAction::DeleteLocal {
                folder_id,
                file_id,
                relative_path,
                version,
            } => {
                let event = SyncEvent {
                    event_seq: 0,
                    folder_id: folder_id.clone(),
                    file_id: file_id.clone(),
                    relative_path: relative_path.clone(),
                    event_type: tusher_core::protocol::SyncEventType::Delete,
                    version: *version,
                    size_bytes: 0,
                    content_hash: "".to_string(),
                    origin_node_id: NodeId::from_str_unchecked("remote"),
                    modified_at: chrono::Utc::now().timestamp(),
                };
                db.apply_remote_delete(&event)?;
            }
            ReconciliationAction::Conflict {
                folder_id,
                file_id,
                conflict_relative_path,
                size_bytes,
                content_hash,
                version,
                origin_node_id,
                ..
            } => {
                // Record the conflict file as a distinct, tracked entity in the catalog
                let event = SyncEvent {
                    event_seq: 0,
                    folder_id: folder_id.clone(),
                    file_id: format!("{}_conflict", file_id),
                    relative_path: conflict_relative_path.clone(),
                    event_type: tusher_core::protocol::SyncEventType::Upsert,
                    version: *version,
                    size_bytes: *size_bytes,
                    content_hash: content_hash.clone(),
                    origin_node_id: origin_node_id.clone(),
                    modified_at: chrono::Utc::now().timestamp(),
                };
                db.apply_remote_upsert(&event)?;
            }
            _ => {}
        }
        Ok(())
    }

    pub async fn get_all_files(&self, folder_id: &str) -> anyhow::Result<Vec<crate::models::FileRecord>> {
        let db = self.db.lock().await;
        db.get_all_files(folder_id)
    }

    pub async fn get_file(&self, folder_id: &str, relative_path: &str) -> anyhow::Result<Option<crate::models::FileRecord>> {
        let db = self.db.lock().await;
        db.get_file(folder_id, relative_path)
    }

    pub async fn get_db_handle(&self) -> Arc<Mutex<MetadataDb>> {
        Arc::clone(&self.db)
    }
}
