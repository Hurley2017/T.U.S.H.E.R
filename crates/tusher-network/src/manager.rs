use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};
use tusher_core::identity::{DeviceIdentity, NodeId};
use tusher_core::protocol::Message;
use tusher_core::types::TransportType;
use crate::connection::PeerConnection;
use crate::discovery::{DiscoveredPeer, LanDiscovery};
use crate::pairing::PairingManager;
use crate::tailscale::TailscaleDetector;
use crate::transport::{EndpointCandidate, TransportAddress};

#[derive(Debug, Clone)]
pub struct PeerStatusInfo {
    pub node_id: NodeId,
    pub node_name: String,
    pub platform: Option<tusher_core::identity::Platform>,
    pub is_connected: bool,
    pub active_transport: Option<TransportType>,
    pub active_addr: Option<SocketAddr>,
    pub latency: Option<Duration>,
    pub available_candidates: Vec<TransportAddress>,
    pub is_paired: bool,
}

pub type RequestHandler = Arc<
    dyn Fn(NodeId, Message) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<Message>> + Send>>
        + Send
        + Sync,
>;

pub struct ConnectionManager {
    identity: Arc<DeviceIdentity>,
    listen_port: u16,
    discovery_port: u16,
    pairing: Arc<PairingManager>,
    candidates: Arc<RwLock<HashMap<NodeId, Vec<EndpointCandidate>>>>,
    active_connections: Arc<RwLock<HashMap<NodeId, Arc<PeerConnection>>>>,
    failed_pings: Arc<RwLock<HashMap<NodeId, u32>>>,
    actual_listen_port: Arc<std::sync::atomic::AtomicU16>,
    simulate_lan_disabled: Arc<AtomicBool>,
    request_handler: Arc<RwLock<Option<RequestHandler>>>,
}

impl ConnectionManager {
    pub fn new(
        identity: Arc<DeviceIdentity>,
        listen_port: u16,
        discovery_port: u16,
    ) -> Self {
        let pairing = Arc::new(PairingManager::new(Arc::clone(&identity)));
        Self {
            identity,
            listen_port,
            actual_listen_port: Arc::new(std::sync::atomic::AtomicU16::new(listen_port)),
            discovery_port,
            pairing,
            candidates: Arc::new(RwLock::new(HashMap::new())),
            active_connections: Arc::new(RwLock::new(HashMap::new())),
            failed_pings: Arc::new(RwLock::new(HashMap::new())),
            simulate_lan_disabled: Arc::new(AtomicBool::new(false)),
            request_handler: Arc::new(RwLock::new(None)),
        }
    }

    pub fn listen_port(&self) -> u16 {
        self.actual_listen_port.load(Ordering::SeqCst)
    }

    pub async fn set_request_handler(&self, handler: RequestHandler) {
        {
            let mut h = self.request_handler.write().await;
            *h = Some(Arc::clone(&handler));
        }

        let conns: Vec<(NodeId, Arc<PeerConnection>)> = {
            let active = self.active_connections.read().await;
            active.iter().map(|(k, v)| (k.clone(), Arc::clone(v))).collect()
        };

        for (peer_id, conn) in conns {
            let handler_clone = Arc::clone(&handler);
            let conn_clone = Arc::clone(&conn);
            tokio::spawn(async move {
                loop {
                    let req_opt = conn_clone.recv_request().await;
                    match req_opt {
                        Some(req) => {
                            if let Some(resp) = handler_clone(peer_id.clone(), req).await {
                                let _ = conn_clone.send(resp).await;
                            }
                        }
                        None => break,
                    }
                }
            });
        }
    }

    pub fn identity(&self) -> &DeviceIdentity {
        &self.identity
    }

    pub fn pairing_manager(&self) -> &Arc<PairingManager> {
        &self.pairing
    }

    pub fn set_simulate_lan_disabled(&self, disabled: bool) {
        self.simulate_lan_disabled.store(disabled, Ordering::SeqCst);
        if disabled {
            warn!("SIMULATION: Direct LAN transport artificially DISABLED");
        } else {
            info!("SIMULATION: Direct LAN transport RESTORED");
        }
    }

    /// Registers or updates a discovered endpoint candidate for a peer
    pub async fn add_candidate(&self, node_id: NodeId, address: TransportAddress) {
        let is_new = {
            let mut map = self.candidates.write().await;
            let list = map.entry(node_id.clone()).or_default();

            if !list.iter().any(|c| c.address == address) {
                info!(
                    "Discovered new {} candidate for peer {}: {}",
                    address.transport_type, address.addr, address.addr
                );
                list.push(EndpointCandidate::new(address));
                list.sort_by_key(|c| c.priority());
                true
            } else {
                false
            }
        };

        if is_new {
            let is_connected = {
                let active = self.active_connections.read().await;
                active.contains_key(&node_id)
            };

            if !is_connected {
                self.attempt_connect_best_candidate(&node_id).await;
            }
        }
    }

    /// Starts the listener and background discovery & reconnection loop
    pub async fn start(
        self: Arc<Self>,
    ) -> anyhow::Result<tokio::task::JoinHandle<()>> {
        // 1. Start TCP listener for incoming peer connections
        let listen_addr = SocketAddr::from(([0, 0, 0, 0], self.listen_port));
        let listener = tokio::net::TcpListener::bind(listen_addr).await?;
        let actual_port = listener.local_addr()?.port();
        self.actual_listen_port.store(actual_port, Ordering::SeqCst);
        info!("T.U.S.H.E.R listening for peers on TCP port {}", actual_port);

        let this_accept = Arc::clone(&self);
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, remote_addr)) => {
                        let mgr = Arc::clone(&this_accept);
                        tokio::spawn(async move {
                            // Determine transport type by IP range
                            let transport_type = if LanDiscovery::is_tailscale_ip(&remote_addr.ip()) {
                                TransportType::Tailscale
                            } else {
                                TransportType::Lan
                            };

                            if transport_type == TransportType::Lan
                                && mgr.simulate_lan_disabled.load(Ordering::SeqCst)
                            {
                                debug!("Rejecting incoming LAN connection due to active simulation");
                                return;
                            }

                            match PeerConnection::accept(
                                stream,
                                &mgr.identity,
                                mgr.listen_port,
                                transport_type,
                            )
                            .await
                            {
                                Ok(conn) => {
                                    let peer_id = conn.remote_node_id().clone();
                                    let listen_addr = conn.remote_addr();
                                    mgr.register_active_connection(peer_id.clone(), conn).await;
                                    mgr.add_candidate(peer_id, TransportAddress::new(listen_addr, transport_type)).await;
                                }
                                Err(e) => {
                                    debug!("Incoming connection handshake failed from {}: {}", remote_addr, e);
                                }
                            }
                        });
                    }
                    Err(e) => {
                        error!("TCP listener accept error: {}", e);
                    }
                }
            }
        });

        // 2. Start LAN Discovery UDP Beacon
        let (disc_tx, mut disc_rx) = mpsc::channel::<DiscoveredPeer>(64);
        let discovery = LanDiscovery::new(
            Arc::clone(&self.identity),
            self.listen_port,
            self.discovery_port,
        );
        let _disc_handle = discovery.start(disc_tx)?;

        let this_disc = Arc::clone(&self);
        tokio::spawn(async move {
            while let Some(discovered) = disc_rx.recv().await {
                if this_disc.simulate_lan_disabled.load(Ordering::SeqCst)
                    && discovered.endpoint.transport_type == TransportType::Lan
                {
                    continue;
                }

                this_disc
                    .add_candidate(discovered.node_id.clone(), discovered.endpoint)
                    .await;
            }
        });

        // 3. Scan local Tailscale peers
        let tailscale_candidates = TailscaleDetector::query_peers(self.listen_port);
        for ts_cand in tailscale_candidates {
            info!("Found local Tailscale peer endpoint candidate: {}", ts_cand.addr);
        }

        // 4. Background Connection Maintenance & Automatic Failover loop
        let this_maint = Arc::clone(&self);
        let maint_handle = tokio::spawn(async move {
            let mut seq = 0u64;
            loop {
                tokio::time::sleep(Duration::from_secs(3)).await;
                seq = seq.wrapping_add(1);

                this_maint.maintain_connections(seq).await;
            }
        });

        Ok(maint_handle)
    }

    async fn register_active_connection(&self, node_id: NodeId, conn: PeerConnection) {
        let mut active = self.active_connections.write().await;
        if let Some(_existing) = active.get(&node_id) {
            info!(
                "Replacing existing connection to peer {} with new {} connection",
                node_id,
                conn.transport_type()
            );
        }
        let conn_arc = Arc::new(conn);
        active.insert(node_id.clone(), Arc::clone(&conn_arc));

        // Open personal mesh mode: auto-trust all active connections without authentication
        self.pairing.set_trusted(node_id.clone(), tusher_core::types::TrustStatus::Paired).await;

        {
            let mut fp = self.failed_pings.write().await;
            fp.remove(&node_id);
        }

        // Spawn request listener for this connection if handler registered
        let handler_opt = {
            let h = self.request_handler.read().await;
            h.clone()
        };

        if let Some(handler) = handler_opt {
            let conn_clone = Arc::clone(&conn_arc);
            let peer_id = node_id.clone();
            tokio::spawn(async move {
                loop {
                    let req_opt = conn_clone.recv_request().await;
                    match req_opt {
                        Some(req) => {
                            if let Some(resp) = handler(peer_id.clone(), req).await {
                                let _ = conn_clone.send(resp).await;
                            }
                        }
                        None => break,
                    }
                }
            });
        }
    }

    /// Health-checks active connections and triggers failover / preference restoration
    async fn maintain_connections(&self, seq: u64) {
        let peers: Vec<NodeId> = {
            let map = self.candidates.read().await;
            map.keys().cloned().collect()
        };

        for peer_id in peers {
            let mut active_conn_opt = {
                let active = self.active_connections.read().await;
                active.get(&peer_id).cloned()
            };

            let mut connection_healthy = false;
            let mut current_transport = None;

            if let Some(conn) = &active_conn_opt {
                // Check if LAN simulation disabled is active
                if conn.transport_type() == TransportType::Lan
                    && self.simulate_lan_disabled.load(Ordering::SeqCst)
                {
                    warn!("LAN severed by simulation for peer {}. Dropping socket.", peer_id);
                    let _ = conn.send(Message::Disconnect {
                        reason: "LAN transport severed".to_string(),
                    }).await;
                } else {
                    current_transport = Some(conn.transport_type());
                    // Probe connection with Ping
                    match conn.ping(seq).await {
                        Ok(rtt) => {
                            debug!("Peer {} ping RTT: {:?}", peer_id, rtt);
                            connection_healthy = true;
                            let mut fp = self.failed_pings.write().await;
                            fp.remove(&peer_id);
                        }
                        Err(e) => {
                            let count = {
                                let mut fp = self.failed_pings.write().await;
                                let c = fp.entry(peer_id.clone()).or_insert(0);
                                *c += 1;
                                *c
                            };
                            if count >= 3 {
                                warn!("Ping failed 3 consecutive times to peer {}: {}. Connection declared lost.", peer_id, e);
                                connection_healthy = false;
                            } else {
                                debug!("Ping missed ({}/3) to peer {}: {}", count, peer_id, e);
                                connection_healthy = true;
                            }
                        }
                    }
                }
            }

            if !connection_healthy {
                // Remove dead connection
                {
                    let mut active = self.active_connections.write().await;
                    active.remove(&peer_id);
                }
                {
                    let mut fp = self.failed_pings.write().await;
                    fp.remove(&peer_id);
                }
                active_conn_opt = None;

                // Automatic Failover: dial best available candidate
                self.attempt_connect_best_candidate(&peer_id).await;
            } else if let Some(TransportType::Tailscale) = current_transport {
                // If currently on Tailscale, check if preferred LAN candidate has become available!
                if !self.simulate_lan_disabled.load(Ordering::SeqCst) {
                    let candidates = {
                        let map = self.candidates.read().await;
                        map.get(&peer_id).cloned().unwrap_or_default()
                    };

                    if let Some(lan_cand) = candidates.iter().find(|c| c.address.transport_type == TransportType::Lan) {
                        debug!("Probing preferred LAN candidate {} for peer {}", lan_cand.address.addr, peer_id);
                        if let Ok(new_lan_conn) = PeerConnection::dial(
                            lan_cand.address.clone(),
                            &self.identity,
                            self.listen_port,
                        ).await {
                            info!(
                                "RESTORED PREFERENCE: Switched peer {} from Tailscale to LAN ({})",
                                peer_id, lan_cand.address.addr
                            );
                            self.register_active_connection(peer_id.clone(), new_lan_conn).await;
                        }
                    }
                }
            }
        }
    }

    async fn attempt_connect_best_candidate(&self, peer_id: &NodeId) {
        {
            let active = self.active_connections.read().await;
            if active.contains_key(peer_id) {
                return;
            }
        }

        let candidates = {
            let map = self.candidates.read().await;
            map.get(peer_id).cloned().unwrap_or_default()
        };

        for cand in candidates {
            if cand.address.transport_type == TransportType::Lan
                && self.simulate_lan_disabled.load(Ordering::SeqCst)
            {
                continue;
            }

            info!(
                "Attempting connection to peer {} via {} ({})",
                peer_id, cand.address.transport_type, cand.address.addr
            );

            match PeerConnection::dial(cand.address.clone(), &self.identity, self.listen_port).await {
                Ok(conn) => {
                    let real_id = conn.remote_node_id().clone();
                    info!(
                        "Successfully connected to peer {} (real id: {}) via {}",
                        peer_id, real_id, cand.address.transport_type
                    );

                    // If peer_id was a temporary placeholder like "remote", migrate candidate to real_id
                    if peer_id != &real_id {
                        let mut map = self.candidates.write().await;
                        map.remove(peer_id);
                        let list = map.entry(real_id.clone()).or_default();
                        if !list.iter().any(|c| c.address == cand.address) {
                            list.push(cand.clone());
                        }
                    }

                    self.register_active_connection(real_id, conn).await;
                    break;
                }
                Err(e) => {
                    debug!("Dial failed to {}: {}", cand.address.addr, e);
                }
            }
        }
    }

    /// Retrieves status snapshot of all known peers
    pub async fn get_peer_statuses(&self) -> Vec<PeerStatusInfo> {
        let mut list = Vec::new();
        let cands_map = self.candidates.read().await;
        let active_map = self.active_connections.read().await;

        for (id, cand_list) in cands_map.iter() {
            let mut is_connected = false;
            let mut active_transport = None;
            let mut active_addr = None;
            let mut latency = None;

            let mut node_name = id.to_string();
            let mut platform = None;
            if let Some(conn) = active_map.get(id) {
                is_connected = true;
                active_transport = Some(conn.transport_type());
                active_addr = Some(conn.remote_addr());
                latency = conn.last_ping_rtt();
                node_name = conn.remote_name().to_string();
                platform = Some(conn.remote_platform());
            }

            let is_paired = self.pairing.is_trusted(id).await;

            list.push(PeerStatusInfo {
                node_id: id.clone(),
                node_name,
                platform,
                is_connected,
                active_transport,
                active_addr,
                latency,
                available_candidates: cand_list.iter().map(|c| c.address.clone()).collect(),
                is_paired,
            });
        }

        list
    }

    pub async fn get_active_connection(
        &self,
        peer_id: &NodeId,
    ) -> Option<Arc<PeerConnection>> {
        let active = self.active_connections.read().await;
        active.get(peer_id).cloned()
    }

    pub async fn get_all_active_connections(&self) -> Vec<(NodeId, Arc<PeerConnection>)> {
        let active = self.active_connections.read().await;
        active.iter().map(|(k, v)| (k.clone(), Arc::clone(v))).collect()
    }

    pub async fn broadcast(&self, message: Message) {
        let conns = self.get_all_active_connections().await;
        for (_, conn) in conns {
            let _ = conn.send(message.clone()).await;
        }
    }
}
