use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CALLBACK_LIMIT: usize = 600;
const CALLBACK_WINDOW: Duration = Duration::from_secs(60);

#[derive(Clone, Default)]
pub struct StoreCallbackLimiter {
    attempts: Arc<Mutex<HashMap<String, VecDeque<Instant>>>>,
}

impl StoreCallbackLimiter {
    pub fn allow(&self, channel_id: &str, source_ip: Option<IpAddr>) -> bool {
        self.allow_at(channel_id, source_ip, Instant::now())
    }

    fn allow_at(&self, channel_id: &str, source_ip: Option<IpAddr>, now: Instant) -> bool {
        let key = format!(
            "{channel_id}|{}",
            source_ip.map_or_else(|| "unknown".to_string(), |ip| ip.to_string())
        );
        let mut attempts = self
            .attempts
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let entries = attempts.entry(key).or_default();
        while entries
            .front()
            .is_some_and(|created_at| now.duration_since(*created_at) >= CALLBACK_WINDOW)
        {
            entries.pop_front();
        }
        if entries.len() >= CALLBACK_LIMIT {
            return false;
        }
        entries.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_scopes_six_hundred_callbacks_by_channel_and_source() {
        let limiter = StoreCallbackLimiter::default();
        let start = Instant::now();
        let source = Some("203.0.113.10".parse().unwrap());
        for _ in 0..600 {
            assert!(limiter.allow_at("stripe-1", source, start));
        }
        assert!(!limiter.allow_at("stripe-1", source, start));
        assert!(limiter.allow_at("stripe-2", source, start));
        assert!(limiter.allow_at("stripe-1", Some("203.0.113.11".parse().unwrap()), start));
        assert!(limiter.allow_at("stripe-1", source, start + CALLBACK_WINDOW));
    }
}
