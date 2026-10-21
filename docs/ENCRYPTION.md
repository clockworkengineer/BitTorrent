# Message Stream Encryption (MSE / PE)

The BitTorrent library supports **Message Stream Encryption** (MSE), also known as **Protocol Encryption** (PE), to obfuscate peer-to-peer traffic and bypass ISP traffic shaping and deep-packet inspection (DPI) that throttles BitTorrent connections.

The implementation lives in [`library/src/mse.rs`](file:///c:/Projects/BitTorrent/library/src/mse.rs) and is integrated into the session worker in [`session/worker.rs`](file:///c:/Projects/BitTorrent/library/src/session/worker.rs).

---

## Overview

MSE wraps the standard BitTorrent handshake with a cryptographic exchange that makes the opening bytes of a peer connection indistinguishable from random data to passive network observers. The protocol involves two phases:

1. **Key Exchange** — A Diffie-Hellman exchange establishes a shared secret without transmitting the secret on the wire.
2. **Stream Obfuscation** — Two RC4 stream ciphers (one for each direction) are derived from the shared secret and used to encrypt all subsequent bytes including and after the BitTorrent handshake.

---

## Diffie-Hellman Key Exchange

### Prime & Generator

The library uses a **128-bit safe prime** for the DH exchange:

```
P = 0xFFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FF43
G = 2
```

Each side independently generates a **64-bit random private key** and computes its **public key** as:

```
public_key = G^private_key mod P
```

### Wire Protocol

```
Initiator                        Receiver
    │                               │
    │── 8 bytes: public_key ────────►│
    │◄─ 8 bytes: public_key ─────────│
    │                               │
    │   Both sides compute:          │
    │   secret = remote_pub^priv mod P
    │                               │
    │── [RC4-encrypted BT handshake]►│
    │◄─ [RC4-encrypted BT handshake]─│
```

Both sides independently compute the same `secret` from the other's public key:

```
secret = remote_public_key^private_key mod P
```

This is the Diffie-Hellman property: `G^(a*b) mod P = G^(b*a) mod P`.

### Overflow-Safe Modular Exponentiation

Computing `base^exp mod P` with 128-bit integers naively overflows because intermediate products can be up to 256 bits wide. The library solves this with a **Russian Peasant mulmod** helper:

```rust
/// Multiplies a * b mod m without overflow using binary doubling.
pub fn mulmod(mut a: u128, mut b: u128, m: u128) -> u128 {
    let mut result: u128 = 0;
    a %= m;
    while b > 0 {
        if b & 1 == 1 { result = result.wrapping_add(a) % m; }
        a = a.wrapping_add(a) % m;
        b >>= 1;
    }
    result
}

pub fn mod_pow(mut base: u128, mut exp: u128, modulus: u128) -> u128 {
    let mut result = 1u128;
    base %= modulus;
    while exp > 0 {
        if exp & 1 == 1 { result = mulmod(result, base, modulus); }
        exp >>= 1;
        base = mulmod(base, base, modulus);
    }
    result
}
```

---

## RC4 Key Derivation

After computing the shared `secret` (8 bytes), two independent RC4 cipher streams are derived — one for each direction — using SHA-1 hashes:

```
enc_key = SHA1(secret || "initiator")   // used by the initiating peer to encrypt
dec_key = SHA1(secret || "receiver")    // used by the initiating peer to decrypt
```

The receiver uses the mirrored derivation (`"receiver"` to encrypt, `"initiator"` to decrypt), so both sides always encrypt with the same key that the other side uses to decrypt.

---

## RC4 Stream Cipher

The library implements RC4 from scratch with no external dependencies:

```rust
pub struct Rc4 {
    state: [u8; 256],
    i: u8,
    j: u8,
}

impl Rc4 {
    pub fn new(key: &[u8]) -> Self { ... }    // KSA: Key Scheduling Algorithm
    pub fn encrypt(&mut self, data: &mut [u8]) { ... }  // PRGA: in-place XOR
}
```

RC4 is symmetric: `encrypt(encrypt(plaintext)) == plaintext`. The same `encrypt()` call is used for both encryption and decryption.

---

## Session Integration

MSE is **opt-in** and controlled via `SessionConfig`:

```rust
pub struct SessionConfig {
    // ...
    /// Enable MSE/PE handshake obfuscation. Default: false.
    pub mse_enabled: bool,
}
```

When `mse_enabled = true`, the worker performs the DH + RC4 exchange **before** writing the standard BitTorrent handshake:

```rust
// In session/worker.rs
if config.mse_enabled {
    let dh = DiffieHellman::new();
    net.write(&dh.public_key.to_be_bytes()).await?;
    let remote_pub = net.read_exact_8().await?;
    let secret = dh.compute_shared_secret(u64::from_be_bytes(remote_pub));
    // derive RC4 keys and install ciphers on PeerNetwork
    net.set_mse_ciphers(rc4_enc, rc4_dec);
}
// Standard BT handshake continues here (now encrypted if MSE is active)
net.write_handshake(&info_hash, &peer_id).await?;
```

The `PeerNetwork` transparently applies RC4 encryption/decryption to all subsequent `write()` and `read()` calls once ciphers are installed.

---

## Limitations & Future Work

| Limitation | Notes |
|---|---|
| **128-bit DH prime** | The MSE specification uses a 768-bit prime. The library uses a 128-bit prime for simplicity — sufficient for traffic obfuscation but not cryptographically strong against targeted attacks. |
| **No plaintext fallback** | The library does not implement automatic fallback to plaintext when a peer does not support MSE. This must be handled at the connection layer. |
| **No encryption profile negotiation** | Full MSE supports `plaintext only`, `prefer encrypted`, and `require encrypted` profiles via a Diffie-Hellman VC + crypto-method field. The library skips the VC/profile negotiation and proceeds directly to RC4. |

---

## References

- [MSE/PE Specification](http://wiki.vuze.com/w/Message_Stream_Encryption)
- [`mse.rs`](file:///c:/Projects/BitTorrent/library/src/mse.rs) — full source
- [`peer_network.rs`](file:///c:/Projects/BitTorrent/library/src/peer_network.rs) — cipher integration
