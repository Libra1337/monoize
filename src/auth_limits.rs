use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Failed-attempt counts older than this are forgotten.
const ATTEMPT_WINDOW: Duration = Duration::from_secs(15 * 60);
/// Per `(username, source IP)` pair: an attacker targeting one account from one
/// address is stopped here.
const PAIR_FAILURE_LIMIT: usize = 5;
/// Per source IP regardless of username: an attacker spraying one password across
/// many accounts is stopped here.
const IP_FAILURE_LIMIT: usize = 25;
/// Bound on tracked keys. Reaching it prunes idle entries before inserting.
const MAX_ENTRIES: usize = 10_000;

#[derive(Clone, Copy)]
struct FailureRecord {
    failures: usize,
    last_failure: Instant,
}

/// CF-70: brute-force protection for the dashboard login endpoint.
///
/// Only failures are counted. A successful login clears the pair, so a user who
/// mistypes a few times and then succeeds is not left throttled. The limiter is
/// in-memory and process-local, which is the right scope: it guards an online
/// guessing attack, and a restart clearing the counters does not help an attacker
/// who is rate-limited by wall-clock time in the same window anyway.
#[derive(Clone, Default)]
pub struct LoginThrottle {
    pairs: Arc<Mutex<HashMap<String, FailureRecord>>>,
    ips: Arc<Mutex<HashMap<IpAddr, FailureRecord>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginThrottleVerdict {
    Allowed,
    Throttled,
}

impl LoginThrottle {
    /// Check both scopes before verifying a password. A throttled caller must not
    /// reach password verification at all.
    pub fn check(&self, username: &str, source_ip: Option<IpAddr>) -> LoginThrottleVerdict {
        self.check_at(username, source_ip, Instant::now())
    }

    fn check_at(
        &self,
        username: &str,
        source_ip: Option<IpAddr>,
        now: Instant,
    ) -> LoginThrottleVerdict {
        let key = pair_key(username, source_ip);
        if count_at(&self.pairs, &key, now) >= PAIR_FAILURE_LIMIT {
            return LoginThrottleVerdict::Throttled;
        }
        if let Some(ip) = source_ip
            && count_at(&self.ips, &ip, now) >= IP_FAILURE_LIMIT
        {
            return LoginThrottleVerdict::Throttled;
        }
        LoginThrottleVerdict::Allowed
    }

    /// Record a failed password verification.
    pub fn record_failure(&self, username: &str, source_ip: Option<IpAddr>) {
        self.record_failure_at(username, source_ip, Instant::now());
    }

    fn record_failure_at(&self, username: &str, source_ip: Option<IpAddr>, now: Instant) {
        bump(&self.pairs, &pair_key(username, source_ip), now);
        if let Some(ip) = source_ip {
            bump(&self.ips, &ip, now);
        }
    }

    /// Clear this caller's counters after a successful login.
    ///
    /// Only the pair's own failures are removed. The source address counter is decremented
    /// by exactly that amount rather than cleared: an attacker who owns one account could
    /// otherwise log into it to wipe the address counter and defeat the spray limit, which
    /// is the whole point of counting per address as well as per pair.
    pub fn record_success(&self, username: &str, source_ip: Option<IpAddr>) {
        let key = pair_key(username, source_ip);
        let cleared = match self.pairs.lock() {
            Ok(mut pairs) => pairs
                .remove(&key)
                .map(|record| record.failures)
                .unwrap_or(0),
            Err(_) => 0,
        };
        if cleared == 0 {
            return;
        }
        let Some(ip) = source_ip else {
            return;
        };
        let Ok(mut ips) = self.ips.lock() else {
            return;
        };
        if let Some(record) = ips.get_mut(&ip) {
            record.failures = record.failures.saturating_sub(cleared);
            if record.failures == 0 {
                ips.remove(&ip);
            }
        }
    }
}

/// The username is lowercased so `Alice` and `alice` share one counter; usernames
/// are compared case-insensitively at lookup, so the throttle must match.
fn pair_key(username: &str, source_ip: Option<IpAddr>) -> String {
    match source_ip {
        Some(ip) => format!("{}|{ip}", username.trim().to_ascii_lowercase()),
        None => format!("{}|unknown", username.trim().to_ascii_lowercase()),
    }
}

fn count_at<K: std::hash::Hash + Eq + Clone>(
    map: &Arc<Mutex<HashMap<K, FailureRecord>>>,
    key: &K,
    now: Instant,
) -> usize {
    let Ok(map) = map.lock() else {
        // A poisoned lock must not open the door; treat it as throttled.
        return usize::MAX;
    };
    match map.get(key) {
        Some(record) if now.duration_since(record.last_failure) < ATTEMPT_WINDOW => record.failures,
        _ => 0,
    }
}

fn bump<K: std::hash::Hash + Eq + Clone>(
    map: &Arc<Mutex<HashMap<K, FailureRecord>>>,
    key: &K,
    now: Instant,
) {
    let Ok(mut map) = map.lock() else {
        return;
    };
    if map.len() >= MAX_ENTRIES {
        map.retain(|_, record| now.duration_since(record.last_failure) < ATTEMPT_WINDOW);
        // Still full after pruning means every entry is active; drop the oldest so a
        // determined attacker cannot pin the table and lock out new callers.
        if map.len() >= MAX_ENTRIES
            && let Some(oldest) = map
                .iter()
                .min_by_key(|(_, record)| record.last_failure)
                .map(|(key, _)| key.clone())
        {
            map.remove(&oldest);
        }
    }
    let record = map.entry(key.clone()).or_insert(FailureRecord {
        failures: 0,
        last_failure: now,
    });
    if now.duration_since(record.last_failure) >= ATTEMPT_WINDOW {
        record.failures = 0;
    }
    record.failures = record.failures.saturating_add(1);
    record.last_failure = now;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, last))
    }

    #[test]
    fn blocks_one_account_after_five_failures_from_one_address() {
        let throttle = LoginThrottle::default();
        let start = Instant::now();
        for _ in 0..PAIR_FAILURE_LIMIT {
            assert_eq!(
                throttle.check_at("victim", Some(ip(1)), start),
                LoginThrottleVerdict::Allowed
            );
            throttle.record_failure_at("victim", Some(ip(1)), start);
        }
        assert_eq!(
            throttle.check_at("victim", Some(ip(1)), start),
            LoginThrottleVerdict::Throttled
        );
    }

    #[test]
    fn a_successful_login_clears_the_counter() {
        let throttle = LoginThrottle::default();
        let start = Instant::now();
        for _ in 0..PAIR_FAILURE_LIMIT {
            throttle.record_failure_at("user", Some(ip(2)), start);
        }
        assert_eq!(
            throttle.check_at("user", Some(ip(2)), start),
            LoginThrottleVerdict::Throttled
        );

        throttle.record_success("user", Some(ip(2)));
        assert_eq!(
            throttle.check_at("user", Some(ip(2)), start),
            LoginThrottleVerdict::Allowed
        );
    }

    #[test]
    fn spraying_many_usernames_from_one_address_is_blocked_by_the_ip_scope() {
        let throttle = LoginThrottle::default();
        let start = Instant::now();
        // Each username stays under the per-pair limit, so only the IP counter can stop it.
        for index in 0..(IP_FAILURE_LIMIT / PAIR_FAILURE_LIMIT + 1) {
            for _ in 0..PAIR_FAILURE_LIMIT {
                throttle.record_failure_at(&format!("user{index}"), Some(ip(3)), start);
            }
        }
        assert_eq!(
            throttle.check_at("a-fresh-username", Some(ip(3)), start),
            LoginThrottleVerdict::Throttled
        );
    }

    #[test]
    fn failures_expire_after_the_window() {
        let throttle = LoginThrottle::default();
        let start = Instant::now();
        for _ in 0..PAIR_FAILURE_LIMIT {
            throttle.record_failure_at("user", Some(ip(4)), start);
        }
        assert_eq!(
            throttle.check_at("user", Some(ip(4)), start),
            LoginThrottleVerdict::Throttled
        );
        assert_eq!(
            throttle.check_at("user", Some(ip(4)), start + ATTEMPT_WINDOW),
            LoginThrottleVerdict::Allowed
        );
    }

    #[test]
    fn username_case_shares_one_counter() {
        let throttle = LoginThrottle::default();
        let start = Instant::now();
        for _ in 0..PAIR_FAILURE_LIMIT {
            throttle.record_failure_at("Alice", Some(ip(5)), start);
        }
        assert_eq!(
            throttle.check_at("alice", Some(ip(5)), start),
            LoginThrottleVerdict::Throttled
        );
    }

    #[test]
    fn logging_into_an_owned_account_does_not_wipe_the_address_counter() {
        // An attacker who owns one account must not be able to clear the spray counter by
        // logging into it successfully.
        let throttle = LoginThrottle::default();
        let start = Instant::now();
        for index in 0..(IP_FAILURE_LIMIT / PAIR_FAILURE_LIMIT + 1) {
            for _ in 0..PAIR_FAILURE_LIMIT {
                throttle.record_failure_at(&format!("user{index}"), Some(ip(8)), start);
            }
        }
        // The attacker's own account is one of them; a successful login clears only its pair.
        throttle.record_success("user0", Some(ip(8)));
        assert_eq!(
            throttle.check_at("a-fresh-username", Some(ip(8)), start),
            LoginThrottleVerdict::Throttled,
            "a successful login must not clear other usernames' failures for the address"
        );
    }

    #[test]
    fn a_different_address_is_not_affected() {
        let throttle = LoginThrottle::default();
        let start = Instant::now();
        for _ in 0..PAIR_FAILURE_LIMIT {
            throttle.record_failure_at("user", Some(ip(6)), start);
        }
        assert_eq!(
            throttle.check_at("user", Some(ip(7)), start),
            LoginThrottleVerdict::Allowed
        );
    }
}

/// CF-73: bounds the wallet overdraft caused by the unlocked balance preflight.
///
/// `ensure_user_can_spend` reads the balance and answers "is it above zero". Between
/// that read and the settlement that charges the request, every other in-flight request
/// for the same user reads the same balance, so N concurrent requests each pass and each
/// settle in full. The observed result is a small negative balance (a few micro-dollars)
/// that the next preflight then rejects -- bounded, but not zero.
///
/// The fix is to subtract what is already in flight for that user when answering the
/// preflight. The amount reserved is the request's own computed cost ceiling, the same
/// bound the plan-funded path already reserves against, so nothing new is invented here.
///
/// This is a bound, not a ledger. It lives in memory, so a restart forgets it; that is
/// acceptable because it only ever makes the preflight stricter, and a restart is not a
/// way to spend money.
#[derive(Clone, Default)]
pub struct InFlightSpend {
    reserved: Arc<Mutex<HashMap<String, i128>>>,
}

/// Holds one reservation. Dropping it releases the amount, so every exit path -- success,
/// error, panic in an inner future, or an early `?` return -- gives the bound back. A
/// leaked reservation would permanently reduce a paying user's available balance, which
/// is why release is tied to `Drop` rather than to a hand-written call at one return site.
///
/// The guard is cloneable because the funding scope that owns it is cloned along the
/// streaming path. Clones share one `Arc`, so the amount is released exactly once, when
/// the last clone drops -- a per-clone release would stop counting a request that is
/// still running.
#[derive(Clone)]
pub struct InFlightReservation {
    // Held only to keep the reservation alive; the release happens in the inner Drop.
    #[allow(dead_code)]
    inner: Arc<InFlightReservationInner>,
}

pub struct InFlightReservationInner {
    tracker: InFlightSpend,
    user_id: String,
    amount: i128,
}

impl Drop for InFlightReservationInner {
    fn drop(&mut self) {
        self.tracker.release(&self.user_id, self.amount);
    }
}

impl InFlightSpend {
    /// Amount already reserved for this user. A poisoned lock reports the reservation as
    /// absent rather than panicking; the preflight then behaves as it did before this
    /// bound existed.
    pub fn reserved_for(&self, user_id: &str) -> i128 {
        let Ok(reserved) = self.reserved.lock() else {
            return 0;
        };
        reserved.get(user_id).copied().unwrap_or(0)
    }

    /// Reserve `amount` for `user_id`, returning a guard that releases it on drop.
    /// A non-positive amount reserves nothing.
    pub fn reserve(&self, user_id: &str, amount: i128) -> InFlightReservation {
        if amount > 0
            && let Ok(mut reserved) = self.reserved.lock()
        {
            let entry = reserved.entry(user_id.to_string()).or_insert(0);
            *entry = entry.saturating_add(amount);
        }
        InFlightReservation {
            inner: Arc::new(InFlightReservationInner {
                tracker: self.clone(),
                user_id: user_id.to_string(),
                amount: amount.max(0),
            }),
        }
    }

    fn release(&self, user_id: &str, amount: i128) {
        if amount <= 0 {
            return;
        }
        let Ok(mut reserved) = self.reserved.lock() else {
            return;
        };
        if let Some(entry) = reserved.get_mut(user_id) {
            *entry -= amount;
            if *entry <= 0 {
                reserved.remove(user_id);
            }
        }
    }
}

#[cfg(test)]
mod in_flight_spend_tests {
    use super::*;

    #[test]
    fn a_reservation_is_visible_while_held_and_gone_after_drop() {
        let tracker = InFlightSpend::default();
        assert_eq!(tracker.reserved_for("user-1"), 0);
        {
            let _first = tracker.reserve("user-1", 500);
            assert_eq!(tracker.reserved_for("user-1"), 500);
            let _second = tracker.reserve("user-1", 250);
            assert_eq!(tracker.reserved_for("user-1"), 750);
        }
        assert_eq!(tracker.reserved_for("user-1"), 0);
    }

    #[test]
    fn reservations_are_per_user() {
        let tracker = InFlightSpend::default();
        let _held = tracker.reserve("user-1", 100);
        assert_eq!(tracker.reserved_for("user-1"), 100);
        assert_eq!(tracker.reserved_for("user-2"), 0);
    }

    #[test]
    fn a_non_positive_reservation_holds_nothing() {
        let tracker = InFlightSpend::default();
        let _held = tracker.reserve("user-1", 0);
        assert_eq!(tracker.reserved_for("user-1"), 0);
    }

    #[test]
    fn releasing_more_than_reserved_clears_the_entry() {
        let tracker = InFlightSpend::default();
        let guard = tracker.reserve("user-1", 100);
        std::mem::forget(guard);
        assert_eq!(tracker.reserved_for("user-1"), 100);
        tracker.release("user-1", 250);
        assert_eq!(tracker.reserved_for("user-1"), 0);
    }
}
