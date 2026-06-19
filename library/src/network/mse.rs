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

use crypto_bigint::{Encoding, U768};
use crypto_bigint::modular::runtime_mod::{DynResidue, DynResidueParams};
use rand::{RngCore, SeedableRng};
use rand::rngs::StdRng;

const PRIME: U768 = U768::from_be_hex(
    "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74\
     020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F1437\
     4FE1356D6D51C245E485B576625E7EC6F44C42E9A63A3620FFFFFFFFFFFFFFFF",
);

/// A Diffie-Hellman key negotiator using the 768-bit Oakley Group 1 prime.
///
/// Each side generates a random private key and computes a public key as
/// `G^private mod P`. The shared secret is then `remote_pub^private mod P`,
/// which equals `G^(a*b) mod P` for both parties — the core DH property.
///
/// ## Prime & Generator
/// ```text
/// P = 768-bit Oakley Group 1 prime
/// G = 2
/// ```
pub struct DiffieHellman {
    /// The private key — never transmitted on the wire.
    private_key: U768,
    /// The 96-byte public key derived as `G^private_key mod P`.
    /// Share this with the remote peer.
    pub public_key: [u8; 96],
}

impl DiffieHellman {
    /// Generates a new random private/public key pair.
    ///
    /// Uses `StdRng` to generate a cryptographically random private key,
    /// then computes `public_key = G^private_key mod P`.
    #[must_use]
    pub fn new() -> Self {
        let mut rng = StdRng::from_entropy();
        let mut private_bytes = [0u8; 96];
        rng.fill_bytes(&mut private_bytes);
        let private_key = U768::from_be_slice(&private_bytes);
        
        let params = DynResidueParams::new(&PRIME);
        let g = DynResidue::new(&U768::from(2u8), params);
        let public_key_big = g.pow(&private_key).retrieve();
        let public_key = public_key_big.to_be_bytes();
        
        DiffieHellman { private_key, public_key }
    }

    /// Computes the 96-byte shared secret from the remote peer's public key.
    ///
    /// Computes `secret = remote_pub^private mod P` and returns it as a big-endian
    /// byte array. Both sides produce the same secret: `G^(a*b) mod P = G^(b*a) mod P`.
    pub fn compute_shared_secret(&self, remote_public_key: [u8; 96]) -> [u8; 96] {
        let remote_pub_big = U768::from_be_slice(&remote_public_key);
        let params = DynResidueParams::new(&PRIME);
        let remote_res = DynResidue::new(&remote_pub_big, params);
        let secret_big = remote_res.pow(&self.private_key).retrieve();
        secret_big.to_be_bytes()
    }
}
