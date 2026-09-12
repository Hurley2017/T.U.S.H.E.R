use rand::RngCore;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tusher_core::crypto::calculate_sas_pin;
use tusher_core::identity::{DeviceIdentity, NodeId};
use tusher_core::types::TrustStatus;

#[derive(Debug, Clone)]
pub struct PairingSession {
    pub ephemeral_token: String,
    pub target_peer_id: Option<NodeId>,
    pub sas_pin: Option<String>,
    pub created_at: std::time::Instant,
}

pub struct PairingManager {
    identity: Arc<DeviceIdentity>,
    trusted_peers: Arc<RwLock<HashMap<NodeId, TrustStatus>>>,
    active_sessions: Arc<RwLock<HashMap<String, PairingSession>>>,
}

impl PairingManager {
    pub fn new(identity: Arc<DeviceIdentity>) -> Self {
        Self {
            identity,
            trusted_peers: Arc::new(RwLock::new(HashMap::new())),
            active_sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn is_trusted(&self, node_id: &NodeId) -> bool {
        let peers = self.trusted_peers.read().await;
        matches!(peers.get(node_id), Some(TrustStatus::Paired))
    }

    pub async fn set_trusted(&self, node_id: NodeId, status: TrustStatus) {
        let mut peers = self.trusted_peers.write().await;
        peers.insert(node_id, status);
    }

    pub async fn create_pairing_invite(&self, hint_addr: &str) -> (String, String) {
        let mut token_bytes = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut token_bytes);
        let token = hex::encode(token_bytes);

        let uri = format!(
            "tusher://pair?id={}&pk={}&hint={}&token={}",
            self.identity.node_id(),
            self.identity.public_key_hex(),
            hint_addr,
            token
        );

        let session = PairingSession {
            ephemeral_token: token.clone(),
            target_peer_id: None,
            sas_pin: None,
            created_at: std::time::Instant::now(),
        };

        self.active_sessions.write().await.insert(token.clone(), session);
        (token, uri)
    }

    pub fn compute_sas(&self, token: &str, remote_pubkey_hex: &str) -> anyhow::Result<String> {
        let my_pk = hex::decode(self.identity.public_key_hex())?;
        let remote_pk = hex::decode(remote_pubkey_hex)?;
        Ok(calculate_sas_pin(token, &my_pk, &remote_pk))
    }
}
