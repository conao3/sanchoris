use chrono::{DateTime, Duration, Utc};
use std::sync::Mutex;

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[derive(Debug)]
pub struct FakeClock {
    now: Mutex<DateTime<Utc>>,
}

impl FakeClock {
    pub fn new(now: DateTime<Utc>) -> Self {
        Self {
            now: Mutex::new(now),
        }
    }

    pub fn set(&self, now: DateTime<Utc>) {
        *self.now.lock().expect("clock lock") = now;
    }

    pub fn advance(&self, by: Duration) {
        *self.now.lock().expect("clock lock") += by;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().expect("clock lock")
    }
}
