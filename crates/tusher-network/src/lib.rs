pub mod connection;
pub mod discovery;
pub mod manager;
pub mod pairing;
pub mod tailscale;
pub mod transport;

pub use connection::PeerConnection;
pub use discovery::{DiscoveredPeer, LanDiscovery, DEFAULT_DISCOVERY_PORT};
pub use manager::{ConnectionManager, PeerStatusInfo};
pub use pairing::PairingManager;
pub use tailscale::TailscaleDetector;
pub use transport::{EndpointCandidate, TransportAddress};
