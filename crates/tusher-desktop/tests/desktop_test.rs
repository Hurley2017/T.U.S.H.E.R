use axum::body::Body;
use http::{Request, StatusCode};
use http_body_util::BodyExt;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tower::ServiceExt;

use tusher_core::identity::DeviceIdentity;
use tusher_desktop::shell::{install_context_menu, is_context_menu_installed, uninstall_context_menu};
use tusher_desktop::web::{create_app, DesktopState};
use tusher_metadata::MetadataService;
use tusher_network::manager::ConnectionManager;
use tusher_sync::SyncCoordinator;
use tusher_transfer::TransferService;

#[tokio::test]
async fn test_desktop_context_menu_lifecycle() {
    #[cfg(windows)]
    {
        // 1. Initially or after uninstall, ensure clean state
        let _ = uninstall_context_menu();
        assert!(!is_context_menu_installed());

        // 2. Install context menu
        let dummy_exe = PathBuf::from("C:\\Program Files\\Tusher\\tusher-desktop.exe");
        install_context_menu(Some(&dummy_exe)).expect("install should succeed");
        assert!(is_context_menu_installed());

        // 3. Uninstall context menu
        uninstall_context_menu().expect("uninstall should succeed");
        assert!(!is_context_menu_installed());
    }
}

#[tokio::test]
async fn test_desktop_web_dashboard_and_api() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let data_dir = temp_dir.path().to_path_buf();

    let staging_dir = data_dir.join("staging");
    let downloads_dir = data_dir.join("downloads");
    std::fs::create_dir_all(&staging_dir)?;
    std::fs::create_dir_all(&downloads_dir)?;

    let identity = Arc::new(DeviceIdentity::load_or_create(&data_dir, "DesktopTestNode")?);
    let transfer_service = Arc::new(TransferService::new(&staging_dir, &downloads_dir).await?);
    let db_path = data_dir.join("tusher_metadata.db");
    let metadata_service = Arc::new(MetadataService::open(&db_path)?);

    let manager = Arc::new(ConnectionManager::new(
        Arc::clone(&identity),
        42430,
        42431,
    ));

    let sync_coordinator = Arc::new(SyncCoordinator::new(
        identity.node_id().clone(),
        Arc::clone(&metadata_service),
        Arc::clone(&transfer_service),
        Arc::clone(&manager),
        std::time::Duration::from_millis(100),
    ));

    let sync_paused = Arc::new(AtomicBool::new(false));

    let state = DesktopState {
        identity: Arc::clone(&identity),
        manager: Arc::clone(&manager),
        metadata_service: Arc::clone(&metadata_service),
        transfer_service: Arc::clone(&transfer_service),
        sync_coordinator: Arc::clone(&sync_coordinator),
        sync_paused: Arc::clone(&sync_paused),
        downloads_dir: downloads_dir.clone(),
        tcp_port: 42430,
        discovery_port: 42431,
        web_port: 42950,
    };

    let app = create_app(state);

    // 1. Test GET / (HTML Dashboard)
    let req = Request::builder().uri("/").body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = resp.into_body().collect().await?.to_bytes();
    let body_str = String::from_utf8_lossy(&body_bytes);
    assert!(body_str.contains("T.U.S.H.E.R"));
    assert!(body_str.contains("Decentralized Mesh Dashboard"));

    // 2. Test GET /api/status
    let req = Request::builder().uri("/api/status").body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = resp.into_body().collect().await?.to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body_bytes)?;
    assert_eq!(json["node_name"], "DesktopTestNode");
    assert_eq!(json["tcp_port"], 42430);
    assert_eq!(json["sync_paused"], false);

    // 3. Test GET /api/peers
    let req = Request::builder().uri("/api/peers").body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = resp.into_body().collect().await?.to_bytes();
    let peers: Vec<serde_json::Value> = serde_json::from_slice(&body_bytes)?;
    assert!(peers.is_empty());

    // 4. Test POST /api/toggle-sync
    let req = Request::builder()
        .method("POST")
        .uri("/api/toggle-sync")
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = resp.into_body().collect().await?.to_bytes();
    let toggle_json: serde_json::Value = serde_json::from_slice(&body_bytes)?;
    assert_eq!(toggle_json["sync_paused"], true);

    // Toggle back
    let req = Request::builder()
        .method("POST")
        .uri("/api/toggle-sync")
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = resp.into_body().collect().await?.to_bytes();
    let toggle_json2: serde_json::Value = serde_json::from_slice(&body_bytes)?;
    assert_eq!(toggle_json2["sync_paused"], false);

    // 5. Test POST /api/folders/add and GET /api/folders
    let test_shared_folder = data_dir.join("test_docs");
    std::fs::create_dir_all(&test_shared_folder)?;
    std::fs::write(test_shared_folder.join("sample.txt"), "hello world")?;

    let add_payload = serde_json::json!({
        "folder_id": "test_folder",
        "folder_name": "Test Docs",
        "path": test_shared_folder.to_string_lossy()
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/folders/add")
        .header("content-type", "application/json")
        .body(Body::from(add_payload.to_string()))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let req = Request::builder().uri("/api/folders").body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = resp.into_body().collect().await?.to_bytes();
    let folders: Vec<serde_json::Value> = serde_json::from_slice(&body_bytes)?;
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0]["folder_id"], "test_folder");
    assert_eq!(folders[0]["folder_name"], "Test Docs");
    assert_eq!(folders[0]["file_count"], 1);

    // 6. Test GET /api/conflicts
    let req = Request::builder().uri("/api/conflicts").body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = resp.into_body().collect().await?.to_bytes();
    let conflicts: Vec<serde_json::Value> = serde_json::from_slice(&body_bytes)?;
    assert!(conflicts.is_empty());

    // 7. Test GET /api/shell/context-menu
    let req = Request::builder().uri("/api/shell/context-menu").body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    Ok(())
}
