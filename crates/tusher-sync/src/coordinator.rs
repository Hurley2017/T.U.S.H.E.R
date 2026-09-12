// crates/tusher-sync/src/coordinator.rs
// Automated two-way synchronization coordinator for T.U.S.H.E.R.

use crate::watcher::{normalize_relative_path, should_ignore_path, FolderWatcher, FsChangeEvent};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, error, info, warn};
use tusher_core::identity::NodeId;
use tusher_core::protocol::Message;
use tusher_metadata::models::ReconciliationAction;
use tusher_metadata::service::MetadataService;
use tusher_network::manager::ConnectionManager;
use tusher_transfer::service::TransferService;

pub struct SyncCoordinator {
    node_id: NodeId,
    metadata: Arc<MetadataService>,
    transfer: Arc<TransferService>,
    network: Arc<ConnectionManager>,
    watcher: Arc<Mutex<FolderWatcher>>,
    registered_folders: Arc<RwLock<HashMap<String, PathBuf>>>,
    last_seen_seqs: Arc<RwLock<HashMap<(String, NodeId), u64>>>,
    in_flight_syncs: Arc<Mutex<HashSet<(String, NodeId)>>>,
    remote_folders: Arc<RwLock<HashMap<NodeId, (String, String, Vec<tusher_core::protocol::SharedFolderInfo>)>>>,
    debounce_duration: Duration,
}

impl SyncCoordinator {
    pub fn new(
        node_id: NodeId,
        metadata: Arc<MetadataService>,
        transfer: Arc<TransferService>,
        network: Arc<ConnectionManager>,
        debounce_duration: Duration,
    ) -> Self {
        let watcher = Arc::new(Mutex::new(FolderWatcher::new(debounce_duration)));
        Self {
            node_id,
            metadata,
            transfer,
            network,
            watcher,
            registered_folders: Arc::new(RwLock::new(HashMap::new())),
            last_seen_seqs: Arc::new(RwLock::new(HashMap::new())),
            in_flight_syncs: Arc::new(Mutex::new(HashSet::new())),
            remote_folders: Arc::new(RwLock::new(HashMap::new())),
            debounce_duration,
        }
    }

    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    pub fn metadata(&self) -> &Arc<MetadataService> {
        &self.metadata
    }

    pub fn transfer(&self) -> &Arc<TransferService> {
        &self.transfer
    }

    pub fn network(&self) -> &Arc<ConnectionManager> {
        &self.network
    }

    pub fn debounce_duration(&self) -> Duration {
        self.debounce_duration
    }

    pub async fn get_remote_folders(
        &self,
    ) -> HashMap<NodeId, (String, String, Vec<tusher_core::protocol::SharedFolderInfo>)> {
        let rf = self.remote_folders.read().await;
        rf.clone()
    }

    /// Registers a folder to participate in the decentralized sync mesh
    pub async fn register_folder<P: AsRef<Path>>(&self, folder_id: &str, path: P) -> anyhow::Result<()> {
        let p = path.as_ref().to_path_buf();
        if !p.exists() {
            tokio::fs::create_dir_all(&p).await?;
        }

        // Register in metadata service DB
        let folder_name = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(folder_id);
        let _ = self.metadata.add_folder(folder_id, folder_name, &p.to_string_lossy()).await;

        // Register in transfer service for incoming file placement
        self.transfer.register_folder_destination(folder_id, &p).await;

        // Save in coordinator map
        {
            let mut rf = self.registered_folders.write().await;
            rf.insert(folder_id.to_string(), p.clone());
        }

        // Dynamically add to active watcher
        {
            let mut w = self.watcher.lock().await;
            let _ = w.add_folder(folder_id, &p).await;
        }

        info!("Registered folder '{}' at {}", folder_id, p.display());

        // Broadcast updated folder list to all active connections
        if let Ok(folders) = self.metadata.list_folders().await {
            let mut info_list = Vec::new();
            for f in folders {
                let count = self.metadata.get_all_files(&f.folder_id).await.map(|files| files.len()).unwrap_or(0);
                info_list.push(tusher_core::protocol::SharedFolderInfo {
                    folder_id: f.folder_id,
                    name: f.name,
                    file_count: count,
                    created_at: f.created_at,
                });
            }
            self.network.broadcast(Message::FolderListResp { folders: info_list }).await;
        }

        Ok(())
    }

    /// Scans existing files on disk, indexing anything not yet recorded in SQLite
    pub async fn initial_scan(&self, folder_id: &str) -> anyhow::Result<()> {
        let folder_root = {
            let rf = self.registered_folders.read().await;
            match rf.get(folder_id) {
                Some(r) => r.clone(),
                None => return Ok(()),
            }
        };

        info!("Scanning existing files for folder '{}' at {}", folder_id, folder_root.display());

        for entry in walkdir::WalkDir::new(&folder_root)
            .into_iter()
            .filter_entry(|e| !should_ignore_path(e.path()))
        {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let rel_opt = normalize_relative_path(&folder_root, path);
            let relative_path = match rel_opt {
                Some(r) => r,
                None => continue,
            };

            let meta = match tokio::fs::metadata(path).await {
                Ok(m) => m,
                Err(_) => continue,
            };

            let size = meta.len();
            let existing_record = self.metadata.get_file(folder_id, &relative_path).await?;

            let mut needs_indexing = true;
            if let Some(record) = existing_record {
                if !record.is_deleted && record.size_bytes == size {
                    // Check if content hash matches
                    let hash = tusher_transfer::hash::hash_file(path).await?;
                    if record.content_hash == hash {
                        needs_indexing = false;
                    }
                }
            }

            if needs_indexing {
                let content_hash = tusher_transfer::hash::hash_file(path).await?;
                info!("Indexing local file: {}/{} ({} bytes)", folder_id, relative_path, size);
                self.metadata
                    .record_local_file(folder_id, &relative_path, size, &content_hash, &self.node_id)
                    .await?;
            }
        }

        Ok(())
    }

    /// Handles incoming network messages from peers
    async fn handle_peer_message(
        self: Arc<Self>,
        peer_id: NodeId,
        msg: Message,
    ) -> Option<Message> {
        info!("Received peer message from {}: {:?}", peer_id, msg);
        match msg {
            Message::ManifestReq {
                folder_id,
                since_event_seq,
            } => {
                info!("Handling ManifestReq for '{}' from peer {} (since seq {})", folder_id, peer_id, since_event_seq);
                match self.metadata.handle_manifest_request(&folder_id, since_event_seq).await {
                    Ok((events, latest_event_seq)) => {
                        info!("Sending ManifestResp for '{}' to peer {} ({} events, latest seq {})", folder_id, peer_id, events.len(), latest_event_seq);
                        Some(Message::ManifestResp {
                            folder_id,
                            events,
                            latest_event_seq,
                        })
                    }
                    Err(e) => {
                        warn!("Failed to produce manifest for {}: {}", folder_id, e);
                        None
                    }
                }
            }

            Message::TransferInit { .. }
            | Message::TransferChunk { .. }
            | Message::TransferComplete { .. } => {
                match self.transfer.handle_incoming_message(msg).await {
                    Ok(resp) => resp,
                    Err(e) => {
                        error!("Transfer error handling incoming message: {}", e);
                        None
                    }
                }
            }

            Message::FilePullReq {
                folder_id,
                file_id: _,
                relative_path,
                dest_path,
            } => {
                let folders = self.registered_folders.read().await;
                if let Some(folder_root) = folders.get(&folder_id) {
                    let local_file_path = folder_root.join(&relative_path);
                    let send_dest = dest_path.unwrap_or_else(|| relative_path.clone());

                    if local_file_path.exists() {
                        if let Some(conn) = self.network.get_active_connection(&peer_id).await {
                            let transfer = Arc::clone(&self.transfer);
                            let folder_id_clone = folder_id.clone();
                            tokio::spawn(async move {
                                info!(
                                    "Streaming requested file {} as {} to peer {}",
                                    local_file_path.display(),
                                    send_dest,
                                    peer_id
                                );
                                if let Err(e) = transfer
                                    .send_file_with_target_path(
                                        &conn,
                                        Some(folder_id_clone),
                                        local_file_path,
                                        send_dest,
                                    )
                                    .await
                                {
                                    error!("Failed to stream file to peer {}: {}", peer_id, e);
                                }
                            });
                        }
                    } else {
                        warn!("Peer requested file pull but file does not exist: {}", local_file_path.display());
                    }
                }
                None
            }

            Message::SyncNotify {
                folder_id,
                latest_event_seq,
            } => {
                let this = Arc::clone(&self);
                let pid = peer_id.clone();
                tokio::spawn(async move {
                    if let Err(e) = this.reconcile_with_peer(&pid, &folder_id, latest_event_seq).await {
                        warn!("Sync reconciliation with peer {} failed: {}", pid, e);
                    }
                });
                None
            }

            Message::FolderListReq => {
                match self.metadata.list_folders().await {
                    Ok(folders) => {
                        let mut info_list = Vec::new();
                        for f in folders {
                            let count = self.metadata.get_all_files(&f.folder_id).await.map(|files| files.len()).unwrap_or(0);
                            info_list.push(tusher_core::protocol::SharedFolderInfo {
                                folder_id: f.folder_id,
                                name: f.name,
                                file_count: count,
                                created_at: f.created_at,
                            });
                        }
                        Some(Message::FolderListResp { folders: info_list })
                    }
                    Err(_) => None,
                }
            }

            Message::FolderListResp { folders } => {
                if let Some(conn) = self.network.get_active_connection(&peer_id).await {
                    let name = conn.remote_name().to_string();
                    let plat = format!("{:?}", conn.remote_platform());
                    let mut rf = self.remote_folders.write().await;
                    rf.insert(peer_id.clone(), (name, plat, folders));
                }
                None
            }

            _ => None,
        }
    }

    /// Reconciles differences with a specific peer for a given shared folder
    pub async fn reconcile_with_peer(
        &self,
        peer_id: &NodeId,
        folder_id: &str,
        peer_latest_seq: u64,
    ) -> anyhow::Result<()> {
        // Concurrency guard: avoid redundant simultaneous syncs for the same folder & peer
        let sync_key = (folder_id.to_string(), peer_id.clone());
        {
            let mut in_flight = self.in_flight_syncs.lock().await;
            if !in_flight.insert(sync_key.clone()) {
                debug!("Sync already in flight for {:?}, skipping duplicate trigger", sync_key);
                return Ok(());
            }
        }

        let result = self.do_reconcile(peer_id, folder_id, peer_latest_seq).await;

        // Release concurrency guard
        {
            let mut in_flight = self.in_flight_syncs.lock().await;
            in_flight.remove(&sync_key);
        }

        result
    }

    async fn do_reconcile(
        &self,
        peer_id: &NodeId,
        folder_id: &str,
        peer_latest_seq: u64,
    ) -> anyhow::Result<()> {
        let conn = match self.network.get_active_connection(peer_id).await {
            Some(c) => c,
            None => {
                debug!("No active connection to peer {} to reconcile", peer_id);
                return Ok(());
            }
        };

        let folder_root = {
            let rf = self.registered_folders.read().await;
            match rf.get(folder_id) {
                Some(p) => p.clone(),
                None => {
                    debug!("Folder '{}' not registered locally, skipping reconcile", folder_id);
                    return Ok(());
                }
            }
        };

        let since_seq = {
            let seqs = self.last_seen_seqs.read().await;
            seqs.get(&(folder_id.to_string(), peer_id.clone())).cloned().unwrap_or(0)
        };

        if since_seq >= peer_latest_seq {
            return Ok(());
        }

        info!(
            "Requesting manifest delta for '{}' from peer {} (seq {} -> {})",
            folder_id, peer_id, since_seq, peer_latest_seq
        );

        conn.send(Message::ManifestReq {
            folder_id: folder_id.to_string(),
            since_event_seq: since_seq,
        })
        .await?;

        // Await ManifestResp
        let resp = tokio::time::timeout(Duration::from_secs(6), conn.recv_manifest_response())
            .await
            .map_err(|_| anyhow::anyhow!("Timeout waiting for ManifestResp from {}", peer_id))?
            .ok_or_else(|| anyhow::anyhow!("Peer connection closed while awaiting ManifestResp"))?;

        let (events, latest_seq) = match resp {
            Message::ManifestResp {
                events,
                latest_event_seq,
                ..
            } => (events, latest_event_seq),
            other => anyhow::bail!("Expected ManifestResp from peer, received {:?}", other),
        };

        info!("Received {} delta sync events from peer {}", events.len(), peer_id);

        let actions = self.metadata.reconcile_remote_events(&events).await?;
        info!("Reconciliation generated {} actions for folder '{}'", actions.len(), folder_id);
        for act in &actions {
            info!("  Action to apply: {:?}", act);
        }

        for action in actions {
            match action {
                ReconciliationAction::DownloadNeeded {
                    ref folder_id,
                    ref file_id,
                    ref relative_path,
                    size_bytes,
                    ref content_hash,
                    ..
                } => {
                    let dest_file_path = folder_root.join(relative_path);
                    if dest_file_path.exists() {
                        if let Ok(local_hash) = tusher_transfer::hash::hash_file(&dest_file_path).await {
                            if &local_hash == content_hash {
                                // Content already identical on disk
                                self.metadata.apply_reconciliation_action(&action).await?;
                                continue;
                            }
                        }
                    }

                    // Suppress watcher on destination path to avoid loop
                    {
                        let w = self.watcher.lock().await;
                        w.suppress_path(&dest_file_path, Duration::from_secs(10)).await;
                    }

                    info!(
                        "PULLING: {}/{} ({} bytes) from {}",
                        folder_id, relative_path, size_bytes, peer_id
                    );

                    conn.send(Message::FilePullReq {
                        folder_id: folder_id.clone(),
                        file_id: file_id.clone(),
                        relative_path: relative_path.clone(),
                        dest_path: None,
                    })
                    .await?;

                    self.metadata.apply_reconciliation_action(&action).await?;
                }

                ReconciliationAction::DeleteLocal {
                    ref folder_id,
                    ref relative_path,
                    ..
                } => {
                    let target_path = folder_root.join(relative_path);
                    if target_path.exists() {
                        {
                            let w = self.watcher.lock().await;
                            w.suppress_path(&target_path, Duration::from_secs(10)).await;
                        }
                        info!("Applying remote delete to local file: {}", target_path.display());
                        let _ = tokio::fs::remove_file(&target_path).await;
                    }
                    self.metadata.apply_reconciliation_action(&action).await?;
                    let _ = self.trigger_sync(folder_id).await;
                }

                ReconciliationAction::Conflict {
                    ref folder_id,
                    ref file_id,
                    ref original_relative_path,
                    ref conflict_relative_path,
                    ..
                } => {
                    let conflict_dest = folder_root.join(conflict_relative_path);
                    {
                        let w = self.watcher.lock().await;
                        w.suppress_path(&conflict_dest, Duration::from_secs(10)).await;
                    }

                    info!(
                        "BRANCHING CONFLICT: Preserving local file {}, pulling remote as {}",
                        original_relative_path, conflict_relative_path
                    );

                    conn.send(Message::FilePullReq {
                        folder_id: folder_id.clone(),
                        file_id: file_id.clone(),
                        relative_path: original_relative_path.clone(),
                        dest_path: Some(conflict_relative_path.clone()),
                    })
                    .await?;

                    self.metadata.apply_reconciliation_action(&action).await?;
                }

                _ => {}
            }
        }

        // Update highest seen seq from peer
        {
            let mut seqs = self.last_seen_seqs.write().await;
            seqs.insert((folder_id.to_string(), peer_id.clone()), latest_seq);
        }

        Ok(())
    }

    /// Triggers an immediate sync broadcast for a shared folder
    pub async fn trigger_sync(&self, folder_id: &str) -> anyhow::Result<()> {
        let (_, latest_seq) = self.metadata.handle_manifest_request(folder_id, 0).await?;
        self.network
            .broadcast(Message::SyncNotify {
                folder_id: folder_id.to_string(),
                latest_event_seq: latest_seq,
            })
            .await;
        Ok(())
    }

    /// Starts the complete synchronization engine
    pub async fn start(self: Arc<Self>) -> anyhow::Result<tokio::task::JoinHandle<()>> {
        // 1. Register file completion callback with TransferService
        let (comp_tx, mut comp_rx) = tokio::sync::mpsc::channel(128);
        self.transfer.set_completion_channel(comp_tx).await;

        // 2. Wire network request handler
        let this_handler = Arc::clone(&self);
        self.network
            .set_request_handler(Arc::new(move |peer_id, msg| {
                let c = Arc::clone(&this_handler);
                Box::pin(async move { c.handle_peer_message(peer_id, msg).await })
            }))
            .await;

        // 3. Perform initial scan on all registered folders
        let initial_folders: HashMap<String, PathBuf> = {
            let rf = self.registered_folders.read().await;
            rf.clone()
        };

        for folder_id in initial_folders.keys() {
            let _ = self.initial_scan(folder_id).await;
        }

        // 4. Start FolderWatcher
        let mut watcher_rx = {
            let mut w = self.watcher.lock().await;
            w.start(initial_folders.clone()).await?
        };

        // 5. Worker task: Transfer Completion listener
        // 5. Worker task: Transfer Completion listener
        let this_comp = Arc::clone(&self);
        tokio::spawn(async move {
            while let Some(info) = comp_rx.recv().await {
                info!(
                    "Transfer completed on disk for {} ({} bytes, hash {})",
                    info.dest_path.display(),
                    info.file_size,
                    &info.content_hash[..16]
                );
                // Suppress watcher for 5 seconds to prevent echo
                {
                    let w = this_comp.watcher.lock().await;
                    w.suppress_path(&info.dest_path, Duration::from_secs(5)).await;
                }
                if let Some(folder_id) = &info.folder_id {
                    let _ = this_comp.trigger_sync(folder_id).await;
                }
            }
        });

        // 6. Worker task: Filesystem Watcher Event listener
        let this_watcher = Arc::clone(&self);
        tokio::spawn(async move {
            while let Some(event) = watcher_rx.recv().await {
                match event {
                    FsChangeEvent::Upsert {
                        folder_id,
                        relative_path,
                        size_bytes,
                        content_hash,
                        ..
                    } => {
                        // Loop prevention check: Does DB already have identical hash?
                        if let Ok(Some(rec)) = this_watcher.metadata.get_file(&folder_id, &relative_path).await {
                            if !rec.is_deleted
                                && rec.content_hash == content_hash
                                && rec.size_bytes == size_bytes
                            {
                                debug!("Ignoring FS event: {} already up to date in DB", relative_path);
                                continue;
                            }
                        }

                        info!(
                            "Local change detected: Upsert {}/{} ({} bytes)",
                            folder_id, relative_path, size_bytes
                        );

                        match this_watcher
                            .metadata
                            .record_local_file(
                                &folder_id,
                                &relative_path,
                                size_bytes,
                                &content_hash,
                                &this_watcher.node_id,
                            )
                            .await
                        {
                            Ok(sync_ev) => {
                                info!(
                                    "Broadcasting SyncNotify for '{}' seq {}",
                                    folder_id, sync_ev.event_seq
                                );
                                this_watcher
                                    .network
                                    .broadcast(Message::SyncNotify {
                                        folder_id: folder_id.clone(),
                                        latest_event_seq: sync_ev.event_seq,
                                    })
                                    .await;
                            }
                            Err(e) => {
                                error!("Failed to record local upsert in DB: {}", e);
                            }
                        }
                    }

                    FsChangeEvent::Delete {
                        folder_id,
                        relative_path,
                        ..
                    } => {
                        // Loop prevention: Check if already deleted in DB
                        if let Ok(Some(rec)) = this_watcher.metadata.get_file(&folder_id, &relative_path).await {
                            if rec.is_deleted {
                                debug!("Ignoring FS event: {} already deleted in DB", relative_path);
                                continue;
                            }
                        }

                        info!("Local change detected: Delete {}/{}", folder_id, relative_path);

                        match this_watcher
                            .metadata
                            .record_local_delete(&folder_id, &relative_path, &this_watcher.node_id)
                            .await
                        {
                            Ok(Some(sync_ev)) => {
                                info!(
                                    "Broadcasting SyncNotify (Delete) for '{}' seq {}",
                                    folder_id, sync_ev.event_seq
                                );
                                this_watcher
                                    .network
                                    .broadcast(Message::SyncNotify {
                                        folder_id: folder_id.clone(),
                                        latest_event_seq: sync_ev.event_seq,
                                    })
                                    .await;
                            }
                            Ok(None) => {}
                            Err(e) => {
                                error!("Failed to record local delete in DB: {}", e);
                            }
                        }
                    }
                }
            }
        });

        // 7. Background Periodic Sync / Heartbeat Broadcast Loop
        let this_periodic = Arc::clone(&self);
        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;

                let folders: Vec<String> = {
                    let rf = this_periodic.registered_folders.read().await;
                    rf.keys().cloned().collect()
                };

                for folder_id in folders {
                    let _ = this_periodic.trigger_sync(&folder_id).await;
                }

                // Query all active connections for their shared folders
                let conns = this_periodic.network.get_all_active_connections().await;
                for (_, conn) in conns {
                    let _ = conn.send(Message::FolderListReq).await;
                }
            }
        });

        Ok(handle)
    }
}
