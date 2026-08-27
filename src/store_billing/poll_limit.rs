use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const POLL_LIMIT: usize = 30;
const POLL_WINDOW: Duration = Duration::from_secs(60);

#[derive(Clone, Default)]
pub struct StoreOrderPollLimiter {
    attempts: Arc<Mutex<HashMap<String, VecDeque<Instant>>>>,
}

impl StoreOrderPollLimiter {
    pub fn allow(&self, user_id: &str) -> bool {
        self.allow_at(user_id, Instant::now())
    }

    fn allow_at(&self, user_id: &str, now: Instant) -> bool {
        let mut attempts = self
            .attempts
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let user_attempts = attempts.entry(user_id.to_string()).or_default();
        while user_attempts
            .front()
            .is_some_and(|created_at| now.duration_since(*created_at) >= POLL_WINDOW)
        {
            user_attempts.pop_front();
        }
        if user_attempts.len() >= POLL_LIMIT {
            return false;
        }
        user_attempts.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_allows_thirty_polls_per_user_per_minute() {
        let limiter = StoreOrderPollLimiter::default();
        let start = Instant::now();
        for _ in 0..30 {
            assert!(limiter.allow_at("user-1", start));
        }
        assert!(!limiter.allow_at("user-1", start));
        assert!(limiter.allow_at("user-2", start));
        assert!(limiter.allow_at("user-1", start + Duration::from_secs(60)));
    }
}
