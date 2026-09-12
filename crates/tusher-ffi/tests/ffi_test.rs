// crates/tusher-ffi/tests/ffi_test.rs
// Integration tests for the UniFFI foreign function interface layer

use std::path::PathBuf;
use tusher_ffi::*;

fn temp_ffi_dir(name: &str) -> PathBuf {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!("tusher_ffi_test_{}_{}_{}", name, std::process::id(), now));
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn test_version_and_sas_pin() {
    let v = tusher_version();
    assert!(v.starts_with("T.U.S.H.E.R v0.1.0"));

    let key_a = hex::encode([1u8; 32]);
    let key_b = hex::encode([2u8; 32]);
    let pin1 = compute_sas_pin("pairing_token_abc".to_string(), key_a.clone(), key_b.clone()).unwrap();
    let pin2 = compute_sas_pin("pairing_token_abc".to_string(), key_b, key_a).unwrap();

    assert_eq!(pin1, pin2, "SAS PIN must be symmetrical and deterministic");
    assert_eq!(pin1.len(), 7, "SAS PIN format must be 'XXX XXX'");
}

#[test]
fn test_tusher_node_lifecycle_and_apis() {
    let data_dir = temp_ffi_dir("node_data");
    let shared_dir = temp_ffi_dir("shared_folder");

    let node = TusherNode::new(
        data_dir.to_string_lossy().to_string(),
        "Android-Tablet".to_string(),
        42988,
        42888,
    )
    .expect("TusherNode initialization must succeed");

    // 1. Verify Status
    let status = node.get_status().expect("Must return status");
    assert_eq!(status.node_name, "Android-Tablet");
    assert!(status.node_id.starts_with("tshr_"));
    assert_eq!(status.listen_port, 42988);

    // 2. Verify Peers query (empty initially)
    let peers = node.get_peers().expect("Must return peer list");
    assert!(peers.is_empty(), "Initial peer list should be empty");

    // 3. Verify Shared Folder registration
    node.add_shared_folder(
        "mobile_vault".to_string(),
        "MobileVault".to_string(),
        shared_dir.to_string_lossy().to_string(),
    )
    .expect("Shared folder must register");

    let folders = node.list_shared_folders().expect("Must list shared folders");
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].folder_id, "mobile_vault");
    assert_eq!(folders[0].name, "MobileVault");

    // 4. Verify Pairing Invite generation
    let invite = node.create_pairing_invite(None).expect("Must generate pairing invite");
    assert!(!invite.token.is_empty());
    assert!(invite.uri.starts_with("tusher://pair?"));

    // Cleanup
    let _ = std::fs::remove_dir_all(&data_dir);
    let _ = std::fs::remove_dir_all(&shared_dir);
}
