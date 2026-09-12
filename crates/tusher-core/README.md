# `tusher-core`

**Foundational Cryptographic Primitives, Identity, and Wire Protocol for T.U.S.H.E.R**

---

## 1. Overview

`tusher-core` provides the low-level building blocks for the entire T.U.S.H.E.R ecosystem:
- **Device Identity**: Ed25519 signing and verifying keypairs.
- **Node ID Derivation**: Base32 canonical string formatting prefixed with `tshr_`.
- **Short Authentication String (SAS)**: Out-of-band numeric PIN computation.
- **Framed Protocol Codec**: Stream serialization/deserialization with length prefix and magic bytes.

---

## 2. Core Modules

### 2.1 Identity (`src/identity.rs`)
- `DeviceIdentity`: Encapsulates an Ed25519 `SigningKey` and `VerifyingKey`. Generates random identities, serializes keys to disk, and signs arbitrary byte payloads.
- `NodeId`: A strongly typed, hashable identifier formatted in lowercase Base32 (e.g. `tshr_osovdezp23ugq45mssgn5fqg`). Guaranteed 1:1 mathematical correspondence to the node's public key.

### 2.2 Short Authentication String (`src/crypto.rs`)
- `generate_sas_pin(key_a: &[u8; 32], key_b: &[u8; 32]) -> String`:
  Computes a deterministic 6-digit numeric Short Authentication String (SAS) from two public keys:
  ```text
  PIN = SHA256(min(A, B) || max(A, B)) mod 1,000,000
  ```
  Guarantees order-independent, identical PIN generation across both participating devices during pairing.

### 2.3 Wire Protocol & Framing (`src/protocol.rs`)
- Wire Format:
  ```text
  +------------------+-------------------+----------------------------+
  | Magic (4 bytes)  | Length (4 bytes)  | JSON Payload (N bytes)     |
  |  0x54 53 48 52   |  Big-Endian u32   | Serde-serialized Message   |
  +------------------+-------------------+----------------------------+
  ```
- `MessageCodec`: Implements `tokio_util::codec::Decoder` and `tokio_util::codec::Encoder` with a 16 MB maximum frame limit to prevent buffer exhaustion.
- `Message`: Core protocol enum supporting:
  - `Hello` / `HelloAck`: Handshake and identity exchange.
  - `Ping` / `Pong`: RTT health checks and keep-alives.
  - `PairingRequest` / `PairingResponse`: SAS numeric trust verification.
  - `TransferInit` / `TransferInitAck`: Chunk transfer negotiation and checkpoint sync.
  - `TransferChunk` / `TransferChunkAck`: 2 MB data blocks with per-chunk integrity verification.
  - `TransferComplete` / `TransferCompleteAck`: Full-file SHA-256 seal verification.
  - `ManifestReq` / `ManifestResp`: Distributed causal sync deltas.
  - `FilePullReq`: Downstream node pull requests.
  - `SyncNotify`: Real-time cascade broadcast notifications.
