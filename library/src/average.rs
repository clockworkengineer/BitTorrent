#[derive(Debug, Default, Clone)]
pub struct Average {
    total: i64,
    count: i64,
}

impl Average {
    pub fn add(&mut self, addition: i64) {
        self.total += addition;
        self.count += 1;
    }

    pub fn get(&self) -> i64 {
        if self.count != 0 {
            self.total / self.count
        } else {
            0
        }
    }
}
