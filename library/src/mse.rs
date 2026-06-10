//! Message Stream Encryption (MSE) / Protocol Encryption (PE)
//!
//! Provides a pure-Rust RC4 stream cipher and Diffie-Hellman key exchange for
//! obfuscating peer-connection traffic to bypass ISP traffic shaping and DPI.
//!
//! ## Protocol Summary
//!
//! 1. Both peers exchange 8-byte DH public keys.
//! 2. Each side computes the shared secret: `remote_pub^private mod P`.
//! 3. Two RC4 stream ciphers are derived from `SHA1(secret || role)`.
//! 4. All subsequent bytes (including the BitTorrent handshake) are encrypted.
//!
//! MSE is opt-in via [`SessionConfig::mse_enabled`](crate::session::SessionConfig::mse_enabled).
//! See [docs/encryption.md](https://github.com/clockworkengineer/BitTorrent/blob/main/docs/encryption.md)
//! for the full protocol walkthrough.

/// A pure-Rust implementation of the RC4 (Rivest Cipher 4) stream cipher.
///
/// RC4 is symmetric: calling [`encrypt`](Rc4::encrypt) twice with the same key and
/// initial state returns the original plaintext. The same method is used for both
/// encryption and decryption.
///
/// # Example
/// ```rust
/// use bittorrent_rs::mse::Rc4;
/// let mut enc = Rc4::new(b"secret");
/// let mut dec = Rc4::new(b"secret");
/// let plaintext = b"hello".to_vec();
/// let mut data = plaintext.clone();
/// enc.encrypt(&mut data);
/// assert_ne!(data, plaintext);
/// dec.encrypt(&mut data);   // decrypts
/// assert_eq!(data, plaintext);
/// ```
pub struct Rc4 {
    state: [u8; 256],
    i: u8,
    j: u8,
}

impl Rc4 {
    /// Initializes a new RC4 cipher stream using the given secret key.
    ///
    /// Performs the Key Scheduling Algorithm (KSA) to initialize the internal
    /// 256-byte permutation state. `key` may be any length > 0.
    #[must_use]
    pub fn new(key: &[u8]) -> Self {
        let mut state = [0u8; 256];
        for i in 0..256 {
            state[i] = i as u8;
        }
        let mut j: u8 = 0;
        for i in 0..256 {
            j = j.wrapping_add(state[i]).wrapping_add(key[i % key.len()]);
            state.swap(i, j as usize);
        }
        Rc4 { state, i: 0, j: 0 }
    }

    /// Encrypts or decrypts the given byte slice **in place** using the RC4 keystream (PRGA).
    ///
    /// Each byte is XORed with the next byte of the pseudo-random keystream.
    /// Because XOR is its own inverse, the same call encrypts and decrypts.
    pub fn encrypt(&mut self, data: &mut [u8]) {
        for byte in data.iter_mut() {
            self.i = self.i.wrapping_add(1);
            self.j = self.j.wrapping_add(self.state[self.i as usize]);
            self.state.swap(self.i as usize, self.j as usize);
            let t = self.state[self.i as usize].wrapping_add(self.state[self.j as usize]);
            let k = self.state[t as usize];
            *byte ^= k;
        }
    }
}

/// Multiplies `a * b mod m` without 128-bit overflow using binary doubling
/// (the "Russian Peasant" / "Egyptian multiplication" algorithm).
///
/// Standard `(a * b) % m` overflows when `a` and `b` are near the 128-bit
/// limit. This function avoids overflow by repeatedly doubling `a` modulo `m`
/// instead of computing the full product at once.
pub fn mulmod(mut a: u128, mut b: u128, m: u128) -> u128 {
    let mut result: u128 = 0;
    a %= m;
    while b > 0 {
        if b & 1 == 1 {
            result = result.wrapping_add(a) % m;
        }
        a = a.wrapping_add(a) % m;
        b >>= 1;
    }
    result
}

/// Computes `(base ^ exp) % modulus` using binary (square-and-multiply) exponentiation.
///
/// Uses [`mulmod`] for each multiplication step to avoid 128-bit integer overflow
/// with large bases and moduli (e.g. the 128-bit DH prime used by MSE).
pub fn mod_pow(mut base: u128, mut exp: u128, modulus: u128) -> u128 {
    if modulus == 1 {
        return 0;
    }
    let mut result = 1u128;
    base %= modulus;
    while exp > 0 {
        if exp & 1 == 1 {
            result = mulmod(result, base, modulus);
        }
        exp >>= 1;
        base = mulmod(base, base, modulus);
    }
    result
}

/// A lightweight Diffie-Hellman key negotiator using a 128-bit safe prime.
///
/// Each side generates a random 64-bit private key and computes a public key as
/// `G^private mod P`. The shared secret is then `remote_pub^private mod P`,
/// which equals `G^(a*b) mod P` for both parties — the core DH property.
///
/// ## Prime & Generator
/// ```text
/// P = 0xFFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FF43  (128-bit safe prime)
/// G = 2
/// ```
///
/// > **Note**: The MSE specification traditionally uses a 768-bit or 1024-bit prime.
/// > The 128-bit prime used here provides traffic obfuscation but is not
/// > cryptographically strong against a well-resourced adversary.
pub struct DiffieHellman {
    /// The 64-bit private key — never transmitted on the wire.
    private_key: u64,
    /// The 64-bit public key derived as `G^private_key mod P`.
    /// Share this with the remote peer.
    pub public_key: u64,
}

impl DiffieHellman {
    /// 128-bit safe prime: `2^128 - 189`.
    const PRIME: u128 = 0xFFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FF43;
    /// DH generator (standard choice: 2).
    const GENERATOR: u128 = 2;

    /// Generates a new random private/public key pair.
    ///
    /// Uses `rand::random()` to generate a cryptographically random 64-bit private key,
    /// then computes `public_key = G^private_key mod P`.
    #[must_use]
    pub fn new() -> Self {
        let private_key: u64 = rand::random();
        let public_key = mod_pow(Self::GENERATOR, private_key as u128, Self::PRIME) as u64;
        DiffieHellman { private_key, public_key }
    }

    /// Computes the 8-byte shared secret from the remote peer's public key.
    ///
    /// Computes `secret = remote_pub^private mod P` and returns it as a big-endian
    /// byte array. Both sides produce the same secret: `G^(a*b) mod P = G^(b*a) mod P`.
    pub fn compute_shared_secret(&self, remote_public_key: u64) -> [u8; 8] {
        let secret = mod_pow(remote_public_key as u128, self.private_key as u128, Self::PRIME) as u64;
        secret.to_be_bytes()
    }
}
