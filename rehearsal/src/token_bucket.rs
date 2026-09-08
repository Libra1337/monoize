use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
struct Bucket {
    tokens: f64,
    last_refill_ms: u64,
    last_seen_ms: u64,
}

#[derive(Debug)]
pub struct BucketMap {
    capacity: f64,
    refill_per_second: f64,
    maximum_entries: usize,
    idle_ms: u64,
    entries: HashMap<IpAddr, Bucket>,
}

impl BucketMap {
    pub fn new(
        capacity: u32,
        refill_per_second: f64,
        maximum_entries: usize,
        idle: Duration,
    ) -> Self {
        Self {
            capacity: f64::from(capacity),
            refill_per_second,
            maximum_entries,
            idle_ms: u64::try_from(idle.as_millis()).unwrap_or(u64::MAX),
            entries: HashMap::with_capacity(maximum_entries.min(10_000)),
        }
    }

    pub fn take(&mut self, ip: IpAddr, now_ms: u64) -> bool {
        if !self.entries.contains_key(&ip) {
            if self.entries.len() >= self.maximum_entries && !self.evict_idle(now_ms) {
                return false;
            }
            self.entries.insert(
                ip,
                Bucket {
                    tokens: self.capacity,
                    last_refill_ms: now_ms,
                    last_seen_ms: now_ms,
                },
            );
        }
        let bucket = self
            .entries
            .get_mut(&ip)
            .expect("entry inserted or existed");
        let elapsed_ms = now_ms.saturating_sub(bucket.last_refill_ms);
        bucket.tokens = (bucket.tokens + elapsed_ms as f64 * self.refill_per_second / 1_000.0)
            .min(self.capacity);
        bucket.last_refill_ms = now_ms;
        bucket.last_seen_ms = now_ms;
        if bucket.tokens < 1.0 {
            return false;
        }
        bucket.tokens -= 1.0;
        true
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        self.entries.contains_key(&ip)
    }

    fn evict_idle(&mut self, now_ms: u64) -> bool {
        let candidate = self
            .entries
            .iter()
            .filter(|(_, bucket)| now_ms.saturating_sub(bucket.last_seen_ms) >= self.idle_ms)
            .min_by(|(left_ip, left), (right_ip, right)| {
                left.last_seen_ms
                    .cmp(&right.last_seen_ms)
                    .then_with(|| left_ip.to_string().cmp(&right_ip.to_string()))
            })
            .map(|(ip, _)| *ip);
        candidate.is_some_and(|ip| self.entries.remove(&ip).is_some())
    }
}
