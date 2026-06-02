//! Average tracker utility
//!
//! A simple structure to compute the moving average of values, used for
//! tracking transfer rates or durations.

#[derive(Debug, Default, Clone)]
pub struct Average {
    total: i64,
    count: i64,
}

impl Average {
    /// Adds a new value to the running total and increments the sample count.
    pub fn add(&mut self, addition: i64) {
        self.total += addition;
        self.count += 1;
    }

    /// Computes and returns the average of all added values.
    /// Returns 0 if no values have been added.
    pub fn get(&self) -> i64 {
        if self.count != 0 {
            self.total / self.count
        } else {
            0
        }
    }
}
