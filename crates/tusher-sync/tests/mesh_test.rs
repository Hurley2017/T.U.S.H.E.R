// crates/tusher-sync/tests/mesh_test.rs
// Comprehensive integration test suite for 3-node mesh transitive replication and offline conflict handling

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tusher_core::identity::DeviceIdentity;
use tusher_core::types::TransportType;
use tusher_metadata::service::MetadataService;
use tusher_network::manager::ConnectionManager;
use tusher_network::transport::TransportAddress;
use tusher_sync::coordinator::SyncCoordinator;
use tusher_transfer::hash::hash_file;
use tusher_transfer::service::TransferService;

fn create_temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tusher_mesh_{}_{}_{}",
        name,
        std::process::id(),
        rand::random::<u32>()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct TestNode {
    pub identity: Arc<DeviceIdentity>,
    pub shared_dir: PathBuf,
    pub staging_dir: PathBuf,
    pub dest_dir: PathBuf,
    pub metadata: Arc<MetadataService>,
    pub _transfer: Arc<TransferService>,
    pub network: Arc<ConnectionManager>,
    pub coordinator: Arc<SyncCoordinator>,
}

impl TestNode {
    async fn new(name: &str, folder_id: &str) -> Self {
        let identity = Arc::new(DeviceIdentity::generate(name.to_string()));
        let shared_dir = create_temp_dir(&format!("{}_shared", name));
        let staging_dir = create_temp_dir(&format!("{}_staging", name));
        let dest_dir = create_temp_dir(&format!("{}_dest", name));

        let metadata = Arc::new(MetadataService::open_in_memory().unwrap());
        let transfer = Arc::new(TransferService::new(&staging_dir, &dest_dir).await.unwrap());
        let network = Arc::new(ConnectionManager::new(Arc::clone(&identity), 0, 0));

        let coordinator = Arc::new(SyncCoordinator::new(
            identity.node_id().clone(),
            Arc::clone(&metadata),
            Arc::clone(&transfer),
            Arc::clone(&network),
            Duration::from_millis(150),
        ));
        coordinator.register_folder(folder_id, &shared_dir).await.unwrap();

        Self {
            identity,
            shared_dir,
            staging_dir,
            dest_dir,
            metadata,
            _transfer: transfer,
            network,
            coordinator,
        }
    }

    async fn start(&self) {
        let _ = Arc::clone(&self.network).start().await.unwrap();
        let _ = Arc::clone(&self.coordinator).start().await.unwrap();
    }

    fn cleanup(&self) {
        let _ = std::fs::remove_dir_all(&self.shared_dir);
        let _ = std::fs::remove_dir_all(&self.staging_dir);
        let _ = std::fs::remove_dir_all(&self.dest_dir);
    }
}

/// Tests that in a 3-node topology A <-> B <-> C (where A and C have NO direct link):
/// Changes made on Node A propagate transitively through Node B to Node C.
/// Reverse changes made on Node C propagate transitively through Node B to Node A.
/// Deletion on Node A propagates through Node B and deletes the file on Node C.
#[tokio::test]
async fn test_three_node_transitive_mesh_propagation() {
    let folder_id = "shared_mesh";

    // 1. Initialize Node A, Node B, Node C
    let node_a = TestNode::new("Desktop-A", folder_id).await;
    let node_b = TestNode::new("Laptop-B", folder_id).await;
    let node_c = TestNode::new("Tablet-C", folder_id).await;

    node_a.start().await;
    node_b.start().await;
    node_c.start().await;

    // 2. Establish Topology: A <-> B and B <-> C (NO direct link between A and C!)
    // Trust setup
    node_a.network.pairing_manager().set_trusted(node_b.identity.node_id().clone(), tusher_core::types::TrustStatus::Paired).await;
    node_b.network.pairing_manager().set_trusted(node_a.identity.node_id().clone(), tusher_core::types::TrustStatus::Paired).await;

    node_b.network.pairing_manager().set_trusted(node_c.identity.node_id().clone(), tusher_core::types::TrustStatus::Paired).await;
    node_c.network.pairing_manager().set_trusted(node_b.identity.node_id().clone(), tusher_core::types::TrustStatus::Paired).await;

    // Connect A -> B
    let addr_b = std::net::SocketAddr::from(([127, 0, 0, 1], node_b.network.listen_port()));
    node_a.network.add_candidate(node_b.identity.node_id().clone(), TransportAddress::new(addr_b, TransportType::Lan)).await;

    // Connect C -> B
    node_c.network.add_candidate(node_b.identity.node_id().clone(), TransportAddress::new(addr_b, TransportType::Lan)).await;

    // Allow connections to establish
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(node_a.network.get_active_connection(node_b.identity.node_id()).await.is_some(), "Node A must be connected to Node B");
    assert!(node_c.network.get_active_connection(node_b.identity.node_id()).await.is_some(), "Node C must be connected to Node B");

    // Verify Node A and Node C are NOT connected directly
    assert!(node_a.network.get_active_connection(node_c.identity.node_id()).await.is_none(), "Node A and Node C must have NO direct connection");

    // --- PHASE 1: Forward Transitive Sync: Node A -> Node B -> Node C ---
    let memo_file_a = node_a.shared_dir.join("mesh_memo.txt");
    let memo_content = b"Global Mesh Announcement: Transitive P2P routing functional!";
    tokio::fs::write(&memo_file_a, memo_content).await.unwrap();

    let memo_file_b = node_b.shared_dir.join("mesh_memo.txt");
    let mut arrived_b = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if memo_file_b.exists() {
            arrived_b = true;
            break;
        }
    }
    assert!(arrived_b, "File from Node A must arrive on intermediate Node B");

    // Verify arrival on downstream leaf Node C
    let memo_file_c = node_c.shared_dir.join("mesh_memo.txt");
    let mut arrived_c = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if memo_file_c.exists() {
            let content_c = tokio::fs::read(&memo_file_c).await.unwrap();
            if content_c == memo_content {
                arrived_c = true;
                break;
            }
        }
    }
    assert!(arrived_c, "File from Node A must transitively propagate through Node B to reach Node C!");

    // Verify 100% SHA-256 match across all 3 nodes
    let hash_a = hash_file(&memo_file_a).await.unwrap();
    let hash_b = hash_file(&memo_file_b).await.unwrap();
    let hash_c = hash_file(&memo_file_c).await.unwrap();
    assert_eq!(hash_a, hash_b);
    assert_eq!(hash_b, hash_c);

    // --- PHASE 2: Reverse Transitive Sync: Node C -> Node B -> Node A ---
    let binary_c = node_c.shared_dir.join("payload.bin");
    let binary_data = vec![77u8; 32768]; // 32 KB binary data
    tokio::fs::write(&binary_c, &binary_data).await.unwrap();

    // Verify arrival on upstream Node A through Node B
    let binary_a = node_a.shared_dir.join("payload.bin");
    let mut arrived_rev_a = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if binary_a.exists() {
            let content_a = tokio::fs::read(&binary_a).await.unwrap();
            if content_a == binary_data {
                arrived_rev_a = true;
                break;
            }
        }
    }
    assert!(arrived_rev_a, "Binary created on Node C must transitively propagate through Node B to reach Node A!");

    let hash_c_bin = hash_file(&binary_c).await.unwrap();
    let hash_a_bin = hash_file(&binary_a).await.unwrap();
    assert_eq!(hash_c_bin, hash_a_bin);

    // --- PHASE 3: Transitive Deletion Propagation ---
    tokio::fs::remove_file(&memo_file_a).await.unwrap();

    // Verify deletion on Node C
    let mut deleted_c = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if !memo_file_c.exists() {
            deleted_c = true;
            break;
        }
    }
    assert!(deleted_c, "Deletion on Node A must transitively propagate and delete file on Node C");

    node_a.cleanup();
    node_b.cleanup();
    node_c.cleanup();
}

/// Tests that concurrent conflicting modifications made on two nodes branch into
/// a conflict file, preserving both versions without silent overwrite across all nodes.
#[tokio::test]
async fn test_offline_concurrent_conflict_resolution_three_nodes() {
    let folder_id = "conflict_mesh";

    let node_a = TestNode::new("Node-A", folder_id).await;
    let node_b = TestNode::new("Node-B", folder_id).await;
    let node_c = TestNode::new("Node-C", folder_id).await;

    // Mutual trust
    node_a.network.pairing_manager().set_trusted(node_b.identity.node_id().clone(), tusher_core::types::TrustStatus::Paired).await;
    node_b.network.pairing_manager().set_trusted(node_a.identity.node_id().clone(), tusher_core::types::TrustStatus::Paired).await;
    node_b.network.pairing_manager().set_trusted(node_c.identity.node_id().clone(), tusher_core::types::TrustStatus::Paired).await;
    node_c.network.pairing_manager().set_trusted(node_b.identity.node_id().clone(), tusher_core::types::TrustStatus::Paired).await;

    // Both start with a common base file 'document.txt'
    let base_content = b"Common Baseline Document Version 1";
    tokio::fs::write(node_a.shared_dir.join("document.txt"), base_content).await.unwrap();
    tokio::fs::write(node_b.shared_dir.join("document.txt"), base_content).await.unwrap();
    tokio::fs::write(node_c.shared_dir.join("document.txt"), base_content).await.unwrap();

    let base_hash = tusher_transfer::hash::hash_bytes(base_content);
    // Index common baseline into all metadata DBs
    node_a.metadata.record_local_file(folder_id, "document.txt", base_content.len() as u64, &base_hash, node_a.identity.node_id()).await.unwrap();
    node_b.metadata.record_local_file(folder_id, "document.txt", base_content.len() as u64, &base_hash, node_b.identity.node_id()).await.unwrap();
    node_c.metadata.record_local_file(folder_id, "document.txt", base_content.len() as u64, &base_hash, node_c.identity.node_id()).await.unwrap();

    // Now, WHILE OFFLINE, Node A and Node B make conflicting edits to 'document.txt'
    let edit_a = b"Modified by Node A offline";
    let edit_b = b"Modified by Node B offline with divergent changes";
    tokio::fs::write(node_a.shared_dir.join("document.txt"), edit_a).await.unwrap();
    tokio::fs::write(node_b.shared_dir.join("document.txt"), edit_b).await.unwrap();

    let hash_edit_a = tusher_transfer::hash::hash_bytes(edit_a);
    let hash_edit_b = tusher_transfer::hash::hash_bytes(edit_b);

    node_a.metadata.record_local_file(folder_id, "document.txt", edit_a.len() as u64, &hash_edit_a, node_a.identity.node_id()).await.unwrap();
    node_b.metadata.record_local_file(folder_id, "document.txt", edit_b.len() as u64, &hash_edit_b, node_b.identity.node_id()).await.unwrap();

    // Start nodes and connect them
    node_a.start().await;
    node_b.start().await;
    node_c.start().await;

    // Connect Node A <-> Node B
    let addr_b = std::net::SocketAddr::from(([127, 0, 0, 1], node_b.network.listen_port()));
    node_a.network.add_candidate(node_b.identity.node_id().clone(), TransportAddress::new(addr_b, TransportType::Lan)).await;

    // Trigger reconciliation
    tokio::time::sleep(Duration::from_millis(500)).await;
    let _ = node_a.coordinator.trigger_sync(folder_id).await;
    let _ = node_b.coordinator.trigger_sync(folder_id).await;

    // Wait for conflict branching to occur on Node B
    let mut conflict_branched = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        // Check if a file containing "Conflict" appeared in Node B's directory
        let mut entries = tokio::fs::read_dir(&node_b.shared_dir).await.unwrap();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.contains("Conflict") {
                conflict_branched = true;
                break;
            }
        }
        if conflict_branched {
            break;
        }
    }
    assert!(conflict_branched, "A conflict file must be created to branch conflicting changes");

    // Both versions must exist on Node B
    let orig_on_b = tokio::fs::read(node_b.shared_dir.join("document.txt")).await.unwrap();
    assert_eq!(orig_on_b, edit_b, "Original local file on Node B must NOT be overwritten");

    // Now connect Node C to Node B -> Node C must receive both files
    node_c.network.add_candidate(node_b.identity.node_id().clone(), TransportAddress::new(addr_b, TransportType::Lan)).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let _ = node_b.coordinator.trigger_sync(folder_id).await;

    let mut c_has_conflict = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mut entries = tokio::fs::read_dir(&node_c.shared_dir).await.unwrap();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.contains("Conflict") {
                c_has_conflict = true;
                break;
            }
        }
        if c_has_conflict {
            break;
        }
    }
    assert!(c_has_conflict, "Node C must receive the branched conflict file transitively from Node B");

    node_a.cleanup();
    node_b.cleanup();
    node_c.cleanup();
}
