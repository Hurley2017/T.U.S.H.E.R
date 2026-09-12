# `tusher-desktop`

**`tusher-desktop`** productizes the decentralized **T.U.S.H.E.R** mesh into an intuitive, native desktop experience for Windows, macOS, and Linux. It combines a zero-privilege Windows Explorer shell extension, an asynchronous native background system tray icon, and an embedded responsive dark-mode Web Dashboard with live REST controls.

---

## 1. Architectural Architecture

```
+-------------------------------------------------------------------------+
|                              Desktop User                               |
|          +--------------------------+    +--------------------------+   |
|          | Windows Explorer Context |    |   Native System Tray     |   |
|          |  "Share via T.U.S.H.E.R" |    |   (Win32 NotifyIconW)    |   |
|          +------------+-------------+    +------------+-------------+   |
+-----------------------|-------------------------------|-----------------+
                        |                               |
                        +---------------+---------------+
                                        |
+---------------------------------------v---------------------------------+
|                       tusher-desktop Application Layer                  |
|                                                                         |
|   +-----------------------------------------------------------------+   |
|   |                  Embedded Web Server (Axum)                     |   |
|   |  - Serves Single-Page Glassmorphic Web Dashboard (HTML5/CSS/JS) |   |
|   |  - Exposes Type-Safe REST API for Status, Peers, Folders, Files  |   |
|   |  - Binds to http://127.0.0.1:42950                              |   |
|   +---------------------------------+-------------------------------+   |
|                                     |                                   |
|   +---------------------------------v-------------------------------+   |
|   |                    Registry / Shell Manager                     |   |
|   |  - HKCU\Software\Classes\*\shell\TusherShare                    |   |
|   |  - HKCU\Software\Classes\Directory\shell\TusherShare            |   |
|   |  - Zero Admin Rights Required                                   |   |
|   +-----------------------------------------------------------------+   |
+-------------------------------------------------------------------------+
                                      |
+-------------------------------------v-----------------------------------+
|                     Underlying Rust Engine Crates                       |
|   tusher-core  |  tusher-network  |  tusher-transfer  |  tusher-sync    |
+-------------------------------------------------------------------------+
```

---

## 2. Key Modules & Components

### 2.1 Native System Tray (`src/tray.rs`)
- **Win32 Message Window**: Initializes a dedicated background thread running a native Win32 window message loop (`GetMessageW` / `DispatchMessageW`).
- **Notification Icon (`Shell_NotifyIconW`)**: Registers `NOTIFYICONDATAW` with custom tooltip (`"T.U.S.H.E.R - Decentralized Sync Mesh"`), callback message `WM_APP + 100`, and icon.
- **Interactive Popup Menu (`TrackPopupMenu`)**:
  - `T.U.S.H.E.R (Active Mesh Node)` [Disabled Status Header]
  - `🌐 Open Web Dashboard` &rarr; Launches `http://127.0.0.1:42950` via default browser
  - `📂 Open Downloads Folder` &rarr; Opens local downloads directory in File Explorer
  - `⏸ Pause / Resume Sync` &rarr; Instant atomic toggle of file synchronization
  - `⚙ Explorer Context Menu (Toggle)` &rarr; One-click install/uninstall of shell menu
  - `❌ Exit T.U.S.H.E.R` &rarr; Graceful teardown of network sockets and tray icon
- **Double Click Action**: Double-clicking the tray icon automatically opens the Web Dashboard.

### 2.2 Windows Explorer Shell Extension (`src/shell.rs`)
- **Zero-Admin Rights**: Integrates under `HKEY_CURRENT_USER\Software\Classes`, ensuring any user can install or remove the context menu without UAC elevation or administrator privileges.
- **Context Targets**:
  - `HKCU\Software\Classes\*\shell\TusherShare` (all files)
  - `HKCU\Software\Classes\Directory\shell\TusherShare` (all directories)
- **Execution Command**: Invokes `"tusher-desktop.exe" share "%1"`.
- **API Methods**:
  - `install_context_menu(exe_path: Option<&Path>) -> Result<()>`
  - `uninstall_context_menu() -> Result<()>`
  - `is_context_menu_installed() -> bool`

### 2.3 Embedded Web Dashboard & REST Control Plane (`src/web.rs`)
- **Self-Contained Single-Page Application**: Bundles HTML5/CSS/JavaScript directly inside the binary via `include_str!("../assets/dashboard.html")`, requiring zero external runtime or Node.js dependencies.
- **Glassmorphic Dark UI**:
  - **Live Mesh Topology**: Real-time peer cards displaying node names, public key previews, IP addresses, transport types (`TCP LAN`, `QUIC WAN`, `Relay`), round-trip latency (`ms`), and pairing status.
  - **Shared Folders**: View registered sync folders, file counts, and trigger manual scans or syncs.
  - **Instant Share**: Direct point-to-point chunked file transfers to any selected peer.
  - **Conflict Resolution Center**: Inspects divergent branches (`(Conflict - <node_id> <timestamp>)`) with zero data loss.
- **REST Endpoints**:
  | Method | Endpoint | Description |
  |--------|----------|-------------|
  | `GET` | `/` | Web Dashboard Single-Page App |
  | `GET` | `/api/status` | Local node ID, ports, peer count, sync state |
  | `GET` | `/api/peers` | Discovered and connected peer list |
  | `POST` | `/api/peers/connect` | Connect to remote peer by `IP:Port` |
  | `POST` | `/api/peers/pair` | Pair peer using 6-digit numeric SAS PIN |
  | `GET` | `/api/folders` | List shared folders and active file counts |
  | `POST` | `/api/folders/add` | Register new shared folder |
  | `POST` | `/api/folders/scan` | Re-index local folder metadata |
  | `POST` | `/api/folders/sync` | Broadcast immediate sync notification |
  | `POST` | `/api/transfers/send` | Send point-to-point file to peer |
  | `POST` | `/api/toggle-sync` | Toggle global pause/resume sync state |
  | `GET` | `/api/conflicts` | List unresolved conflict branches |
  | `GET` | `/api/shell/context-menu`| Query context menu installation status |
  | `POST` | `/api/shell/context-menu`| Install (`true`) or uninstall (`false`) menu |

---

## 3. CLI Usage

```bash
# Run desktop daemon with System Tray and Web Dashboard (default)
tusher-desktop

# Run with custom ports and automatically launch browser
tusher-desktop --port 42424 --web-port 42950 --open

# Run in headless mode (no system tray)
tusher-desktop --no-tray

# Manage Windows Explorer shell context menu
tusher-desktop shell install
tusher-desktop shell status
tusher-desktop shell uninstall

# Share a file or folder directly (invoked by Windows Explorer right-click)
tusher-desktop share "C:\Users\Docs\project_report.pdf"
tusher-desktop share "D:\SharedVault"
```
