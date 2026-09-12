# `tusher-cli`

**Interactive Node Daemon, Control CLI, and Testing Tools for T.U.S.H.E.R**

---

## 1. Overview

`tusher-cli` is the command-line interface and background daemon executable for T.U.S.H.E.R nodes on desktop and server environments. It wires together:
- `tusher-core`
- `tusher-network`
- `tusher-transfer`
- `tusher-metadata`
- `tusher-sync`

---

## 2. Command-Line Options

```text
Usage: tusher [OPTIONS]

Options:
  -n, --name <NAME>                     Human-readable node name [default: hostname]
  -p, --port <PORT>                     TCP listen port for peer connections [default: 42931]
      --discovery-port <PORT>           UDP port for LAN discovery beacons [default: 42831]
  -d, --data-dir <DIR>                  Directory for identity, database, and staging [default: .tusher_data]
      --non-interactive                 Run as a background daemon without interactive stdin
  -h, --help                            Print help
  -V, --version                         Print version
```

---

## 3. Interactive Shell Commands

When running interactively, `tusher-cli` provides an interactive REPL:

```text
============================================================
       T.U.S.H.E.R - Decentralized File & Media Mesh        
============================================================
 Node Name:       Desktop-A
 Node ID:         tshr_osovdezp23ugq45mssgn5fqg
 Platform:        windows
 Public Key:      78c7720275be7f61...
 TCP Listen Port: 42941
 UDP Discovery:   42841
 Storage Dir:     .node_a
 Downloads Dir:   .node_a\downloads
 Metadata DB:     .node_a\tusher_metadata.db
============================================================
```

| Command | Arguments | Description |
| :--- | :--- | :--- |
| `peers` | | Display all active and discovered peer nodes with latency and transport type. |
| `status` | | Display local node identification, listen ports, and database paths. |
| `invite` | | Generate a pairing invitation token and hint URI. |
| `connect` | `<ip:port>` | Connect manually to an endpoint candidate. |
| `pair` | `<peer_id>` | Initiate Short Authentication String (SAS) numeric PIN pairing. |
| `trust-all` | | Mark all currently connected peers as paired & trusted (for automation). |
| `folders` | | List all currently configured shared folders and their local paths. |
| `add-folder` | `<id> <name> <path>` | Register and watch a local directory for automatic mesh synchronization. |
| `sync` | `<folder_id>` | Manually trigger and broadcast a synchronization notification. |
| `index` | `<folder_id>` | Scan local folder on disk and synchronize database metadata. |
| `send` | `<peer_id> <file_path>` | Stream a point-to-point chunked file transfer to a peer. |
| `simulate-lan-drop` | | Artificially sever LAN sockets to verify automatic failover. |
| `quit` | | Gracefully shut down the node daemon. |
