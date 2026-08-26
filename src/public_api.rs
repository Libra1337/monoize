use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Response, StatusCode, header};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const TOKEN_CAPACITY: f64 = 20.0;
const REFILL_PER_SECOND: f64 = 1.0;
const MAX_BUCKETS: usize = 10_000;
const IDLE_AFTER: Duration = Duration::from_secs(120);

#[derive(Clone, Copy)]
struct Bucket {
    tokens: f64,
    last_refill: Instant,
    last_seen: Instant,
}

pub struct PublicRateLimiter {
    buckets: HashMap<IpAddr, Bucket>,
    capacity: usize,
}

impl PublicRateLimiter {
    fn new(capacity: usize) -> Self {
        Self {
            buckets: HashMap::new(),
            capacity,
        }
    }

    fn admit(&mut self, ip: IpAddr, now: Instant) -> bool {
        if let Some(bucket) = self.buckets.get_mut(&ip) {
            let elapsed = now
                .saturating_duration_since(bucket.last_refill)
                .as_secs_f64();
            bucket.tokens = (bucket.tokens + elapsed * REFILL_PER_SECOND).min(TOKEN_CAPACITY);
            bucket.last_refill = now;
            bucket.last_seen = now;
            if bucket.tokens < 1.0 {
                return false;
            }
            bucket.tokens -= 1.0;
            return true;
        }

        if self.buckets.len() >= self.capacity {
            let candidate = self
                .buckets
                .iter()
                .filter(|(_, bucket)| now.saturating_duration_since(bucket.last_seen) >= IDLE_AFTER)
                .min_by_key(|(_, bucket)| bucket.last_seen)
                .map(|(address, _)| *address);
            let Some(candidate) = candidate else {
                return false;
            };
            self.buckets.remove(&candidate);
        }

        self.buckets.insert(
            ip,
            Bucket {
                tokens: TOKEN_CAPACITY - 1.0,
                last_refill: now,
                last_seen: now,
            },
        );
        true
    }
}

static PUBLIC_RATE_LIMITER: OnceLock<Mutex<PublicRateLimiter>> = OnceLock::new();

fn limiter() -> &'static Mutex<PublicRateLimiter> {
    PUBLIC_RATE_LIMITER.get_or_init(|| Mutex::new(PublicRateLimiter::new(MAX_BUCKETS)))
}

pub fn admit(headers: &HeaderMap) -> bool {
    let address = crate::client_ip::canonical_client_ip_from_headers(headers)
        .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    limiter()
        .lock()
        .is_ok_and(|mut guard| guard.admit(address, Instant::now()))
}

pub fn rate_limited_response() -> Response<Body> {
    json_response(
        StatusCode::TOO_MANY_REQUESTS,
        br#"{"error":{"code":"rate_limited","message":"public API rate limit exceeded"}}"#.to_vec(),
        None,
        "no-store",
    )
}

pub fn cacheable_json_response(
    request_headers: &HeaderMap,
    bytes: Vec<u8>,
    cache_control: &'static str,
) -> Response<Body> {
    let etag = weak_etag(&bytes);
    if request_headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| if_none_match(value, &etag))
    {
        return json_response(
            StatusCode::NOT_MODIFIED,
            Vec::new(),
            Some(&etag),
            cache_control,
        );
    }
    json_response(StatusCode::OK, bytes, Some(&etag), cache_control)
}

fn json_response(
    status: StatusCode,
    bytes: Vec<u8>,
    etag: Option<&str>,
    cache_control: &'static str,
) -> Response<Body> {
    let mut response = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, cache_control)
        .header(header::VARY, "Accept-Encoding")
        .body(Body::from(bytes))
        .expect("static public response headers are valid");
    if let Some(etag) = etag.and_then(|value| HeaderValue::from_str(value).ok()) {
        response.headers_mut().insert(header::ETAG, etag);
    }
    response
}

fn weak_etag(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    format!("W/\"{encoded}\"")
}

fn if_none_match(value: &str, current: &str) -> bool {
    value.split(',').map(str::trim).any(|candidate| {
        candidate == "*"
            || entity_tag_value(candidate)
                .zip(entity_tag_value(current))
                .is_some_and(|(candidate, current)| candidate == current)
    })
}

fn entity_tag_value(value: &str) -> Option<&str> {
    let value = value.strip_prefix("W/").unwrap_or(value);
    (value.len() >= 2 && value.starts_with('"') && value.ends_with('"'))
        .then(|| &value[1..value.len() - 1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_bucket_enforces_burst_and_refill() {
        let mut limiter = PublicRateLimiter::new(10);
        let now = Instant::now();
        let ip = "192.0.2.1".parse().unwrap();
        for _ in 0..20 {
            assert!(limiter.admit(ip, now));
        }
        assert!(!limiter.admit(ip, now));
        assert!(limiter.admit(ip, now + Duration::from_secs(1)));
        assert!(!limiter.admit(ip, now + Duration::from_secs(1)));
    }

    #[test]
    fn capacity_evicts_only_the_oldest_idle_bucket() {
        let mut limiter = PublicRateLimiter::new(2);
        let now = Instant::now();
        assert!(limiter.admit("192.0.2.1".parse().unwrap(), now));
        assert!(limiter.admit("192.0.2.2".parse().unwrap(), now));
        assert!(!limiter.admit("192.0.2.3".parse().unwrap(), now + Duration::from_secs(119)));
        assert!(limiter.admit("192.0.2.3".parse().unwrap(), now + Duration::from_secs(120)));
        assert!(!limiter.buckets.contains_key(&"192.0.2.1".parse().unwrap()));
    }

    #[test]
    fn weak_etag_list_and_wildcard_match() {
        let etag = weak_etag(b"site");
        assert!(if_none_match(
            "\"other\", W/\"site-does-not-match\"",
            "W/\"site-does-not-match\""
        ));
        assert!(if_none_match("*", &etag));
        assert!(if_none_match(&etag.replace("W/", ""), &etag));
        assert!(!if_none_match("malformed", &etag));
    }
}
