//! Piece and peer selector
//!
//! Implements strategy algorithms for selecting which pieces to request next
//! (random/missing search) and which peers are the best candidates to fetch pieces from.

use rand::SeedableRng;
use rand::rngs::StdRng;

/// Selects the next piece to download and ranks remote peers for block requests.
#[derive(Debug)]
pub struct Selector {
    _random_seed: StdRng,
}

impl Default for Selector {
    /// Returns the default `Selector` initialized with a random entropy seed.
    fn default() -> Self {
        Selector::new()
    }
}

impl Selector {
    /// Creates a new `Selector` instance using entropy to initialize the random number generator.
    pub fn new() -> Self {
        Selector {
            _random_seed: StdRng::from_entropy(),
        }
    }
}
