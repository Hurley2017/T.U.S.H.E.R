use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;
use tusher_core::identity::DeviceIdentity;
use tusher_core::protocol::Message;
use tusher_core::types::TrustStatus;
use tusher_metadata::models::ReconciliationAction;
use tusher_metadata::MetadataService;
use tusher_network::manager::ConnectionManager;
use tusher_transfer::TransferService;
use tusher_sync::SyncCoordinator;

#[derive(Parser, Debug)]
#[command(author, version, about = "T.U.S.H.E.R - Decentralized Mesh Node", long_about = None)]
struct Args {
    #[arg(short, long, default_value = "TusherNode")]
    name: String,

    #[arg(short, long, default_value = "42424")]
    port: u16,

    #[arg(long, default_value = "42425")]
    discovery_port: u16,

    #[arg(short, long, default_value = "./.tusher_data")]
    data_dir: PathBuf,

    #[arg(long, default_value_t = false)]
    non_interactive: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .with_target(false)
        .with_writer(std::io::stdout)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    let args = Args::parse();

    let identity = Arc::new(DeviceIdentity::load_or_create(&args.data_dir, &args.name)?);

    let staging_dir = args.data_dir.join(".staging");
    let downloads_dir = args.data_dir.join("downloads");
    let transfer_service = Arc::new(TransferService::new(&staging_dir, &downloads_dir).await?);

    let db_path = args.data_dir.join("tusher_metadata.db");
    let metadata_service = Arc::new(MetadataService::open(&db_path)?);

    println!("\n============================================================");
    println!("       T.U.S.H.E.R - Decentralized File & Media Mesh        ");
    println!("============================================================");
    println!(" Node Name:       {}", identity.node_name());
    println!(" Node ID:         {}", identity.node_id());
    println!(" Platform:        {}", identity.platform());
    println!(" Public Key:      {}...", &identity.public_key_hex()[..16]);
    println!(" TCP Listen Port: {}", args.port);
    println!(" UDP Discovery:   {}", args.discovery_port);
    println!(" Storage Dir:     {}", args.data_dir.display());
    println!(" Downloads Dir:   {}", downloads_dir.display());
    println!(" Metadata DB:     {}", db_path.display());
    println!("============================================================");
    println!(" Commands: 'peers', 'status', 'invite', 'pair <id>', 'send <id> <file>',");
    println!("           'folders', 'add-folder <id> <name> <path>', 'sync <id>', 'index <id>',");
    println!("           'events <id>', 'manifest <peer> <id>', 'simulate-lan-drop', 'quit'\n");

    let manager = Arc::new(ConnectionManager::new(
        Arc::clone(&identity),
        args.port,
        args.discovery_port,
    ));

    let sync_coordinator = Arc::new(SyncCoordinator::new(
        identity.node_id().clone(),
        Arc::clone(&metadata_service),
        Arc::clone(&transfer_service),
        Arc::clone(&manager),
        std::time::Duration::from_millis(300),
    ));

    // Register existing folders from DB
    if let Ok(existing_folders) = metadata_service.list_folders().await {
        for f in existing_folders {
            let _ = sync_coordinator.register_folder(&f.folder_id, &f.local_path).await;
        }
    }

    let _maint_handle = Arc::clone(&manager).start().await?;
    let _sync_handle = Arc::clone(&sync_coordinator).start().await?;

    if args.non_interactive {
        tokio::signal::ctrl_c().await?;
        return Ok(());
    }

    let mut lines = BufReader::new(tokio::io::stdin()).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        match parts[0] {
            "peers" => {
                let peers = manager.get_peer_statuses().await;
                println!("\n--- MESH PEERS ({}) ---", peers.len());
                if peers.is_empty() {
                    println!("No peers discovered yet. Listening on LAN UDP {}...", args.discovery_port);
                } else {
                    for (i, p) in peers.iter().enumerate() {
                        let conn_str = if p.is_connected {
                            format!("CONNECTED via {:?}", p.active_transport.unwrap())
                        } else {
                            "DISCONNECTED".to_string()
                        };
                        let latency_str = match p.latency {
                            Some(d) => format!("{:.2} ms", d.as_secs_f64() * 1000.0),
                            None => "N/A".to_string(),
                        };
                        println!(
                            "[{}] Peer: {} ({})\n    Status: {} | Latency: {} | Paired: {}\n    Candidates: {:?}",
                            i + 1,
                            p.node_name,
                            p.node_id,
                            conn_str,
                            latency_str,
                            p.is_paired,
                            p.available_candidates.iter().map(|c| format!("{}:{}", c.transport_type, c.addr)).collect::<Vec<_>>()
                        );
                    }
                }
                println!("------------------------\n");
            }
            "status" => {
                println!("\n--- NODE STATUS ---");
                println!("Node ID:    {}", identity.node_id());
                println!("Name:       {}", identity.node_name());
                println!("Platform:   {}", identity.platform());
                println!("TCP Port:   {}", args.port);
                println!("UDP Port:   {}", args.discovery_port);
                println!("Public Key: {}", identity.public_key_hex());
                println!("Downloads:  {}", downloads_dir.display());
                println!("-------------------\n");
            }
            "invite" => {
                let hint = format!("127.0.0.1:{}", args.port);
                let (token, uri) = manager.pairing_manager().create_pairing_invite(&hint).await;
                println!("\n--- PAIRING INVITE ---");
                println!("Token: {}", token);
                println!("URI:   {}", uri);
                println!("Share this URI or scan QR code on the joining device.");
                println!("----------------------\n");
            }
            "connect" => {
                if parts.len() < 2 {
                    println!("Usage: connect <ip:port>");
                } else {
                    let addr_str = parts[1];
                    if let Ok(addr) = addr_str.parse::<std::net::SocketAddr>() {
                        println!("Connecting to {}...", addr);
                        let target = tusher_network::transport::TransportAddress::new(addr, tusher_core::types::TransportType::Lan);
                        manager.add_candidate(tusher_core::identity::NodeId::from_str_unchecked("remote"), target).await;
                        println!("Candidate added for {}", addr);
                    } else {
                        println!("Invalid address: {}", addr_str);
                    }
                }
            }
            "trust-all" => {
                let peers = manager.get_peer_statuses().await;
                for p in peers {
                    manager.pairing_manager().set_trusted(p.node_id.clone(), TrustStatus::Paired).await;
                    println!("Marked peer {} ({}) as PAIRED & TRUSTED.", p.node_name, p.node_id);
                }
            }
            "pair" => {
                if parts.len() < 2 {
                    println!("Usage: pair <peer_id>");
                } else {
                    let id_str = parts[1];
                    let peers = manager.get_peer_statuses().await;
                    if let Some(target) = peers.iter().find(|p| p.node_id.as_str() == id_str || p.node_id.as_str().starts_with(id_str) || p.node_name.eq_ignore_ascii_case(id_str)) {
                        manager.pairing_manager().set_trusted(target.node_id.clone(), TrustStatus::Paired).await;
                        info!("Peer {} ({}) marked as PAIRED & TRUSTED.", target.node_name, target.node_id);
                    } else {
                        println!("Peer '{}' not found in discovered peers.", id_str);
                    }
                }
            }
            "send" => {
                if parts.len() < 3 {
                    println!("Usage: send <peer_id_or_name> <file_path>");
                } else {
                    let id_str = parts[1];
                    let file_path = PathBuf::from(parts[2]);
                    if !file_path.exists() {
                        println!("File '{}' does not exist.", file_path.display());
                        continue;
                    }

                    let peers = manager.get_peer_statuses().await;
                    if let Some(target) = peers.iter().find(|p| p.node_id.as_str() == id_str || p.node_id.as_str().starts_with(id_str) || p.node_name.eq_ignore_ascii_case(id_str)) {
                        if let Some(conn) = manager.get_active_connection(&target.node_id).await {
                            println!(">>> Initiating transfer of '{}' to {} ({})...", file_path.display(), target.node_name, target.node_id);
                            match transfer_service.send_file(&conn, &file_path).await {
                                Ok(stats) => {
                                    println!(
                                        ">>> Transfer complete! {} bytes ({} chunks sent, {} resumed) in {:?}",
                                        stats.total_bytes, stats.chunks_sent, stats.chunks_skipped, stats.elapsed
                                    );
                                }
                                Err(e) => {
                                    println!(">>> Transfer failed: {}", e);
                                }
                            }
                        } else {
                            println!("Peer {} is currently not connected.", target.node_id);
                        }
                    } else {
                        println!("Peer with ID prefix '{}' not found in discovered peers.", id_str);
                    }
                }
            }
            "folders" => {
                let folders = metadata_service.list_folders().await.unwrap_or_default();
                println!("\n--- SHARED FOLDERS ({}) ---", folders.len());
                if folders.is_empty() {
                    println!("No shared folders configured. Use 'add-folder <id> <name> <path>' to register one.");
                } else {
                    for f in folders {
                        println!("- ID:   {}\n  Name: {}\n  Path: {}", f.folder_id, f.name, f.local_path);
                    }
                }
                println!("---------------------------\n");
            }
            "add-folder" => {
                if parts.len() < 4 {
                    println!("Usage: add-folder <folder_id> <name> <path>");
                } else {
                    let folder_id = parts[1];
                    let name = parts[2];
                    let path = PathBuf::from(parts[3]);
                    match sync_coordinator.register_folder(folder_id, &path).await {
                        Ok(_) => {
                            println!("Added and watching shared folder '{}' ({}) at '{}'", name, folder_id, path.display());
                            let _ = sync_coordinator.trigger_sync(folder_id).await;
                        }
                        Err(e) => println!("Failed to register folder: {}", e),
                    }
                }
            }
            "sync" => {
                if parts.len() < 2 {
                    println!("Usage: sync <folder_id>");
                } else {
                    let folder_id = parts[1];
                    println!(">>> Broadcasting sync notification for folder '{}'...", folder_id);
                    if let Err(e) = sync_coordinator.trigger_sync(folder_id).await {
                        println!("Sync failed: {}", e);
                    }
                }
            }
            "index" => {
                if parts.len() < 2 {
                    println!("Usage: index <folder_id>");
                } else {
                    let folder_id = parts[1];
                    match sync_coordinator.initial_scan(folder_id).await {
                        Ok(_) => println!(">>> Initial scan and indexing complete for folder '{}'!", folder_id),
                        Err(e) => println!("Failed to index folder: {}", e),
                    }
                }
            }
            "events" => {
                if parts.len() < 2 {
                    println!("Usage: events <folder_id>");
                } else {
                    let folder_id = parts[1];
                    match metadata_service.handle_manifest_request(folder_id, 0).await {
                        Ok((events, latest)) => {
                            println!("\n--- SYNC EVENTS FOR FOLDER '{}' (Total: {}, Latest Seq: {}) ---", folder_id, events.len(), latest);
                            for ev in events {
                                println!(
                                    "  [Seq #{:04}] {:?} | Ver: {} | File: {} ({} bytes, hash: {}...) Origin: {}",
                                    ev.event_seq,
                                    ev.event_type,
                                    ev.version,
                                    ev.relative_path,
                                    ev.size_bytes,
                                    &ev.content_hash[..8.min(ev.content_hash.len())],
                                    ev.origin_node_id,
                                );
                            }
                            println!("--------------------------------------------------------------------\n");
                        }
                        Err(e) => println!("Failed to query events: {}", e),
                    }
                }
            }
            "manifest" => {
                if parts.len() < 3 {
                    println!("Usage: manifest <peer_id_or_name> <folder_id>");
                } else {
                    let id_str = parts[1];
                    let folder_id = parts[2];
                    let peers = manager.get_peer_statuses().await;
                    if let Some(target) = peers.iter().find(|p| p.node_id.as_str() == id_str || p.node_id.as_str().starts_with(id_str) || p.node_name.eq_ignore_ascii_case(id_str)) {
                        if let Some(conn) = manager.get_active_connection(&target.node_id).await {
                            println!(">>> Querying manifest for folder '{}' from {} ({})...", folder_id, target.node_name, target.node_id);
                            let req = Message::ManifestReq {
                                folder_id: folder_id.to_string(),
                                since_event_seq: 0,
                            };
                            if let Err(e) = conn.send(req).await {
                                println!("Failed to send ManifestReq: {}", e);
                                continue;
                            }

                            // Await ManifestResp
                            match tokio::time::timeout(std::time::Duration::from_secs(5), conn.recv_manifest_response()).await {
                                Ok(Some(Message::ManifestResp { events, latest_event_seq, .. })) => {
                                    println!(">>> Received {} events (Latest Seq: {}) from remote peer!", events.len(), latest_event_seq);
                                    let actions = metadata_service.reconcile_remote_events(&events).await.unwrap_or_default();
                                    println!("\n--- RECONCILIATION ACTIONS ({}) ---", actions.len());
                                    for act in &actions {
                                        match act {
                                            ReconciliationAction::DownloadNeeded { relative_path, version, content_hash, .. } => {
                                                println!("  [DOWNLOAD NEEDED] {} (Version: {}, Hash: {}...)", relative_path, version, &content_hash[..8.min(content_hash.len())]);
                                            }
                                            ReconciliationAction::UpToDate { file_id, version } => {
                                                println!("  [UP TO DATE] File {} at Version {}", file_id, version);
                                            }
                                            ReconciliationAction::DeleteLocal { relative_path, version, .. } => {
                                                println!("  [DELETE LOCAL] {} (Tombstone Version: {})", relative_path, version);
                                            }
                                            ReconciliationAction::Conflict { original_relative_path, conflict_relative_path, .. } => {
                                                println!("  [CONFLICT DETECTED] '{}' -> Branching to '{}' (Zero data loss)", original_relative_path, conflict_relative_path);
                                            }
                                            ReconciliationAction::Ignore => {
                                                println!("  [IGNORE] Obsolete or redundant event");
                                            }
                                        }
                                        let _ = metadata_service.apply_reconciliation_action(act).await;
                                    }
                                    println!("------------------------------------\n");
                                }
                                Ok(Some(other)) => println!("Unexpected response: {:?}", other),
                                Ok(None) => println!("Connection closed while awaiting ManifestResp"),
                                Err(_) => println!("Timeout awaiting ManifestResp"),
                            }
                        } else {
                            println!("Peer {} is not currently connected.", target.node_id);
                        }
                    } else {
                        println!("Peer '{}' not found in discovered peers.", id_str);
                    }
                }
            }
            "simulate-lan-drop" => {
                manager.set_simulate_lan_disabled(true);
                println!(">> SIMULATION: Direct LAN path severed. Automatic failover active.");
            }
            "simulate-lan-restore" => {
                manager.set_simulate_lan_disabled(false);
                println!(">> SIMULATION: Direct LAN path restored. Preferred LAN path will recover.");
            }
            "help" => {
                println!("Commands: peers, status, invite, pair <peer_id>, send <peer_id> <file>,");
                println!("          folders, add-folder <id> <name> <path>, index <id>, events <id>, manifest <peer> <id>,");
                println!("          simulate-lan-drop, simulate-lan-restore, quit");
            }
            "quit" | "exit" => {
                println!("Shutting down T.U.S.H.E.R node...");
                break;
            }
            other => {
                println!("Unknown command: '{}'. Type 'help' for options.", other);
            }
        }
    }

    Ok(())
}
