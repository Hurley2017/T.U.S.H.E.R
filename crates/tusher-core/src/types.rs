use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrustStatus {
    Discovered,
    PendingPair,
    Paired,
    Blocked,
}

impl fmt::Display for TrustStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TrustStatus::Discovered => write!(f, "discovered"),
            TrustStatus::PendingPair => write!(f, "pending_pair"),
            TrustStatus::Paired => write!(f, "paired"),
            TrustStatus::Blocked => write!(f, "blocked"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TransportType {
    Lan,
    Tailscale,
    Manual,
}

impl fmt::Display for TransportType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportType::Lan => write!(f, "LAN"),
            TransportType::Tailscale => write!(f, "Tailscale"),
            TransportType::Manual => write!(f, "Manual"),
        }
    }
}
