use anyhow::Result;
use axum::{
    extract::{Json, State},
    response::{Html, IntoResponse},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::info;

use tusher_core::identity::DeviceIdentity;
use tusher_metadata::MetadataService;
use tusher_network::manager::ConnectionManager;
use tusher_sync::SyncCoordinator;
use tusher_transfer::TransferService;

use crate::shell::{install_context_menu, is_context_menu_installed, uninstall_context_menu};

#[derive(Clone)]
pub struct DesktopState {
    pub identity: Arc<DeviceIdentity>,
    pub manager: Arc<ConnectionManager>,
    pub metadata_service: Arc<MetadataService>,
    pub transfer_service: Arc<TransferService>,
    pub sync_coordinator: Arc<SyncCoordinator>,
    pub sync_paused: Arc<AtomicBool>,
    pub downloads_dir: PathBuf,
    pub tcp_port: u16,
    pub discovery_port: u16,
    pub web_port: u16,
}

#[derive(Serialize)]
pub struct NodeStatusDto {
    pub node_id: String,
    pub node_name: String,
    pub platform: String,
    pub tcp_port: u16,
    pub discovery_port: u16,
    pub web_port: u16,
    pub downloads_dir: String,
    pub active_peers: usize,
    pub shared_folders: usize,
    pub sync_paused: bool,
    pub context_menu_installed: bool,
}

#[derive(Serialize)]
pub struct PeerDto {
    pub node_id: String,
    pub node_name: String,
    pub is_connected: bool,
    pub transport: Option<String>,
    pub latency_ms: Option<f64>,
    pub is_paired: bool,
    pub candidates: Vec<String>,
}

#[derive(Serialize)]
pub struct FolderDto {
    pub folder_id: String,
    pub folder_name: String,
    pub local_path: String,
    pub file_count: usize,
}

#[derive(Serialize)]
pub struct ConflictDto {
    pub folder_id: String,
    pub relative_path: String,
    pub file_size: u64,
    pub modified_at: i64,
}

#[derive(Deserialize)]
pub struct ConnectPeerRequest {
    pub addr: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub struct PairPeerRequest {
    pub addr: String,
    pub pin: String,
}

#[derive(Deserialize)]
pub struct AddFolderRequest {
    pub folder_id: String,
    pub folder_name: String,
    pub path: String,
}

#[derive(Deserialize)]
pub struct FolderActionRequest {
    pub folder_id: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub struct SendFileRequest {
    pub peer_id: String,
    pub folder_id: String,
    pub file_path: String,
}

#[derive(Deserialize)]
pub struct ContextMenuRequest {
    pub install: bool,
}

pub fn create_app(state: DesktopState) -> Router {
    Router::new()
        .route("/", get(index_handler))
        .route("/api/status", get(get_status_handler))
        .route("/api/peers", get(get_peers_handler))
        .route("/api/peers/connect", post(connect_peer_handler))
        .route("/api/peers/pair", post(pair_peer_handler))
        .route("/api/folders", get(get_folders_handler))
        .route("/api/folders/add", post(add_folder_handler))
        .route("/api/folders/scan", post(scan_folder_handler))
        .route("/api/folders/sync", post(sync_folder_handler))
        .route("/api/transfers/send", post(send_file_handler))
        .route("/api/toggle-sync", post(toggle_sync_handler))
        .route("/api/conflicts", get(get_conflicts_handler))
        .route("/api/shell/context-menu", get(get_context_menu_status).post(set_context_menu_status))
        .with_state(state)
}

pub async fn start_web_server(state: DesktopState) -> Result<()> {
    let port = state.web_port;
    let app = create_app(state);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    info!("Starting T.U.S.H.E.R Web Dashboard at http://{}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn index_handler() -> impl IntoResponse {
    Html(include_str!("../assets/dashboard.html"))
}

async fn get_status_handler(State(state): State<DesktopState>) -> impl IntoResponse {
    let peers = state.manager.get_peer_statuses().await;
    let folders = state.metadata_service.list_folders().await.unwrap_or_default();
    let status = NodeStatusDto {
        node_id: state.identity.node_id().to_string(),
        node_name: state.identity.node_name().to_string(),
        platform: state.identity.platform().to_string(),
        tcp_port: state.tcp_port,
        discovery_port: state.discovery_port,
        web_port: state.web_port,
        downloads_dir: state.downloads_dir.to_string_lossy().to_string(),
        active_peers: peers.iter().filter(|p| p.is_connected).count(),
        shared_folders: folders.len(),
        sync_paused: state.sync_paused.load(Ordering::SeqCst),
        context_menu_installed: is_context_menu_installed(),
    };
    Json(status)
}

async fn get_peers_handler(State(state): State<DesktopState>) -> impl IntoResponse {
    let raw_peers = state.manager.get_peer_statuses().await;
    let peers: Vec<PeerDto> = raw_peers
        .into_iter()
        .map(|p| PeerDto {
            node_id: p.node_id.to_string(),
            node_name: p.node_name.clone(),
            is_connected: p.is_connected,
            transport: p.active_transport.map(|t| format!("{:?}", t)),
            latency_ms: p.latency.map(|d| (d.as_secs_f64() * 1000.0 * 100.0).round() / 100.0),
            is_paired: p.is_paired,
            candidates: p
                .available_candidates
                .into_iter()
                .map(|c| format!("{}:{}", c.transport_type, c.addr))
                .collect(),
        })
        .collect();
    Json(peers)
}

async fn connect_peer_handler(
    State(state): State<DesktopState>,
    Json(payload): Json<ConnectPeerRequest>,
) -> impl IntoResponse {
    match payload.addr.parse::<std::net::SocketAddr>() {
        Ok(addr) => {
            let target = tusher_network::transport::TransportAddress::new(addr, tusher_core::types::TransportType::Lan);
            let placeholder = tusher_core::identity::NodeId::from_str_unchecked("remote");
            state.manager.add_candidate(placeholder, target).await;
            Json(serde_json::json!({ "success": true }))
        }
        Err(e) => Json(serde_json::json!({ "success": false, "error": format!("Invalid address: {}", e) })),
    }
}

async fn pair_peer_handler(
    State(state): State<DesktopState>,
    Json(payload): Json<PairPeerRequest>,
) -> impl IntoResponse {
    let target_id = tusher_core::identity::NodeId::from_str_unchecked(&payload.addr);
    state
        .manager
        .pairing_manager()
        .set_trusted(target_id, tusher_core::types::TrustStatus::Paired)
        .await;
    Json(serde_json::json!({ "success": true }))
}

async fn get_folders_handler(State(state): State<DesktopState>) -> impl IntoResponse {
    let mut result = Vec::new();
    if let Ok(folders) = state.metadata_service.list_folders().await {
        for f in folders {
            let count = state
                .metadata_service
                .get_all_files(&f.folder_id)
                .await
                .map(|files| files.len())
                .unwrap_or(0);

            result.push(FolderDto {
                folder_id: f.folder_id,
                folder_name: f.name,
                local_path: f.local_path,
                file_count: count,
            });
        }
    }
    Json(result)
}

async fn add_folder_handler(
    State(state): State<DesktopState>,
    Json(payload): Json<AddFolderRequest>,
) -> impl IntoResponse {
    let path = std::path::Path::new(&payload.path);
    if !path.exists() {
        if let Err(e) = std::fs::create_dir_all(path) {
            return Json(serde_json::json!({ "success": false, "error": format!("Failed to create folder: {}", e) }));
        }
    }

    match state.sync_coordinator.register_folder(&payload.folder_id, path).await {
        Ok(_) => {
            let _ = state.metadata_service.add_folder(&payload.folder_id, &payload.folder_name, &payload.path).await;
            let _ = state.sync_coordinator.initial_scan(&payload.folder_id).await;
            let _ = state.sync_coordinator.trigger_sync(&payload.folder_id).await;
            Json(serde_json::json!({ "success": true }))
        }
        Err(e) => Json(serde_json::json!({ "success": false, "error": e.to_string() })),
    }
}

async fn scan_folder_handler(
    State(state): State<DesktopState>,
    Json(payload): Json<FolderActionRequest>,
) -> impl IntoResponse {
    match state.sync_coordinator.initial_scan(&payload.folder_id).await {
        Ok(_) => Json(serde_json::json!({ "success": true })),
        Err(e) => Json(serde_json::json!({ "success": false, "error": e.to_string() })),
    }
}

async fn sync_folder_handler(
    State(state): State<DesktopState>,
    Json(payload): Json<FolderActionRequest>,
) -> impl IntoResponse {
    match state.sync_coordinator.trigger_sync(&payload.folder_id).await {
        Ok(_) => Json(serde_json::json!({ "success": true })),
        Err(e) => Json(serde_json::json!({ "success": false, "error": e.to_string() })),
    }
}

async fn send_file_handler(
    State(state): State<DesktopState>,
    Json(payload): Json<SendFileRequest>,
) -> impl IntoResponse {
    let file_path = std::path::Path::new(&payload.file_path);
    if !file_path.exists() {
        return Json(serde_json::json!({ "success": false, "error": "File does not exist" }));
    }

    let target_id = tusher_core::identity::NodeId::from_str_unchecked(&payload.peer_id);
    let conn_opt = state.manager.get_active_connection(&target_id).await;
    let conn = match conn_opt {
        Some(c) => c,
        None => return Json(serde_json::json!({ "success": false, "error": "Peer is not currently connected" })),
    };

    match state.transfer_service.send_file(&conn, file_path).await {
        Ok(stats) => Json(serde_json::json!({
            "success": true,
            "total_bytes": stats.total_bytes,
            "chunks_sent": stats.chunks_sent,
            "chunks_skipped": stats.chunks_skipped,
            "elapsed_ms": stats.elapsed.as_millis()
        })),
        Err(e) => Json(serde_json::json!({ "success": false, "error": e.to_string() })),
    }
}

async fn toggle_sync_handler(State(state): State<DesktopState>) -> impl IntoResponse {
    let current = state.sync_paused.load(Ordering::SeqCst);
    let new_val = !current;
    state.sync_paused.store(new_val, Ordering::SeqCst);
    Json(serde_json::json!({ "sync_paused": new_val }))
}

async fn get_conflicts_handler(State(state): State<DesktopState>) -> impl IntoResponse {
    let mut conflicts = Vec::new();
    if let Ok(folders) = state.metadata_service.list_folders().await {
        for f in folders {
            if let Ok(files) = state.metadata_service.get_all_files(&f.folder_id).await {
                for file in files {
                    if !file.is_deleted && file.relative_path.contains("(Conflict -") {
                        conflicts.push(ConflictDto {
                            folder_id: f.folder_id.clone(),
                            relative_path: file.relative_path,
                            file_size: file.size_bytes,
                            modified_at: file.modified_at,
                        });
                    }
                }
            }
        }
    }
    Json(conflicts)
}

async fn get_context_menu_status() -> impl IntoResponse {
    Json(serde_json::json!({ "installed": is_context_menu_installed() }))
}

async fn set_context_menu_status(Json(payload): Json<ContextMenuRequest>) -> impl IntoResponse {
    let res = if payload.install {
        install_context_menu(None)
    } else {
        uninstall_context_menu()
    };
    match res {
        Ok(()) => Json(serde_json::json!({ "success": true, "installed": is_context_menu_installed() })),
        Err(e) => Json(serde_json::json!({ "success": false, "error": e.to_string() })),
    }
}
