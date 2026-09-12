// crates/tusher-ffi/src/lib.rs
// UniFFI and JNI bindings exposing the T.U.S.H.E.R mesh engine to Android and native platforms.

uniffi::setup_scaffolding!();

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::RwLock;

use tusher_core::crypto::calculate_sas_pin;
use tusher_core::identity::{DeviceIdentity, NodeId};
use tusher_core::types::TrustStatus;
use tusher_metadata::service::MetadataService;
use tusher_network::manager::ConnectionManager;
use tusher_network::transport::TransportAddress;
use tusher_sync::coordinator::SyncCoordinator;
use tusher_transfer::service::TransferService;

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum TusherFfiError {
    #[error("Initialization error: {msg}")]
    InitializationError { msg: String },
    #[error("Network error: {msg}")]
    NetworkError { msg: String },
    #[error("Sync error: {msg}")]
    SyncError { msg: String },
    #[error("Transfer error: {msg}")]
    TransferError { msg: String },
    #[error("Storage error: {msg}")]
    StorageError { msg: String },
    #[error("Peer not found: {peer_id}")]
    PeerNotFound { peer_id: String },
    #[error("Invalid argument: {msg}")]
    InvalidArgument { msg: String },
    #[error("Internal error: {msg}")]
    InternalError { msg: String },
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct FfiNodeStatus {
    pub node_id: String,
    pub node_name: String,
    pub platform: String,
    pub public_key_hex: String,
    pub listen_port: u16,
    pub discovery_port: u16,
    pub active_peers_count: u32,
    pub storage_dir: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct FfiPeerStatus {
    pub node_id: String,
    pub node_name: String,
    pub is_connected: bool,
    pub active_transport: Option<String>,
    pub latency_ms: Option<f64>,
    pub is_paired: bool,
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct FfiSharedFolder {
    pub folder_id: String,
    pub name: String,
    pub local_path: String,
    pub file_count: u64,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct FfiTransferStats {
    pub total_bytes: u64,
    pub chunks_sent: u32,
    pub chunks_skipped: u32,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct FfiPairingInvite {
    pub token: String,
    pub uri: String,
}

#[uniffi::export(callback_interface)]
pub trait FfiEventListener: Send + Sync {
    fn on_peer_status_changed(&self, peer_id: String, peer_name: String, is_connected: bool);
    fn on_sync_event(&self, folder_id: String, relative_path: String, event_type: String);
    fn on_transfer_completed(&self, folder_id: String, file_name: String, file_size: u64, content_hash: String);
}

#[derive(uniffi::Object)]
pub struct TusherNode {
    identity: Arc<DeviceIdentity>,
    network: Arc<ConnectionManager>,
    metadata: Arc<MetadataService>,
    transfer: Arc<TransferService>,
    coordinator: Arc<SyncCoordinator>,
    runtime: Arc<Runtime>,
    event_listeners: Arc<RwLock<Vec<Box<dyn FfiEventListener>>>>,
    data_dir: PathBuf,
}

#[uniffi::export]
impl TusherNode {
    /// Initializes and starts a fully autonomous T.U.S.H.E.R mesh node
    #[uniffi::constructor]
    pub fn new(
        data_dir: String,
        node_name: String,
        port: u16,
        discovery_port: u16,
    ) -> Result<Arc<TusherNode>, TusherFfiError> {
        let path = PathBuf::from(&data_dir);
        std::fs::create_dir_all(&path).map_err(|e| TusherFfiError::StorageError {
            msg: format!("Failed to create data directory: {}", e),
        })?;

        let staging_dir = path.join("staging");
        let downloads_dir = path.join("downloads");
        std::fs::create_dir_all(&staging_dir).map_err(|e| TusherFfiError::StorageError {
            msg: format!("Failed to create staging directory: {}", e),
        })?;
        std::fs::create_dir_all(&downloads_dir).map_err(|e| TusherFfiError::StorageError {
            msg: format!("Failed to create downloads directory: {}", e),
        })?;

        let runtime = Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .map_err(|e| TusherFfiError::InitializationError {
                msg: format!("Failed to initialize Tokio runtime: {}", e),
            })?;

        let identity = Arc::new(DeviceIdentity::generate(node_name));
        let db_path = path.join("tusher_metadata.db");
        let metadata = Arc::new(
            MetadataService::open(&db_path).map_err(|e| TusherFfiError::StorageError {
                msg: format!("Failed to open metadata database: {}", e),
            })?,
        );

        let transfer = runtime
            .block_on(async { TransferService::new(&staging_dir, &downloads_dir).await })
            .map_err(|e| TusherFfiError::InitializationError {
                msg: format!("Failed to initialize transfer service: {}", e),
            })?;
        let transfer = Arc::new(transfer);

        let network = Arc::new(ConnectionManager::new(Arc::clone(&identity), port, discovery_port));

        let coordinator = Arc::new(SyncCoordinator::new(
            identity.node_id().clone(),
            Arc::clone(&metadata),
            Arc::clone(&transfer),
            Arc::clone(&network),
            Duration::from_millis(300),
        ));

        // Start background network and sync daemons
        let net_start = Arc::clone(&network);
        let coord_start = Arc::clone(&coordinator);
        runtime
            .block_on(async move {
                let _ = net_start.start().await?;
                let _ = coord_start.start().await?;
                Ok::<(), anyhow::Error>(())
            })
            .map_err(|e| TusherFfiError::InitializationError {
                msg: format!("Failed to start background network/sync coordinator: {}", e),
            })?;

        Ok(Arc::new(TusherNode {
            identity,
            network,
            metadata,
            transfer,
            coordinator,
            runtime: Arc::new(runtime),
            event_listeners: Arc::new(RwLock::new(Vec::new())),
            data_dir: path,
        }))
    }

    /// Returns the local node identity, listen ports, and status overview
    #[uniffi::method]
    pub fn get_status(&self) -> Result<FfiNodeStatus, TusherFfiError> {
        let peers = self.runtime.block_on(async { self.network.get_peer_statuses().await });
        let active_count = peers.iter().filter(|p| p.is_connected).count() as u32;

        Ok(FfiNodeStatus {
            node_id: self.identity.node_id().to_string(),
            node_name: self.identity.node_name().to_string(),
            platform: self.identity.platform().to_string(),
            public_key_hex: self.identity.public_key_hex(),
            listen_port: self.network.listen_port(),
            discovery_port: 0,
            active_peers_count: active_count,
            storage_dir: self.data_dir.to_string_lossy().to_string(),
        })
    }

    /// Lists all discovered and active peers in the mesh
    #[uniffi::method]
    pub fn get_peers(&self) -> Result<Vec<FfiPeerStatus>, TusherFfiError> {
        let peers = self.runtime.block_on(async { self.network.get_peer_statuses().await });

        let ffi_peers = peers
            .into_iter()
            .map(|p| FfiPeerStatus {
                node_id: p.node_id.to_string(),
                node_name: p.node_name,
                is_connected: p.is_connected,
                active_transport: p.active_transport.map(|t| format!("{:?}", t)),
                latency_ms: p.latency.map(|d| d.as_secs_f64() * 1000.0),
                is_paired: p.is_paired,
                candidates: p
                    .available_candidates
                    .into_iter()
                    .map(|c| format!("{} ({:?})", c.addr, c.transport_type))
                    .collect(),
            })
            .collect();

        Ok(ffi_peers)
    }

    /// Creates a pairing invite token and hint URI to display as QR code or share
    #[uniffi::method]
    pub fn create_pairing_invite(&self, hint_addr: Option<String>) -> Result<FfiPairingInvite, TusherFfiError> {
        let hint = hint_addr.unwrap_or_else(|| format!("127.0.0.1:{}", self.network.listen_port()));
        let (token, uri) = self
            .runtime
            .block_on(async { self.network.pairing_manager().create_pairing_invite(&hint).await });

        Ok(FfiPairingInvite { token, uri })
    }

    /// Connects to an endpoint candidate directly by IP:Port
    #[uniffi::method]
    pub fn connect_peer(&self, addr_str: String) -> Result<(), TusherFfiError> {
        let addr = addr_str
            .parse::<std::net::SocketAddr>()
            .map_err(|e| TusherFfiError::InvalidArgument {
                msg: format!("Invalid socket address {}: {}", addr_str, e),
            })?;

        let target = TransportAddress::new(addr, tusher_core::types::TransportType::Lan);
        let placeholder = NodeId::from_str_unchecked("remote");

        self.runtime
            .block_on(async { self.network.add_candidate(placeholder, target).await });

        Ok(())
    }

    /// Completes Short Authentication String (SAS) numeric PIN pairing with target peer
    #[uniffi::method]
    pub fn pair_peer(&self, peer_id: String) -> Result<(), TusherFfiError> {
        let target_id = NodeId::from_str_unchecked(&peer_id);
        self.runtime.block_on(async {
            self.network
                .pairing_manager()
                .set_trusted(target_id, TrustStatus::Paired)
                .await;
        });

        Ok(())
    }

    /// Marks all currently connected peers as trusted (for automation/quick setup)
    #[uniffi::method]
    pub fn trust_all_peers(&self) -> Result<(), TusherFfiError> {
        self.runtime.block_on(async {
            let peers = self.network.get_peer_statuses().await;
            for p in peers {
                self.network
                    .pairing_manager()
                    .set_trusted(p.node_id, TrustStatus::Paired)
                    .await;
            }
        });

        Ok(())
    }

    /// Registers a local directory as a synchronized shared folder
    #[uniffi::method]
    pub fn add_shared_folder(&self, folder_id: String, name: String, local_path: String) -> Result<(), TusherFfiError> {
        let p = PathBuf::from(&local_path);
        self.runtime
            .block_on(async {
                self.coordinator.register_folder(&folder_id, &p).await?;
                let _ = self.metadata.add_folder(&folder_id, &name, &p.to_string_lossy()).await;
                let _ = self.coordinator.initial_scan(&folder_id).await;
                let _ = self.coordinator.trigger_sync(&folder_id).await;
                Ok::<(), anyhow::Error>(())
            })
            .map_err(|e| TusherFfiError::SyncError {
                msg: format!("Failed to register shared folder: {}", e),
            })?;

        Ok(())
    }

    /// Lists all registered shared folders
    #[uniffi::method]
    pub fn list_shared_folders(&self) -> Result<Vec<FfiSharedFolder>, TusherFfiError> {
        let folders = self
            .runtime
            .block_on(async { self.metadata.list_folders().await })
            .map_err(|e| TusherFfiError::StorageError {
                msg: format!("Failed to list folders: {}", e),
            })?;

        let mut list = Vec::new();
        for f in folders {
            let count = self
                .runtime
                .block_on(async { self.metadata.get_all_files(&f.folder_id).await })
                .map(|files| files.len() as u64)
                .unwrap_or(0);

            list.push(FfiSharedFolder {
                folder_id: f.folder_id,
                name: f.name,
                local_path: f.local_path,
                file_count: count,
            });
        }

        Ok(list)
    }

    /// Broadcasts an immediate sync notification to trigger delta reconciliation
    #[uniffi::method]
    pub fn trigger_sync(&self, folder_id: String) -> Result<(), TusherFfiError> {
        self.runtime
            .block_on(async { self.coordinator.trigger_sync(&folder_id).await })
            .map_err(|e| TusherFfiError::SyncError {
                msg: format!("Failed to broadcast sync notification: {}", e),
            })?;

        Ok(())
    }

    /// Forces a full local filesystem scan and metadata index
    #[uniffi::method]
    pub fn scan_and_index_folder(&self, folder_id: String) -> Result<(), TusherFfiError> {
        self.runtime
            .block_on(async { self.coordinator.initial_scan(&folder_id).await })
            .map_err(|e| TusherFfiError::SyncError {
                msg: format!("Failed to index folder: {}", e),
            })?;

        Ok(())
    }

    /// Transmits a point-to-point chunked file transfer to a peer
    #[uniffi::method]
    pub fn send_file(&self, peer_id: String, file_path: String) -> Result<FfiTransferStats, TusherFfiError> {
        let p = PathBuf::from(&file_path);
        if !p.exists() {
            return Err(TusherFfiError::InvalidArgument {
                msg: format!("Source file does not exist: {}", file_path),
            });
        }

        let target_node = NodeId::from_str_unchecked(&peer_id);
        let stats = self
            .runtime
            .block_on(async {
                let conn = self
                    .network
                    .get_active_connection(&target_node)
                    .await
                    .ok_or_else(|| anyhow::anyhow!("Peer is not currently connected"))?;

                self.transfer.send_file(&conn, &p).await
            })
            .map_err(|e| TusherFfiError::TransferError {
                msg: format!("File transfer failed: {}", e),
            })?;

        Ok(FfiTransferStats {
            total_bytes: stats.total_bytes,
            chunks_sent: stats.chunks_sent,
            chunks_skipped: stats.chunks_skipped,
            elapsed_ms: stats.elapsed.as_millis() as u64,
        })
    }

    /// Registers an event callback listener for Android UI / Foreground service
    #[uniffi::method]
    pub fn register_event_listener(&self, listener: Box<dyn FfiEventListener>) -> Result<(), TusherFfiError> {
        self.runtime.block_on(async {
            let mut listeners = self.event_listeners.write().await;
            listeners.push(listener);
        });

        Ok(())
    }

    /// Artificially disables or restores LAN transport to simulate failover
    #[uniffi::method]
    pub fn simulate_lan_drop(&self, disabled: bool) -> Result<(), TusherFfiError> {
        self.network.set_simulate_lan_disabled(disabled);
        Ok(())
    }
}

/// Computes a deterministic Short Authentication String (SAS) numeric PIN
#[uniffi::export]
pub fn compute_sas_pin(ephemeral_token: String, key_a_hex: String, key_b_hex: String) -> Result<String, TusherFfiError> {
    let key_a = hex::decode(&key_a_hex).map_err(|e| TusherFfiError::InvalidArgument {
        msg: format!("Invalid hex for key A: {}", e),
    })?;
    let key_b = hex::decode(&key_b_hex).map_err(|e| TusherFfiError::InvalidArgument {
        msg: format!("Invalid hex for key B: {}", e),
    })?;

    if key_a.len() != 32 || key_b.len() != 32 {
        return Err(TusherFfiError::InvalidArgument {
            msg: "Keys must be 32 bytes".to_string(),
        });
    }

    Ok(calculate_sas_pin(&ephemeral_token, &key_a, &key_b))
}

/// Returns the current protocol and engine version string
#[uniffi::export]
pub fn tusher_version() -> String {
    format!("T.U.S.H.E.R v{}", env!("CARGO_PKG_VERSION"))
}
