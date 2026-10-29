//! Retry a request to the order cache, with a backoff between tries.

use std::thread::sleep;
use std::time::Duration;

/// The longest time that one try can take before the cache times out.
const TIMEOUT: Duration = Duration::from_millis(500);

pub struct Backoff {
    tries: u32,
    delay: Duration,
}

impl Backoff {
    pub fn new(tries: u32) -> Self {
        Backoff { tries, delay: Duration::from_millis(50) }
    }

    /// Run `op` until it succeeds or the tries are used.
    pub fn run<T, E>(&mut self, mut op: impl FnMut() -> Result<T, E>) -> Result<T, E> {
        loop {
            match op() {
                Ok(value) => return Ok(value),
                Err(err) if self.tries == 0 => return Err(err),
                Err(_) => {
                    self.tries -= 1;
                    sleep(self.delay.min(TIMEOUT));
                    self.delay *= 2;
                }
            }
        }
    }
}
