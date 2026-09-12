# `tusher-transfer`

**Resumable, Chunked File Transfer & Content Integrity Engine for T.U.S.H.E.R**

---

## 1. Overview

`tusher-transfer` implements point-to-point streaming file transmission. It guarantees:
- **Streaming 2 MB Chunking**: Files are broken into fixed 2 MB chunks (configurable down to 64 KB).
- **Dual-Layer Hash Integrity**: Every individual chunk carries a verification hash; the assembled file must match the expected SHA-256 before disk placement.
- **Resumable Transfers**: Interrupted transfers save checkpoints in staging manifests (`.part`), allowing transfers to resume without re-downloading existing chunks.
- **Atomic File Placement**: Partially received files remain in `.staging` until 100% verified, preventing corrupted or incomplete files from corrupting shared folders.

---

## 2. Core Modules

### 2.1 File Sender (`src/sender.rs`)
- `FileSender`:
  - Computes source file length and overall SHA-256.
  - Sends `TransferInit` with transfer metadata and awaits `TransferInitAck`.
  - Reads `existing_chunks` from receiver to skip previously completed chunks during resumes.
  - Sequentially streams `TransferChunk` frames and awaits `TransferChunkAck`.
  - Dispatches final `TransferComplete` and awaits receiver seal.

### 2.2 File Receiver (`src/receiver.rs`)
- `FileReceiver`:
  - `handle_init`: Pre-allocates target file space in `.staging/<transfer_id>.part` and checks for previously saved chunks.
  - `handle_chunk`: Verifies chunk hash, writes directly at specified byte offset, checkpoints completed chunk set, and replies with `TransferChunkAck`.
  - `handle_complete`: Computes full SHA-256 over `.staging/<transfer_id>.part`, verifies 100% match against expected hash, and atomically renames the completed file to the destination path.
  - Fires `TransferCompletedInfo` over an internal channel to trigger coordinator notifications.

### 2.3 Transfer State Persistence (`src/state.rs`)
- `TransferState`: JSON manifest stored alongside the partial file:
  ```json
  {
    "transfer_id": "xfer_1a79aca1a28ec54e",
    "folder_id": "mesh_vault",
    "file_name": "shared_doc.txt",
    "file_size": 45,
    "content_hash": "1a79aca1a28ec54e...",
    "chunk_size": 2097152,
    "total_chunks": 1,
    "completed_chunks": [0]
  }
  ```

### 2.4 Cryptographic Hashing (`src/hash.rs`)
- `hash_bytes`: BLAKE3/SHA-256 byte hashing.
- `hash_file`: Asynchronous chunked file stream hashing using `tokio::io`.
