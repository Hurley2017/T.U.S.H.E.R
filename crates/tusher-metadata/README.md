# `tusher-metadata`

**Distributed Causal Metadata Store, Event Sourcing & Conflict Reconciliation for T.U.S.H.E.R**

---

## 1. Overview

`tusher-metadata` provides the distributed state foundation of the mesh:
- **Event Sourcing**: Every file creation, modification, or deletion is recorded as a monotonic event in a local SQLite database (`tusher_metadata.db`).
- **Causal Versioning**: Lamport-style monotonically increasing versions track causality across nodes.
- **Delta Manifest Exchange**: Peers exchange only events occurring after their highest known sequence number (`since_event_seq`), keeping bandwidth minimal.
- **Tombstone Lifecycle**: Deletions are logged as tombstones (`is_deleted = 1`) to ensure deleted files are not resurrected by older replicas.
- **Zero-Data-Loss Conflict Resolution**: Concurrent conflicting modifications branch into deterministic conflict paths.

---

## 2. Core Modules

### 2.1 Database Schema (`src/db.rs`)
- `files`: Tracks current file state on disk:
  ```sql
  CREATE TABLE files (
      file_id TEXT NOT NULL,
      folder_id TEXT NOT NULL,
      relative_path TEXT NOT NULL,
      filename TEXT NOT NULL,
      size_bytes INTEGER NOT NULL,
      content_hash TEXT NOT NULL,
      version INTEGER NOT NULL,
      origin_node_id TEXT NOT NULL,
      modified_at INTEGER NOT NULL,
      is_deleted INTEGER NOT NULL DEFAULT 0,
      tombstone_at INTEGER,
      PRIMARY KEY (folder_id, relative_path)
  );
  ```
- `sync_events`: Monotonic append-only replication log:
  ```sql
  CREATE TABLE sync_events (
      event_seq INTEGER PRIMARY KEY AUTOINCREMENT,
      folder_id TEXT NOT NULL,
      file_id TEXT NOT NULL,
      relative_path TEXT NOT NULL,
      event_type TEXT NOT NULL, -- UPSERT or DELETE
      version INTEGER NOT NULL,
      size_bytes INTEGER NOT NULL,
      content_hash TEXT NOT NULL,
      origin_node_id TEXT NOT NULL,
      modified_at INTEGER NOT NULL,
      created_at INTEGER NOT NULL
  );
  ```

### 2.2 Causal Reconciliation Engine (`src/service.rs`)
- `reconcile_remote_events`: Compares remote event sequence against local SQLite state:
  - **`UpToDate`**: Local state is already identical or ahead in version.
  - **`DownloadNeeded`**: Remote version is strictly higher ($\text{remote.version} > \text{local.version}$). Node issues `FilePullReq`.
  - **`DeleteLocal`**: Remote event is a deletion tombstone for an existing local file.
  - **`Conflict`**: Concurrent edits detected ($\text{version}_{\text{remote}} == \text{version}_{\text{local}}$ with differing content hashes). Branches deterministically into:
    ```text
    filename (Conflict - <origin_node_id> <timestamp>).ext
    ```
    Preserves both versions without data loss.

### 2.3 Transitive Event Propagation
When a relay node applies a remote event from Node A, it writes the record to `files` and appends the event to `sync_events` (preserving Node A's `origin_node_id`). Downstream peers querying the relay discover the event, enabling multi-hop propagation without direct links.
