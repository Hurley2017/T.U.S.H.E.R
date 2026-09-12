// crates/tusher-sync/src/watcher.rs
// Native filesystem watcher with debouncing, ignore filtering, and loop suppression.

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{debug, info, warn};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsChangeEvent {
    Upsert {
        folder_id: String,
        relative_path: String,
        size_bytes: u64,
        content_hash: String,
        modified_at: i64,
    },
    Delete {
        folder_id: String,
        relative_path: String,
        deleted_at: i64,
    },
}

/// Helper to determine if a path or component should be ignored
pub fn should_ignore_path(path: &Path) -> bool {
    for comp in path.components() {
        if let std::path::Component::Normal(os_str) = comp {
            let s = os_str.to_string_lossy();
            // Ignore hidden files / directories and tusher internal staging
            if s.starts_with('.') || s.starts_with('~') || s.starts_with("~$") {
                return true;
            }
            // Ignore common temp extensions
            if s.ends_with(".tmp")
                || s.ends_with(".crdownload")
                || s.ends_with(".part")
                || s.ends_with(".swp")
                || s.ends_with(".bak")
            {
                return true;
            }
        }
    }
    false
}

/// Normalizes a path relative to a base folder into a cross-platform forward-slash string
pub fn normalize_relative_path(base: &Path, full_path: &Path) -> Option<String> {
    let rel = full_path.strip_prefix(base).ok()?;
    let mut safe = PathBuf::new();
    for comp in rel.components() {
        if let std::path::Component::Normal(c) = comp {
            safe.push(c);
        }
    }
    if safe.as_os_str().is_empty() {
        return None;
    }
    Some(safe.to_string_lossy().replace('\\', "/"))
}

pub struct FolderWatcher {
    /// folder_id -> root_path
    watched_folders: Arc<RwLock<HashMap<String, PathBuf>>>,
    /// Paths currently suppressed to prevent feedback loops when coordinator writes files
    suppressed_paths: Arc<Mutex<HashMap<PathBuf, Instant>>>,
    watcher: Option<RecommendedWatcher>,
    debounce_duration: Duration,
}

impl FolderWatcher {
    pub fn new(debounce_duration: Duration) -> Self {
        Self {
            watched_folders: Arc::new(RwLock::new(HashMap::new())),
            suppressed_paths: Arc::new(Mutex::new(HashMap::new())),
            watcher: None,
            debounce_duration,
        }
    }

    /// Temporarily suppress watcher events for a given path for the specified duration (e.g. during downloads)
    pub async fn suppress_path(&self, path: &Path, duration: Duration) {
        let mut suppressed = self.suppressed_paths.lock().await;
        suppressed.insert(path.to_path_buf(), Instant::now() + duration);
    }

    /// Starts watching registered folders and streaming debounced `FsChangeEvent`s
    pub async fn start(
        &mut self,
        folders: HashMap<String, PathBuf>,
    ) -> anyhow::Result<mpsc::Receiver<FsChangeEvent>> {
        {
            let mut wf = self.watched_folders.write().await;
            *wf = folders.clone();
        }

        let (raw_tx, raw_rx) = mpsc::channel::<notify::Result<Event>>(256);
        let (out_tx, out_rx) = mpsc::channel::<FsChangeEvent>(128);

        // Create the underlying OS watcher
        let mut watcher = RecommendedWatcher::new(
            move |res| {
                let _ = raw_tx.blocking_send(res);
            },
            Config::default(),
        )?;

        // Register each folder with the OS watcher
        for (folder_id, path) in &folders {
            if !path.exists() {
                tokio::fs::create_dir_all(path).await?;
            }
            info!("Watching shared folder '{}' at {}", folder_id, path.display());
            watcher.watch(path, RecursiveMode::Recursive)?;
        }

        self.watcher = Some(watcher);

        let watched_folders = Arc::clone(&self.watched_folders);
        let suppressed_paths = Arc::clone(&self.suppressed_paths);
        let debounce_duration = self.debounce_duration;

        // Background Debouncer Task allocated on heap
        tokio::spawn(Box::pin(run_debouncer(
            raw_rx,
            out_tx,
            watched_folders,
            suppressed_paths,
            debounce_duration,
        )));

        Ok(out_rx)
    }

    /// Dynamically add a shared folder to the active watcher
    pub async fn add_folder<P: AsRef<Path>>(&mut self, folder_id: &str, path: P) -> anyhow::Result<()> {
        let p = path.as_ref().to_path_buf();
        if !p.exists() {
            tokio::fs::create_dir_all(&p).await?;
        }
        {
            let mut wf = self.watched_folders.write().await;
            wf.insert(folder_id.to_string(), p.clone());
        }
        if let Some(watcher) = &mut self.watcher {
            watcher.watch(&p, RecursiveMode::Recursive)?;
        }
        Ok(())
    }
}

async fn run_debouncer(
    mut raw_rx: mpsc::Receiver<notify::Result<Event>>,
    out_tx: mpsc::Sender<FsChangeEvent>,
    watched_folders: Arc<RwLock<HashMap<String, PathBuf>>>,
    suppressed_paths: Arc<Mutex<HashMap<PathBuf, Instant>>>,
    debounce_duration: Duration,
) {
    let mut pending_paths: HashMap<(String, PathBuf), Instant> = HashMap::new();

    loop {
        let sleep_duration = if let Some((_, earliest_deadline)) =
            pending_paths.iter().min_by_key(|(_, deadline)| *deadline)
        {
            let now = Instant::now();
            if *earliest_deadline <= now {
                Duration::from_millis(0)
            } else {
                *earliest_deadline - now
            }
        } else {
            Duration::from_millis(100)
        };

        tokio::select! {
            Some(event_res) = raw_rx.recv() => {
                match event_res {
                    Ok(event) => {
                        match event.kind {
                            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                                let folders_map = watched_folders.read().await;

                                for path in event.paths {
                                    if should_ignore_path(&path) {
                                        continue;
                                    }

                                    // Check suppression
                                    {
                                        let mut suppressed = suppressed_paths.lock().await;
                                        let now = Instant::now();
                                        suppressed.retain(|_, until| *until > now);
                                        if suppressed.contains_key(&path) {
                                            debug!("Suppressing watcher event for path {}", path.display());
                                            continue;
                                        }
                                    }

                                    // Match against registered shared folders
                                    for (folder_id, folder_root) in folders_map.iter() {
                                        if path.starts_with(folder_root) {
                                            let key = (folder_id.clone(), path.clone());
                                            let deadline = Instant::now() + debounce_duration;
                                            pending_paths.insert(key, deadline);
                                            break;
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    Err(e) => {
                        warn!("Filesystem watcher error: {}", e);
                    }
                }
            }

            _ = tokio::time::sleep(sleep_duration), if !pending_paths.is_empty() => {
                let now = Instant::now();
                let mut ready_keys = Vec::new();

                for (key, deadline) in &pending_paths {
                    if *deadline <= now {
                        ready_keys.push(key.clone());
                    }
                }

                for key in ready_keys {
                    pending_paths.remove(&key);
                    let (folder_id, full_path) = key;

                    let folders_map = watched_folders.read().await;
                    let folder_root = match folders_map.get(&folder_id) {
                        Some(r) => r.clone(),
                        None => continue,
                    };
                    drop(folders_map);

                    let rel_path_opt = normalize_relative_path(&folder_root, &full_path);
                    let relative_path = match rel_path_opt {
                        Some(r) => r,
                        None => continue,
                    };

                    // Double-check suppression
                    {
                        let mut suppressed = suppressed_paths.lock().await;
                        let now = Instant::now();
                        suppressed.retain(|_, until| *until > now);
                        if suppressed.contains_key(&full_path) {
                            continue;
                        }
                    }

                    if full_path.exists() {
                        if full_path.is_file() {
                            match tokio::fs::metadata(&full_path).await {
                                Ok(meta) => {
                                    let size_bytes = meta.len();
                                    let modified_at = meta
                                        .modified()
                                        .ok()
                                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                        .map(|d| d.as_secs() as i64)
                                        .unwrap_or_else(|| chrono::Utc::now().timestamp());

                                    match tusher_transfer::hash::hash_file(&full_path).await {
                                        Ok(content_hash) => {
                                            let ev = FsChangeEvent::Upsert {
                                                folder_id,
                                                relative_path,
                                                size_bytes,
                                                content_hash,
                                                modified_at,
                                            };
                                            let _ = out_tx.send(ev).await;
                                        }
                                        Err(e) => {
                                            debug!("Failed to hash file {}: {}", full_path.display(), e);
                                        }
                                    }
                                }
                                Err(e) => {
                                    debug!("Failed to get metadata for {}: {}", full_path.display(), e);
                                }
                            }
                        }
                    } else {
                        // File was deleted or moved away
                        let ev = FsChangeEvent::Delete {
                            folder_id,
                            relative_path,
                            deleted_at: chrono::Utc::now().timestamp(),
                        };
                        let _ = out_tx.send(ev).await;
                    }
                }
            }
        }
    }
}

