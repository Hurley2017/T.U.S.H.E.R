# T.U.S.H.E.R

**Decentralized Personal File & Media Mesh**  
*Seamless, private, peer-to-peer data synchronization and media access across all personal devices.*

[![Rust](https://img.shields.io/badge/Rust-1.85%2B-orange.svg)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Platform: Cross-Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20Linux%20%7C%20macOS%20%7C%20Android-green.svg)]()
[![Tests: Passing](https://img.shields.io/badge/Tests-23%2F23%20Passing-brightgreen.svg)]()

---

## 1. Executive Product Definition

**T.U.S.H.E.R** transforms a user's collection of personal devices (Android phones, tablets, Windows workstations, laptops, Linux machines, and macOS computers) into **one private logical file environment**.

Every device operates as an **autonomous, sovereign node**:
- **Zero Central Servers**: No reliance on third-party cloud infrastructure, centralized relays, or subscription accounts.
- **Direct Peer-to-Peer Transport**: Transfers prioritize local gigabit LANs, automatically transition to Tailscale peer-to-peer tunnels off-LAN, and relay transitively when endpoints are not directly reachable.
- **End-to-End Cryptographic Security**: Mutual authentication rooted in Ed25519 public keys and out-of-band numeric Short Authentication String (SAS) pairing.
- **Deterministic Causality & Zero Data Loss**: Monotonic distributed event logs reconcile concurrent file edits without data destruction—divergent modifications branch into preserved conflict files.

```
       +--------------------------------------------------------+
       |               T.U.S.H.E.R Logical Mesh                 |
       +--------------------------------------------------------+
               /                   |                    \
              v                    v                     v
      +---------------+    +---------------+     +---------------+
      |   Desktop-A   |<-->|   Laptop-B    |<--->|   Tablet-C    |
      |   (Windows)   |    |    (Linux)    |     |   (Android)   |
      +---------------+    +---------------+     +---------------+
              \                                         /
               \ - - - - No Direct Connection - - - - -/
               [Transitive Forwarding & Sync via Node B]
```

---

## 2. Architecture Overview

T.U.S.H.E.R is structured as a high-performance modular Rust workspace partitioned across focused crates:

```
crates/
├── tusher-core/       # Identity, Ed25519 cryptography, wire protocol & frame codec
├── tusher-network/    # Multi-transport management (LAN, Tailscale), discovery, health pings, pairing
├── tusher-transfer/   # Resumable, chunked file transfer engine (2 MB chunks, SHA-256 / BLAKE3)
├── tusher-metadata/   # SQLite event store, monotonic causal versioning & reconciliation
├── tusher-sync/       # Native filesystem watcher, debouncing, echo suppression & mesh cascade
├── tusher-cli/        # Interactive node daemon, CLI control interface, and testing tools
├── tusher-ffi/        # Mozilla UniFFI 0.28 FFI bridge & Kotlin bindings for Android
└── tusher-desktop/    # Native Desktop System Tray, Windows Explorer Context Menus & Axum Web Dashboard
```

### Layered Architecture Diagram

```
+-------------------------------------------------------------------------------+
|                             User & Product Layer                              |
|   CLI REPL (tusher-cli)       |  Desktop Daemon & Web Dashboard (tusher-desktop)|
|   Windows Explorer Context Menu|  Android SDK & UniFFI Bridge (tusher-ffi)     |
+-------------------------------------------------------------------------------+
                                        |
+-------------------------------------------------------------------------------+
|                       Synchronization & Coordinator Layer                     |
|                               crates/tusher-sync                              |
|   - SyncCoordinator: Causal event sync, delta manifests, peer reconciliation  |
|   - FolderWatcher: Native OS hooks (ReadDirectoryChangesW, inotify, FSEvents)  |
|   - Echo Suppression Cache: Prevents sync loops during file ingestion        |
|   - Conflict Resolution: Zero-data-loss deterministic branch preservation     |
+-------------------------------------------------------------------------------+
                                        |
       +--------------------------------+-------------------------------+
       |                                                                |
+-----------------------------------------------+ +-----------------------------------------------+
|          Distributed Metadata Layer           | |              File Transfer Engine             |
|             crates/tusher-metadata            | |             crates/tusher-transfer            |
|   - Local SQLite Event Store (WAL mode)       | |   - 2 MB Stream Chunking & Checkpoints        |
|   - Monotonic sync_events causality log       | |   - BLAKE3 & SHA-256 Full-File Verification   |
|   - Fast-Forward & Divergence Reconciliation  | |   - Resumable Interrupted Transfers (.part)   |
|   - Tombstone tracking for clean deletes      | |   - Parallel chunk delivery & flow control    |
+-----------------------------------------------+ +-----------------------------------------------+
                                        \                              /
+-------------------------------------------------------------------------------+
|                         Network & Transport Layer                             |
|                           crates/tusher-network                               |
|   - ConnectionManager: Dynamic route selection, failover & priority restoration|
|   - Transports: Local Area Network (TCP/UDP), Tailscale Mesh (WireGuard)       |
|   - LanDiscovery: UDP multicast/broadcast peer beacons                        |
|   - PairingManager: SAS numeric PIN comparison, mutual cryptographic trust     |
|   - Channel Multiplexer: Dedicated response, manifest, and pong streams        |
+-------------------------------------------------------------------------------+
                                        |
+-------------------------------------------------------------------------------+
|                          Core Protocol & Crypto Layer                         |
|                               crates/tusher-core                              |
|   - Identity: Ed25519 signing/verifying keys & Base32 NodeId (`tshr_...`)     |
|   - Wire Protocol: Length-delimited JSON framing codec (MessageCodec)         |
|   - SAS PIN Generator: RFC-compliant numeric verification from public keys    |
+-------------------------------------------------------------------------------+
```

---

## 3. Protocol & Engine Specifications

### 3.1 Identity & Cryptographic Security
1. **Device Keys**: Every node generates an Ed25519 cryptographic keypair on initial boot.
2. **Node ID**: Deterministic Node IDs are derived directly from the Ed25519 public key using a Base32 representation with a human-readable prefix:
   ```text
   tshr_osovdezp23ugq45mssgn5fqg
   ```
3. **Short Authentication String (SAS) Pairing**:
   - Pairing avoids vulnerable shared passwords.
   - Both nodes exchange public identities and compute a deterministic 6-digit numeric PIN:
     $$\text{PIN} = (\text{SHA256}(\text{Sort}(K_A, K_B)) \bmod 1\,000\,000)$$
   - Users compare the 6 digits on both screens out-of-band. Once confirmed, mutual trust is committed to the local encrypted SQLite state store.

### 3.2 Network Transport & Failover
- **LAN Discovery**: Background UDP beacons broadcast discovery announcements periodically on local subnets.
- **Tailscale Autodetection**: Automatically detects Tailscale peer interfaces (`100.64.0.0/10`) to establish direct peer-to-peer tunnels across NATs without public port forwarding.
- **Priority Failover**: The transport manager constantly measures Round-Trip Time (RTT) via lightweight pings. If high-speed direct LAN drops, it falls back seamlessly to Tailscale; as soon as LAN connectivity is restored, connections automatically migrate back to optimal LAN sockets.
- **Multiplexed Connection Streams**: Each peer connection is isolated into dedicated non-blocking channels (`write_tx`, `request_rx`, `response_rx`, `manifest_rx`, `pong_rx`), guaranteeing ping health checks and chunk acknowledgments never block manifest delta synchronizations.

### 3.3 Chunked & Resumable Transfer
- Files are partitioned into uniform 2 MB chunks (configurable down to 64 KB for tests).
- Chunks carry individual BLAKE3/SHA-256 verification hashes.
- In-progress transfers persist metadata into a `.part` staging manifest. If a connection drops, resumed transfers skip all previously verified chunks, re-transmitting only remaining blocks.
- On completion, the entire assembled file undergoes full cryptographic SHA-256 integrity verification before being atomically placed into the destination directory.

### 3.4 Distributed Metadata & Causal Reconciliation
- Metadata modifications are logged to SQLite in monotonic sequence:
  ```sql
  CREATE TABLE sync_events (
      event_seq INTEGER PRIMARY KEY AUTOINCREMENT,
      folder_id TEXT NOT NULL,
      file_id TEXT NOT NULL,
      relative_path TEXT NOT NULL,
      event_type TEXT NOT NULL, -- UPSERT or DELETE
      version INTEGER NOT NULL,
      size_bytes INTEGER NOT NULL,
      content_hash TEXT NOT NULL,
      origin_node_id TEXT NOT NULL,
      modified_at INTEGER NOT NULL,
      created_at INTEGER NOT NULL
  );
  ```
- **Fast-Forward Reconciliation**: If remote version is strictly greater than local version, the change is applied cleanly.
- **Tombstones**: Deletions are committed as tombstones (`is_deleted = 1`), preventing old versions from resurrecting deleted files.
- **Zero-Data-Loss Conflict Branching**: If concurrent edits occur across partitioned nodes (differing content hash with equivalent versions), the engine avoids silent overwrites. The incoming file is saved as:
  ```text
  filename (Conflict - <origin_node_id> <timestamp>).ext
  ```
  Both versions are preserved, indexed, and replicated to all nodes.

### 3.5 Transitive Multi-Node Mesh ($A \leftrightarrow B \leftrightarrow C$)
In a mesh topology where Node $A$ connects to Node $B$, and Node $B$ connects to Node $C$, but Node $A$ and Node $C$ cannot establish a direct connection:
1. Node $A$ commits a change and notifies Node $B$.
2. Node $B$ pulls the file from Node $A$ and commits it to its local storage.
3. Upon transfer completion, Node $B$ appends the event into its local replication log (preserving Node $A$'s origin metadata) and broadcasts `SyncNotify` downstream to Node $C$.
4. Node $C$ pulls the file from Node $B$.
5. The file and its full metadata arrive on Node $C$ with a 100% SHA-256 hash match.

---

## 4. Verification & Testing

T.U.S.H.E.R includes comprehensive automated tests covering all layers:

### 4.1 Running Workspace Unit & Integration Tests
```powershell
# Set compiler toolchain path if needed
$env:PATH = "C:\Users\Tusher Mondal\llvm-mingw\bin;C:\Users\Tusher Mondal\.cargo\bin;" + $env:PATH

# Execute all 23 tests across the workspace
cargo test --workspace
```

Test coverage includes:
- `tusher_core`: Protocol codecs, Ed25519 identity, SAS PIN determinism.
- `tusher_metadata`: Schema migrations, monotonic sequencing, fast-forward causality, tombstones, conflict branching.
- `tusher_network`: Pairings, handshakes, ping/pong RTT latency, transport discovery and failover.
- `tusher_transfer`: Streaming chunks, SHA-256/BLAKE3 integrity, corruption detection, resumable transfer after interruption.
- `tusher_sync`: Watcher debouncing, echo suppression, 3-node transitive mesh sync ($A \to B \to C$), reverse transitive sync ($C \to B \to A$), and offline conflict resolution across 3 nodes.

### 4.2 Multi-Process End-to-End Simulation
A live multi-process PowerShell integration test launches 3 independent native nodes:
```powershell
powershell.exe -ExecutionPolicy Bypass -File scripts\test_milestone5_mesh_sync.ps1
```

---

## 5. CLI Usage & Interactive Commands

Build the CLI executable:
```bash
cargo build --release -p tusher-cli
```

Launch a node:
```bash
# Node A
./target/release/tusher --name Desktop-A --port 42941 --discovery-port 42841 -d .node_a

# Node B
./target/release/tusher --name Laptop-B --port 42942 --discovery-port 42842 -d .node_b
```

Interactive Commands:
| Command | Description |
| :--- | :--- |
| `peers` | List all discovered and active peer connections with transport type and RTT. |
| `status` | Show local node identity, public key, and active listening ports. |
| `invite` | Generate a pairing invite token and hint URI. |
| `connect <ip:port>` | Connect manually to an endpoint candidate. |
| `pair <id>` | Complete SAS numeric pairing with a target peer. |
| `trust-all` | Trust all currently connected peers (testing/automation mode). |
| `add-folder <id> <name> <path>` | Register and watch a local directory for bidirectional synchronization. |
| `folders` | List all configured shared folders and their local paths. |
| `sync <folder_id>` | Broadcast an immediate sync notification to all connected peers. |
| `index <folder_id>` | Perform a full filesystem scan and update SQLite metadata. |
| `send <peer_id> <file_path>` | Perform a direct point-to-point chunked file transfer. |
| `simulate-lan-drop` | Artificially drop LAN transport to test automatic failover. |
| `quit` | Gracefully terminate the node daemon. |

---

## 6. Implementation Roadmap

- [x] **Milestone 1**: Foundation & Core Network Connectivity (Framing Codec, Ping/Pong, Ed25519 Identity)
- [x] **Milestone 2**: Device Pairing & Mutual Cryptographic Trust (SAS 6-digit PIN, Trust Store)
- [x] **Milestone 3**: Resumable Chunked Transfer Engine (2 MB blocks, SHA-256 / BLAKE3, Checkpointing)
- [x] **Milestone 4**: Distributed Metadata, Causal Synchronization & Native Filesystem Watcher
- [x] **Milestone 5**: Multi-Node Mesh (3+ Nodes) Transitive Replication & Offline Conflict Handling
- [ ] **Milestone 6**: Android Integration (UniFFI / Kotlin JNI Bindings, Storage Access Framework, Foreground Service)
- [ ] **Milestone 7**: Desktop Productization & Native UI (Tauri / Flutter frontend, System Tray, File Explorer Menus)
