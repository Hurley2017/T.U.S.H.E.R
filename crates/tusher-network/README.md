# `tusher-network`

**Multi-Transport Networking, LAN Discovery, Tailscale Integration, and Mutual Pairing for T.U.S.H.E.R**

---

## 1. Overview

`tusher-network` manages all physical and logical connections between nodes in the mesh. It provides:
- **Dynamic Transport Abstraction**: Seamless priority switching between Local Area Network (LAN) and Tailscale (WireGuard peer-to-peer).
- **LAN Discovery Beacons**: UDP broadcast/multicast peer discovery.
- **Connection Health Checks & RTT Tracking**: Automatic keepalive pings, RTT tracking, and dead connection replacement.
- **Multiplexed Connection Streams**: Completely isolated background channels for write, request, response, manifest, and pong packets.
- **Cryptographic Device Pairing**: Out-of-band Short Authentication String (SAS) numeric PIN verification.

---

## 2. Core Modules

### 2.1 Transport Address & Types (`src/transport.rs`)
- `TransportType`: `Lan` (Priority 1) vs. `Tailscale` (Priority 2).
- `TransportAddress`: Encapsulates `SocketAddr` and `TransportType`.
- `EndpointCandidate`: Tracked endpoint for peers, sorted by priority.

### 2.2 Peer Connection (`src/connection.rs`)
- `PeerConnection`: Full-duplex asynchronous connection wrapping a framed TCP stream.
- **Channel Isolation**:
  - `write_tx`: Background sink writer for outgoing messages.
  - `request_rx`: Unsolicited requests (e.g. `FilePullReq`, `SyncNotify`, `TransferInit`).
  - `response_rx`: Transactional responses (`TransferInitAck`, `TransferChunkAck`, `TransferCompleteAck`).
  - `manifest_rx`: Dedicated manifest responses (`ManifestResp`), isolating delta exchange from chunk transfer ACKs.
  - `pong_rx`: Isolated keepalive pong receiver, preventing ping checks from interfering with data streams.

### 2.3 Connection Manager (`src/manager.rs`)
- `ConnectionManager`:
  - Maintains `active_connections` map by `NodeId`.
  - Background maintenance loop checks connection health every 3 seconds using `ping(seq)`.
  - Tolerates transient network hiccups (up to 3 consecutive missed pings before dropping).
  - **Dynamic Candidate Migration**: Automatically reconciles placeholder addresses (`remote`) to the verified `NodeId` upon handshake.
  - **Priority Restoration**: If a peer is active over Tailscale and a direct LAN route becomes available, connections dynamically migrate to the higher-speed LAN.

### 2.4 LAN Discovery (`src/discovery.rs`)
- `LanDiscovery`: Background UDP socket broadcasting discovery beacons (`DiscoveryBeacon`) on local subnets.
- Discovered endpoints are automatically registered as candidates in the connection manager.

### 2.5 Tailscale Detection (`src/tailscale.rs`)
- `TailscaleDetector`: Detects local Tailscale virtual IP interfaces (`100.64.0.0/10`) to provide direct zero-config P2P tunneling through NATs.

### 2.6 Pairing Manager (`src/pairing.rs`)
- `PairingManager`:
  - Computes 6-digit numeric Short Authentication Strings (SAS PINs) from device public keys.
  - Manages `TrustStatus`: `Unknown`, `Discovered`, `PairingPending`, `Paired`, `Revoked`.
  - Persists paired peer certificates in local storage.
