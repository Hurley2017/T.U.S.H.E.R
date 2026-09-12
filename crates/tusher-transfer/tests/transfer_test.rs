use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tusher_core::identity::DeviceIdentity;
use tusher_core::protocol::Message;
use tusher_core::types::TransportType;
use tusher_network::connection::PeerConnection;
use tusher_network::transport::TransportAddress;
use tusher_transfer::hash::hash_file;
use tusher_transfer::receiver::FileReceiver;
use tusher_transfer::sender::FileSender;

async fn setup_peer_connection() -> (PeerConnection, PeerConnection) {
    let id_a = DeviceIdentity::generate("Sender".to_string());
    let id_b = DeviceIdentity::generate("Receiver".to_string());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let listen_addr = listener.local_addr().unwrap();

    let accept_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        PeerConnection::accept(stream, &id_b, listen_addr.port(), TransportType::Lan)
            .await
            .unwrap()
    });

    let target = TransportAddress::new(listen_addr, TransportType::Lan);
    let conn_a = PeerConnection::dial(target, &id_a, 0).await.unwrap();
    let conn_b = accept_task.await.unwrap();

    (conn_a, conn_b)
}

fn create_temp_dirs(test_name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!("tusher_test_{}_{}", test_name, rand::random::<u32>()));
    let src_dir = base.join("src");
    let staging_dir = base.join("staging");
    let dest_dir = base.join("dest");

    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&staging_dir).unwrap();
    std::fs::create_dir_all(&dest_dir).unwrap();

    (src_dir, staging_dir, dest_dir)
}

#[tokio::test]
async fn test_small_file_transfer() {
    let (src_dir, staging_dir, dest_dir) = create_temp_dirs("small_file");
    let file_path = src_dir.join("notes.txt");
    tokio::fs::write(&file_path, b"Hello T.U.S.H.E.R file transfer!").await.unwrap();

    let (mut conn_sender, conn_receiver) = setup_peer_connection().await;

    // Receiver event loop task
    let receiver = Arc::new(Mutex::new(
        FileReceiver::new(&staging_dir, &dest_dir).await.unwrap(),
    ));

    let rx_clone = Arc::clone(&receiver);
    let rx_task = tokio::spawn(async move {
        while let Ok(Some(msg)) = conn_receiver.recv().await {
            match msg {
                Message::TransferInit {
                    transfer_id,
                    folder_id,
                    file_name,
                    file_size,
                    content_hash,
                    chunk_size,
                    total_chunks,
                } => {
                    let mut rx = rx_clone.lock().await;
                    let resp = rx
                        .handle_init(
                            transfer_id,
                            folder_id,
                            file_name,
                            file_size,
                            content_hash,
                            chunk_size,
                            total_chunks,
                        )
                        .await
                        .unwrap();
                    conn_receiver.send(resp).await.unwrap();
                }
                Message::TransferChunk {
                    transfer_id,
                    chunk_index,
                    offset,
                    data,
                    chunk_hash,
                } => {
                    let mut rx = rx_clone.lock().await;
                    let resp = rx
                        .handle_chunk(transfer_id, chunk_index, offset, data, chunk_hash)
                        .await
                        .unwrap();
                    conn_receiver.send(resp).await.unwrap();
                }
                Message::TransferComplete {
                    transfer_id,
                    content_hash,
                } => {
                    let mut rx = rx_clone.lock().await;
                    let resp = rx.handle_complete(transfer_id, content_hash).await.unwrap();
                    conn_receiver.send(resp).await.unwrap();
                    break;
                }
                _ => {}
            }
        }
    });

    let sender = FileSender::new(64 * 1024); // 64KB chunk
    let stats = sender.send_file(&mut conn_sender, &file_path).await.unwrap();
    rx_task.await.unwrap();

    assert_eq!(stats.chunks_sent, 1);
    assert_eq!(stats.chunks_skipped, 0);

    // Verify file received and identical
    let dest_file = dest_dir.join("notes.txt");
    assert!(dest_file.exists());
    let src_hash = hash_file(&file_path).await.unwrap();
    let dest_hash = hash_file(&dest_file).await.unwrap();
    assert_eq!(src_hash, dest_hash);

    let _ = std::fs::remove_dir_all(src_dir.parent().unwrap());
}

#[tokio::test]
async fn test_large_binary_chunked_transfer() {
    let (src_dir, staging_dir, dest_dir) = create_temp_dirs("large_binary");
    let file_path = src_dir.join("video.bin");

    // 2.5 MB binary file
    let data_len = 2500 * 1024;
    let mut dummy_data = Vec::with_capacity(data_len);
    for i in 0..data_len {
        dummy_data.push((i % 256) as u8);
    }
    tokio::fs::write(&file_path, &dummy_data).await.unwrap();

    let (mut conn_sender, conn_receiver) = setup_peer_connection().await;

    let receiver = Arc::new(Mutex::new(
        FileReceiver::new(&staging_dir, &dest_dir).await.unwrap(),
    ));

    let rx_clone = Arc::clone(&receiver);
    let rx_task = tokio::spawn(async move {
        while let Ok(Some(msg)) = conn_receiver.recv().await {
            match msg {
                Message::TransferInit {
                    transfer_id,
                    folder_id,
                    file_name,
                    file_size,
                    content_hash,
                    chunk_size,
                    total_chunks,
                } => {
                    let mut rx = rx_clone.lock().await;
                    let resp = rx
                        .handle_init(
                            transfer_id,
                            folder_id,
                            file_name,
                            file_size,
                            content_hash,
                            chunk_size,
                            total_chunks,
                        )
                        .await
                        .unwrap();
                    conn_receiver.send(resp).await.unwrap();
                }
                Message::TransferChunk {
                    transfer_id,
                    chunk_index,
                    offset,
                    data,
                    chunk_hash,
                } => {
                    let mut rx = rx_clone.lock().await;
                    let resp = rx
                        .handle_chunk(transfer_id, chunk_index, offset, data, chunk_hash)
                        .await
                        .unwrap();
                    conn_receiver.send(resp).await.unwrap();
                }
                Message::TransferComplete {
                    transfer_id,
                    content_hash,
                } => {
                    let mut rx = rx_clone.lock().await;
                    let resp = rx.handle_complete(transfer_id, content_hash).await.unwrap();
                    conn_receiver.send(resp).await.unwrap();
                    break;
                }
                _ => {}
            }
        }
    });

    let chunk_size = 256 * 1024; // 256 KB chunks -> 10 chunks total
    let sender = FileSender::new(chunk_size);
    let stats = sender.send_file(&mut conn_sender, &file_path).await.unwrap();
    rx_task.await.unwrap();

    assert_eq!(stats.chunks_sent, 10);
    assert_eq!(stats.total_bytes, data_len as u64);

    let dest_file = dest_dir.join("video.bin");
    assert!(dest_file.exists());
    let src_hash = hash_file(&file_path).await.unwrap();
    let dest_hash = hash_file(&dest_file).await.unwrap();
    assert_eq!(src_hash, dest_hash);

    let _ = std::fs::remove_dir_all(src_dir.parent().unwrap());
}

#[tokio::test]
async fn test_resumable_transfer_after_interruption() {
    let (src_dir, staging_dir, dest_dir) = create_temp_dirs("resume_test");
    let file_path = src_dir.join("archive.tar");

    // 1 MB file, 100 KB chunks = 10 chunks
    let data_len = 1000 * 1024;
    let dummy_data: Vec<u8> = (0..data_len).map(|i| (i % 251) as u8).collect();
    tokio::fs::write(&file_path, &dummy_data).await.unwrap();

    let chunk_size = 100 * 1024; // 100 KB per chunk

    // --- PHASE 1: Transfer interrupted after 4 chunks ---
    let (mut conn_sender_1, conn_receiver_1) = setup_peer_connection().await;
    let receiver = Arc::new(Mutex::new(
        FileReceiver::new(&staging_dir, &dest_dir).await.unwrap(),
    ));

    let rx_clone = Arc::clone(&receiver);
    let rx_task_1 = tokio::spawn(async move {
        let mut chunks_received = 0;
        while let Ok(Some(msg)) = conn_receiver_1.recv().await {
            match msg {
                Message::TransferInit {
                    transfer_id,
                    folder_id,
                    file_name,
                    file_size,
                    content_hash,
                    chunk_size,
                    total_chunks,
                } => {
                    let mut rx = rx_clone.lock().await;
                    let resp = rx
                        .handle_init(
                            transfer_id,
                            folder_id,
                            file_name,
                            file_size,
                            content_hash,
                            chunk_size,
                            total_chunks,
                        )
                        .await
                        .unwrap();
                    conn_receiver_1.send(resp).await.unwrap();
                }
                Message::TransferChunk {
                    transfer_id,
                    chunk_index,
                    offset,
                    data,
                    chunk_hash,
                } => {
                    let mut rx = rx_clone.lock().await;
                    let resp = rx
                        .handle_chunk(transfer_id, chunk_index, offset, data, chunk_hash)
                        .await
                        .unwrap();
                    conn_receiver_1.send(resp).await.unwrap();
                    chunks_received += 1;

                    // SIMULATE SUDDEN NETWORK SEVER AFTER 4 CHUNKS!
                    if chunks_received == 4 {
                        println!("Intentionally severing connection at chunk 4...");
                        break;
                    }
                }
                _ => {}
            }
        }
    });

    let sender = FileSender::new(chunk_size);
    // This will error when receiver disconnects
    let _ = sender.send_file(&mut conn_sender_1, &file_path).await;
    let _ = rx_task_1.await;

    // Verify receiver staging has state persisted
    let transfer_id = format!("xfer_{}", &hash_file(&file_path).await.unwrap()[..16]);
    let state = tusher_transfer::state::TransferState::load(&staging_dir, &transfer_id).await.unwrap();
    assert_eq!(state.completed_chunks.len(), 4, "Should have exactly 4 chunks checkpointed");

    // Final file must NOT exist yet (incomplete file never exposed)
    assert!(!dest_dir.join("archive.tar").exists());

    // --- PHASE 2: Reconnection and Resume ---
    println!("Reconnecting and resuming transfer...");
    let (mut conn_sender_2, conn_receiver_2) = setup_peer_connection().await;

    let rx_clone_2 = Arc::clone(&receiver);
    let rx_task_2 = tokio::spawn(async move {
        while let Ok(Some(msg)) = conn_receiver_2.recv().await {
            match msg {
                Message::TransferInit {
                    transfer_id,
                    folder_id,
                    file_name,
                    file_size,
                    content_hash,
                    chunk_size,
                    total_chunks,
                } => {
                    let mut rx = rx_clone_2.lock().await;
                    let resp = rx
                        .handle_init(
                            transfer_id,
                            folder_id,
                            file_name,
                            file_size,
                            content_hash,
                            chunk_size,
                            total_chunks,
                        )
                        .await
                        .unwrap();
                    conn_receiver_2.send(resp).await.unwrap();
                }
                Message::TransferChunk {
                    transfer_id,
                    chunk_index,
                    offset,
                    data,
                    chunk_hash,
                } => {
                    let mut rx = rx_clone_2.lock().await;
                    let resp = rx
                        .handle_chunk(transfer_id, chunk_index, offset, data, chunk_hash)
                        .await
                        .unwrap();
                    conn_receiver_2.send(resp).await.unwrap();
                }
                Message::TransferComplete {
                    transfer_id,
                    content_hash,
                } => {
                    let mut rx = rx_clone_2.lock().await;
                    let resp = rx.handle_complete(transfer_id, content_hash).await.unwrap();
                    conn_receiver_2.send(resp).await.unwrap();
                    break;
                }
                _ => {}
            }
        }
    });

    let stats_resumed = sender.send_file(&mut conn_sender_2, &file_path).await.unwrap();
    rx_task_2.await.unwrap();

    // Verify resumption stats: exactly 4 chunks skipped, only remaining 6 sent!
    assert_eq!(stats_resumed.chunks_skipped, 4, "Must skip 4 already received chunks");
    assert_eq!(stats_resumed.chunks_sent, 6, "Must only send remaining 6 chunks");

    // Final file verified and placed atomically
    let dest_file = dest_dir.join("archive.tar");
    assert!(dest_file.exists(), "Final file must be placed upon completion");
    let src_hash = hash_file(&file_path).await.unwrap();
    let dest_hash = hash_file(&dest_file).await.unwrap();
    assert_eq!(src_hash, dest_hash, "Resumed file must match original hash 100%");

    let _ = std::fs::remove_dir_all(src_dir.parent().unwrap());
}

#[tokio::test]
async fn test_corrupted_transfer_rejected() {
    let (src_dir, staging_dir, dest_dir) = create_temp_dirs("corrupt_test");
    let file_path = src_dir.join("secret.data");
    tokio::fs::write(&file_path, b"Strict integrity check data").await.unwrap();

    let (mut conn_sender, conn_receiver) = setup_peer_connection().await;

    let receiver = Arc::new(Mutex::new(
        FileReceiver::new(&staging_dir, &dest_dir).await.unwrap(),
    ));

    let rx_clone = Arc::clone(&receiver);
    let rx_task = tokio::spawn(async move {
        while let Ok(Some(msg)) = conn_receiver.recv().await {
            match msg {
                Message::TransferInit {
                    transfer_id,
                    folder_id,
                    file_name,
                    file_size,
                    content_hash,
                    chunk_size,
                    total_chunks,
                } => {
                    let mut rx = rx_clone.lock().await;
                    let resp = rx
                        .handle_init(
                            transfer_id,
                            folder_id,
                            file_name,
                            file_size,
                            content_hash,
                            chunk_size,
                            total_chunks,
                        )
                        .await
                        .unwrap();
                    conn_receiver.send(resp).await.unwrap();
                }
                Message::TransferChunk {
                    transfer_id,
                    chunk_index,
                    offset,
                    data,
                    chunk_hash,
                } => {
                    let mut rx = rx_clone.lock().await;
                    // Tamper with data before saving to simulate bitflip / corruption
                    let mut corrupted_data = data;
                    if !corrupted_data.is_empty() {
                        corrupted_data[0] ^= 0xFF;
                    }
                    let resp = rx
                        .handle_chunk(transfer_id, chunk_index, offset, corrupted_data, chunk_hash)
                        .await
                        .unwrap();
                    conn_receiver.send(resp).await.unwrap();
                }
                _ => {}
            }
        }
    });

    let sender = FileSender::new(64 * 1024);
    // send_file should fail because receiver rejects corrupted chunk
    let result = sender.send_file(&mut conn_sender, &file_path).await;
    assert!(result.is_err(), "Corrupted transfer must fail!");

    // Dest file must never be created
    let dest_file = dest_dir.join("secret.data");
    assert!(!dest_file.exists(), "Corrupted file must never be placed in destination directory");

    rx_task.abort();
    let _ = std::fs::remove_dir_all(src_dir.parent().unwrap());
}
