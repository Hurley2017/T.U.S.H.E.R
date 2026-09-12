use serde::Deserialize;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::process::Command;
use std::str::FromStr;
use tracing::debug;
use tusher_core::types::TransportType;
use crate::transport::TransportAddress;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TailscalePeerStatus {
    #[serde(rename = "HostName")]
    pub host_name: Option<String>,
    #[serde(rename = "TailscaleIPs")]
    pub tailscale_ips: Option<Vec<String>>,
    #[serde(rename = "Online")]
    pub online: Option<bool>,
    #[serde(rename = "OS")]
    pub os: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TailscaleStatusOutput {
    #[serde(rename = "Self")]
    pub self_node: Option<TailscalePeerStatus>,
    #[serde(rename = "Peer")]
    pub peer: Option<HashMap<String, TailscalePeerStatus>>,
}

#[derive(Debug, Clone)]
pub struct TailscalePeerInfo {
    pub hostname: String,
    pub ip: Ipv4Addr,
    pub online: bool,
    pub os: String,
}

pub struct TailscaleDetector;

impl TailscaleDetector {
    /// Detects if Tailscale is installed and queries the Tailnet status
    pub fn query_peers(default_port: u16) -> Vec<TransportAddress> {
        let mut endpoints = Vec::new();

        // Attempt running `tailscale status --json`
        let output = match Command::new("tailscale").args(["status", "--json"]).output() {
            Ok(out) => out,
            Err(_) => {
                // Fallback: check typical Windows install path
                match Command::new(r"C:\Program Files\Tailscale\tailscale.exe")
                    .args(["status", "--json"])
                    .output()
                {
                    Ok(out) => out,
                    Err(e) => {
                        debug!("Tailscale CLI not accessible: {}", e);
                        return endpoints;
                    }
                }
            }
        };

        if !output.status.success() {
            debug!("Tailscale status command returned non-zero code");
            return endpoints;
        }

        if let Ok(status) = serde_json::from_slice::<TailscaleStatusOutput>(&output.stdout) {
            if let Some(peers) = status.peer {
                for (_, p) in peers {
                    // Only include online peers with valid IPv4
                    if let Some(ips) = p.tailscale_ips {
                        for ip_str in ips {
                            if let Ok(IpAddr::V4(ipv4)) = IpAddr::from_str(&ip_str) {
                                let addr = SocketAddr::new(IpAddr::V4(ipv4), default_port);
                                endpoints.push(TransportAddress::new(addr, TransportType::Tailscale));
                                break;
                            }
                        }
                    }
                }
            }
        }

        endpoints
    }
}
