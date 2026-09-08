use hmac::{Hmac, KeyInit, Mac};
use monoize::store_billing::adapters::stripe::{
    StripeWebhookError, parse_stripe_payment_event, verify_stripe_webhook,
};
use monoize::store_billing::money::Currency;
use monoize::store_billing::payment::{
    AdapterError, CheckoutAction, PaymentQuery, ProviderPaymentState, ProviderRefundState,
    RefundRequest, validate_return_url,
};
use sha2::Sha256;

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
fn stripe_payment_query_requires_the_exact_checkout_contract() {
    let credential = monoize::store_billing::adapters::stripe::StripeCredential::from_json(
        br#"{
            "secret_key":"sk_test_query",
            "publishable_key":"pk_test_query",
            "webhook_signing_secret":"whsec_query",
            "api_version":"2026-08-01",
            "account_id":"acct_query",
            "live_mode":false
        }"#,
    )
    .unwrap();
    let query = PaymentQuery {
        provider_object_id: "cs_query_1".to_string(),
        merchant_order_number: "LS-QUERY-1".to_string(),
        amount_minor: "1234".to_string(),
        currency: monoize::store_billing::money::Currency::CNY,
    };
    let prepared =
        monoize::store_billing::adapters::stripe::prepare_payment_query(&credential, &query)
            .unwrap();
    assert_eq!(
        prepared.endpoint,
        "https://api.stripe.com/v1/checkout/sessions/cs_query_1"
    );
    assert_eq!(prepared.api_version, "2026-08-01");
    assert!(!format!("{prepared:?}").contains("sk_test_query"));

    let paid = br#"{
        "id":"cs_query_1","object":"checkout.session","amount_total":1234,
        "currency":"cny","client_reference_id":"LS-QUERY-1",
        "payment_intent":"pi_query_1","payment_status":"paid","status":"complete"
    }"#;
    assert_eq!(
        monoize::store_billing::adapters::stripe::parse_payment_query_response(
            reqwest::StatusCode::OK,
            paid,
            &query,
        )
        .unwrap(),
        ProviderPaymentState::Paid {
            provider_transaction_id: "pi_query_1".to_string()
        }
    );
    let not_found = br#"{"error":{"type":"invalid_request_error","code":"resource_missing","message":"missing"}}"#;
    assert_eq!(
        monoize::store_billing::adapters::stripe::parse_payment_query_response(
            reqwest::StatusCode::NOT_FOUND,
            not_found,
            &query,
        )
        .unwrap(),
        ProviderPaymentState::NotFound
    );
    assert_eq!(
        monoize::store_billing::adapters::stripe::parse_payment_query_response(
            reqwest::StatusCode::OK,
            br#"{
                "id":"cs_query_1","object":"checkout.session","amount_total":1235,
                "currency":"cny","client_reference_id":"LS-QUERY-1",
                "payment_intent":"pi_query_1","payment_status":"paid","status":"complete"
            }"#,
            &query,
        )
        .unwrap_err(),
        AdapterError::Verification
    );
}

fn refund_request(currency: Currency) -> RefundRequest {
    RefundRequest {
        provider_transaction_id: "provider-transaction-1".to_string(),
        merchant_order_number: "LS-REFUND-1".to_string(),
        amount_minor: "1234".to_string(),
        currency,
        idempotency_key: "refund-local-1".to_string(),
    }
}

#[test]
fn stripe_refund_create_and_query_bind_the_stable_contract() {
    let credential = monoize::store_billing::adapters::stripe::StripeCredential::from_json(
        br#"{
            "secret_key":"sk_test_refund","publishable_key":"pk_test_refund",
            "webhook_signing_secret":"whsec_refund","api_version":"2026-08-01",
            "account_id":"acct_refund","live_mode":false
        }"#,
    )
    .unwrap();
    let request = refund_request(Currency::USD);
    let create =
        monoize::store_billing::adapters::stripe::prepare_refund_create(&credential, &request)
            .unwrap();
    assert_eq!(create.idempotency_key, "refund-local-1");
    assert_eq!(create.account_id, "acct_refund");
    assert_eq!(create.form["payment_intent"], "provider-transaction-1");
    assert_eq!(create.form["amount"], "1234");
    assert_eq!(create.form["metadata[store_refund_id]"], "refund-local-1");

    let created = br#"{
        "id":"re_1","object":"refund","amount":1234,"currency":"usd",
        "payment_intent":"provider-transaction-1","status":"succeeded",
        "metadata":{"store_refund_id":"refund-local-1","store_order_number":"LS-REFUND-1"}
    }"#;
    let parsed = monoize::store_billing::adapters::stripe::parse_refund_create_response(
        reqwest::StatusCode::OK,
        created,
        &request,
    )
    .unwrap();
    assert_eq!(parsed.state, ProviderRefundState::Succeeded);
    assert_eq!(parsed.provider_refund_id.as_deref(), Some("re_1"));

    let query =
        monoize::store_billing::adapters::stripe::prepare_refund_query(&credential, &request, None)
            .unwrap();
    assert!(
        query
            .endpoint
            .contains("payment_intent=provider-transaction-1")
    );
    assert!(!query.endpoint.contains("metadata%5B"));
    let matched = monoize::store_billing::adapters::stripe::parse_refund_query_response(
        reqwest::StatusCode::OK,
        br#"{
            "object":"list",
            "data":[
                {
                    "id":"re_other","object":"refund","amount":1234,"currency":"usd",
                    "payment_intent":"provider-transaction-1","status":"succeeded",
                    "metadata":{"store_refund_id":"another-refund","store_order_number":"LS-REFUND-1"}
                },
                {
                    "id":"re_1","object":"refund","amount":1234,"currency":"usd",
                    "payment_intent":"provider-transaction-1","status":"succeeded",
                    "metadata":{"store_refund_id":"refund-local-1","store_order_number":"LS-REFUND-1"}
                }
            ]
        }"#,
        &request,
        None,
    )
    .unwrap();
    assert_eq!(matched.provider_refund_id.as_deref(), Some("re_1"));
    let missing = monoize::store_billing::adapters::stripe::parse_refund_query_response(
        reqwest::StatusCode::OK,
        br#"{"object":"list","data":[]}"#,
        &request,
        None,
    )
    .unwrap();
    assert_eq!(missing.state, ProviderRefundState::NotFound);
    assert!(missing.not_found_is_definitive);
    for (status, body) in [
        (
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            b"upstream".as_slice(),
        ),
        (reqwest::StatusCode::BAD_REQUEST, b"not-json".as_slice()),
    ] {
        assert_eq!(
            monoize::store_billing::adapters::stripe::parse_refund_create_response(
                status, body, &request,
            )
            .unwrap_err(),
            AdapterError::Ambiguous
        );
    }

    assert_eq!(
        monoize::store_billing::adapters::stripe::parse_refund_create_response(
            reqwest::StatusCode::OK,
            std::str::from_utf8(created)
                .unwrap()
                .replace("1234", "1235")
                .as_bytes(),
            &request,
        )
        .unwrap_err(),
        AdapterError::Verification
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
