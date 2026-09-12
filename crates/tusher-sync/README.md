# `tusher-sync`

**Native Filesystem Watcher, Automated Synchronization & Mesh Coordinator for T.U.S.H.E.R**

---

## 1. Overview

`tusher-sync` binds the local filesystem to the distributed mesh:
- **Real-Time Filesystem Watching**: Cross-platform file change detection using `notify` (`ReadDirectoryChangesW` on Windows, `inotify` on Linux, `kqueue`/`FSEvents` on macOS).
- **Debouncing & Aggregation**: Collapses bursty OS write events to ensure files are fully written before hashing and broadcasting.
- **Echo Suppression Map**: Time-based suppression cache prevents infinite synchronization echo loops when writing files pulled from peers.
- **Transitive Mesh Synchronization ($A \leftrightarrow B \leftrightarrow C$)**: Coordinates multi-hop replication through intermediate relay nodes.

---

## 2. Core Modules

### 2.1 Native Filesystem Watcher (`src/watcher.rs`)
- `FolderWatcher`:
  - Dynamically registers directories with `notify::RecommendedWatcher`.
  - Debounce window: 300 ms (configurable).
  - Automatically filters ignored patterns (`.staging/`, `.tmp`, hidden system files).
  - Emits normalized `FsChangeEvent::Upsert` and `FsChangeEvent::Delete`.
- `suppress_path(path, duration)`:
  - Temporarily suppresses watcher events for a path for $N$ seconds.
  - Used when writing remote downloads or applying remote deletions to avoid re-triggering local sync events.

### 2.2 Synchronization Coordinator (`src/coordinator.rs`)
- `SyncCoordinator`:
  - **Local Ingestion**:
    $$\text{Watcher Event} \to \text{SHA-256 Hash} \to \text{SQLite Upsert} \to \text{Broadcast SyncNotify}$$
  - **Remote Notification Handling**:
    $$\text{SyncNotify} \to \text{Query Manifest Delta} \to \text{Reconciliation} \to \text{FilePullReq} \to \text{Download} \to \text{Disk Placement}$$
  - **Transitive Cascading**:
    When a file download completes on disk on Node B, `trigger_sync(folder_id)` is invoked, broadcasting `SyncNotify` to downstream Node C.
  - **In-Flight Concurrency Scoping**:
    Prevents redundant simultaneous sync operations for the same `(folder_id, peer_id)` pair while ensuring locks are cleanly released across early exits.
