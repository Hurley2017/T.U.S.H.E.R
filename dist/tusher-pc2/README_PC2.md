# T.U.S.H.E.R - Secondary PC Setup Guide

This folder contains a fully self-contained, standalone build of the **T.U.S.H.E.R** mesh node for Windows.

**No developer tools, Rust, Java, or Python are required on this PC!**

---

## 1. Quick Start

1. Copy the entire `tusher-pc2` folder to your secondary PC (via USB drive, local network share, or download).
2. Double-click **`run_pc2.bat`**.
3. A terminal window will open and automatically start the node:
   - Node Name: `TusherPC-2`
   - TCP Sync Port: `42426`
   - Web Dashboard: `http://127.0.0.1:42951`
4. Your default web browser will automatically open to the dashboard.

---

## 2. Joining the Mesh

### Option A: Automatic Local Discovery (Recommended)
If this secondary PC is connected to the **same home Wi-Fi / LAN router** as your primary PC, both nodes will automatically discover each other via UDP beacons within ~5 seconds and appear in the **Mesh Peers** list!

### Option B: Tailscale Mesh
If this secondary PC has Tailscale installed:
1. Note the secondary PC's Tailscale IP (e.g. `100.x.y.z`).
2. On the primary PC's dashboard (`http://127.0.0.1:42950`), under **Connect / Pair Remote Node**, enter:
   `100.x.y.z:42426`
3. Click **Connect**!

### Option C: Mobile Tablet Pairing
1. On your tablet app, tap **"🔗 PAIR WITH DESKTOP"**.
2. Enter the secondary PC's IP and port `42951`, or scan its QR code from `http://127.0.0.1:42951`.
3. Now all 3 devices (**Primary PC <-> Tablet <-> Secondary PC**) form a full decentralized personal mesh!