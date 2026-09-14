//! Client-facing error masking per `spec/upstream-error-sanitization.spec.md`.
//!
//! The masking algorithm follows the New API reference implementation
//! (`relaykit/relayconvert/kitutil/mask.go` in QuantumNous/new-api): URLs,
//! bare domain names, IPv4 addresses, and `api_key:` values are masked before
//! upstream-derived error text reaches API clients, and again at read time
//! when a non-admin dashboard user views persisted request-log error detail
//! (SAN-14). Persisted request-log fields keep the raw truncated detail for
//! admin visibility; the server tracing log keeps the unbounded raw detail.

use regex::Regex;
use std::sync::LazyLock;

/// SAN-D2: maximum Unicode scalar values retained by [`truncate_error_detail`].
pub const ERROR_DETAIL_MAX_CHARS: usize = 2048;
const TRUNCATION_SUFFIX: &str = "... (truncated)";

static URL_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(http|https)://[^\s/$.?#].[^\s]*").expect("valid URL regex"));
static DOMAIN_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?\.)+[a-zA-Z]{2,}\b")
        .expect("valid domain regex")
});
static IP_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").expect("valid IPv4 regex"));
static API_KEY_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(['"]?)api_key:([^\s'"]+)(['"]?)"#).expect("valid api_key regex")
});

/// SAN-D1 `MASK`: mask URLs, bare domains, IPv4 addresses, and `api_key:`
/// values in `text`. Idempotent: masked output contains no residual match for
/// the domain/IP/api_key patterns, and re-masking an already-masked URL is a
/// fixed point.
pub fn mask_sensitive_text(text: &str) -> String {
    let masked = URL_PATTERN.replace_all(text, |captures: &regex::Captures<'_>| {
        mask_url(&captures[0])
    });
    let masked = DOMAIN_PATTERN.replace_all(&masked, |captures: &regex::Captures<'_>| {
        mask_host_for_plain_domain(&captures[0])
    });
    let masked = IP_PATTERN.replace_all(&masked, "***.***.***.***");
    API_KEY_PATTERN
        .replace_all(&masked, "${1}api_key:***${3}")
        .into_owned()
}

/// SAN-CFG5: apply [`mask_sensitive_text`] only when the runtime setting
/// `mask_sensitive_info` is enabled; otherwise return `text` unchanged.
pub fn maybe_mask_sensitive_text(text: &str, mask_sensitive_info: bool) -> String {
    if mask_sensitive_info {
        mask_sensitive_text(text)
    } else {
        text.to_string()
    }
}

/// SAN-D2a `GENERIC_QUOTA_TEXT`: the only quota-related wording a downstream
/// client may see. Window sizes, plan tiers, and account identifiers in the
/// raw upstream text never cross this boundary.
pub const GENERIC_QUOTA_TEXT: &str =
    "upstream provider quota exceeded; please retry later or contact the operator";

/// SAN-D2a quota signals, matched on word boundaries over lowercased text.
const QUOTA_SIGNALS: &[&str] = &[
    "insufficient_quota",
    "quota_exceeded",
    "quota exceeded",
    "rate_limit_exceeded",
    "rate limit exceeded",
    "rate_limit_error",
    "too_many_requests",
    "429_resource_exhausted",
    "resource_exhausted",
    "daily_quota",
    "hourly_quota",
    "5 hour quota",
    "5-hour quota",
    "per_hour_quota",
    "monthly_quota",
    "usage_limit_reached",
    "usage limit reached",
    "usage_limit_exceeded",
    "billing_limit_reached",
    "billing_hard_limit_reached",
    "current_quota",
    "over quota",
    "exceeded your current quota",
    "org_monthly_spend_limit",
    "spend_limit_reached",
    "credits_exhausted",
    "credit_balance_too_low",
];

fn text_has_quota_signal(text: &str) -> bool {
    let lowered = text.to_lowercase();
    QUOTA_SIGNALS.iter().any(|signal| {
        let signal = signal.to_lowercase();
        match signal.find(['_', ' ', '-']) {
            // Multi-word signals already carry separators that word-boundary
            // matching would reject (`5 hour quota`); match them as substrings.
            Some(_) => lowered.contains(&signal),
            None => {
                let bytes = lowered.as_bytes();
                let start = match lowered.find(signal.as_str()) {
                    Some(start) => start,
                    None => return false,
                };
                let end = start + signal.len();
                let before_ok = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
                let after_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
                before_ok && after_ok
            }
        }
    })
}

/// SAN-D2a `QUOTA` predicate over the free-text and enumerated fields of one
/// upstream error surface (message, code, type, param — any may be absent).
pub fn error_value_is_quota(
    message: Option<&str>,
    code: Option<&str>,
    error_type: Option<&str>,
    param: Option<&str>,
) -> bool {
    [message, code, error_type, param]
        .into_iter()
        .flatten()
        .any(|field| text_has_quota_signal(field))
}

/// SAN-2a/SAN-4a/SAN-11a: the client-facing text for an upstream error whose
/// `QUOTA` classification holds. Quota wording is replaced even when
/// `mask_sensitive_info` is disabled because the quota text itself is the
/// sensitive surface (window sizes, plan tiers, operator account state).
pub fn sanitize_quota_error_text(raw: &str, mask_sensitive_info: bool) -> String {
    if text_has_quota_signal(raw) {
        GENERIC_QUOTA_TEXT.to_string()
    } else if mask_sensitive_info {
        mask_sensitive_text(raw)
    } else {
        raw.to_string()
    }
}

/// SAN-11a `QUOTA` predicate for one mid-stream `UrpStreamEvent::Error`: the
/// event's own `code` and `message`, plus the nested upstream `error` object's
/// `message`/`code`/`type`/`param` when the decoder attached one.
pub fn stream_error_is_quota(
    code: Option<&str>,
    message: &str,
    extra_body_error: Option<&serde_json::Value>,
) -> bool {
    if text_has_quota_signal(message) || code.is_some_and(text_has_quota_signal) {
        return true;
    }
    let Some(error) = extra_body_error else {
        return false;
    };
    let nested = error
        .get("message")
        .and_then(serde_json::Value::as_str)
        .or_else(|| error.as_str());
    error_value_is_quota(
        nested,
        error.get("code").and_then(serde_json::Value::as_str),
        error.get("type").and_then(serde_json::Value::as_str),
        error.get("param").and_then(serde_json::Value::as_str),
    )
}

/// SAN-D2 `TRUNC`: bound persisted error detail to [`ERROR_DETAIL_MAX_CHARS`]
/// Unicode scalar values, appending a fixed truncation marker when clipped.
pub fn truncate_error_detail(text: &str) -> String {
    match text.char_indices().nth(ERROR_DETAIL_MAX_CHARS) {
        None => text.to_string(),
        Some((byte_index, _)) => format!("{}{}", &text[..byte_index], TRUNCATION_SUFFIX),
    }
}

/// SAN-D1 tail rule: keep two labels for likely country-code TLDs
/// (e.g. `co.uk`, `com.cn`), otherwise keep only the TLD.
fn preserved_tail_len(parts: &[&str]) -> usize {
    if parts.len() < 2 {
        return parts.len();
    }
    let last = parts[parts.len() - 1];
    let second_last = parts[parts.len() - 2];
    if last.len() == 2 && second_last.len() <= 3 {
        2
    } else {
        1
    }
}

/// `MASKHOST`: collapse all subdomain labels into one `***.` prefix.
fn mask_host_for_url(host: &str) -> String {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() < 2 {
        return "***".to_string();
    }
    let tail_len = preserved_tail_len(&parts);
    format!("***.{}", parts[parts.len() - tail_len..].join("."))
}

/// Bare-domain masking reflects subdomain depth with repeated `***.` labels
/// (e.g. `api.openai.com` becomes `***.***.com`).
fn mask_host_for_plain_domain(domain: &str) -> String {
    let parts: Vec<&str> = domain.split('.').collect();
    if parts.len() < 2 {
        return domain.to_string();
    }
    let tail_len = preserved_tail_len(&parts);
    let star_count = (parts.len() - tail_len).max(1);
    format!(
        "{}{}",
        "***.".repeat(star_count),
        parts[parts.len() - tail_len..].join(".")
    )
}

fn mask_url(url_str: &str) -> String {
    let Ok(url) = reqwest::Url::parse(url_str) else {
        return url_str.to_string();
    };
    let Some(host) = url.host_str() else {
        return url_str.to_string();
    };

    let mut result = format!("{}://{}", url.scheme(), mask_host_for_url(host));
    if let Some(port) = url.port() {
        result.push_str(&format!(":{port}"));
    }

    let path = url.path();
    if !path.is_empty() && path != "/" {
        let masked_segments: Vec<&str> = path
            .trim_matches('/')
            .split('/')
            .map(|segment| if segment.is_empty() { "" } else { "***" })
            .collect();
        result.push('/');
        result.push_str(&masked_segments.join("/"));
    } else if path == "/" {
        result.push('/');
    }

    if url.query().is_some_and(|query| !query.is_empty()) {
        let masked_params: Vec<String> = url
            .query_pairs()
            .map(|(key, _)| format!("{key}=***"))
            .collect();
        if masked_params.is_empty() {
            result.push_str("?***");
        } else {
            result.push('?');
            result.push_str(&masked_params.join("&"));
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_full_url_with_account_path() {
        let input = "error sending request for url (https://api.cloudflare.com/client/v4/accounts/ebb3b05a7371fbcbd62bde8264c86cfe/ai/v1/chat/completions)";
        let masked = mask_sensitive_text(input);
        assert!(!masked.contains("cloudflare"), "{masked}");
        assert!(
            !masked.contains("ebb3b05a7371fbcbd62bde8264c86cfe"),
            "{masked}"
        );
        assert!(!masked.contains("accounts"), "{masked}");
        assert!(masked.contains("https://***.com/***"), "{masked}");
    }

    #[test]
    fn masks_url_query_values_and_keeps_keys() {
        let masked = mask_sensitive_text("GET https://api.test.org/v1/users/123?key=secret failed");
        assert!(!masked.contains("secret"), "{masked}");
        assert!(!masked.contains("api.test.org"), "{masked}");
        assert!(
            masked.contains("https://***.org/***/***/***?key=***"),
            "{masked}"
        );
    }

    #[test]
    fn masks_bare_domains_with_subdomain_depth() {
        assert_eq!(mask_sensitive_text("openai.com"), "***.com");
        assert_eq!(mask_sensitive_text("api.openai.com"), "***.***.com");
        assert_eq!(
            mask_sensitive_text("sub.domain.co.uk refused"),
            "***.***.co.uk refused"
        );
    }

    #[test]
    fn masks_ipv4_addresses() {
        assert_eq!(
            mask_sensitive_text("connect to 192.168.1.1 refused"),
            "connect to ***.***.***.*** refused"
        );
    }

    #[test]
    fn masks_api_key_values() {
        assert_eq!(
            mask_sensitive_text("api_key:AIzaSyAAAaUooTUni8AdaOkSRMda30n_Q4vrV70 rejected"),
            "api_key:*** rejected"
        );
    }

    #[test]
    fn masking_is_idempotent() {
        let input = "url (https://api.cloudflare.com/client/v4/accounts/abc/ai?token=x) via 10.0.0.1 and api.openai.com";
        let once = mask_sensitive_text(input);
        assert_eq!(mask_sensitive_text(&once), once);
    }

    #[test]
    fn leaves_plain_text_unchanged() {
        let input = "invalid request: missing field `messages`";
        assert_eq!(mask_sensitive_text(input), input);
    }

    // SAN-CFG5 item 1: the gated wrapper is `MASK` when enabled and the
    // identity when disabled.
    #[test]
    fn maybe_mask_applies_mask_only_when_enabled() {
        let input = "rejected by https://api.cloudflare.com/client/v4/accounts/abc123/ai";
        assert_eq!(
            maybe_mask_sensitive_text(input, true),
            mask_sensitive_text(input)
        );
        assert_eq!(maybe_mask_sensitive_text(input, false), input);
    }

    // SAN-D2a: the word-boundary matcher must not fire on embedded substrings
    // that merely contain a signal as a fragment of a longer word.
    #[test]
    fn quota_signal_requires_word_boundary_for_single_word_signals() {
        assert!(text_has_quota_signal(
            "You exceeded your current quota of 5 hour requests"
        ));
        assert!(text_has_quota_signal("insufficient_quota"));
        assert!(text_has_quota_signal("Error code: 429_resource_exhausted"));
        assert!(text_has_quota_signal("5-hour quota exceeded for this org"));
        assert!(!text_has_quota_signal("quotaless mode is active"));
        assert!(!text_has_quota_signal("invalid request: missing field"));
    }

    // SAN-2a/SAN-11a: quota wording collapses to the fixed generic text even
    // with masking disabled, and never leaks the window size or numbers.
    #[test]
    fn quota_text_is_replaced_by_generic_message_even_unmasked() {
        let raw = "5 hour quota exceeded: you have used 87% of your 2026-09-15 window";
        assert_eq!(sanitize_quota_error_text(raw, false), GENERIC_QUOTA_TEXT);
        assert_eq!(sanitize_quota_error_text(raw, true), GENERIC_QUOTA_TEXT);
        let not_quota = "context length exceeded: 200000 tokens";
        assert_eq!(sanitize_quota_error_text(not_quota, false), not_quota);
        assert_eq!(
            sanitize_quota_error_text(not_quota, true),
            mask_sensitive_text(not_quota)
        );
    }

    #[test]
    fn quota_predicate_covers_code_and_type_fields() {
        assert!(error_value_is_quota(
            Some("request failed"),
            Some("quota_exceeded"),
            None,
            None
        ));
        assert!(error_value_is_quota(
            None,
            None,
            Some("rate_limit_error"),
            None
        ));
        assert!(!error_value_is_quota(
            Some("invalid_api_key"),
            Some("authentication_error"),
            None,
            None
        ));
    }

    #[test]
    fn truncates_long_detail_at_char_boundary() {
        let short = "x".repeat(ERROR_DETAIL_MAX_CHARS);
        assert_eq!(truncate_error_detail(&short), short);

        let long = "é".repeat(ERROR_DETAIL_MAX_CHARS + 5);
        let truncated = truncate_error_detail(&long);
        assert!(truncated.ends_with(TRUNCATION_SUFFIX));
        assert_eq!(
            truncated.chars().count(),
            ERROR_DETAIL_MAX_CHARS + TRUNCATION_SUFFIX.chars().count()
        );
    }
}
