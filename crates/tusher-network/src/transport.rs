use std::net::SocketAddr;
use std::time::Duration;
use tusher_core::types::TransportType;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TransportAddress {
    pub addr: SocketAddr,
    pub transport_type: TransportType,
}

impl TransportAddress {
    pub fn new(addr: SocketAddr, transport_type: TransportType) -> Self {
        Self { addr, transport_type }
    }
}

#[derive(Debug, Clone)]
pub struct EndpointCandidate {
    pub address: TransportAddress,
    pub latency: Option<Duration>,
    pub last_success: Option<std::time::Instant>,
    pub consecutive_failures: u32,
}

impl EndpointCandidate {
    pub fn new(address: TransportAddress) -> Self {
        Self {
            address,
            latency: None,
            last_success: None,
            consecutive_failures: 0,
        }
    }

    pub fn priority(&self) -> u8 {
        match self.address.transport_type {
            TransportType::Lan => 1,
            TransportType::Tailscale => 2,
            TransportType::Manual => 3,
        }
    }
}
