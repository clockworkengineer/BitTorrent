# NAT-PMP Port Mapping

The library includes a built-in **NAT-PMP** (Network Address Translation — Port Mapping Protocol) client in [`library/src/nat.rs`](file:///c:/Projects/BitTorrent/library/src/nat.rs) that automatically opens ports on home NAT routers, enabling the client to receive incoming peer connections.

---

## Why Port Forwarding Matters

Most home users sit behind a NAT router. Without port forwarding:
- Remote peers **cannot initiate connections** to the local client.
- The client is limited to **outgoing-only** peer connections, significantly reducing swarm participation and download speeds.

By automatically registering a port mapping with the router, the client becomes **reachable** from the internet, increasing the number of potential peers.

---

## NAT-PMP Protocol Overview

NAT-PMP is a lightweight **UDP-based protocol** standardized in [RFC 6886](https://www.rfc-editor.org/rfc/rfc6886). The client sends a 12-byte request to the gateway router on port **5351**, and the router responds with a 16-byte reply confirming the mapping.

```
Client                      Router (gateway:5351)
  │                               │
  │── 12 bytes: mapping request ─►│
  │◄─ 16 bytes: mapping response ─│
  │                               │
  │ External port is now open      │
```

---

## Gateway Discovery

The client infers the default gateway IP by inspecting its own local IP address and replacing the last octet with `.1`:

```rust
pub fn get_default_gateway() -> Ipv4Addr {
    let local_ip = get_ip();                     // e.g., "192.168.1.105"
    let octets = local_ip.parse::<Ipv4Addr>().octets();
    Ipv4Addr::new(octets[0], octets[1], octets[2], 1)  // → 192.168.1.1
}
```

Loopback addresses (`127.x.x.x`) fall back to the conventional `192.168.1.1`.

> [!NOTE]
> This heuristic works on the vast majority of home networks but may not be accurate on complex multi-subnet or corporate networks. Future work includes proper default-route discovery via the OS routing table.

---

## Request Packet Format

All NAT-PMP mapping requests are 12 bytes:

```
 0               1               2               3
 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|    version    |    opcode     |           reserved            |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|       internal port           |       external port           |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                         lifetime (seconds)                    |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Field | Value |
|---|---|
| `version` | Always `0` |
| `opcode` | `1` = UDP mapping, `2` = TCP mapping |
| `internal port` | Local listening port (e.g., `6881`) |
| `external port` | Requested external port (e.g., `6881`; `0` = let router choose) |
| `lifetime` | Seconds the mapping should last (e.g., `3600`; `0` = delete mapping) |

---

## Response Packet Format

The router replies with a 16-byte response:

```
 0               1               2               3
 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|    version    |  opcode+128   |          result code          |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                      seconds since epoch                      |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|       internal port           |       external port           |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                         lifetime (seconds)                    |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| `result code` | Meaning |
|---|---|
| `0` | Success |
| `1` | Unsupported version |
| `2` | Not authorised |
| `3` | Network failure |
| `4` | Out of resources |
| `5` | Unsupported opcode |

---

## `NatPmpClient` API

```rust
pub struct NatPmpClient {
    gateway: Ipv4Addr,
}

impl NatPmpClient {
    /// Creates a new client targeting the given gateway IP.
    pub fn new(gateway: Ipv4Addr) -> Self;

    /// Sends a port mapping request and returns the external port on success.
    pub fn request_mapping(
        &self,
        is_tcp: bool,
        internal_port: u16,
        external_port: u16,
        lifetime_secs: u32,
    ) -> Result<u16, BitTorrentError>;

    /// Deletes a port mapping by sending a lifetime=0 request.
    pub fn release_mapping(&self, is_tcp: bool, internal_port: u16) -> Result<(), BitTorrentError>;

    /// Builds a raw 12-byte mapping request packet (useful for testing).
    pub fn build_mapping_request(
        &self, is_tcp: bool, internal_port: u16,
        external_port: u16, lifetime_secs: u32,
    ) -> Vec<u8>;

    /// Parses a raw 16-byte mapping response into (internal_port, external_port, lifetime).
    pub fn parse_mapping_response(buf: &[u8]) -> Result<(u16, u16, u32), BitTorrentError>;
}
```

---

## Session Lifecycle Integration

Port mappings are automatically managed by `TorrentSession`:

### On `start_download()`
```rust
// Maps port 6881 for both TCP and UDP with a 3600-second (1-hour) lifetime
let gateway = nat::get_default_gateway();
let client = NatPmpClient::new(gateway);
client.request_mapping(true,  6881, 6881, 3600)?;  // TCP
client.request_mapping(false, 6881, 6881, 3600)?;  // UDP
```

### On `stop()`
```rust
// Releases both mappings (sends lifetime=0 requests)
client.release_mapping(true,  6881)?;  // TCP
client.release_mapping(false, 6881)?;  // UDP
```

Mappings are stored in the `TorrentSession.nat_pmp` field as `Option<Arc<NatPmpClient>>`.

---

## Error Handling

NAT-PMP failures are non-fatal. If the gateway does not respond within 2 seconds or returns an error:
- The mapping attempt is logged.
- `start_download()` continues without port forwarding (the client operates in outgoing-only mode).

---

## Limitations & Future Work

| Limitation | Notes |
|---|---|
| **UPnP / SSDP not implemented** | NAT-PMP works on most modern home routers. Older routers may only support UPnP (SOAP/HTTP). UPnP support is planned as a future enhancement. |
| **Heuristic gateway discovery** | The `.1` subnet heuristic works for most home networks but not all. OS routing table query is a future improvement. |
| **No mapping renewal** | Mappings expire after `lifetime_secs`. Long-running sessions should renew before expiry. A background renewal loop is a planned enhancement. |
| **IPv6 not supported** | NAT-PMP is an IPv4-only protocol. Its successor, PCP (RFC 6887), supports IPv6 and is a future consideration. |

---

## References

- [RFC 6886 — NAT-PMP](https://www.rfc-editor.org/rfc/rfc6886)
- [RFC 6887 — PCP (successor)](https://www.rfc-editor.org/rfc/rfc6887)
- [`nat.rs`](file:///c:/Projects/BitTorrent/library/src/nat.rs) — full source
- [`session.rs`](file:///c:/Projects/BitTorrent/library/src/session.rs) — lifecycle integration
