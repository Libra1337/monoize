use hmac::{Hmac, KeyInit, Mac};
use monoize::store_billing::adapters::alipay::canonical_alipay_parameters;
use monoize::store_billing::adapters::stripe::{
    StripeWebhookError, parse_stripe_payment_event, verify_stripe_webhook,
};
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

#[test]
fn stripe_payment_event_requires_exact_checkout_contract_fields() {
    let body = br#"{
        "id":"evt_paid","type":"checkout.session.completed","api_version":"2026-08-01",
        "account":"acct_1","data":{"object":{
            "id":"cs_1","object":"checkout.session","amount_total":1000,"currency":"cny",
            "client_reference_id":"LS-1","metadata":{"store_attempt_id":"attempt-1"},
            "payment_intent":"pi_1","payment_status":"paid","status":"complete"
        }}
    }"#;
    let verified = monoize::store_billing::adapters::stripe::VerifiedStripeEvent {
        id: "evt_paid".to_string(),
        kind: "checkout.session.completed".to_string(),
        api_version: "2026-08-01".to_string(),
    };

    let event = parse_stripe_payment_event(body, &verified, "acct_1").unwrap();
    assert_eq!(event.attempt_id, "attempt-1");
    assert_eq!(event.order_number, "LS-1");
    assert_eq!(event.checkout_session_id, "cs_1");
    assert_eq!(event.payment_intent_id, "pi_1");
    assert_eq!(event.amount_minor, "1000");

    assert_eq!(
        parse_stripe_payment_event(body, &verified, "acct_other").unwrap_err(),
        StripeWebhookError::InvalidPaymentEvent
    );
}

#[test]
fn stripe_credentials_are_strict_and_redacted() {
    let credential = monoize::store_billing::adapters::stripe::StripeCredential::from_json(
        br#"{
            "secret_key":"sk_test_secret",
            "publishable_key":"pk_test_public",
            "webhook_signing_secret":"whsec_test",
            "api_version":"2026-08-01",
            "account_id":"acct_1",
            "live_mode":false
        }"#,
    )
    .unwrap();
    assert_eq!(credential.account_id(), "acct_1");
    assert!(!format!("{credential:?}").contains("sk_test_secret"));

    assert!(
        monoize::store_billing::adapters::stripe::StripeCredential::from_json(
            br#"{
                "secret_key":"sk_test_secret",
                "publishable_key":"pk_test_public",
                "webhook_signing_secret":"whsec_test",
                "api_version":"2026-08-01",
                "account_id":"acct_1",
                "live_mode":false,
                "endpoint":"https://attacker.example"
            }"#,
        )
        .is_err()
    );
    assert!(
        monoize::store_billing::adapters::stripe::StripeCredential::from_json(
            br#"{
                "secret_key":"sk_test_secret",
                "publishable_key":"pk_test_public",
                "webhook_signing_secret":"whsec_test",
                "api_version":"2026-08-01.",
                "account_id":"acct_1",
                "live_mode":false
            }"#,
        )
        .is_err()
    );
}

#[test]
fn stripe_checkout_request_uses_exact_amount_and_idempotency() {
    let credential = monoize::store_billing::adapters::stripe::StripeCredential::from_json(
        br#"{
            "secret_key":"sk_test_secret",
            "publishable_key":"pk_test_public",
            "webhook_signing_secret":"whsec_test",
            "api_version":"2026-08-01",
            "account_id":"acct_1",
            "live_mode":false
        }"#,
    )
    .unwrap();
    let checkout = monoize::store_billing::payment::CheckoutRequest {
        attempt_id: "attempt-1".to_string(),
        order_number: "LS-ORDER-1".to_string(),
        amount_minor: "1234".to_string(),
        currency: monoize::store_billing::money::Currency::USD,
        success_url: url::Url::parse("https://lynshen.org/dashboard/orders?payment=success")
            .unwrap(),
        cancel_url: url::Url::parse("https://lynshen.org/dashboard/store?payment=cancelled")
            .unwrap(),
    };
    let prepared =
        monoize::store_billing::adapters::stripe::prepare_checkout_request(&credential, &checkout)
            .unwrap();

    assert_eq!(prepared.idempotency_key, "LS-ORDER-1");
    assert_eq!(prepared.authorization.as_str(), "Bearer sk_test_secret");
    assert!(!format!("{prepared:?}").contains("sk_test_secret"));
    assert_eq!(prepared.form.get("mode").unwrap(), "payment");
    assert_eq!(
        prepared
            .form
            .get("line_items[0][price_data][unit_amount]")
            .unwrap(),
        "1234"
    );
    assert_eq!(
        prepared
            .form
            .get("line_items[0][price_data][currency]")
            .unwrap(),
        "usd"
    );
    assert_eq!(
        prepared.form.get("client_reference_id").unwrap(),
        "LS-ORDER-1"
    );
    assert_eq!(
        prepared.form.get("metadata[store_attempt_id]").unwrap(),
        "attempt-1"
    );
}

#[test]
fn stripe_checkout_response_requires_https_and_returns_provider_object() {
    let result = monoize::store_billing::adapters::stripe::parse_checkout_response(
        br#"{"id":"cs_test_1","object":"checkout.session","payment_status":"unpaid","url":"https://checkout.stripe.com/c/pay_test","expires_at":1787788800}"#,
    )
    .unwrap();
    assert_eq!(result.provider_object_id, "cs_test_1");
    assert!(matches!(
        result.action,
        CheckoutAction::Redirect { ref url, .. }
            if url == "https://checkout.stripe.com/c/pay_test"
    ));

    assert!(
        monoize::store_billing::adapters::stripe::parse_checkout_response(
            br#"{"id":"cs_test_1","url":"http://checkout.stripe.com/c/pay_test","expires_at":1787788800}"#,
        )
        .is_err()
    );
}

#[test]
fn stripe_checkout_rejects_only_recognized_client_error_responses() {
    use monoize::store_billing::payment::AdapterError;
    use reqwest::StatusCode;

    assert_eq!(
        monoize::store_billing::adapters::stripe::classify_checkout_error_response(
            StatusCode::BAD_REQUEST,
            br#"{"error":{"type":"invalid_request_error","message":"invalid amount"}}"#,
        ),
        AdapterError::Rejected
    );
    for (status, body) in [
        (
            StatusCode::BAD_REQUEST,
            br#"{"unexpected":true}"#.as_slice(),
        ),
        (
            StatusCode::FOUND,
            br#"{"error":{"type":"redirect","message":"moved"}}"#.as_slice(),
        ),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            br#"{"error":{"type":"api_error","message":"failed"}}"#.as_slice(),
        ),
    ] {
        assert_eq!(
            monoize::store_billing::adapters::stripe::classify_checkout_error_response(
                status, body,
            ),
            AdapterError::Ambiguous
        );
    }
}
