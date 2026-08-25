use monoize_lynshen_rehearsal::token_bucket::BucketMap;
use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

fn ip(last: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(192, 0, 2, last))
}

#[test]
fn bucket_enforces_burst_and_continuous_refill() {
    let mut buckets = BucketMap::new(20, 1.0, 10_000, Duration::from_secs(120));
    assert_eq!((0..20).filter(|_| buckets.take(ip(1), 0)).count(), 20);
    assert!(!buckets.take(ip(1), 0));
    assert!(!buckets.take(ip(1), 999));
    assert!(buckets.take(ip(1), 1_000));
}

#[test]
fn endpoint_kinds_share_one_ip_bucket() {
    let mut buckets = BucketMap::new(2, 1.0, 10, Duration::from_secs(120));
    assert!(buckets.take(ip(1), 0));
    assert!(buckets.take(ip(1), 0));
    assert!(!buckets.take(ip(1), 0));
}

#[test]
fn capacity_evicts_least_recently_seen_idle_entry() {
    let mut buckets = BucketMap::new(1, 1.0, 2, Duration::from_secs(120));
    assert!(buckets.take(ip(1), 0));
    assert!(buckets.take(ip(2), 1));
    assert!(buckets.take(ip(3), 120_001));
    assert_eq!(buckets.len(), 2);
    assert!(!buckets.contains(ip(1)));
}

#[test]
fn capacity_rejects_new_ip_when_no_entry_is_idle() {
    let mut buckets = BucketMap::new(1, 1.0, 2, Duration::from_secs(120));
    assert!(buckets.take(ip(1), 0));
    assert!(buckets.take(ip(2), 1));
    assert!(!buckets.take(ip(3), 2));
    assert_eq!(buckets.len(), 2);
}
