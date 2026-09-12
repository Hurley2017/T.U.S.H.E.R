use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_util::codec::Framed;
use tracing::info;
use tusher_core::identity::{DeviceIdentity, NodeId, Platform};
use tusher_core::protocol::{Message, MessageCodec, PROTOCOL_VERSION};
use tusher_core::types::TransportType;
use crate::transport::TransportAddress;

#[derive(Clone)]
pub struct PeerConnection {
    remote_node_id: NodeId,
    remote_name: String,
    remote_platform: Platform,
    remote_pubkey_hex: String,
    transport_address: TransportAddress,
    write_tx: mpsc::Sender<Message>,
    request_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<Message>>>,
    response_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<Message>>>,
    manifest_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<Message>>>,
    pong_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<(u64, i64)>>>,
    last_ping_rtt_us: Arc<AtomicU64>,
}

impl PeerConnection {
    /// Outgoing connection handshake: Dials peer, sends Hello, verifies remote Hello/HelloAck
    pub async fn dial(
        target: TransportAddress,
        local_identity: &DeviceIdentity,
        listen_port: u16,
    ) -> anyhow::Result<Self> {
        let stream = timeout(Duration::from_secs(5), TcpStream::connect(target.addr))
            .await
            .map_err(|_| anyhow::anyhow!("Connection timeout dialing {}", target.addr))??;

        let mut framed = Framed::new(stream, MessageCodec::default());

        // 1. Send Hello
        let hello = Message::Hello {
            version: PROTOCOL_VERSION,
            node_id: local_identity.node_id().clone(),
            node_name: local_identity.node_name().to_string(),
            platform: local_identity.platform(),
            public_key_hex: local_identity.public_key_hex(),
            listen_port,
        };

        framed.send(hello).await?;

        // 2. Wait for Hello / HelloAck response
        let resp = timeout(Duration::from_secs(5), framed.next())
            .await
            .map_err(|_| anyhow::anyhow!("Handshake response timeout from {}", target.addr))?
            .ok_or_else(|| anyhow::anyhow!("Peer closed connection during handshake"))??;

        let (version, node_id, node_name, platform, public_key_hex) = match resp {
            Message::Hello {
                version,
                node_id,
                node_name,
                platform,
                public_key_hex,
                ..
            } => (version, node_id, node_name, platform, public_key_hex),
            Message::HelloAck {
                version,
                node_id,
                node_name,
                platform,
                public_key_hex,
                accepted: true,
                ..
            } => (version, node_id, node_name, platform, public_key_hex),
            Message::HelloAck {
                accepted: false,
                reason,
                ..
            } => {
                anyhow::bail!("Peer rejected connection: {:?}", reason);
            }
            other => {
                anyhow::bail!("Unexpected handshake message: {:?}", other);
            }
        };

        if version != PROTOCOL_VERSION {
            anyhow::bail!("Protocol version mismatch: peer has version {}", version);
        }

        // Verify that node_id matches the public key
        let pubkey_bytes = hex::decode(&public_key_hex)?;
        let vk = ed25519_dalek::VerifyingKey::try_from(pubkey_bytes.as_slice())
            .map_err(|e| anyhow::anyhow!("Invalid peer public key: {}", e))?;
        let computed_id = NodeId::from_verifying_key(&vk);
        if computed_id != node_id {
            anyhow::bail!("Peer NodeId does not match claimed public key!");
        }

        info!(
            "Connected to peer {} ({}) via {} at {}",
            node_name, node_id, target.transport_type, target.addr
        );

        let (conn, _rtt_holder) = Self::spawn_duplex(
            framed,
            node_id,
            node_name,
            platform,
            public_key_hex,
            target,
        );

        Ok(conn)
    }

    /// Incoming connection handshake: Accepts incoming stream, waits for Hello, sends HelloAck
    pub async fn accept(
        stream: TcpStream,
        local_identity: &DeviceIdentity,
        _listen_port: u16,
        transport_type: TransportType,
    ) -> anyhow::Result<Self> {
        let peer_addr = stream.peer_addr()?;
        let mut framed = Framed::new(stream, MessageCodec::default());

        // Wait for Hello from remote peer
        let incoming = timeout(Duration::from_secs(5), framed.next())
            .await
            .map_err(|_| anyhow::anyhow!("Handshake timeout from {}", peer_addr))?
            .ok_or_else(|| anyhow::anyhow!("Stream closed before Hello"))??;

        if let Message::Hello {
            version,
            node_id,
            node_name,
            platform,
            public_key_hex,
            listen_port,
        } = incoming
        {
            if version != PROTOCOL_VERSION {
                let reject = Message::HelloAck {
                    version: PROTOCOL_VERSION,
                    node_id: local_identity.node_id().clone(),
                    node_name: local_identity.node_name().to_string(),
                    platform: local_identity.platform(),
                    public_key_hex: local_identity.public_key_hex(),
                    accepted: false,
                    reason: Some("Protocol version mismatch".to_string()),
                };
                let _ = framed.send(reject).await;
                anyhow::bail!("Version mismatch with peer {}", peer_addr);
            }

            // Verify public key matches node_id
            let pubkey_bytes = hex::decode(&public_key_hex)?;
            let vk = ed25519_dalek::VerifyingKey::try_from(pubkey_bytes.as_slice())
                .map_err(|e| anyhow::anyhow!("Invalid public key: {}", e))?;
            let computed_id = NodeId::from_verifying_key(&vk);
            if computed_id != node_id {
                anyhow::bail!("Node ID verification failed");
            }

            // Send HelloAck
            let ack = Message::HelloAck {
                version: PROTOCOL_VERSION,
                node_id: local_identity.node_id().clone(),
                node_name: local_identity.node_name().to_string(),
                platform: local_identity.platform(),
                public_key_hex: local_identity.public_key_hex(),
                accepted: true,
                reason: None,
            };
            framed.send(ack).await?;

            let remote_listen_addr = SocketAddr::new(peer_addr.ip(), listen_port);
            let target = TransportAddress::new(remote_listen_addr, transport_type);

            info!(
                "Accepted incoming peer {} ({}) via {} from {}",
                node_name, node_id, transport_type, peer_addr
            );

            let (conn, _rtt_holder) = Self::spawn_duplex(
                framed,
                node_id,
                node_name,
                platform,
                public_key_hex,
                target,
            );

            Ok(conn)
        } else {
            anyhow::bail!("Expected Hello message, received {:?}", incoming);
        }
    }

    fn spawn_duplex(
        framed: Framed<TcpStream, MessageCodec>,
        remote_node_id: NodeId,
        remote_name: String,
        remote_platform: Platform,
        remote_pubkey_hex: String,
        transport_address: TransportAddress,
    ) -> (Self, Arc<AtomicU64>) {
        let (mut sink, mut stream) = framed.split();
        let (write_tx, mut write_rx) = mpsc::channel::<Message>(128);
        let (request_tx, request_rx) = mpsc::channel::<Message>(128);
        let (response_tx, response_rx) = mpsc::channel::<Message>(128);
        let (manifest_tx, manifest_rx) = mpsc::channel::<Message>(128);
        let (pong_tx, pong_rx) = mpsc::channel::<(u64, i64)>(64);
        let last_ping_rtt_us = Arc::new(AtomicU64::new(0));

        // Background Writer Task
        tokio::spawn(async move {
            while let Some(msg) = write_rx.recv().await {
                if let Err(e) = sink.send(msg).await {
                    tracing::warn!("TCP stream write error: {}", e);
                    break;
                }
            }
            tracing::info!("TCP stream writer ended for peer connection");
        });

        // Background Reader Task with multiplexing: Ping, Pong, Manifest, Requests, Responses
        let auto_pong_tx = write_tx.clone();
        tokio::spawn(async move {
            while let Some(res) = stream.next().await {
                match res {
                    Ok(msg) => match &msg {
                        Message::Ping { sequence, timestamp_ms } => {
                            let _ = auto_pong_tx.send(Message::Pong {
                                sequence: *sequence,
                                timestamp_ms: *timestamp_ms,
                            }).await;
                        }
                        Message::Pong { sequence, timestamp_ms } => {
                            let _ = pong_tx.send((*sequence, *timestamp_ms)).await;
                        }
                        Message::ManifestResp { .. } => {
                            let _ = manifest_tx.send(msg).await;
                        }
                        Message::TransferInitAck { .. }
                        | Message::TransferChunkAck { .. }
                        | Message::TransferCompleteAck { .. }
                        | Message::FolderListResp { .. } => {
                            let _ = response_tx.send(msg).await;
                        }
                        _ => {
                            let _ = request_tx.send(msg).await;
                        }
                    },
                    Err(e) => {
                        tracing::warn!("TCP stream read error: {}", e);
                        break;
                    }
                }
            }
            tracing::info!("TCP stream reader ended for peer connection");
        });

        (
            Self {
                remote_node_id,
                remote_name,
                remote_platform,
                remote_pubkey_hex,
                transport_address,
                write_tx,
                request_rx: Arc::new(tokio::sync::Mutex::new(request_rx)),
                response_rx: Arc::new(tokio::sync::Mutex::new(response_rx)),
                manifest_rx: Arc::new(tokio::sync::Mutex::new(manifest_rx)),
                pong_rx: Arc::new(tokio::sync::Mutex::new(pong_rx)),
                last_ping_rtt_us: Arc::clone(&last_ping_rtt_us),
            },
            last_ping_rtt_us,
        )
    }

    pub fn remote_node_id(&self) -> &NodeId {
        &self.remote_node_id
    }

    pub fn remote_name(&self) -> &str {
        &self.remote_name
    }

    pub fn remote_platform(&self) -> Platform {
        self.remote_platform
    }

    pub fn remote_pubkey_hex(&self) -> &str {
        &self.remote_pubkey_hex
    }

    pub fn transport_type(&self) -> TransportType {
        self.transport_address.transport_type
    }

    pub fn remote_addr(&self) -> SocketAddr {
        self.transport_address.addr
    }

    pub fn last_ping_rtt(&self) -> Option<Duration> {
        let us = self.last_ping_rtt_us.load(Ordering::Relaxed);
        if us > 0 {
            Some(Duration::from_micros(us))
        } else {
            None
        }
    }

    pub async fn send(&self, msg: Message) -> anyhow::Result<()> {
        self.write_tx.send(msg).await.map_err(|e| anyhow::anyhow!("Send failed: {}", e))?;
        Ok(())
    }

    pub async fn recv_request(&self) -> Option<Message> {
        let mut rx = self.request_rx.lock().await;
        rx.recv().await
    }

    pub async fn recv_response(&self) -> Option<Message> {
        let mut rx = self.response_rx.lock().await;
        rx.recv().await
    }

    pub async fn recv_manifest_response(&self) -> Option<Message> {
        let mut rx = self.manifest_rx.lock().await;
        rx.recv().await
    }

    pub async fn recv(&self) -> anyhow::Result<Option<Message>> {
        let mut req_rx = self.request_rx.lock().await;
        let mut resp_rx = self.response_rx.lock().await;
        tokio::select! {
            res = resp_rx.recv() => Ok(res),
            req = req_rx.recv() => Ok(req),
        }
    }

    /// Measures Round-Trip Time (RTT) using Ping / Pong
    pub async fn ping(&self, seq: u64) -> anyhow::Result<Duration> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let start = Instant::now();
        self.send(Message::Ping {
            sequence: seq,
            timestamp_ms: now_ms,
        })
        .await?;

        // Wait for matching Pong from pong_rx
        let deadline = Duration::from_secs(6);
        let mut rx = self.pong_rx.lock().await;
        let ping_result = timeout(deadline, async {
            loop {
                if let Some((sequence, _)) = rx.recv().await {
                    if sequence == seq {
                        return Ok(());
                    }
                } else {
                    anyhow::bail!("Stream closed while awaiting pong");
                }
            }
        })
        .await;

        match ping_result {
            Ok(Ok(())) => {
                let rtt = start.elapsed();
                self.last_ping_rtt_us.store(rtt.as_micros() as u64, Ordering::Relaxed);
                Ok(rtt)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => anyhow::bail!("Ping timed out"),
        }
    }
}
