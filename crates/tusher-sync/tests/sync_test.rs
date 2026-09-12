// crates/tusher-sync/tests/sync_test.rs
// Comprehensive integration test suite for native filesystem watcher and automated two-way synchronization

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tusher_core::identity::DeviceIdentity;
use tusher_core::types::TransportType;
use tusher_metadata::service::MetadataService;
use tusher_network::manager::ConnectionManager;
use tusher_network::transport::TransportAddress;
use tusher_sync::coordinator::SyncCoordinator;
use tusher_sync::watcher::{FolderWatcher, FsChangeEvent};
use tusher_transfer::hash::hash_file;
use tusher_transfer::service::TransferService;

fn create_temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tusher_sync_{}_{}_{}", name, std::process::id(), rand::random::<u32>()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn test_filesystem_watcher_debouncing_and_deletion() {
    let watch_dir = create_temp_dir("watcher_test");
    let mut folders = HashMap::new();
    folders.insert("folder_1".to_string(), watch_dir.clone());

    let mut watcher = FolderWatcher::new(Duration::from_millis(250));
    let mut rx = watcher.start(folders).await.unwrap();

    // 1. Burst writes to the same file in rapid succession (simulating editor saves)
    let test_file = watch_dir.join("test.txt");
    for i in 0..5 {
        tokio::fs::write(&test_file, format!("Burst iteration {}", i)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    // Wait for the debounced event
    let event = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("Timeout waiting for debounced upsert")
        .expect("Channel closed unexpectedly");

    match event {
        FsChangeEvent::Upsert {
            folder_id,
            relative_path,
            size_bytes,
            content_hash,
            ..
        } => {
            assert_eq!(folder_id, "folder_1");
            assert_eq!(relative_path, "test.txt");
            let final_content = b"Burst iteration 4";
            assert_eq!(size_bytes, final_content.len() as u64);
            let expected_hash = hash_file(&test_file).await.unwrap();
            assert_eq!(content_hash, expected_hash);
        }
        other => panic!("Expected Upsert event, got {:?}", other),
    }

    // 2. Delete the file and verify FsChangeEvent::Delete
    tokio::fs::remove_file(&test_file).await.unwrap();

    let del_event = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("Timeout waiting for delete event")
        .expect("Channel closed unexpectedly");

    match del_event {
        FsChangeEvent::Delete {
            folder_id,
            relative_path,
            ..
        } => {
            assert_eq!(folder_id, "folder_1");
            assert_eq!(relative_path, "test.txt");
        }
        other => panic!("Expected Delete event, got {:?}", other),
    }

    let _ = std::fs::remove_dir_all(&watch_dir);
}

#[tokio::test]
async fn test_initial_folder_scan_and_ignore_filtering() {
    let root_dir = create_temp_dir("scan_test");
    let staging_dir = create_temp_dir("scan_staging");
    let dest_dir = create_temp_dir("scan_dest");

    // Pre-populate folder with various files including subfolders and ignored files
    let sub_dir = root_dir.join("documents").join("work");
    std::fs::create_dir_all(&sub_dir).unwrap();

    std::fs::write(root_dir.join("root_file.txt"), b"Root file content").unwrap();
    std::fs::write(sub_dir.join("nested_doc.pdf"), b"Nested document payload").unwrap();
    std::fs::write(root_dir.join(".hidden.txt"), b"Should be ignored").unwrap();
    std::fs::write(root_dir.join("temp_download.crdownload"), b"Should be ignored").unwrap();
    std::fs::write(root_dir.join("backup.tmp"), b"Should be ignored").unwrap();

    let identity = Arc::new(DeviceIdentity::generate("ScannerNode".to_string()));
    let metadata = Arc::new(MetadataService::open_in_memory().unwrap());
    let transfer = Arc::new(TransferService::new(&staging_dir, &dest_dir).await.unwrap());
    let network = Arc::new(ConnectionManager::new(Arc::clone(&identity), 0, 0));

    let coordinator = Arc::new(SyncCoordinator::new(
        identity.node_id().clone(),
        metadata.clone(),
        transfer,
        network,
        Duration::from_millis(200),
    ));

    coordinator.register_folder("docs_folder", &root_dir).await.unwrap();
    coordinator.initial_scan("docs_folder").await.unwrap();

    // Verify database contents
    let files = metadata.get_all_files("docs_folder").await.unwrap();
    assert_eq!(files.len(), 2, "Only legitimate non-ignored files should be indexed");

    let paths: Vec<String> = files.into_iter().map(|f| f.relative_path).collect();
    assert!(paths.contains(&"root_file.txt".to_string()));
    assert!(paths.contains(&"documents/work/nested_doc.pdf".to_string()));

    let _ = std::fs::remove_dir_all(&root_dir);
    let _ = std::fs::remove_dir_all(&staging_dir);
    let _ = std::fs::remove_dir_all(&dest_dir);
}

#[tokio::test]
async fn test_end_to_end_automated_bidirectional_sync() {
    // Setup Node A
    let dir_a = create_temp_dir("node_a_shared");
    let staging_a = create_temp_dir("node_a_staging");
    let dest_a = create_temp_dir("node_a_dest");
    let id_a = Arc::new(DeviceIdentity::generate("Desktop-A".to_string()));
    let meta_a = Arc::new(MetadataService::open_in_memory().unwrap());
    let xfer_a = Arc::new(TransferService::new(&staging_a, &dest_a).await.unwrap());
    let net_a = Arc::new(ConnectionManager::new(Arc::clone(&id_a), 0, 0));

    let coord_a = Arc::new(SyncCoordinator::new(
        id_a.node_id().clone(),
        meta_a.clone(),
        xfer_a,
        Arc::clone(&net_a),
        Duration::from_millis(150),
    ));
    coord_a.register_folder("sync_vault", &dir_a).await.unwrap();

    // Setup Node B
    let dir_b = create_temp_dir("node_b_shared");
    let staging_b = create_temp_dir("node_b_staging");
    let dest_b = create_temp_dir("node_b_dest");
    let id_b = Arc::new(DeviceIdentity::generate("Laptop-B".to_string()));
    let meta_b = Arc::new(MetadataService::open_in_memory().unwrap());
    let xfer_b = Arc::new(TransferService::new(&staging_b, &dest_b).await.unwrap());
    let net_b = Arc::new(ConnectionManager::new(Arc::clone(&id_b), 0, 0));

    let coord_b = Arc::new(SyncCoordinator::new(
        id_b.node_id().clone(),
        meta_b.clone(),
        xfer_b,
        Arc::clone(&net_b),
        Duration::from_millis(150),
    ));
    coord_b.register_folder("sync_vault", &dir_b).await.unwrap();

    // Start network managers
    let _h_net_a = Arc::clone(&net_a).start().await.unwrap();
    let _h_net_b = Arc::clone(&net_b).start().await.unwrap();

    // Mutual trust
    net_a.pairing_manager().set_trusted(id_b.node_id().clone(), tusher_core::types::TrustStatus::Paired).await;
    net_b.pairing_manager().set_trusted(id_a.node_id().clone(), tusher_core::types::TrustStatus::Paired).await;

    // Start coordinators
    let _h_coord_a = Arc::clone(&coord_a).start().await.unwrap();
    let _h_coord_b = Arc::clone(&coord_b).start().await.unwrap();

    // Connect Node A to Node B via bound TCP port
    let addr_b = std::net::SocketAddr::from(([127, 0, 0, 1], net_b.listen_port()));
    net_a.add_candidate(id_b.node_id().clone(), TransportAddress::new(addr_b, TransportType::Lan)).await;

    // Wait for connection to be active
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(net_a.get_active_connection(id_b.node_id()).await.is_some(), "Node A must be connected to Node B");

    // --- PHASE 1: Write file on Node A -> Automatically synchronizes to Node B ---
    let file_a_path = dir_a.join("contract.md");
    let contract_content = b"# T.U.S.H.E.R Partnership Contract\nDecentralized File Mesh verified.";
    tokio::fs::write(&file_a_path, contract_content).await.unwrap();

    // Poll until file arrives on Node B
    let file_b_path = dir_b.join("contract.md");
    let mut synced_a_to_b = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if file_b_path.exists() {
            let content_b = tokio::fs::read(&file_b_path).await.unwrap();
            if content_b == contract_content {
                synced_a_to_b = true;
                break;
            }
        }
    }
    assert!(synced_a_to_b, "File written on Node A must automatically synchronize to Node B");

    // Verify SHA-256 match
    let hash_a = hash_file(&file_a_path).await.unwrap();
    let hash_b = hash_file(&file_b_path).await.unwrap();
    assert_eq!(hash_a, hash_b, "Synchronized file SHA-256 must match 100%");

    // --- PHASE 2: Write file on Node B -> Automatically synchronizes to Node A ---
    let file_b_reverse = dir_b.join("receipt.dat");
    let receipt_content = vec![42u8; 1024 * 64]; // 64 KB binary payload
    tokio::fs::write(&file_b_reverse, &receipt_content).await.unwrap();

    let file_a_reverse = dir_a.join("receipt.dat");
    let mut synced_b_to_a = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if file_a_reverse.exists() {
            let content_a = tokio::fs::read(&file_a_reverse).await.unwrap();
            if content_a == receipt_content {
                synced_b_to_a = true;
                break;
            }
        }
    }
    assert!(synced_b_to_a, "File written on Node B must automatically synchronize to Node A");

    // --- PHASE 3: Delete file on Node A -> Automatically synchronizes deletion to Node B ---
    tokio::fs::remove_file(&file_a_path).await.unwrap();

    let mut deletion_synced = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if !file_b_path.exists() {
            deletion_synced = true;
            break;
        }
    }
    assert!(deletion_synced, "Deleted file on Node A must be automatically removed on Node B");

    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&staging_a);
    let _ = std::fs::remove_dir_all(&dest_a);
    let _ = std::fs::remove_dir_all(&dir_b);
    let _ = std::fs::remove_dir_all(&staging_b);
    let _ = std::fs::remove_dir_all(&dest_b);
}

#[tokio::test]
async fn test_offline_conflict_branching_preserves_both_versions() {
    let dir_a = create_temp_dir("conflict_a_shared");
    let staging_a = create_temp_dir("conflict_a_staging");
    let dest_a = create_temp_dir("conflict_a_dest");
    let id_a = Arc::new(DeviceIdentity::generate("Desktop-A".to_string()));
    let meta_a = Arc::new(MetadataService::open_in_memory().unwrap());
    let xfer_a = Arc::new(TransferService::new(&staging_a, &dest_a).await.unwrap());
    let net_a = Arc::new(ConnectionManager::new(Arc::clone(&id_a), 0, 0));

    let coord_a = Arc::new(SyncCoordinator::new(
        id_a.node_id().clone(),
        meta_a.clone(),
        xfer_a,
        Arc::clone(&net_a),
        Duration::from_millis(150),
    ));
    coord_a.register_folder("work_folder", &dir_a).await.unwrap();

    let dir_b = create_temp_dir("conflict_b_shared");
    let staging_b = create_temp_dir("conflict_b_staging");
    let dest_b = create_temp_dir("conflict_b_dest");
    let id_b = Arc::new(DeviceIdentity::generate("Laptop-B".to_string()));
    let meta_b = Arc::new(MetadataService::open_in_memory().unwrap());
    let xfer_b = Arc::new(TransferService::new(&staging_b, &dest_b).await.unwrap());
    let net_b = Arc::new(ConnectionManager::new(Arc::clone(&id_b), 0, 0));

    let coord_b = Arc::new(SyncCoordinator::new(
        id_b.node_id().clone(),
        meta_b.clone(),
        xfer_b,
        Arc::clone(&net_b),
        Duration::from_millis(150),
    ));
    coord_b.register_folder("work_folder", &dir_b).await.unwrap();

    // 1. OFFLINE EDITS: Both nodes independently create conflicting content for the same relative path
    let file_a_path = dir_a.join("agenda.txt");
    let file_b_path = dir_b.join("agenda.txt");

    let content_a = b"Node A agenda: Release v1.0 immediately";
    let content_b = b"Node B agenda: Add more tests first";

    tokio::fs::write(&file_a_path, content_a).await.unwrap();
    tokio::fs::write(&file_b_path, content_b).await.unwrap();

    // Index offline files into local DBs
    coord_a.initial_scan("work_folder").await.unwrap();
    coord_b.initial_scan("work_folder").await.unwrap();

    // 2. Start network and coordinators
    let _h_net_a = Arc::clone(&net_a).start().await.unwrap();
    let _h_net_b = Arc::clone(&net_b).start().await.unwrap();

    net_a.pairing_manager().set_trusted(id_b.node_id().clone(), tusher_core::types::TrustStatus::Paired).await;
    net_b.pairing_manager().set_trusted(id_a.node_id().clone(), tusher_core::types::TrustStatus::Paired).await;

    let _h_coord_a = Arc::clone(&coord_a).start().await.unwrap();
    let _h_coord_b = Arc::clone(&coord_b).start().await.unwrap();

    // 3. Connect nodes
    let addr_b = std::net::SocketAddr::from(([127, 0, 0, 1], net_b.listen_port()));
    net_a.add_candidate(id_b.node_id().clone(), TransportAddress::new(addr_b, TransportType::Lan)).await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Trigger reconciliation from Node A to Node B
    coord_b.reconcile_with_peer(id_a.node_id(), "work_folder", 100).await.unwrap();

    // Wait up to 5 seconds for conflict resolution transfer
    let mut conflict_file_found = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;

        let entries = std::fs::read_dir(&dir_b).unwrap();
        for entry in entries {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("agenda (Conflict -") {
                let conflict_data = std::fs::read(entry.path()).unwrap();
                if conflict_data == content_a {
                    conflict_file_found = true;
                    break;
                }
            }
        }
        if conflict_file_found {
            break;
        }
    }

    assert!(conflict_file_found, "A conflict file must be branched and saved without data loss");

    // Verify Node B's original file was NOT destroyed or overwritten!
    let original_b_data = std::fs::read(&file_b_path).unwrap();
    assert_eq!(
        original_b_data, content_b,
        "Local file on Node B must be completely preserved without being overwritten"
    );

    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&staging_a);
    let _ = std::fs::remove_dir_all(&dest_a);
    let _ = std::fs::remove_dir_all(&dir_b);
    let _ = std::fs::remove_dir_all(&staging_b);
    let _ = std::fs::remove_dir_all(&dest_b);
}

