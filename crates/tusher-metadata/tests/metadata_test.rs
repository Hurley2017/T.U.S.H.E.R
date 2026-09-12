use tusher_core::identity::DeviceIdentity;
use tusher_core::protocol::SyncEventType;
use tusher_metadata::models::ReconciliationAction;
use tusher_metadata::service::MetadataService;

#[tokio::test]
async fn test_schema_initialization_and_folder_management() {
    let service = MetadataService::open_in_memory().unwrap();

    let folder = service
        .add_folder("f_work", "Work Documents", "C:/Users/Test/Work")
        .await
        .unwrap();

    assert_eq!(folder.folder_id, "f_work");
    assert_eq!(folder.name, "Work Documents");

    let list = service.list_folders().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].folder_id, "f_work");

    let found = service.get_folder("f_work").await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().name, "Work Documents");
}

#[tokio::test]
async fn test_local_event_logging_monotonic() {
    let service = MetadataService::open_in_memory().unwrap();
    let id_a = DeviceIdentity::generate("NodeA".to_string());
    let folder_id = "f_docs";

    service
        .add_folder(folder_id, "Docs", "D:/Docs")
        .await
        .unwrap();

    // 1. First file creation
    let ev1 = service
        .record_local_file(folder_id, "notes.txt", 100, "hash_v1", id_a.node_id())
        .await
        .unwrap();
    assert_eq!(ev1.event_seq, 1);
    assert_eq!(ev1.version, 1);
    assert_eq!(ev1.event_type, SyncEventType::Upsert);

    // 2. File modification
    let ev2 = service
        .record_local_file(folder_id, "notes.txt", 200, "hash_v2", id_a.node_id())
        .await
        .unwrap();
    assert_eq!(ev2.event_seq, 2);
    assert_eq!(ev2.version, 2);
    assert_eq!(ev2.event_type, SyncEventType::Upsert);

    // 3. File deletion (tombstone)
    let ev3_opt = service
        .record_local_delete(folder_id, "notes.txt", id_a.node_id())
        .await
        .unwrap();
    assert!(ev3_opt.is_some());
    let ev3 = ev3_opt.unwrap();
    assert_eq!(ev3.event_seq, 3);
    assert_eq!(ev3.version, 3);
    assert_eq!(ev3.event_type, SyncEventType::Delete);

    // 4. Query all events since seq 0
    let (events, latest) = service.handle_manifest_request(folder_id, 0).await.unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(latest, 3);
    assert_eq!(events[0].event_seq, 1);
    assert_eq!(events[1].event_seq, 2);
    assert_eq!(events[2].event_seq, 3);
}

#[tokio::test]
async fn test_manifest_delta_exchange() {
    let service = MetadataService::open_in_memory().unwrap();
    let id_a = DeviceIdentity::generate("NodeA".to_string());
    let folder_id = "f_sync";

    service
        .add_folder(folder_id, "Sync", "D:/Sync")
        .await
        .unwrap();

    // Log 5 events
    for i in 1..=5 {
        service
            .record_local_file(
                folder_id,
                &format!("file_{}.txt", i),
                1000 * i as u64,
                &format!("hash_{}", i),
                id_a.node_id(),
            )
            .await
            .unwrap();
    }

    // Peer requests events since seq 2
    let (delta, latest_seq) = service.handle_manifest_request(folder_id, 2).await.unwrap();
    assert_eq!(latest_seq, 5);
    assert_eq!(delta.len(), 3);
    assert_eq!(delta[0].event_seq, 3);
    assert_eq!(delta[0].relative_path, "file_3.txt");
    assert_eq!(delta[1].event_seq, 4);
    assert_eq!(delta[2].event_seq, 5);
}

#[tokio::test]
async fn test_causality_and_fast_forward_reconciliation() {
    let service_b = MetadataService::open_in_memory().unwrap();
    let id_a = DeviceIdentity::generate("NodeA".to_string());
    let folder_id = "f_shared";

    service_b
        .add_folder(folder_id, "Shared", "D:/Shared")
        .await
        .unwrap();

    // Step 1: Remote Node A created presentation.pptx (version 1)
    let remote_event_v1 = tusher_core::protocol::SyncEvent {
        event_seq: 1,
        folder_id: folder_id.to_string(),
        file_id: "f_pres".to_string(),
        relative_path: "presentation.pptx".to_string(),
        event_type: SyncEventType::Upsert,
        version: 1,
        size_bytes: 5_000_000,
        content_hash: "hash_pres_v1".to_string(),
        origin_node_id: id_a.node_id().clone(),
        modified_at: 1000,
    };

    // Node B reconciles: New file!
    let actions = service_b
        .reconcile_remote_events(&[remote_event_v1.clone()])
        .await
        .unwrap();
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReconciliationAction::DownloadNeeded { relative_path, version, content_hash, .. } => {
            assert_eq!(relative_path, "presentation.pptx");
            assert_eq!(*version, 1);
            assert_eq!(content_hash, "hash_pres_v1");
        }
        other => panic!("Expected DownloadNeeded, got {:?}", other),
    }

    // Node B downloads and applies action
    service_b.apply_reconciliation_action(&actions[0]).await.unwrap();

    // Step 2: Remote Node A updates presentation.pptx (version 2)
    let remote_event_v2 = tusher_core::protocol::SyncEvent {
        event_seq: 2,
        folder_id: folder_id.to_string(),
        file_id: "f_pres".to_string(),
        relative_path: "presentation.pptx".to_string(),
        event_type: SyncEventType::Upsert,
        version: 2,
        size_bytes: 6_000_000,
        content_hash: "hash_pres_v2".to_string(),
        origin_node_id: id_a.node_id().clone(),
        modified_at: 2000,
    };

    // Node B reconciles: Clean fast forward
    let actions_v2 = service_b
        .reconcile_remote_events(&[remote_event_v2.clone()])
        .await
        .unwrap();
    assert_eq!(actions_v2.len(), 1);
    match &actions_v2[0] {
        ReconciliationAction::DownloadNeeded { version, content_hash, .. } => {
            assert_eq!(*version, 2);
            assert_eq!(content_hash, "hash_pres_v2");
        }
        other => panic!("Expected DownloadNeeded for v2, got {:?}", other),
    }
}

#[tokio::test]
async fn test_tombstone_propagation() {
    let service_b = MetadataService::open_in_memory().unwrap();
    let id_a = DeviceIdentity::generate("NodeA".to_string());
    let folder_id = "f_vault";

    service_b
        .add_folder(folder_id, "Vault", "D:/Vault")
        .await
        .unwrap();

    // Node B already has secret.key at v1
    let init_event = tusher_core::protocol::SyncEvent {
        event_seq: 1,
        folder_id: folder_id.to_string(),
        file_id: "f_key".to_string(),
        relative_path: "secret.key".to_string(),
        event_type: SyncEventType::Upsert,
        version: 1,
        size_bytes: 256,
        content_hash: "hash_key".to_string(),
        origin_node_id: id_a.node_id().clone(),
        modified_at: 100,
    };
    service_b
        .apply_reconciliation_action(&ReconciliationAction::DownloadNeeded {
            folder_id: folder_id.to_string(),
            file_id: "f_key".to_string(),
            relative_path: "secret.key".to_string(),
            size_bytes: 256,
            content_hash: "hash_key".to_string(),
            version: 1,
            origin_node_id: id_a.node_id().clone(),
        })
        .await
        .unwrap();

    // Node A deletes secret.key at v2
    let delete_event = tusher_core::protocol::SyncEvent {
        event_seq: 2,
        folder_id: folder_id.to_string(),
        file_id: "f_key".to_string(),
        relative_path: "secret.key".to_string(),
        event_type: SyncEventType::Delete,
        version: 2,
        size_bytes: 0,
        content_hash: "hash_key".to_string(),
        origin_node_id: id_a.node_id().clone(),
        modified_at: 200,
    };

    let actions = service_b
        .reconcile_remote_events(&[delete_event])
        .await
        .unwrap();
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReconciliationAction::DeleteLocal { relative_path, .. } => {
            assert_eq!(relative_path, "secret.key");
        }
        other => panic!("Expected DeleteLocal, got {:?}", other),
    }

    service_b.apply_reconciliation_action(&actions[0]).await.unwrap();

    let active_files = service_b.get_all_files(folder_id).await.unwrap();
    assert!(active_files.is_empty(), "Deleted file must not be listed in active files");

    // Old resurrection attempt should be ignored
    let old_actions = service_b
        .reconcile_remote_events(&[init_event])
        .await
        .unwrap();
    assert_eq!(old_actions[0], ReconciliationAction::Ignore);
}

#[tokio::test]
async fn test_offline_concurrent_conflict_resolution() {
    let service_b = MetadataService::open_in_memory().unwrap();
    let id_a = DeviceIdentity::generate("NodeA".to_string());
    let id_b = DeviceIdentity::generate("NodeB".to_string());
    let folder_id = "f_budget";

    service_b
        .add_folder(folder_id, "Budget", "D:/Budget")
        .await
        .unwrap();

    // Node B has local edit on budget.xlsx: version 2, modified locally by Node B
    service_b
        .record_local_file(folder_id, "budget.xlsx", 50_000, "hash_b_local", id_b.node_id())
        .await
        .unwrap();

    // Node A also made concurrent offline edit on budget.xlsx: version 2, modified by Node A
    let remote_event = tusher_core::protocol::SyncEvent {
        event_seq: 10,
        folder_id: folder_id.to_string(),
        file_id: "f_budget".to_string(),
        relative_path: "budget.xlsx".to_string(),
        event_type: SyncEventType::Upsert,
        version: 2,
        size_bytes: 60_000,
        content_hash: "hash_a_remote".to_string(),
        origin_node_id: id_a.node_id().clone(),
        modified_at: chrono::Utc::now().timestamp(),
    };

    // Node B reconciles Node A's concurrent event
    let actions = service_b
        .reconcile_remote_events(&[remote_event])
        .await
        .unwrap();

    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReconciliationAction::Conflict {
            original_relative_path,
            conflict_relative_path,
            content_hash,
            ..
        } => {
            assert_eq!(original_relative_path, "budget.xlsx");
            assert!(
                conflict_relative_path.starts_with("budget (Conflict - "),
                "Conflict path was: {}",
                conflict_relative_path
            );
            assert!(
                conflict_relative_path.ends_with(".xlsx"),
                "Conflict path must preserve extension"
            );
            assert_eq!(content_hash, "hash_a_remote");
        }
        other => panic!("Expected Conflict action, got {:?}", other),
    }

    // Apply the conflict action: local budget.xlsx is PRESERVED, conflict file is added
    service_b.apply_reconciliation_action(&actions[0]).await.unwrap();

    // Both files now exist as distinct entities! Zero data loss!
    let original = service_b.get_file(folder_id, "budget.xlsx").await.unwrap().unwrap();
    assert_eq!(original.content_hash, "hash_b_local");

    let all_files = service_b.get_all_files(folder_id).await.unwrap();
    assert_eq!(all_files.len(), 2, "Both local file and conflict file must be indexed");
}
