use crate::models::{FileRecord, SharedFolder};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use tusher_core::identity::NodeId;
use tusher_core::protocol::{SyncEvent, SyncEventType};

pub struct MetadataDb {
    conn: Connection,
}

impl MetadataDb {
    pub fn open<P: AsRef<Path>>(db_path: P) -> anyhow::Result<Self> {
        if let Some(parent) = db_path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        let mut db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        let mut db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&mut self) -> anyhow::Result<()> {
        self.conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;

             CREATE TABLE IF NOT EXISTS shared_folders (
                 folder_id TEXT PRIMARY KEY,
                 name TEXT NOT NULL,
                 local_path TEXT NOT NULL,
                 created_at INTEGER NOT NULL
             );

             CREATE TABLE IF NOT EXISTS folder_members (
                 folder_id TEXT NOT NULL REFERENCES shared_folders(folder_id) ON DELETE CASCADE,
                 peer_id TEXT NOT NULL,
                 sync_policy TEXT NOT NULL DEFAULT 'full',
                 PRIMARY KEY (folder_id, peer_id)
             );

             CREATE TABLE IF NOT EXISTS files (
                 file_id TEXT PRIMARY KEY,
                 folder_id TEXT NOT NULL REFERENCES shared_folders(folder_id) ON DELETE CASCADE,
                 relative_path TEXT NOT NULL,
                 filename TEXT NOT NULL,
                 size_bytes INTEGER NOT NULL,
                 content_hash TEXT NOT NULL,
                 version INTEGER NOT NULL,
                 origin_node_id TEXT NOT NULL,
                 modified_at INTEGER NOT NULL,
                 is_deleted INTEGER NOT NULL DEFAULT 0,
                 tombstone_at INTEGER,
                 UNIQUE (folder_id, relative_path)
             );

             CREATE INDEX IF NOT EXISTS idx_files_lookup ON files(folder_id, relative_path);

             CREATE TABLE IF NOT EXISTS sync_events (
                 event_seq INTEGER PRIMARY KEY AUTOINCREMENT,
                 folder_id TEXT NOT NULL REFERENCES shared_folders(folder_id) ON DELETE CASCADE,
                 file_id TEXT NOT NULL,
                 relative_path TEXT NOT NULL,
                 event_type TEXT NOT NULL,
                 version INTEGER NOT NULL,
                 size_bytes INTEGER NOT NULL,
                 content_hash TEXT NOT NULL,
                 origin_node_id TEXT NOT NULL,
                 modified_at INTEGER NOT NULL,
                 created_at INTEGER NOT NULL
             );

             CREATE INDEX IF NOT EXISTS idx_sync_events_seq ON sync_events(folder_id, event_seq);",
        )?;
        Ok(())
    }

    pub fn add_folder(&mut self, folder: &SharedFolder) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO shared_folders (folder_id, name, local_path, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(folder_id) DO UPDATE SET
                name = excluded.name,
                local_path = excluded.local_path",
            params![
                folder.folder_id,
                folder.name,
                folder.local_path,
                folder.created_at
            ],
        )?;
        Ok(())
    }

    pub fn get_folders(&self) -> anyhow::Result<Vec<SharedFolder>> {
        let mut stmt = self.conn.prepare(
            "SELECT folder_id, name, local_path, created_at FROM shared_folders ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SharedFolder {
                folder_id: row.get(0)?,
                name: row.get(1)?,
                local_path: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    pub fn get_folder(&self, folder_id: &str) -> anyhow::Result<Option<SharedFolder>> {
        let mut stmt = self.conn.prepare(
            "SELECT folder_id, name, local_path, created_at FROM shared_folders WHERE folder_id = ?1",
        )?;
        let folder = stmt
            .query_row(params![folder_id], |row| {
                Ok(SharedFolder {
                    folder_id: row.get(0)?,
                    name: row.get(1)?,
                    local_path: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })
            .optional()?;
        Ok(folder)
    }

    pub fn get_file(&self, folder_id: &str, relative_path: &str) -> anyhow::Result<Option<FileRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT file_id, folder_id, relative_path, filename, size_bytes, content_hash,
                    version, origin_node_id, modified_at, is_deleted, tombstone_at
             FROM files
             WHERE folder_id = ?1 AND relative_path = ?2",
        )?;
        let norm_path = normalize_path(relative_path);
        let record = stmt
            .query_row(params![folder_id, norm_path], |row| {
                let origin_str: String = row.get(7)?;
                Ok(FileRecord {
                    file_id: row.get(0)?,
                    folder_id: row.get(1)?,
                    relative_path: row.get(2)?,
                    filename: row.get(3)?,
                    size_bytes: row.get(4)?,
                    content_hash: row.get(5)?,
                    version: row.get(6)?,
                    origin_node_id: NodeId::from_str_unchecked(&origin_str),
                    modified_at: row.get(8)?,
                    is_deleted: row.get::<_, i64>(9)? != 0,
                    tombstone_at: row.get(10)?,
                })
            })
            .optional()?;
        Ok(record)
    }

    pub fn get_all_files(&self, folder_id: &str) -> anyhow::Result<Vec<FileRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT file_id, folder_id, relative_path, filename, size_bytes, content_hash,
                    version, origin_node_id, modified_at, is_deleted, tombstone_at
             FROM files
             WHERE folder_id = ?1 AND is_deleted = 0
             ORDER BY relative_path ASC",
        )?;
        let rows = stmt.query_map(params![folder_id], |row| {
            let origin_str: String = row.get(7)?;
            Ok(FileRecord {
                file_id: row.get(0)?,
                folder_id: row.get(1)?,
                relative_path: row.get(2)?,
                filename: row.get(3)?,
                size_bytes: row.get(4)?,
                content_hash: row.get(5)?,
                version: row.get(6)?,
                origin_node_id: NodeId::from_str_unchecked(&origin_str),
                modified_at: row.get(8)?,
                is_deleted: row.get::<_, i64>(9)? != 0,
                tombstone_at: row.get(10)?,
            })
        })?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    /// Records a local file creation or modification, increments version, and appends to sync_events log
    pub fn record_local_upsert(
        &mut self,
        folder_id: &str,
        relative_path: &str,
        size_bytes: u64,
        content_hash: &str,
        origin_node_id: &NodeId,
    ) -> anyhow::Result<SyncEvent> {
        let norm_path = normalize_path(relative_path);
        let filename = Path::new(&norm_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unnamed")
            .to_string();

        let now = chrono::Utc::now().timestamp();
        let existing = self.get_file(folder_id, &norm_path)?;

        let (file_id, new_version) = match &existing {
            Some(curr) => (curr.file_id.clone(), curr.version + 1),
            None => {
                let id = format!("f_{}", &content_hash[..16.min(content_hash.len())]);
                (id, 1)
            }
        };

        let tx = self.conn.transaction()?;

        tx.execute(
            "INSERT INTO files (
                file_id, folder_id, relative_path, filename, size_bytes,
                content_hash, version, origin_node_id, modified_at, is_deleted, tombstone_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, NULL)
             ON CONFLICT(folder_id, relative_path) DO UPDATE SET
                file_id = excluded.file_id,
                filename = excluded.filename,
                size_bytes = excluded.size_bytes,
                content_hash = excluded.content_hash,
                version = excluded.version,
                origin_node_id = excluded.origin_node_id,
                modified_at = excluded.modified_at,
                is_deleted = 0,
                tombstone_at = NULL",
            params![
                file_id,
                folder_id,
                norm_path,
                filename,
                size_bytes as i64,
                content_hash,
                new_version as i64,
                origin_node_id.as_str(),
                now
            ],
        )?;

        tx.execute(
            "INSERT INTO sync_events (
                folder_id, file_id, relative_path, event_type, version,
                size_bytes, content_hash, origin_node_id, modified_at, created_at
             ) VALUES (?1, ?2, ?3, 'UPSERT', ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                folder_id,
                file_id,
                norm_path,
                new_version as i64,
                size_bytes as i64,
                content_hash,
                origin_node_id.as_str(),
                now,
                now
            ],
        )?;

        let event_seq = tx.last_insert_rowid() as u64;
        tx.commit()?;

        Ok(SyncEvent {
            event_seq,
            folder_id: folder_id.to_string(),
            file_id,
            relative_path: norm_path,
            event_type: SyncEventType::Upsert,
            version: new_version,
            size_bytes,
            content_hash: content_hash.to_string(),
            origin_node_id: origin_node_id.clone(),
            modified_at: now,
        })
    }

    /// Records a local file deletion as a tombstone and appends to sync_events log
    pub fn record_local_delete(
        &mut self,
        folder_id: &str,
        relative_path: &str,
        origin_node_id: &NodeId,
    ) -> anyhow::Result<Option<SyncEvent>> {
        let norm_path = normalize_path(relative_path);
        let existing = match self.get_file(folder_id, &norm_path)? {
            Some(f) if !f.is_deleted => f,
            _ => return Ok(None),
        };

        let now = chrono::Utc::now().timestamp();
        let new_version = existing.version + 1;

        let tx = self.conn.transaction()?;

        tx.execute(
            "UPDATE files
             SET is_deleted = 1, tombstone_at = ?1, version = ?2, modified_at = ?1, origin_node_id = ?3
             WHERE folder_id = ?4 AND relative_path = ?5",
            params![now, new_version as i64, origin_node_id.as_str(), folder_id, norm_path],
        )?;

        tx.execute(
            "INSERT INTO sync_events (
                folder_id, file_id, relative_path, event_type, version,
                size_bytes, content_hash, origin_node_id, modified_at, created_at
             ) VALUES (?1, ?2, ?3, 'DELETE', ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                folder_id,
                existing.file_id,
                norm_path,
                new_version as i64,
                0i64,
                existing.content_hash,
                origin_node_id.as_str(),
                now,
                now
            ],
        )?;

        let event_seq = tx.last_insert_rowid() as u64;
        tx.commit()?;

        Ok(Some(SyncEvent {
            event_seq,
            folder_id: folder_id.to_string(),
            file_id: existing.file_id,
            relative_path: norm_path,
            event_type: SyncEventType::Delete,
            version: new_version,
            size_bytes: 0,
            content_hash: existing.content_hash,
            origin_node_id: origin_node_id.clone(),
            modified_at: now,
        }))
    }

    /// Applies an incoming remote upsert into the local catalog and replication log
    pub fn apply_remote_upsert(&mut self, event: &SyncEvent) -> anyhow::Result<()> {
        let norm_path = normalize_path(&event.relative_path);
        let filename = Path::new(&norm_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unnamed")
            .to_string();

        let tx = self.conn.transaction()?;

        tx.execute(
            "INSERT INTO files (
                file_id, folder_id, relative_path, filename, size_bytes,
                content_hash, version, origin_node_id, modified_at, is_deleted, tombstone_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, NULL)
             ON CONFLICT(folder_id, relative_path) DO UPDATE SET
                file_id = excluded.file_id,
                filename = excluded.filename,
                size_bytes = excluded.size_bytes,
                content_hash = excluded.content_hash,
                version = excluded.version,
                origin_node_id = excluded.origin_node_id,
                modified_at = excluded.modified_at,
                is_deleted = 0,
                tombstone_at = NULL",
            params![
                event.file_id,
                event.folder_id,
                norm_path,
                filename,
                event.size_bytes as i64,
                event.content_hash,
                event.version as i64,
                event.origin_node_id.as_str(),
                event.modified_at
            ],
        )?;

        // Idempotently record into sync_events for transitive mesh propagation
        let already_present: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sync_events WHERE folder_id = ?1 AND relative_path = ?2 AND version = ?3 AND content_hash = ?4 AND event_type = 'UPSERT')",
            params![event.folder_id, norm_path, event.version as i64, event.content_hash],
            |row| row.get(0),
        )?;

        if !already_present {
            let now = chrono::Utc::now().timestamp();
            tx.execute(
                "INSERT INTO sync_events (
                    folder_id, file_id, relative_path, event_type, version,
                    size_bytes, content_hash, origin_node_id, modified_at, created_at
                 ) VALUES (?1, ?2, ?3, 'UPSERT', ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    event.folder_id,
                    event.file_id,
                    norm_path,
                    event.version as i64,
                    event.size_bytes as i64,
                    event.content_hash,
                    event.origin_node_id.as_str(),
                    event.modified_at,
                    now
                ],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    /// Applies an incoming remote tombstone into the local catalog and replication log
    pub fn apply_remote_delete(&mut self, event: &SyncEvent) -> anyhow::Result<()> {
        let norm_path = normalize_path(&event.relative_path);
        let now = chrono::Utc::now().timestamp();

        let tx = self.conn.transaction()?;

        tx.execute(
            "UPDATE files
             SET is_deleted = 1, tombstone_at = ?1, version = ?2, modified_at = ?3, origin_node_id = ?4
             WHERE folder_id = ?5 AND relative_path = ?6",
            params![
                now,
                event.version as i64,
                event.modified_at,
                event.origin_node_id.as_str(),
                event.folder_id,
                norm_path
            ],
        )?;

        // Idempotently record tombstone into sync_events for transitive mesh propagation
        let already_present: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sync_events WHERE folder_id = ?1 AND relative_path = ?2 AND version = ?3 AND event_type = 'DELETE')",
            params![event.folder_id, norm_path, event.version as i64],
            |row| row.get(0),
        )?;

        if !already_present {
            tx.execute(
                "INSERT INTO sync_events (
                    folder_id, file_id, relative_path, event_type, version,
                    size_bytes, content_hash, origin_node_id, modified_at, created_at
                 ) VALUES (?1, ?2, ?3, 'DELETE', ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    event.folder_id,
                    event.file_id,
                    norm_path,
                    event.version as i64,
                    0i64,
                    event.content_hash,
                    event.origin_node_id.as_str(),
                    event.modified_at,
                    now
                ],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    /// Retrieves events since a given sequence number for delta exchange
    pub fn get_events_since(
        &self,
        folder_id: &str,
        since_event_seq: u64,
        limit: usize,
    ) -> anyhow::Result<(Vec<SyncEvent>, u64)> {
        let mut stmt = self.conn.prepare(
            "SELECT event_seq, folder_id, file_id, relative_path, event_type, version,
                    size_bytes, content_hash, origin_node_id, modified_at
             FROM sync_events
             WHERE folder_id = ?1 AND event_seq > ?2
             ORDER BY event_seq ASC
             LIMIT ?3",
        )?;

        let rows = stmt.query_map(params![folder_id, since_event_seq as i64, limit as i64], |row| {
            let type_str: String = row.get(4)?;
            let event_type = if type_str == "DELETE" {
                SyncEventType::Delete
            } else {
                SyncEventType::Upsert
            };
            let origin_str: String = row.get(8)?;

            Ok(SyncEvent {
                event_seq: row.get(0)?,
                folder_id: row.get(1)?,
                file_id: row.get(2)?,
                relative_path: row.get(3)?,
                event_type,
                version: row.get(5)?,
                size_bytes: row.get(6)?,
                content_hash: row.get(7)?,
                origin_node_id: NodeId::from_str_unchecked(&origin_str),
                modified_at: row.get(9)?,
            })
        })?;

        let mut events = Vec::new();
        for r in rows {
            events.push(r?);
        }

        let latest_seq = self.get_latest_event_seq(folder_id)?;
        Ok((events, latest_seq))
    }

    pub fn get_latest_event_seq(&self, folder_id: &str) -> anyhow::Result<u64> {
        let mut stmt = self.conn.prepare(
            "SELECT COALESCE(MAX(event_seq), 0) FROM sync_events WHERE folder_id = ?1",
        )?;
        let max_seq: i64 = stmt.query_row(params![folder_id], |row| row.get(0))?;
        Ok(max_seq as u64)
    }
}

fn normalize_path(p: &str) -> String {
    p.replace('\\', "/").trim_start_matches('/').to_string()
}
