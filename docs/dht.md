# Distributed Hash Table (DHT) Peer Discovery

The BitTorrent library includes a self-contained client implementing the Kademlia Distributed Hash Table (DHT) protocol conforming to **BEP 5**. This enables decentralized, trackerless peer discovery using the UDP transport layer.

---

## Architecture Design

### Routing Table Layout
The DHT routing table manages active nodes in the 160-bit ID space:
- **XOR Distance Metric**: The distance between two nodes (or a node and a target info-hash) is calculated as their bitwise XOR (`NodeId ^ TargetId`).
- **160 Routing Buckets**: The routing table is split into 160 buckets, representing nodes that share a common prefix length. The bucket index is calculated by counting the number of leading zeros in the XOR distance array.
- **Bucket Size Limit**: Each bucket has a maximum capacity of 8 active nodes. When a bucket is full and a new node is discovered:
  1. The client pings the oldest node in that bucket.
  2. If the oldest node replies, it is kept, and the new node is discarded.
  3. If the oldest node fails to reply, it is removed, and the new node is inserted.

### KRPC Messages
DHT communication is driven by the KRPC protocol (Bencode-encoded UDP packets):
- **Lexicographical Dict Ordering**: Bencode specification requires dictionary keys to be ordered alphabetically. The DHT module implements a custom serializing engine that writes pre-sorted dictionary keys directly to a buffer, avoiding sorting overhead and allocations at runtime.
- **Message Types**:
  - **Query (`q`)**: Outgoing requests containing a query method name and arguments (`a`).
  - **Response (`r`)**: Incoming replies to queries, returning requested values.
  - **Error (`e`)**: Messages indicating protocol failures or invalid queries.

---

## Supported Queries

The DHT client implements the four core Kademlia queries:

1. **`ping`**: Verifies if a remote node is online and updates its active timestamp in the routing table.
2. **`find_node`**: Requests the contact details (IP and port) of the 8 closest nodes to a target Node ID.
3. **`get_peers`**: Requests contact details of peers currently downloading/seeding a specific `info_hash`.
   - If the remote node has peers registered for the hash, it returns a list of compact peer addresses.
   - If the remote node does not have peers registered, it returns the contact details of the 8 closest nodes in its routing table to help the requester continue searching recursively.
   - Returns a secure `token` used for subsequent announces.
4. **`announce_peer`**: Registers the client's listening port for an `info_hash` on a remote node, proving identity using the `token` received during a previous `get_peers` query.

---

## Discovery & Session Integration

### 1. Bootstrapping
When the DHT starts up, the local routing table is populated by querying public bootstrap routers:
- `router.bittorrent.com:6881`
- `router.utorrent.com:6881`
- `dht.transmissionbt.com:6881`

The client transmits a `find_node` query targeting its own `NodeId` to the bootstrap nodes. The returned nodes are recursively queried, rapidly populating our routing table.

### 2. Recursive Lookup Loop
For peer discovery:
1. The client initiates a `get_peers` search targeting the torrent's `info_hash`.
2. Initial queries are sent to the 8 closest nodes in the local routing table.
3. If peers are returned, they are immediately queued for session connection.
4. If node lists are returned, they are added to a search queue, and the client continues querying the closest uncontacted nodes recursively.

### 3. Session Integration
In [session.rs](file:///c:/Projects/BitTorrent/library/src/session.rs), if `dht_enabled` is set to `true`:
- The `Dht` server binds to the UDP port specified by `dht_port` (defaulting to `6881`).
- The session starts the UDP receiver listener thread.
- Discovered peers are pushed through a thread-safe Channel, where the session downloader thread spawns worker instances to connect to them.
- When `session.stop()` is invoked, the UDP socket is closed, shutting down the background listener threads cleanly.
