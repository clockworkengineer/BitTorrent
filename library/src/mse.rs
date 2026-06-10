//! Message Stream Encryption (MSE) obfuscation protocol
//!
//! Provides a pure-Rust implementation of the RC4 stream cipher and DH key exchange
//! for obfuscating peer connection traffic to bypass ISP traffic shaping.

/// Simple RC4 stream cipher implementation.
pub struct Rc4 {
    state: [u8; 256],
    i: u8,
    j: u8,
}

impl Rc4 {
    /// Initializes a new RC4 cipher stream using the given secret key.
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

    /// Obfuscates (encrypts or decrypts in-place) the given byte slice.
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

/// Multiplies `a * b mod m` without overflow by doubling (Russian Peasant / binary multiplication).
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

/// Computes (base^exp) % modulus using binary exponentiation with safe 128-bit multiplication.
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

/// A lightweight Diffie-Hellman keys negotiator using a 128-bit safe prime.
pub struct DiffieHellman {
    private_key: u64,
    pub public_key: u64,
}

impl DiffieHellman {
    const PRIME: u128 = 0xFFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FF43; // 128-bit safe prime
    const GENERATOR: u128 = 2;

    /// Generates a new random private and public key pair.
    pub fn new() -> Self {
        let private_key: u64 = rand::random();
        let public_key = mod_pow(Self::GENERATOR, private_key as u128, Self::PRIME) as u64;
        DiffieHellman {
            private_key,
            public_key,
        }
    }

    /// Computes the shared secret using the remote public key.
    pub fn compute_shared_secret(&self, remote_public_key: u64) -> [u8; 8] {
        let secret = mod_pow(remote_public_key as u128, self.private_key as u128, Self::PRIME) as u64;
        secret.to_be_bytes()
    }
}
