use hmac::{Hmac, KeyInit, Mac};
use monoize::store_billing::adapters::alipay::canonical_alipay_parameters;
use monoize::store_billing::adapters::stripe::{StripeWebhookError, verify_stripe_webhook};
use monoize::store_billing::adapters::wechat::wechat_signature_message;
use monoize::store_billing::payment::{CheckoutAction, validate_return_url};
use sha2::Sha256;
use std::collections::BTreeMap;

fn hmac_hex(secret: &[u8], payload: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
    mac.update(payload);
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn return_urls_require_the_exact_configured_https_origin() {
    assert_eq!(
        validate_return_url(
            "https://lynshen.org",
            "https://lynshen.org/dashboard/store/return?order=1"
        )
        .unwrap()
        .as_str(),
        "https://lynshen.org/dashboard/store/return?order=1"
    );
    for invalid in [
        "http://lynshen.org/dashboard/store/return",
        "https://api.lynshen.org/dashboard/store/return",
        "https://lynshen.org:444/dashboard/store/return",
        "https://user@lynshen.org/dashboard/store/return",
    ] {
        assert!(
            validate_return_url("https://lynshen.org", invalid).is_err(),
            "accepted {invalid}"
        );
    }
}

#[test]
fn checkout_actions_expose_only_browser_safe_fields() {
    let action = CheckoutAction::Redirect {
        url: "https://checkout.stripe.com/c/pay_test".to_string(),
        expires_at: "2026-08-27T00:30:00Z".to_string(),
    };
    let json = serde_json::to_string(&action).unwrap();
    assert!(json.contains("redirect"));
    assert!(!json.contains("secret"));
}

#[test]
fn alipay_canonical_parameters_are_sorted_and_exclude_signature_fields() {
    let parameters = BTreeMap::from([
        ("timestamp".to_string(), "2026-08-27 12:00:00".to_string()),
        ("app_id".to_string(), "app-1".to_string()),
        ("sign".to_string(), "must-not-be-signed".to_string()),
        ("sign_type".to_string(), "RSA2".to_string()),
        ("empty".to_string(), String::new()),
        (
            "biz_content".to_string(),
            "{\"out_trade_no\":\"LS-1\"}".to_string(),
        ),
    ]);

    assert_eq!(
        canonical_alipay_parameters(&parameters),
        "app_id=app-1&biz_content={\"out_trade_no\":\"LS-1\"}&timestamp=2026-08-27 12:00:00"
    );
}

#[test]
fn wechat_signature_message_preserves_required_terminal_newline() {
    assert_eq!(
        wechat_signature_message(
            "POST",
            "/v3/pay/transactions/native",
            "1710000000",
            "nonce-1",
            r#"{"mchid":"merchant-1"}"#,
        ),
        "POST\n/v3/pay/transactions/native\n1710000000\nnonce-1\n{\"mchid\":\"merchant-1\"}\n"
    );
}

#[test]
fn stripe_webhook_checks_timestamp_signature_and_api_version() {
    let secret = b"whsec_test";
    let timestamp = 1_777_000_000_i64;
    let body = br#"{"id":"evt_1","type":"checkout.session.completed","api_version":"2026-08-01"}"#;
    let signed = format!("{timestamp}.{}", std::str::from_utf8(body).unwrap());
    let header = format!("t={timestamp},v1={}", hmac_hex(secret, signed.as_bytes()));

    let event =
        verify_stripe_webhook(secret, &header, body, timestamp + 30, 300, "2026-08-01").unwrap();
    assert_eq!(event.id, "evt_1");
    assert_eq!(event.kind, "checkout.session.completed");

    assert_eq!(
        verify_stripe_webhook(secret, &header, body, timestamp + 301, 300, "2026-08-01")
            .unwrap_err(),
        StripeWebhookError::TimestampOutsideTolerance
    );
    assert_eq!(
        verify_stripe_webhook(secret, &header, body, timestamp, 300, "2025-12-01").unwrap_err(),
        StripeWebhookError::ApiVersionMismatch
    );
}
