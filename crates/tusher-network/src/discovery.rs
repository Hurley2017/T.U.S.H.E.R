use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use tusher_core::identity::{DeviceIdentity, NodeId, Platform};
use tusher_core::types::TransportType;
use crate::transport::TransportAddress;

pub const DISCOVERY_MAGIC: &str = "TUSHER_DISCOVER_V1";
pub const DEFAULT_DISCOVERY_PORT: u16 = 42425;
pub const MULTICAST_IPV4: Ipv4Addr = Ipv4Addr::new(239, 255, 42, 42);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryBeacon {
    pub tusher_magic: String,
    pub node_id: NodeId,
    pub node_name: String,
    pub platform: Platform,
    pub port: u16,
    pub public_key_hex: String,
    pub version: u32,
}

#[derive(Debug, Clone)]
pub struct DiscoveredPeer {
    pub node_id: NodeId,
    pub node_name: String,
    pub platform: Platform,
    pub public_key_hex: String,
    pub endpoint: TransportAddress,
}

pub struct LanDiscovery {
    identity: Arc<DeviceIdentity>,
    listen_port: u16,
    discovery_port: u16,
    running: Arc<AtomicBool>,
}

impl LanDiscovery {
    pub fn new(identity: Arc<DeviceIdentity>, listen_port: u16, discovery_port: u16) -> Self {
        Self {
            identity,
            listen_port,
            discovery_port,
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Determines if an IP is in the Tailscale CGNAT range (100.64.0.0/10)
    pub fn is_tailscale_ip(ip: &IpAddr) -> bool {
        match ip {
            IpAddr::V4(ipv4) => {
                let octets = ipv4.octets();
                octets[0] == 100 && (octets[1] >= 64 && octets[1] <= 127)
            }
            _ => false,
        }
    }

    pub fn start(
        &self,
        peer_tx: mpsc::Sender<DiscoveredPeer>,
    ) -> anyhow::Result<tokio::task::JoinHandle<()>> {
        self.running.store(true, Ordering::SeqCst);
        let running = Arc::clone(&self.running);
        let identity = Arc::clone(&self.identity);
        let listen_port = self.listen_port;
        let discovery_port = self.discovery_port;

        let handle = tokio::spawn(async move {
            let socket = (|| -> anyhow::Result<UdpSocket> {
                let domain = socket2::Domain::IPV4;
                let sk = socket2::Socket::new(domain, socket2::Type::DGRAM, None)?;
                sk.set_reuse_address(true)?;
                sk.set_broadcast(true)?;
                sk.set_nonblocking(true)?;
                let bind_addr: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), discovery_port);
                sk.bind(&bind_addr.into())?;
                let std_sock: std::net::UdpSocket = sk.into();
                let tokio_sock = UdpSocket::from_std(std_sock)?;
                Ok(tokio_sock)
            })();

            let socket = match socket {
                Ok(s) => {
                    info!("Discovery service listening on UDP 0.0.0.0:{}", discovery_port);
                    s
                }
                Err(e) => {
                    warn!(
                        "Could not bind discovery socket to port {} with reuse ({}). Creating ephemeral socket.",
                        discovery_port, e
                    );
                    match UdpSocket::bind("0.0.0.0:0").await {
                        Ok(s) => s,
                        Err(e2) => {
                            error!("Failed to create UDP discovery socket: {}", e2);
                            return;
                        }
                    }
                }
            };

            let _ = socket.set_broadcast(true);

            let socket = Arc::new(socket);
            let socket_recv = Arc::clone(&socket);
            let running_recv = Arc::clone(&running);
            let local_id = identity.node_id().clone();

            // Background Receiver Task
            let recv_task = tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                while running_recv.load(Ordering::SeqCst) {
                    tokio::select! {
                        res = socket_recv.recv_from(&mut buf) => {
                            match res {
                                Ok((len, src_addr)) => {
                                    if let Ok(beacon) = serde_json::from_slice::<DiscoveryBeacon>(&buf[..len]) {
                                        if beacon.tusher_magic == DISCOVERY_MAGIC && beacon.node_id != local_id {
                                            let transport_type = if Self::is_tailscale_ip(&src_addr.ip()) {
                                                TransportType::Tailscale
                                            } else {
                                                TransportType::Lan
                                            };

                                            let peer_endpoint = TransportAddress::new(
                                                SocketAddr::new(src_addr.ip(), beacon.port),
                                                transport_type,
                                            );

                                            let discovered = DiscoveredPeer {
                                                node_id: beacon.node_id,
                                                node_name: beacon.node_name,
                                                platform: beacon.platform,
                                                public_key_hex: beacon.public_key_hex,
                                                endpoint: peer_endpoint,
                                            };

                                            let _ = peer_tx.send(discovered).await;
                                        }
                                    }
                                }
                                Err(e) => {
                                    debug!("UDP recv error: {}", e);
                                    tokio::time::sleep(Duration::from_millis(500)).await;
                                }
                            }
                        }
                    }
                }
            });

            // Background Broadcast Transmitter Task
            let socket_send = Arc::clone(&socket);
            let beacon = DiscoveryBeacon {
                tusher_magic: DISCOVERY_MAGIC.to_string(),
                node_id: identity.node_id().clone(),
                node_name: identity.node_name().to_string(),
                platform: identity.platform(),
                port: listen_port,
                public_key_hex: identity.public_key_hex(),
                version: tusher_core::PROTOCOL_VERSION,
            };

            let beacon_bytes = serde_json::to_vec(&beacon).unwrap_or_default();

            while running.load(Ordering::SeqCst) {
                // Broadcast to standard broadcast address
                let broadcast_addr = SocketAddr::new(
                    IpAddr::V4(Ipv4Addr::BROADCAST),
                    discovery_port,
                );
                let _ = socket_send.send_to(&beacon_bytes, broadcast_addr).await;

                // Also send to local multicast
                let multicast_addr = SocketAddr::new(
                    IpAddr::V4(MULTICAST_IPV4),
                    discovery_port,
                );
                let _ = socket_send.send_to(&beacon_bytes, multicast_addr).await;

                // Broadcast interval (3 seconds)
                tokio::time::sleep(Duration::from_secs(3)).await;
            }

            recv_task.abort();
        });

        Ok(handle)
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}
