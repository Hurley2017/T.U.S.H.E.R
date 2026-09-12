use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tusher_core::identity::DeviceIdentity;
use tusher_core::types::TransportType;
use tusher_network::connection::PeerConnection;
use tusher_network::manager::ConnectionManager;
use tusher_network::transport::TransportAddress;

#[tokio::test]
async fn test_node_handshake_ping_pong() {
    let id_a = DeviceIdentity::generate("TestNodeA".to_string());
    let id_b = DeviceIdentity::generate("TestNodeB".to_string());
    let id_b_node_id = id_b.node_id().clone();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let listen_addr = listener.local_addr().unwrap();

    // Node B accepts in background
    let accept_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        PeerConnection::accept(stream, &id_b, listen_addr.port(), TransportType::Lan)
            .await
            .unwrap()
    });

    // Node A dials Node B
    let target = TransportAddress::new(listen_addr, TransportType::Lan);
    let conn_a = PeerConnection::dial(target, &id_a, 0).await.unwrap();
    let conn_b = accept_task.await.unwrap();

    // Verify cryptographic mutual identity
    assert_eq!(conn_a.remote_node_id(), &id_b_node_id);
    assert_eq!(conn_b.remote_node_id(), id_a.node_id());
    assert_eq!(conn_a.remote_name(), "TestNodeB");
    assert_eq!(conn_b.remote_name(), "TestNodeA");
    assert_eq!(conn_a.transport_type(), TransportType::Lan);

    // Node B responds to Ping with Pong
    tokio::spawn(async move {
        while let Ok(Some(msg)) = conn_b.recv().await {
            if let tusher_core::protocol::Message::Ping { sequence, timestamp_ms } = msg {
                let _ = conn_b.send(tusher_core::protocol::Message::Pong { sequence, timestamp_ms }).await;
                break;
            }
        }
    });

    // Verify Ping / Pong roundtrip
    let rtt = conn_a.ping(1).await.unwrap();
    println!("Ping successful! RTT: {:?}", rtt);
    assert!(rtt < Duration::from_millis(500));
}

#[tokio::test]
async fn test_pairing_sas_pin_generation() {
    let id_a = Arc::new(DeviceIdentity::generate("Phone".to_string()));
    let id_b = Arc::new(DeviceIdentity::generate("Tablet".to_string()));

    let mgr_a = ConnectionManager::new(id_a, 42601, 42603);
    let mgr_b = ConnectionManager::new(id_b, 42602, 42603);

    // Node A creates pairing invite
    let (token, _uri) = mgr_a.pairing_manager().create_pairing_invite("192.168.1.50:42601").await;

    // Both compute SAS verification PIN
    let pin_a = mgr_a
        .pairing_manager()
        .compute_sas(&token, &mgr_b.identity().public_key_hex())
        .unwrap();

    let pin_b = mgr_b
        .pairing_manager()
        .compute_sas(&token, &mgr_a.identity().public_key_hex())
        .unwrap();

    // SAS PIN must match on both screens!
    assert_eq!(pin_a, pin_b);
    assert_eq!(pin_a.len(), 7); // Format: "XXX XXX"
    println!("Pairing verified! Symmetric SAS PIN: {}", pin_a);
}

#[tokio::test]
async fn test_discovery_and_auto_failover() {
    let id_1 = Arc::new(DeviceIdentity::generate("DesktopNode".to_string()));
    let id_2 = Arc::new(DeviceIdentity::generate("LaptopNode".to_string()));

    let port_1: u16 = 42701;
    let port_2: u16 = 42702;
    let disc_port: u16 = 42703;

    let mgr_1 = Arc::new(ConnectionManager::new(id_1, port_1, disc_port));
    let mgr_2 = Arc::new(ConnectionManager::new(id_2, port_2, disc_port));

    // Register LAN and secondary candidates
    let addr_2_lan = TransportAddress::new(format!("127.0.0.1:{}", port_2).parse().unwrap(), TransportType::Lan);
    let addr_2_tailscale = TransportAddress::new(format!("127.0.0.1:{}", port_2).parse().unwrap(), TransportType::Tailscale);

    mgr_1.add_candidate(mgr_2.identity().node_id().clone(), addr_2_lan).await;
    mgr_1.add_candidate(mgr_2.identity().node_id().clone(), addr_2_tailscale).await;

    let _h1 = Arc::clone(&mgr_1).start().await.unwrap();
    let _h2 = Arc::clone(&mgr_2).start().await.unwrap();

    // Wait for connection to establish
    sleep(Duration::from_secs(4)).await;

    let statuses = mgr_1.get_peer_statuses().await;
    assert!(!statuses.is_empty(), "Should have discovered peer");
    let peer_status = &statuses[0];
    assert!(peer_status.is_connected, "Peer should be connected");
    assert_eq!(peer_status.active_transport, Some(TransportType::Lan), "Should prefer LAN path");

    // Simulate LAN dropped
    mgr_1.set_simulate_lan_disabled(true);
    println!("Simulating LAN loss...");
    sleep(Duration::from_secs(5)).await;

    let failover_statuses = mgr_1.get_peer_statuses().await;
    let failover_status = &failover_statuses[0];
    assert_eq!(
        failover_status.active_transport,
        Some(TransportType::Tailscale),
        "Should have failed over to Tailscale/secondary candidate"
    );

    // Restore LAN
    mgr_1.set_simulate_lan_disabled(false);
    println!("Restoring LAN...");
    sleep(Duration::from_secs(5)).await;

    let restored_statuses = mgr_1.get_peer_statuses().await;
    let restored_status = &restored_statuses[0];
    assert_eq!(
        restored_status.active_transport,
        Some(TransportType::Lan),
        "Should restore preference back to direct LAN"
    );
}
