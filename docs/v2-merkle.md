# BitTorrent v2 & Merkle Hash Trees (BEP 52)

This document provides a technical deep dive into the **BitTorrent v2 (BEP 52)** specification implemented in `bittorrent-rs`. It covers canonical SHA-256 Merkle hash trees, on-the-fly block-level verification, persistent piece layer disk caching, dual-stack Hybrid swarms, wire hash transfer protocols, and legacy `.torrent` upgrading.

---

## 1. Why BitTorrent v2?

Legacy BitTorrent v1 uses a single, flat SHA-1 hash per piece (typically 512 KiB to 16 MiB). If any 16 KiB block within that piece is corrupted or poisoned by a malicious peer, the entire piece fails validation, forcing the client to discard the entire piece and re-download all component blocks. Furthermore, SHA-1 is cryptographically broken, and files sharing identical content across different torrents cannot be cross-seeded due to differing piece alignment.

### Advantages of BEP 52 (BitTorrent v2)

| Feature | BitTorrent v1 | BitTorrent v2 (BEP 52) |
| :--- | :--- | :--- |
| **Hash Algorithm** | SHA-1 (20 bytes, cryptographically weak) | SHA-256 (32 bytes, cryptographically secure) |
| **Verification Granularity** | Entire piece (e.g. 1 MiB–16 MiB) | Individual 16 KiB block (`BLOCK_SIZE = 16384`) |
| **Poisoning Defense** | Must download whole piece before discovering corruption | Immediate rejection of corrupt 16 KiB blocks |
| **Metadata File Structure** | Flat `files` list with global concatenated offsets | Hierarchical `file tree` dictionary with per-file Merkle roots |
| **Deduplication** | None (files tied to piece boundaries) | Perfect content deduplication across swarms based on file root |
| **Piece Alignment** | Arbitrary byte offsets across files | Piece-aligned file boundaries with padding |

---

## 2. Canonical SHA-256 Merkle Hash Trees

The core cryptographic engine is implemented in [`library/src/core/merkle.rs`](file:///c:/Projects/BitTorrent/library/src/core/merkle.rs).

### 2.1 Leaf and Internal Node Hashing
1. **Leaf Block Size**: Files are partitioned into 16 KiB leaf blocks ($16384\text{ bytes}$).
2. **Leaf Node Hash**:
   $$\text{leaf\_hash} = \text{SHA-256}(\text{block\_data})$$
3. **Power-of-Two Padding**: If the number of leaf blocks is not a power of two, the leaf layer is zero-padded to the next power of two using $[0u8; 32]$.
4. **Internal Node Hash**: Pairwise parent hashes are computed canonically as:
   $$\text{parent} = \text{SHA-256}(\text{left\_child} \mathbin{\Vert} \text{right\_child})$$
5. **Root Hash (`pieces_root`)**: The single 32-byte hash at the apex of the tree serves as the file's canonical identifier.

```
                   Root Hash (pieces_root)
                         [32 bytes]
                        /          \
               Node A                  Node B
              /      \                /      \
          Leaf 0     Leaf 1       Leaf 2     [Pad: 0x00...00]
         (16 KiB)   (16 KiB)     (16 KiB)      (32 bytes)
```

### 2.2 Constant-Memory Streaming Hashing
To hash multi-gigabyte files without exhausting system RAM:
- `MerkleTree::from_reader<R: Read>(reader: R)` processes 16 KiB chunks incrementally, maintaining only the active vertical stack of ancestor hashes in memory.
- `MerkleTree::from_file<P: AsRef<Path>>(path: P)` allows high-throughput hashing directly from disk files.

---

## 3. Logarithmic Audit Proofs (`MerkleProof`)

Rather than waiting for a complete piece to finish downloading, incoming 16 KiB blocks are verified **immediately on arrival** using minimal logarithmic audit paths:

```rust
pub struct MerkleProof {
    pub leaf_index: usize,
    pub sibling_hashes: Vec<[u8; 32]>,
}
```

### 3.1 Verification Algorithm
Given an incoming 16 KiB block, its index `i`, and the sibling proof hashes:
1. Compute $\text{current} = \text{SHA-256}(\text{block})$.
2. For each sibling hash in `proof.sibling_hashes`:
   - If index bit is `0`: $\text{current} = \text{SHA-256}(\text{current} \mathbin{\Vert} \text{sibling})$.
   - If index bit is `1`: $\text{current} = \text{SHA-256}(\text{sibling} \mathbin{\Vert} \text{current})$.
   - Shift index right: $\text{index} = \text{index} \gg 1$.
3. Check: $\text{current} == \text{pieces\_root}$.
4. If the check fails, the block is discarded immediately, and the peer's bad block score is incremented. Corrupt data never touches disk storage.

---

## 4. `piece layers` & Two-Tier Disk Cache

For files larger than the piece length, BEP 52 specifies that the `.torrent` file contains a top-level `piece layers` dictionary mapping the file's `pieces_root` to the concatenated 32-byte hashes of the layer corresponding to the piece size.

The library implements persistent caching in [`library/src/storage/piece_layers_cache.rs`](file:///c:/Projects/BitTorrent/library/src/storage/piece_layers_cache.rs):

### 4.1 Binary `.layers` File Format
```
+---------------+---------------+---------------------------------------+
|  Magic "BT2L" |  Version (1)  |              Layer Count              |
|   (4 bytes)   |   (2 bytes)   |               (4 bytes)               |
+---------------+---------------+---------------------------------------+
|                       File Size (u64, 8 bytes)                        |
+-----------------------------------------------------------------------+
|                       Piece Length (u32, 4 bytes)                     |
+-----------------------------------------------------------------------+
|         Concatenated 32-byte SHA-256 Layer Hashes (32 * N bytes)      |
+-----------------------------------------------------------------------+
|               Trailing SHA-256 File Checksum (32 bytes)               |
+-----------------------------------------------------------------------+
```

### 4.2 Two-Tier LRU Architecture
- **Memory Tier**: High-speed LRU memory cache (`MemoryTierCache`) bounded by `v2_piece_layers_cache_max_mb`.
- **Storage Tier**: Scans and saves `.layers` files in the cache directory, pruning oldest entries upon exceeding capacity limits.
- **On-the-Fly Tree Reconstruction**: `reconstruct_tree_from_piece_layer` rebuilds full upper Merkle trees from single layer hashes in constant time.

---

## 5. Dual-Stack Hybrid Torrents (v1 + v2)

To maintain backward compatibility with legacy BitTorrent v1 clients while offering v2 benefits to modern peers, the library supports **Hybrid torrents**:

- **Dual Info-Hashes**:
  - `v1_info_hash`: 20-byte SHA-1 hash over the v1 metadata.
  - `v2_info_hash`: 32-byte SHA-256 hash over the v2 metadata dictionary.
- **Pad Files (`.pad/<length>`)**: In Hybrid torrents, files must align with piece boundaries. The builder automatically inserts zero-filled pad files into the v1 `files` dictionary so that every content file begins and ends on an exact piece boundary.
- **Wire Compatibility**: Legacy peers connect using the v1 SHA-1 hash and download piece-aligned blocks; modern peers negotiate BEP 52 hash requests and verify 16 KiB blocks against Merkle roots.

---

## 6. Peer Wire Hash Transfer Protocol

When a client needs intermediate hashes to verify blocks, it queries connected peers using BEP 52 wire messages:

1. **`HashRequest` (ID 21)**:
   - 48-byte payload: `pieces_root` (32 bytes), `base_layer` (u32), `index` (u32), `length` (u32), `proof_layers` (u32).
2. **`Hashes` (ID 22)**:
   - 48-byte header followed by concatenated 32-byte hashes (base layer hashes + ancestor uncle proof hashes). Internal sub-tree proof layers are omitted per BEP 52 rules.
3. **`HashReject` (ID 23)**:
   - Emitted if the requested hashes cannot be served.

---

## 7. Tooling: `TorrentBuilder` & `TorrentUpgrader`

### Programmatic Creation via `TorrentBuilder`
```rust
use bittorrent_rs::core::torrent_builder::TorrentBuilder;

// Build a Hybrid v1+v2 torrent with 64 KiB pieces
let builder = TorrentBuilder::new("./data_folder")
    .piece_length(65536)
    .add_tracker("https://tracker.example.com/announce");

let hybrid_bytes = builder.build_hybrid()?;
std::fs::write("output.torrent", hybrid_bytes)?;
```

### Upgrading Legacy Torrents via CLI (`bt-cli`)
```bash
# Upgrade legacy v1 .torrent file using downloaded files on disk
bt-cli upgrade legacy.torrent ./downloads --format hybrid --output upgraded.torrent

# Inspect v2 and Merkle properties of any torrent file
bt-cli v2-info upgraded.torrent
```

---

## 8. References

- [BEP 52 — The BitTorrent Protocol Specification v2](https://www.bittorrent.org/beps/bep_0052.html)
- Source: [`library/src/core/merkle.rs`](file:///c:/Projects/BitTorrent/library/src/core/merkle.rs)
- Source: [`library/src/core/torrent_upgrader.rs`](file:///c:/Projects/BitTorrent/library/src/core/torrent_upgrader.rs)
- Source: [`library/src/storage/piece_layers_cache.rs`](file:///c:/Projects/BitTorrent/library/src/storage/piece_layers_cache.rs)
