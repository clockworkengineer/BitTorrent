# WebTorrent & WebRTC Bridging Guide

This document describes the **WebTorrent and WebRTC Data Channel** peer transport and cross-swarm bridging architecture in **BitTorrent-rs**.

---

## 1. Overview

WebTorrent enables web browser clients (running JavaScript in Chrome, Firefox, Safari, or Edge) to participate in peer-to-peer torrent swarms without requiring browser extensions or plugins. However, browsers cannot open raw TCP or UDP sockets due to sandbox security restrictions; instead, they rely on **WebRTC Data Channels** for peer connections and **WebSocket trackers (`wss://` and `ws://`)** for signaling.

Standard BitTorrent desktop clients (qBittorrent, Transmission) only support TCP and uTP, isolating browser swarms from standard BitTorrent swarms.

**BitTorrent-rs bridges this divide**: It natively speaks both standard BitTorrent (TCP / uTP / UDP trackers) and WebTorrent (WebRTC Data Channels / WebSocket trackers), acting as a **high-throughput cross-swarm bridge** between browsers and the global BitTorrent network.

```
┌──────────────────────────────┐              ┌──────────────────────────────┐
│     Web Browser Clients      │              │    Standard Desktop Peers    │
│ (WebTorrent / Brave Browser) │              │    (qBittorrent / Deluge)    │
└──────────────────────────────┘              └──────────────────────────────┘
               ▲                                             ▲
               │ WebRTC Data Channel                         │ TCP / uTP
               ▼                                             ▼
┌────────────────────────────────────────────────────────────────────────────┐
│                       BitTorrent-rs Client / Daemon                        │
│                                                                            │
│  [WebTorrentBridge Coordinator] ◄──────────────► [Standard Swarm Engine]   │
│   ├── WebSocket Announcer (wss://)                ├── UDP Tracker (BEP 15) │
│   ├── WebRtcSocketAdapter (AsyncSocket)           ├── TCP Socket           │
│   └── Cross-Swarm Piece Cache & Have Broadcasts   └── uTP Socket (BEP 29)  │
└────────────────────────────────────────────────────────────────────────────┘
```

---

## 2. WebRTC Data Channel Socket Adapter (`WebRtcSocketAdapter`)

Implemented in [`library/src/network/webrtc.rs`](file:///c:/Projects/BitTorrent/library/src/network/webrtc.rs):
- **`AsyncSocket` Conformance**: Implements `AsyncSocket` (reading and writing bytes asynchronously) over message-oriented WebRTC binary channels (standard `"webtorrent"` label).
- **Framing & Demuxing**: Adapts packetized binary frames into continuous byte streams for the peer wire decoder.
- **Channel State Machine**: Manages states (`Connecting`, `Open`, `Closing`, `Closed`) and tracks metrics (`bytes_sent`, `bytes_received`, `messages_sent`, `messages_received`).
- **Loopback Testing**: Provides `WebRtcSocketAdapter::create_pair()` for deterministic unit and integration testing without network interfaces.

---

## 3. WebSocket Tracker Signaling Protocol

Standard HTTP/UDP tracker announcements cannot reach web browsers. WebTorrent utilizes WebSockets for signaling:

### 3.1 JSON Announce Request
The client opens a WebSocket connection to `wss://tracker.openwebtorrent.com` (or other trackers) and sends JSON announce payloads containing SDP offers:

```json
{
  "action": "announce",
  "info_hash": "...",
  "peer_id": "-RS0100-...",
  "uploaded": 0,
  "downloaded": 0,
  "left": 1048576,
  "numwant": 25,
  "offers": [
    {
      "offer_id": "...",
      "offer": { "type": "offer", "sdp": "v=0\r\no=..." }
    }
  ]
}
```

### 3.2 Inbound Answer & Offer Demuxing
The `WebTorrentAnnouncer` parses incoming frames into `WebTorrentIncomingMessage`:
- **`PeerAnnouncement`**: Swarm peer contacts and interval updates.
- **`InboundOffer`**: WebRTC offer from a browser peer wanting to connect. The client automatically generates an SDP answer and returns it through the tracker.
- **`InboundAnswer`**: SDP answer from a remote peer accepting the client's offer, triggering WebRTC channel binding.

---

## 4. Cross-Swarm Bridge Coordinator (`WebTorrentBridge`)

The `WebTorrentBridge` seamlessly routes data across transport boundaries:
- **Download from TCP ➔ Serve to WebRTC**: Pieces downloaded and verified from standard TCP/uTP swarms are recorded in the local bitfield. When connected to browser peers over WebRTC, the client issues `Have` broadcasts and serves pieces using standard peer wire `Piece` messages.
- **Download from WebRTC ➔ Seed to TCP**: Rare pieces downloaded from browser seeds are validated, written to disk, and immediately advertised to standard TCP peers.
- **Transport Metrics**: Tracks `tcp_to_webrtc_bytes` and `webrtc_to_tcp_bytes` to measure bridging throughput.

---

## 5. Magnet URI Auto-Injection (`WebTorrentMagnetInjector`)

Implemented in [`library/src/network/webtorrent_injector.rs`](file:///c:/Projects/BitTorrent/library/src/network/webtorrent_injector.rs):
- **WebSeed & Tracker Extraction**: Extracts BEP 19 WebSeeds (`ws=`), exact sources (`xs=`), and WebSocket trackers (`wss://`/`ws://`) from incoming Magnet links.
- **Automated Signaling Injection**: Injects public WebTorrent trackers (`wss://tracker.openwebtorrent.com`, `wss://tracker.btorrent.xyz`, `wss://tracker.webtorrent.dev`) into standard torrent sessions.
- **Pre-Generated SDP Offers**: Automatically pre-generates WebRTC SDP offers on startup for instant browser peer handshakes without delay.

---

## 6. Transport Badges in GUI and Web UI

Both the Desktop client and Web UI display live transport indicators in the **"👥 Peers"** list:

| Badge | Transport | Notes |
| :---: | :--- | :--- |
| **`WebRTC`** | WebRTC Data Channel | Browser client (WebTorrent / Brave) |
| **`uTP`** | Micro Transport Protocol | Low-latency background UDP peer (BEP 29) |
| **`TCP`** | Standard TCP Socket | Traditional BitTorrent peer wire connection |

---

## 7. Configuration via `TorrentSessionBuilder`

```rust
use bittorrent_rs::TorrentSessionBuilder;

let session = TorrentSessionBuilder::new("movie.torrent", "./downloads")
    .enable_webtorrent(true)          // Enable WebSocket trackers and WebRTC
    .webtorrent_bridge_mode(true)     // Enable cross-swarm piece bridging
    .webtorrent_max_web_peers(25)     // Max concurrent browser peers
    .build()?;
```
