# BitTorrent Enhancement Proposal (BEP) Compliance Matrix

This document provides the authoritative compliance and specification conformance reference for the **BitTorrent-rs** library and client ecosystem.

---

## 1. Compliance Summary Table

The library implements and verifies **24 official BitTorrent Enhancement Proposals (BEPs)**:

| BEP | Specification Title | Status | Primary Implementation Modules |
| :---: | :--- | :---: | :--- |
| **[BEP 3](https://www.bittorrent.org/beps/bep_0003.html)** | The BitTorrent Protocol Specification | **Full** | [`metainfo.rs`](file:///c:/Projects/BitTorrent/library/src/core/metainfo.rs), [`peer.rs`](file:///c:/Projects/BitTorrent/library/src/network/peer.rs), [`peer_message.rs`](file:///c:/Projects/BitTorrent/library/src/network/peer_message.rs), [`announcer.rs`](file:///c:/Projects/BitTorrent/library/src/network/announcer.rs) |
| **[BEP 5](https://www.bittorrent.org/beps/bep_0005.html)** | DHT Protocol (Kademlia) | **Full** | [`dht.rs`](file:///c:/Projects/BitTorrent/library/src/network/dht.rs) |
| **[BEP 6](https://www.bittorrent.org/beps/bep_0006.html)** | Fast Extension | **Full** | [`peer_message.rs`](file:///c:/Projects/BitTorrent/library/src/network/peer_message.rs), [`util.rs`](file:///c:/Projects/BitTorrent/library/src/utils/util.rs), [`torrent_context.rs`](file:///c:/Projects/BitTorrent/library/src/core/torrent_context.rs) |
| **[BEP 7](https://www.bittorrent.org/beps/bep_0007.html)** | IPv6 Tracker Extension | **Full** | [`announcer.rs`](file:///c:/Projects/BitTorrent/library/src/network/announcer.rs), [`peer_details.rs`](file:///c:/Projects/BitTorrent/library/src/network/peer_details.rs) |
| **[BEP 9](https://www.bittorrent.org/beps/bep_0009.html)** | Extension for Peers to Send Metadata Files | **Full** | [`metadata_fetcher.rs`](file:///c:/Projects/BitTorrent/library/src/network/metadata_fetcher.rs) |
| **[BEP 10](https://www.bittorrent.org/beps/bep_0010.html)** | Extension Protocol | **Full** | [`peer.rs`](file:///c:/Projects/BitTorrent/library/src/network/peer.rs), [`peer_message.rs`](file:///c:/Projects/BitTorrent/library/src/network/peer_message.rs) |
| **[BEP 11](https://www.bittorrent.org/beps/bep_0011.html)** | Peer Exchange (PEX) | **Full** | [`pex.rs`](file:///c:/Projects/BitTorrent/library/src/network/pex.rs), [`worker.rs`](file:///c:/Projects/BitTorrent/library/src/session/worker.rs) |
| **[BEP 14](https://www.bittorrent.org/beps/bep_0014.html)** | Local Service Discovery (LSD) | **Full** | [`lsd.rs`](file:///c:/Projects/BitTorrent/library/src/network/lsd.rs) |
| **[BEP 15](https://www.bittorrent.org/beps/bep_0015.html)** | UDP Tracker Protocol | **Full** | [`announcer.rs`](file:///c:/Projects/BitTorrent/library/src/network/announcer.rs) |
| **[BEP 16](https://www.bittorrent.org/beps/bep_0016.html)** | Super-Seeding (Initial Seeding) | **Full** | [`torrent_context.rs`](file:///c:/Projects/BitTorrent/library/src/core/torrent_context.rs), [`peer.rs`](file:///c:/Projects/BitTorrent/library/src/network/peer.rs) |
| **[BEP 17](https://www.bittorrent.org/beps/bep_0017.html)** | HTTP Seeding (Hoffman-style) | **Full** | [`webseed.rs`](file:///c:/Projects/BitTorrent/library/src/session/webseed.rs) |
| **[BEP 19](https://www.bittorrent.org/beps/bep_0019.html)** | WebSeed — HTTP/FTP Seeding (GetRight) | **Full** | [`webseed.rs`](file:///c:/Projects/BitTorrent/library/src/session/webseed.rs), [`magnet.rs`](file:///c:/Projects/BitTorrent/library/src/core/magnet.rs) |
| **[BEP 21](https://www.bittorrent.org/beps/bep_0021.html)** | Extension Protocol Dynamic Capability Extension | **Full** | [`peer.rs`](file:///c:/Projects/BitTorrent/library/src/network/peer.rs), [`pex.rs`](file:///c:/Projects/BitTorrent/library/src/network/pex.rs) |
| **[BEP 24](https://www.bittorrent.org/beps/bep_0024.html)** | Tracker Returns External IP | **Full** | [`announcer.rs`](file:///c:/Projects/BitTorrent/library/src/network/announcer.rs) |
| **[BEP 27](https://www.bittorrent.org/beps/bep_0027.html)** | Private Torrents | **Full** | [`metainfo.rs`](file:///c:/Projects/BitTorrent/library/src/core/metainfo.rs), [`session.rs`](file:///c:/Projects/BitTorrent/library/src/session/session.rs) |
| **[BEP 29](https://www.bittorrent.org/beps/bep_0029.html)** | uTorrent Transport Protocol (uTP) | **Full** | [`utp.rs`](file:///c:/Projects/BitTorrent/library/src/network/utp.rs) |
| **[BEP 32](https://www.bittorrent.org/beps/bep_0032.html)** | IPv6 DHT Extension | **Full** | [`dht.rs`](file:///c:/Projects/BitTorrent/library/src/network/dht.rs) |
| **[BEP 38](https://www.bittorrent.org/beps/bep_0038.html)** | Finding Local IPs & WebSeed Mirror Failover | **Full** | [`webseed_tracker.rs`](file:///c:/Projects/BitTorrent/library/src/network/webseed_tracker.rs), [`host.rs`](file:///c:/Projects/BitTorrent/library/src/network/host.rs) |
| **[BEP 47](https://www.bittorrent.org/beps/bep_0047.html)** | Padding Files & Fast Resume | **Full** | [`fast_resume.rs`](file:///c:/Projects/BitTorrent/library/src/storage/fast_resume.rs), [`metainfo.rs`](file:///c:/Projects/BitTorrent/library/src/core/metainfo.rs) |
| **[BEP 48](https://www.bittorrent.org/beps/bep_0048.html)** | Tracker Scrape Convention | **Full** | [`announcer.rs`](file:///c:/Projects/BitTorrent/library/src/network/announcer.rs) |
| **[BEP 51](https://www.bittorrent.org/beps/bep_0051.html)** | DHT Infohash Scrape | **Full** | [`dht.rs`](file:///c:/Projects/BitTorrent/library/src/network/dht.rs) |
| **[BEP 52](https://www.bittorrent.org/beps/bep_0052.html)** | BitTorrent v2 (Merkle Trees & Hybrid) | **Full** | [`merkle.rs`](file:///c:/Projects/BitTorrent/library/src/core/merkle.rs), [`piece_layers_cache.rs`](file:///c:/Projects/BitTorrent/library/src/storage/piece_layers_cache.rs), [`torrent_builder.rs`](file:///c:/Projects/BitTorrent/library/src/core/torrent_builder.rs), [`torrent_upgrader.rs`](file:///c:/Projects/BitTorrent/library/src/core/torrent_upgrader.rs) |
| **[BEP 53](https://www.bittorrent.org/beps/bep_0053.html)** | Magnet URI File Selection (`so=`) | **Full** | [`magnet.rs`](file:///c:/Projects/BitTorrent/library/src/core/magnet.rs), [`metadata_fetcher.rs`](file:///c:/Projects/BitTorrent/library/src/network/metadata_fetcher.rs) |
| **[BEP 55](https://www.bittorrent.org/beps/bep_0055.html)** | Holepunch Extension (`ut_holepunch`) | **Full** | [`holepunch.rs`](file:///c:/Projects/BitTorrent/library/src/network/holepunch.rs), [`worker.rs`](file:///c:/Projects/BitTorrent/library/src/session/worker.rs) |

---

## 2. Detailed Conformance Breakdown

### BEP 52: BitTorrent v2 & Hybrid Swarms
- **Cryptographic Merkle Trees**: Computes canonical binary Merkle trees using SHA-256 over 16 KiB blocks (`BLOCK_SIZE = 16384`) with zero-padding to powers of two.
- **On-the-Fly Block Proofs**: Generates and verifies logarithmic audit paths (`MerkleProof`) before assembling incoming blocks into disk storage, rejecting corrupted blocks instantly.
- **`piece layers` Dictionary**: Extracts and caches piece layer hashes in `.layers` persistent disk files (`PieceLayersDiskCache`).
- **Dual Info-Hashes**: Calculates both 20-byte SHA-1 info-hashes (for v1 backward compatibility) and 32-byte SHA-256 info-hashes.
- **Pad Files**: Automatically aligns piece boundaries in hybrid torrents using `.pad/<length>` file entries.
- **Wire Hash Transfer**: Implements `HashRequest` (ID 21), `Hashes` (ID 22 with ancestor uncle proofs), and `HashReject` (ID 23).

### BEP 29: Micro Transport Protocol (uTP)
- **Framing & State Machine**: Implements `ST_DATA`, `ST_FIN`, `ST_STATE`, `ST_RESET`, and `ST_SYN` packets with standard 20-byte headers.
- **RFC 6817 LEDBAT**: Congestion window (`cwnd`) scaling calculated against a 100ms queuing delay target using a sliding 4-minute delay baseline (`DelayTracker`).
- **SACK Extension**: Parses chained extension headers with selective acknowledgment bitmasks (bit 0 = `ack_nr + 2`).
- **Out-of-Order Reassembly**: Bounded reassembly buffer (`BTreeMap<u16, Vec<u8>>` capped at 128 packets) with sequential user buffer stitching.
- **Fast Retransmission & RTO**: Proactive circular sequence gap detection (`detect_receive_gaps`) and microsecond RFC 6298 RTT/RTO exponential backoff.

### BEP 16 & BEP 6: Super-Seeding & Fast Extension
- **Bitfield Concealment**: Super-seeder presents as an incomplete peer with an empty bitfield upon handshake.
- **Piece Distribution Control**: Distributes rarest unseeded pieces via `Have` announcements, tracking propagation across the swarm before granting additional pieces.
- **Synergy with BEP 6**: Issues `Suggest Piece` and `Allowed Fast` grants alongside `Have` messages to Fast Extension peers, enabling immediate download without waiting for unchoke cycles.
- **Auto-Exit**: Reverts to standard seeding if another complete seed appears in the swarm.

### BEP 11: Peer Exchange (PEX)
- **Dual-Stack IPv4 / IPv6**: Compact 6-byte IPv4 (`added`/`dropped`) and 18-byte IPv6 (`added6`/`dropped6`) records.
- **Flag Attributes**: `added.f` and `added6.f` bit flags (`PREF_ENCRYPT`, `SEED`, `UTP`, `HOLEPUNCH`, `OUTGOING`).
- **Differential Swarm Generation**: Capped at 50 peers per announcement with circular dropped-peer history.
- **Private Torrent Guard**: Strict suppression when `is_private` is set (BEP 27).

### BEP 55: Holepunch Extension (`ut_holepunch`)
- **Binary Wire Protocol**: Encodes and decodes 12-byte IPv4 and 24-byte IPv6 packets (`Rendezvous = 0x00`, `Connect = 0x01`, `Error = 0x02`).
- **Relay Coordination**: Intermediate peers validate connected targets, reject self-rendezvous (`NoSelf`), and dispatch reciprocal `Connect` commands to both peers.

### BEP 53: Magnet URI File Selection (`so=`)
- **Query Parameter Parsing**: Full support for comma-separated indices and compact ranges (e.g., `so=0,2,4,6-8`).
- **Canonical Serialization**: Compresses contiguous ranges into compact hyphenated notation (`start-end`).
- **Expedited Manifest Resolution**: Scans initial metadata blocks (`PartialMetadataManifest`) to evaluate piece and bandwidth savings before complete metadata arrival.
