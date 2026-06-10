# uTorrent Transport Protocol (uTP — BEP 29)

The library provides a **uTP framing adapter** in [`library/src/utp.rs`](file:///c:/Projects/BitTorrent/library/src/utp.rs), which wraps a UDP socket as a `AsyncSocket`-compatible transport that can carry BitTorrent peer wire messages.

---

## Overview

uTP (μTP) is a UDP-based transport protocol designed to transfer data while being **friendly to other network traffic**. Unlike TCP, uTP uses the **LEDBAT** (Low Extra Delay Background Transport) congestion control algorithm, which backs off aggressively when it detects router queue buildup. This prevents a torrent client from saturating the local router and degrading web browsing and VoIP quality.

### Why uTP Matters

| Property | TCP | uTP |
|---|---|---|
| Transport | TCP | UDP |
| Congestion control | TCP Reno / CUBIC | LEDBAT (delay-based) |
| NAT traversal | Hard | Easier via UDP hole-punching |
| Effect on other traffic | Can saturate routers | Self-throttles based on latency |

---

## Packet Structure

Every uTP packet begins with a fixed **20-byte header** (`UtpHeader`):

```
 0               1               2               3
 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
| type  |  ver  |   extension   |         connection_id         |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                          timestamp_us                         |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                     timestamp_difference_us                   |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                           wnd_size                            |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|              seq_nr           |             ack_nr            |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

### Field Descriptions

| Field | Size | Description |
|---|---|---|
| `type` | 4 bits | Packet type (Data, Ack, Syn, Reset, State) |
| `version` | 4 bits | Protocol version (always `1`) |
| `extension` | 1 byte | Extension header type (`0` = none) |
| `connection_id` | 2 bytes | Connection identifier shared by both peers |
| `timestamp_us` | 4 bytes | Sender's current time in microseconds |
| `timestamp_difference_us` | 4 bytes | One-way delay measurement (remote ts − local ts) |
| `wnd_size` | 4 bytes | Receive window size advertised by sender |
| `seq_nr` | 2 bytes | Sequence number of this packet |
| `ack_nr` | 2 bytes | Sequence number of the last packet acknowledged |

---

## Packet Types

```rust
pub enum UtpPacketType {
    Data  = 0,  // ST_DATA  — carries payload bytes
    Ack   = 1,  // ST_FIN   — graceful connection close
    Syn   = 2,  // ST_STATE — pure acknowledgement (no payload)
    Reset = 3,  // ST_RESET — abort connection immediately
    State = 4,  // ST_SYN   — initiate a new connection
}
```

### Connection State Machine

```
Initiator                       Receiver
    │                               │
    │── SYN (seq=0, conn_id=X) ────►│  Receiver allocates connection X+1
    │◄─ STATE (ack=0, conn_id=X+1) ─│  SYN-ACK equivalent
    │                               │
    │── DATA (seq=1) ──────────────►│
    │◄─ STATE (ack=1) ──────────────│  Pure ACK
    │                               │
    │── ACK/FIN ───────────────────►│  Graceful close
    │◄─ RESET (on error) ───────────│  Abort
```

---

## `UtpSocketAdapter`

`UtpSocketAdapter` adapts a `std::net::UdpSocket` to implement the `AsyncSocket` trait, making it usable anywhere a TCP socket could be used (e.g., as an injected `socket_factory` transport for peer connections).

```rust
pub struct UtpSocketAdapter {
    socket: Arc<UdpSocket>,
    conn_id_send: u16,
    conn_id_recv: u16,
    seq_nr: Mutex<u16>,
    ack_nr: Mutex<u16>,
}
```

### Key Behaviours

| Method | Behaviour |
|---|---|
| `connect(ip, port)` | Sends a SYN packet and waits for a STATE response to establish the connection |
| `write(buf)` | Splits the buffer into uTP DATA packets with incrementing sequence numbers; sends each and waits for STATE ACK |
| `read(buf)` | Receives a UDP datagram, parses the uTP header, sends an ACK, and returns the payload bytes |
| `close()` | Sends a RESET packet to signal connection termination |

---

## Current Implementation Scope

> [!IMPORTANT]
> The current `UtpSocketAdapter` implements **uTP framing and connection management** but does **not** implement the full **LEDBAT congestion control algorithm**. Specifically:
> - There is no delay measurement or `timestamp_difference_us`-based congestion window adjustment.
> - The `wnd_size` field is set to a fixed 1 MB and not dynamically adjusted.
> - Retransmission on packet loss is not implemented.

This means uTP connections will work for basic data transfer but will not self-throttle in congested network conditions. Full LEDBAT support is a planned future enhancement.

---

## Usage via `SessionConfig`

To use uTP as the peer transport, inject a `UtpSocketFactory` via `SessionConfig`:

```rust
use bittorrent_rs::utp::UtpSocketAdapter;
use bittorrent_rs::session::SessionConfig;

let config = SessionConfig {
    socket_factory: Arc::new(UtpSocketFactory::default()),
    ..SessionConfig::default()
};
```

> [!NOTE]
> `UtpSocketFactory` is a convenience wrapper that creates `UtpSocketAdapter` instances on demand. It is a planned addition that will complement the existing `TcpSocketFactory`.

---

## Future Work

- **LEDBAT congestion control**: Implement one-way delay measurement and dynamic window sizing to achieve true delay-based backoff.
- **Selective Acknowledgement (SACK)**: Support selective ACK extension headers for efficient out-of-order delivery.
- **Packet retransmission**: Detect lost packets via sequence number gaps and re-send.
- **`UtpSocketFactory`**: A proper `SocketFactory` impl for injecting uTP as a first-class peer transport.

---

## References

- [BEP 29 — uTorrent Transport Protocol](https://www.bittorrent.org/beps/bep_0029.html)
- [LEDBAT — RFC 6817](https://www.rfc-editor.org/rfc/rfc6817)
- [`utp.rs`](file:///c:/Projects/BitTorrent/library/src/utp.rs) — full source
