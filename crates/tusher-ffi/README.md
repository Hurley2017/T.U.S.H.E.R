# `tusher-ffi`

**UniFFI Foreign Function Interface & Kotlin JNI Bindings for T.U.S.H.E.R**

---

## 1. Overview

`tusher-ffi` exposes the asynchronous Tokio-driven Rust mesh core to Android (Kotlin / Java) and other mobile/native platforms using **Mozilla UniFFI**:
- **`cdylib` / `staticlib` / `rlib` targets**: Can be compiled into shared native libraries (`.so` for Android, `.dylib` for iOS/macOS, `.dll` for Windows).
- **Zero Manual JNI Boilerplate**: Auto-generates safe, memory-managed Kotlin bindings with proper type safety and automatic exception mapping.
- **Embedded Tokio Runtime**: Houses a multi-threaded Tokio runtime inside a persistent `TusherNode` object instance, allowing mobile applications to run asynchronous P2P networking without blocking the Android main UI thread.
- **Bi-directional Event Callbacks**: `FfiEventListener` interface allows Rust to dispatch peer updates, transfer completions, and sync events directly into Android ViewModels and notifications.

---

## 2. Exported API Reference

### 2.1 Structs & Records
- `FfiNodeStatus`: Local device identification, active listen ports, connected peer count, and storage path.
- `FfiPeerStatus`: Discovered peer information, latency in milliseconds, active transport (`Lan` / `Tailscale`), and endpoint candidates.
- `FfiSharedFolder`: Configured shared directory, folder ID, and total indexed file count.
- `FfiTransferStats`: Chunks transferred, chunks skipped via resume, total bytes, and elapsed time.
- `FfiPairingInvite`: Out-of-band pairing token and `tusher://pair?...` URI for QR code sharing.

### 2.2 Methods on `TusherNode`
| Method | Description |
| :--- | :--- |
| `TusherNode(dataDir, nodeName, port, discoveryPort)` | **Constructor**: Launches the embedded multi-thread Tokio runtime and daemon. |
| `getStatus()` | Retrieves node status, listen port, and active peer counts. |
| `getPeers()` | Lists all discovered and active peers in the mesh. |
| `createPairingInvite(hintAddr)` | Generates pairing token and QR code URI. |
| `connectPeer(addrStr)` | Manually connects to an IP:Port candidate. |
| `pairPeer(peerId)` | Marks a peer as paired and trusted via SAS numeric PIN. |
| `trustAllPeers()` | Trusts all currently connected peers (automation mode). |
| `addSharedFolder(folderId, name, localPath)` | Registers and watches a folder for automated P2P sync. |
| `listSharedFolders()` | Lists all configured shared folders. |
| `triggerSync(folderId)` | Broadcasts an immediate sync notification to trigger delta reconciliation. |
| `scanAndIndexFolder(folderId)` | Forces an initial scan of local disk files. |
| `sendFile(peerId, filePath)` | Initiates a direct chunked file transfer. |
| `registerEventListener(listener)` | Subscribes an `FfiEventListener` callback implementation. |
| `simulateLanDrop(disabled)` | Simulates LAN connection severed for testing failover. |
| `close()` | Gracefully stops the node daemon and frees native resources. |

---

## 3. Building for Android

### 3.1 Toolchain Prerequisites
Install the Android NDK and Rust cross-compilation targets:
```bash
rustup target add aarch64-linux-android
rustup target add armv7-linux-androideabi
rustup target add x86_64-linux-android
rustup target add i686-linux-android
cargo install cargo-ndk
```

### 3.2 Compiling Native Libraries (`.so`)
```bash
cargo ndk -t aarch64-linux-android -t x86_64-linux-android -o android/tusher-sdk/src/main/jniLibs build --release -p tusher-ffi
```

### 3.3 Generating Kotlin Bindings
```bash
cargo run -p tusher-ffi --bin uniffi-bindgen -- generate \
    --library target/x86_64-pc-windows-gnullvm/debug/tusher_ffi.dll \
    --language kotlin \
    --out-dir android/tusher-sdk/src/main/java
```

---

## 4. Android Integration Example

```kotlin
// In an Android Foreground Service or ViewModel
val dataDir = context.filesDir.resolve("tusher_mesh").absolutePath
val node = TusherNode(dataDir, "Pixel-Phone", 42931u, 42831u)

// Register for real-time mesh events
node.registerEventListener(object : FfiEventListener {
    override fun onPeerStatusChanged(peerId: String, peerName: String, isConnected: Boolean) {
        Log.i("TUSHER", "Peer $peerName status changed: connected=$isConnected")
    }

    override fun onSyncEvent(folderId: String, relativePath: String, eventType: String) {
        Log.i("TUSHER", "File updated: $relativePath ($eventType)")
    }

    override fun onTransferCompleted(folderId: String, fileName: String, fileSize: ULong, contentHash: String) {
        Log.i("TUSHER", "Transfer complete: $fileName")
    }
})

// Add a synchronized folder
val syncFolder = context.filesDir.resolve("shared_vault").absolutePath
node.addSharedFolder("mesh_vault", "MeshVault", syncFolder)
```
