use aes_gcm::aead::{Aead, KeyInit as AesKeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chrono::{TimeZone, Utc};
use hmac::{Hmac, Mac};
use monoize::store_billing::adapters::alipay::{
    AlipayCallbackError, canonical_alipay_parameters, verify_alipay_payment_callback,
};
use monoize::store_billing::adapters::stripe::{
    StripeWebhookError, parse_stripe_payment_event, verify_stripe_webhook,
};
use monoize::store_billing::adapters::wechat::{
    WechatCallbackError, WechatCredential, WechatPlatformVerifier, verify_wechat_payment_callback,
    wechat_callback_signature_message, wechat_signature_message,
};
use monoize::store_billing::crypto::sign_rsa_sha256_base64;
use monoize::store_billing::money::Currency;
use monoize::store_billing::payment::{
    AdapterError, CheckoutAction, PaymentQuery, ProviderPaymentState, ProviderRefundState,
    RefundRequest, validate_return_url,
};
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::rand_core::OsRng;
use rsa::{RsaPrivateKey, RsaPublicKey};
use sha2::Sha256;
use std::collections::BTreeMap;
use url::Url;

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
fn wechat_account_identity_covers_only_merchant_side_credential_fields() {
    fn credential(overrides: serde_json::Value) -> zeroize::Zeroizing<WechatCredential> {
        let mut value = serde_json::json!({
            "merchant_id":"1900000109",
            "app_id":"wx1234567890",
            "api_v3_key":"0123456789abcdef0123456789abcdef",
            "merchant_certificate_serial":"merchant-certificate-1",
            "merchant_private_key_pem":"merchant-private-key-1",
            "platform_certificate_serial":"platform-certificate-1",
            "platform_public_key_pem":"platform-public-key-1"
        });
        for (key, replacement) in overrides.as_object().unwrap() {
            value[key] = replacement.clone();
        }
        WechatCredential::from_json(value.to_string().as_bytes()).unwrap()
    }

    let original = credential(serde_json::json!({}));
    let original_digest = original.account_identity_digest();
    for changed in [
        serde_json::json!({"app_id":"wx-different"}),
        serde_json::json!({"api_v3_key":"abcdef0123456789abcdef0123456789"}),
        serde_json::json!({"merchant_certificate_serial":"merchant-certificate-2"}),
        serde_json::json!({"merchant_private_key_pem":"merchant-private-key-2"}),
    ] {
        assert_ne!(
            credential(changed).account_identity_digest(),
            original_digest
        );
    }
    assert_eq!(
        credential(serde_json::json!({
            "platform_certificate_serial":"platform-certificate-2",
            "platform_public_key_pem":"platform-public-key-2"
        }))
        .account_identity_digest(),
        original_digest
    );
}

#[test]
fn wechat_callback_verifies_platform_signature_and_decrypted_payment() {
    let platform_private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
    let platform_public = RsaPublicKey::from(&platform_private);
    let platform_private_pem = platform_private.to_pkcs8_pem(LineEnding::LF).unwrap();
    let platform_public_pem = platform_public.to_public_key_pem(LineEnding::LF).unwrap();
    let credential = WechatCredential::from_json(
        serde_json::json!({
            "merchant_id":"1900000109",
            "app_id":"wx1234567890",
            "api_v3_key":"0123456789abcdef0123456789abcdef",
            "merchant_certificate_serial":"7777777777777777777777777777777777777777",
            "merchant_private_key_pem":platform_private_pem.as_str(),
            "platform_certificate_serial":"PLATFORM-CERTIFICATE-1",
            "platform_public_key_pem":platform_public_pem
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let resource_nonce = *b"0123456789ab";
    let associated_data = b"transaction";
    let resource = serde_json::to_vec(&serde_json::json!({
        "appid":"wx1234567890",
        "mchid":"1900000109",
        "out_trade_no":"LS-WECHAT-CALLBACK-1",
        "transaction_id":"4200000001202608270001",
        "trade_state":"SUCCESS",
        "amount":{"total":1234,"currency":"CNY"}
    }))
    .unwrap();
    let key = *b"0123456789abcdef0123456789abcdef";
    let ciphertext = Aes256Gcm::new_from_slice(&key)
        .unwrap()
        .encrypt(
            &Nonce::try_from(resource_nonce.as_slice()).unwrap(),
            Payload {
                msg: &resource,
                aad: associated_data,
            },
        )
        .unwrap();
    let body = serde_json::to_vec(&serde_json::json!({
        "id":"event-wechat-1",
        "event_type":"TRANSACTION.SUCCESS",
        "resource":{
            "original_type":"transaction",
            "algorithm":"AEAD_AES_256_GCM",
            "ciphertext":STANDARD.encode(ciphertext),
            "associated_data":"transaction",
            "nonce":"0123456789ab"
        }
    }))
    .unwrap();
    let timestamp = "1777000000";
    let nonce = "callback-nonce-1";
    let message = wechat_callback_signature_message(timestamp, nonce, &body);
    let signature = sign_rsa_sha256_base64(platform_private_pem.as_str(), &message).unwrap();

    let payment = verify_wechat_payment_callback(
        &credential,
        timestamp,
        nonce,
        "PLATFORM-CERTIFICATE-1",
        &signature,
        &body,
        1_777_000_030,
        300,
    )
    .unwrap();
    assert_eq!(payment.provider_event_id, "event-wechat-1");
    assert_eq!(payment.provider_transaction_id, "4200000001202608270001");
    assert_eq!(payment.order_number, "LS-WECHAT-CALLBACK-1");
    assert_eq!(payment.amount_minor, "1234");

    assert_eq!(
        verify_wechat_payment_callback(
            &credential,
            timestamp,
            nonce,
            "OTHER-CERTIFICATE",
            &signature,
            &body,
            1_777_000_030,
            300,
        )
        .unwrap_err(),
        WechatCallbackError::Authentication
    );
    assert_eq!(
        verify_wechat_payment_callback(
            &credential,
            timestamp,
            nonce,
            "PLATFORM-CERTIFICATE-1",
            &signature,
            &body,
            1_777_000_301,
            300,
        )
        .unwrap_err(),
        WechatCallbackError::TimestampOutsideTolerance
    );
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

#[test]
fn alipay_payment_query_verifies_the_exact_signed_response_node() {
    let private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
    let public = RsaPublicKey::from(&private);
    let private_pem = private.to_pkcs8_pem(LineEnding::LF).unwrap();
    let public_pem = public.to_public_key_pem(LineEnding::LF).unwrap();
    let credential = monoize::store_billing::adapters::alipay::AlipayCredential::from_json(
        serde_json::json!({
            "app_id":"2026000000000001",
            "seller_id":"2088000000000001",
            "merchant_private_key_pem":private_pem.as_str(),
            "alipay_public_key_pem":public_pem,
            "environment":"sandbox"
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let query = PaymentQuery {
        provider_object_id: "LS-ALIPAY-QUERY-1".to_string(),
        merchant_order_number: "LS-ALIPAY-QUERY-1".to_string(),
        amount_minor: "1234".to_string(),
        currency: monoize::store_billing::money::Currency::CNY,
    };
    let prepared = monoize::store_billing::adapters::alipay::prepare_payment_query(
        &credential,
        &query,
        Utc.with_ymd_and_hms(2026, 8, 27, 12, 0, 0).unwrap(),
    )
    .unwrap();
    assert!(prepared.endpoint.contains("sandbox"));
    assert_eq!(prepared.fields["method"], "alipay.trade.query");
    assert!(prepared.fields["biz_content"].contains("LS-ALIPAY-QUERY-1"));

    let node = r#"{"code":"10000","msg":"Success","out_trade_no":"LS-ALIPAY-QUERY-1","trade_no":"2026082722001003","trade_status":"TRADE_SUCCESS","total_amount":"12.34","seller_id":"2088000000000001"}"#;
    let signature = sign_rsa_sha256_base64(private_pem.as_str(), node.as_bytes()).unwrap();
    let response = format!(r#"{{"alipay_trade_query_response":{node},"sign":"{signature}"}}"#);
    assert_eq!(
        monoize::store_billing::adapters::alipay::parse_payment_query_response(
            reqwest::StatusCode::OK,
            response.as_bytes(),
            &credential,
            &query,
        )
        .unwrap(),
        ProviderPaymentState::Paid {
            provider_transaction_id: "2026082722001003".to_string()
        }
    );
    let tampered = response.replace("12.34", "12.35");
    assert_eq!(
        monoize::store_billing::adapters::alipay::parse_payment_query_response(
            reqwest::StatusCode::OK,
            tampered.as_bytes(),
            &credential,
            &query,
        )
        .unwrap_err(),
        AdapterError::Verification
    );
}

#[test]
fn wechat_payment_query_signs_request_and_verifies_response() {
    let platform_private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
    let platform_public = RsaPublicKey::from(&platform_private);
    let private_pem = platform_private.to_pkcs8_pem(LineEnding::LF).unwrap();
    let public_pem = platform_public.to_public_key_pem(LineEnding::LF).unwrap();
    let credential = WechatCredential::from_json(
        serde_json::json!({
            "merchant_id":"1900000109",
            "app_id":"wx1234567890",
            "api_v3_key":"0123456789abcdef0123456789abcdef",
            "merchant_certificate_serial":"7777777777777777777777777777777777777777",
            "merchant_private_key_pem":private_pem.as_str(),
            "platform_certificate_serial":"PLATFORM-CERTIFICATE-QUERY",
            "platform_public_key_pem":public_pem
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let query = PaymentQuery {
        provider_object_id: "LS-WECHAT-QUERY-1".to_string(),
        merchant_order_number: "LS-WECHAT-QUERY-1".to_string(),
        amount_minor: "1234".to_string(),
        currency: monoize::store_billing::money::Currency::CNY,
    };
    let prepared = monoize::store_billing::adapters::wechat::prepare_payment_query(
        &credential,
        &query,
        1_777_000_000,
        "query-nonce-1",
    )
    .unwrap();
    assert_eq!(
        prepared.canonical_url,
        "/v3/pay/transactions/out-trade-no/LS-WECHAT-QUERY-1?mchid=1900000109"
    );
    assert!(!format!("{prepared:?}").contains("PRIVATE KEY"));

    let body = serde_json::to_vec(&serde_json::json!({
        "appid":"wx1234567890","mchid":"1900000109",
        "out_trade_no":"LS-WECHAT-QUERY-1","transaction_id":"4200000001202608270004",
        "trade_state":"SUCCESS","amount":{"total":1234,"currency":"CNY"}
    }))
    .unwrap();
    let response_timestamp = "1777000010";
    let response_nonce = "response-nonce-1";
    let signature = sign_rsa_sha256_base64(
        private_pem.as_str(),
        &wechat_callback_signature_message(response_timestamp, response_nonce, &body),
    )
    .unwrap();
    assert_eq!(
        monoize::store_billing::adapters::wechat::parse_payment_query_response(
            reqwest::StatusCode::OK,
            response_timestamp,
            response_nonce,
            "PLATFORM-CERTIFICATE-QUERY",
            &signature,
            &body,
            &credential,
            &query,
            1_777_000_020,
            300,
        )
        .unwrap(),
        ProviderPaymentState::Paid {
            provider_transaction_id: "4200000001202608270004".to_string()
        }
    );

    let refund_body = serde_json::to_vec(&serde_json::json!({
        "appid":"wx1234567890","mchid":"1900000109",
        "out_trade_no":"LS-WECHAT-QUERY-1","transaction_id":"4200000001202608270004",
        "trade_state":"REFUND","amount":{"total":1234,"currency":"CNY"}
    }))
    .unwrap();
    let refund_signature = sign_rsa_sha256_base64(
        private_pem.as_str(),
        &wechat_callback_signature_message(response_timestamp, response_nonce, &refund_body),
    )
    .unwrap();
    assert_eq!(
        monoize::store_billing::adapters::wechat::parse_payment_query_response(
            reqwest::StatusCode::OK,
            response_timestamp,
            response_nonce,
            "PLATFORM-CERTIFICATE-QUERY",
            &refund_signature,
            &refund_body,
            &credential,
            &query,
            1_777_000_020,
            300,
        )
        .unwrap(),
        ProviderPaymentState::Ambiguous
    );

    let not_found_body = br#"{"code":"ORDER_NOT_EXIST","message":"order does not exist"}"#;
    let not_found_signature = sign_rsa_sha256_base64(
        private_pem.as_str(),
        &wechat_callback_signature_message(response_timestamp, response_nonce, not_found_body),
    )
    .unwrap();
    assert_eq!(
        monoize::store_billing::adapters::wechat::parse_payment_query_response(
            reqwest::StatusCode::NOT_FOUND,
            response_timestamp,
            response_nonce,
            "PLATFORM-CERTIFICATE-QUERY",
            &not_found_signature,
            not_found_body,
            &credential,
            &query,
            1_777_000_020,
            300,
        )
        .unwrap(),
        ProviderPaymentState::NotFound
    );
    assert_eq!(
        monoize::store_billing::adapters::wechat::parse_payment_query_response(
            reqwest::StatusCode::NOT_FOUND,
            response_timestamp,
            response_nonce,
            "PLATFORM-CERTIFICATE-QUERY",
            &signature,
            not_found_body,
            &credential,
            &query,
            1_777_000_020,
            300,
        )
        .unwrap_err(),
        AdapterError::Verification
    );

    let wrong_amount_body = serde_json::to_vec(&serde_json::json!({
        "appid":"wx1234567890","mchid":"1900000109",
        "out_trade_no":"LS-WECHAT-QUERY-1","transaction_id":"4200000001202608270004",
        "trade_state":"SUCCESS","amount":{"total":1235,"currency":"CNY"}
    }))
    .unwrap();
    let wrong_amount_signature = sign_rsa_sha256_base64(
        private_pem.as_str(),
        &wechat_callback_signature_message(response_timestamp, response_nonce, &wrong_amount_body),
    )
    .unwrap();
    assert_eq!(
        monoize::store_billing::adapters::wechat::parse_payment_query_response(
            reqwest::StatusCode::OK,
            response_timestamp,
            response_nonce,
            "PLATFORM-CERTIFICATE-QUERY",
            &wrong_amount_signature,
            &wrong_amount_body,
            &credential,
            &query,
            1_777_000_020,
            300,
        )
        .unwrap_err(),
        AdapterError::Verification
    );
}

#[test]
fn wechat_payment_query_accepts_a_rotated_platform_verifier() {
    let merchant_private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
    let merchant_private_pem = merchant_private.to_pkcs8_pem(LineEnding::LF).unwrap();
    let old_platform_private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
    let old_platform_public = RsaPublicKey::from(&old_platform_private)
        .to_public_key_pem(LineEnding::LF)
        .unwrap();
    let credential = WechatCredential::from_json(
        serde_json::json!({
            "merchant_id":"1900000109",
            "app_id":"wx1234567890",
            "api_v3_key":"0123456789abcdef0123456789abcdef",
            "merchant_certificate_serial":"MERCHANT-CERTIFICATE-QUERY",
            "merchant_private_key_pem":merchant_private_pem.as_str(),
            "platform_certificate_serial":"PLATFORM-CERTIFICATE-OLD",
            "platform_public_key_pem":old_platform_public
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let rotated_private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
    let rotated_private_pem = rotated_private.to_pkcs8_pem(LineEnding::LF).unwrap();
    let rotated_public = RsaPublicKey::from(&rotated_private)
        .to_public_key_pem(LineEnding::LF)
        .unwrap();
    let verifier =
        WechatPlatformVerifier::new("PLATFORM-CERTIFICATE-NEW".to_string(), rotated_public)
            .unwrap();
    let query = PaymentQuery {
        provider_object_id: "LS-WECHAT-ROTATED-1".to_string(),
        merchant_order_number: "LS-WECHAT-ROTATED-1".to_string(),
        amount_minor: "1234".to_string(),
        currency: Currency::CNY,
    };
    let body = serde_json::to_vec(&serde_json::json!({
        "appid":"wx1234567890","mchid":"1900000109",
        "out_trade_no":"LS-WECHAT-ROTATED-1","transaction_id":"4200000001202608270099",
        "trade_state":"SUCCESS","amount":{"total":1234,"currency":"CNY"}
    }))
    .unwrap();
    let timestamp = "1777000010";
    let nonce = "rotated-response-nonce";
    let signature = sign_rsa_sha256_base64(
        rotated_private_pem.as_str(),
        &wechat_callback_signature_message(timestamp, nonce, &body),
    )
    .unwrap();

    assert_eq!(
        monoize::store_billing::adapters::wechat::parse_payment_query_response_with_verifier(
            reqwest::StatusCode::OK,
            timestamp,
            nonce,
            "PLATFORM-CERTIFICATE-NEW",
            &signature,
            &body,
            &credential,
            &verifier,
            &query,
            1_777_000_020,
            300,
        )
        .unwrap(),
        ProviderPaymentState::Paid {
            provider_transaction_id: "4200000001202608270099".to_string()
        }
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
fn alipay_refund_create_and_query_use_rsa2_and_verified_cny_contract() {
    let private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
    let public = RsaPublicKey::from(&private);
    let private_pem = private.to_pkcs8_pem(LineEnding::LF).unwrap();
    let public_pem = public.to_public_key_pem(LineEnding::LF).unwrap();
    let credential = monoize::store_billing::adapters::alipay::AlipayCredential::from_json(
        serde_json::json!({
            "app_id":"2026000000000001","seller_id":"2088000000000001",
            "merchant_private_key_pem":private_pem.as_str(),
            "alipay_public_key_pem":public_pem,"environment":"sandbox"
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let request = refund_request(Currency::CNY);
    let prepared = monoize::store_billing::adapters::alipay::prepare_refund_create(
        &credential,
        &request,
        Utc.with_ymd_and_hms(2026, 8, 28, 12, 0, 0).unwrap(),
    )
    .unwrap();
    assert_eq!(prepared.fields["method"], "alipay.trade.refund");
    assert_eq!(prepared.fields["sign_type"], "RSA2");
    assert!(prepared.fields["biz_content"].contains("\"refund_amount\":\"12.34\""));
    assert!(prepared.fields["biz_content"].contains("\"out_request_no\":\"refund-local-1\""));

    let node = r#"{"code":"10000","msg":"Success","out_trade_no":"LS-REFUND-1","trade_no":"provider-transaction-1","out_request_no":"refund-local-1","refund_fee":"12.34"}"#;
    let signature = sign_rsa_sha256_base64(private_pem.as_str(), node.as_bytes()).unwrap();
    let response = format!(r#"{{"alipay_trade_refund_response":{node},"sign":"{signature}"}}"#);
    let parsed = monoize::store_billing::adapters::alipay::parse_refund_create_response(
        reqwest::StatusCode::OK,
        response.as_bytes(),
        &credential,
        &request,
    )
    .unwrap();
    assert_eq!(parsed.state, ProviderRefundState::Succeeded);

    let query = monoize::store_billing::adapters::alipay::prepare_refund_query(
        &credential,
        &request,
        Utc.with_ymd_and_hms(2026, 8, 28, 12, 1, 0).unwrap(),
    )
    .unwrap();
    assert_eq!(query.fields["method"], "alipay.trade.fastpay.refund.query");
    let missing_node =
        r#"{"code":"40004","msg":"Business Failed","sub_code":"ACQ.REFUND_NOT_EXIST"}"#;
    let missing_signature =
        sign_rsa_sha256_base64(private_pem.as_str(), missing_node.as_bytes()).unwrap();
    let missing_response = format!(
        r#"{{"alipay_trade_fastpay_refund_query_response":{missing_node},"sign":"{missing_signature}"}}"#
    );
    assert_eq!(
        monoize::store_billing::adapters::alipay::parse_refund_query_response(
            reqwest::StatusCode::OK,
            missing_response.as_bytes(),
            &credential,
            &request,
        )
        .unwrap_err(),
        AdapterError::Ambiguous
    );
    assert_eq!(
        monoize::store_billing::adapters::alipay::prepare_refund_create(
            &credential,
            &refund_request(Currency::USD),
            Utc::now(),
        )
        .unwrap_err(),
        AdapterError::InvalidRequest
    );
}

#[test]
fn wechat_refund_create_and_query_sign_and_verify_the_merchant_contract() {
    let private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
    let public = RsaPublicKey::from(&private);
    let private_pem = private.to_pkcs8_pem(LineEnding::LF).unwrap();
    let public_pem = public.to_public_key_pem(LineEnding::LF).unwrap();
    let credential = WechatCredential::from_json(
        serde_json::json!({
            "merchant_id":"1900000109","app_id":"wx1234567890",
            "api_v3_key":"0123456789abcdef0123456789abcdef",
            "merchant_certificate_serial":"MERCHANT-REFUND",
            "merchant_private_key_pem":private_pem.as_str(),
            "platform_certificate_serial":"PLATFORM-REFUND",
            "platform_public_key_pem":public_pem
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let request = refund_request(Currency::CNY);
    let prepared = monoize::store_billing::adapters::wechat::prepare_refund_create(
        &credential,
        &request,
        1_777_000_000,
        "refund-nonce",
    )
    .unwrap();
    assert_eq!(prepared.canonical_url, "/v3/refund/domestic/refunds");
    assert_eq!(prepared.body["out_refund_no"], "refund-local-1");
    assert_eq!(prepared.body["amount"]["refund"], 1234);
    assert_eq!(prepared.body["amount"]["total"], 1234);

    let query = monoize::store_billing::adapters::wechat::prepare_refund_query(
        &credential,
        &request,
        1_777_000_001,
        "query-refund-nonce",
    )
    .unwrap();
    assert_eq!(
        query.canonical_url,
        "/v3/refund/domestic/refunds/refund-local-1"
    );
    assert_eq!(
        monoize::store_billing::adapters::wechat::prepare_refund_create(
            &credential,
            &refund_request(Currency::USD),
            1_777_000_000,
            "refund-nonce",
        )
        .unwrap_err(),
        AdapterError::InvalidRequest
    );

    let body = serde_json::to_vec(&serde_json::json!({
        "refund_id":"5030000000202608280001",
        "out_refund_no":"refund-local-1",
        "transaction_id":"provider-transaction-1",
        "out_trade_no":"LS-REFUND-1",
        "status":"SUCCESS",
        "amount":{"refund":1234,"total":1234,"currency":"CNY"}
    }))
    .unwrap();
    let response_timestamp = "1777000010";
    let response_nonce = "refund-response-nonce";
    let signature = sign_rsa_sha256_base64(
        private_pem.as_str(),
        &wechat_callback_signature_message(response_timestamp, response_nonce, &body),
    )
    .unwrap();
    let verifier = credential.platform_verifier().unwrap();
    let parsed = monoize::store_billing::adapters::wechat::parse_refund_response_with_verifier(
        reqwest::StatusCode::OK,
        response_timestamp,
        response_nonce,
        "PLATFORM-REFUND",
        &signature,
        &body,
        &credential,
        &verifier,
        &request,
        1_777_000_020,
        300,
    )
    .unwrap();
    assert_eq!(parsed.state, ProviderRefundState::Succeeded);
    assert_eq!(
        parsed.provider_refund_id.as_deref(),
        Some("5030000000202608280001")
    );

    let missing_body = br#"{"code":"RESOURCE_NOT_EXISTS","message":"refund does not exist"}"#;
    let missing_signature = sign_rsa_sha256_base64(
        private_pem.as_str(),
        &wechat_callback_signature_message(response_timestamp, response_nonce, missing_body),
    )
    .unwrap();
    let missing = monoize::store_billing::adapters::wechat::parse_refund_response_with_verifier(
        reqwest::StatusCode::NOT_FOUND,
        response_timestamp,
        response_nonce,
        "PLATFORM-REFUND",
        &missing_signature,
        missing_body,
        &credential,
        &verifier,
        &request,
        1_777_000_020,
        300,
    )
    .unwrap();
    assert_eq!(missing.state, ProviderRefundState::NotFound);
    assert!(missing.not_found_is_definitive);
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
fn alipay_checkout_builds_a_signed_official_form() {
    let private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
    let public = RsaPublicKey::from(&private);
    let private_pem = private.to_pkcs8_pem(LineEnding::LF).unwrap();
    let public_pem = public.to_public_key_pem(LineEnding::LF).unwrap();
    let credential = monoize::store_billing::adapters::alipay::AlipayCredential::from_json(
        serde_json::json!({
            "app_id":"2026000000000001",
            "seller_id":"2088000000000001",
            "merchant_private_key_pem":private_pem.as_str(),
            "alipay_public_key_pem":public_pem,
            "environment":"sandbox"
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let checkout = monoize::store_billing::payment::CheckoutRequest {
        attempt_id: "attempt-alipay".to_string(),
        order_number: "LS-ALIPAY-1".to_string(),
        amount_minor: "1234".to_string(),
        currency: monoize::store_billing::money::Currency::CNY,
        success_url: Url::parse("https://lynshen.org/dashboard/store?checkout=success").unwrap(),
        cancel_url: Url::parse("https://lynshen.org/dashboard/store?checkout=cancel").unwrap(),
    };
    let result = monoize::store_billing::adapters::alipay::prepare_checkout(
        &credential,
        &checkout,
        monoize::store_billing::adapters::alipay::AlipayProduct::ComputerWeb,
        Url::parse("https://lynshen.org/api/store/callbacks/alipay-1").unwrap(),
        Utc.with_ymd_and_hms(2026, 8, 27, 17, 2, 3).unwrap(),
    )
    .unwrap();

    assert_eq!(result.provider_object_id, "LS-ALIPAY-1");
    let monoize::store_billing::payment::CheckoutAction::Form { action, fields, .. } =
        result.action
    else {
        panic!("expected form")
    };
    assert_eq!(
        action,
        "https://openapi-sandbox.dl.alipaydev.com/gateway.do"
    );
    let fields = fields.into_iter().collect::<BTreeMap<_, _>>();
    assert_eq!(fields["method"], "alipay.trade.page.pay");
    assert_eq!(fields["sign_type"], "RSA2");
    assert!(!fields["sign"].is_empty());
    assert!(fields["biz_content"].contains("\"total_amount\":\"12.34\""));
    let canonical = fields
        .iter()
        .filter(|(key, value)| key.as_str() != "sign" && !value.is_empty())
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    monoize::store_billing::crypto::verify_rsa_sha256_base64(
        &public_pem,
        canonical.as_bytes(),
        &fields["sign"],
    )
    .unwrap();
    let mobile = monoize::store_billing::adapters::alipay::prepare_checkout(
        &credential,
        &checkout,
        monoize::store_billing::adapters::alipay::AlipayProduct::MobileWeb,
        Url::parse("https://lynshen.org/api/store/callbacks/alipay-1").unwrap(),
        Utc.with_ymd_and_hms(2026, 8, 27, 17, 2, 3).unwrap(),
    )
    .unwrap();
    let CheckoutAction::Form { fields, .. } = mobile.action else {
        panic!("expected mobile form")
    };
    let fields = fields.into_iter().collect::<BTreeMap<_, _>>();
    assert_eq!(fields["method"], "alipay.trade.wap.pay");
    assert!(fields["biz_content"].contains("\"product_code\":\"QUICK_WAP_WAY\""));
    assert!(!format!("{credential:?}").contains("PRIVATE KEY"));
}

#[test]
fn alipay_callback_verifies_rsa2_and_exact_payment_fields() {
    let private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
    let public = RsaPublicKey::from(&private);
    let private_pem = private.to_pkcs8_pem(LineEnding::LF).unwrap();
    let public_pem = public.to_public_key_pem(LineEnding::LF).unwrap();
    let credential = monoize::store_billing::adapters::alipay::AlipayCredential::from_json(
        serde_json::json!({
            "app_id":"2026000000000001",
            "seller_id":"2088000000000001",
            "merchant_private_key_pem":private_pem.as_str(),
            "alipay_public_key_pem":public_pem,
            "environment":"sandbox"
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let mut fields = BTreeMap::from([
        ("notify_id".to_string(), "notify-alipay-1".to_string()),
        ("app_id".to_string(), "2026000000000001".to_string()),
        ("seller_id".to_string(), "2088000000000001".to_string()),
        ("out_trade_no".to_string(), "LS-ALIPAY-1".to_string()),
        ("trade_no".to_string(), "2026082722001001".to_string()),
        ("trade_status".to_string(), "TRADE_SUCCESS".to_string()),
        ("total_amount".to_string(), "12.34".to_string()),
        ("charset".to_string(), "utf-8".to_string()),
        ("sign_type".to_string(), "RSA2".to_string()),
    ]);
    let canonical = canonical_alipay_parameters(&fields);
    fields.insert(
        "sign".to_string(),
        monoize::store_billing::crypto::sign_rsa_sha256_base64(&private_pem, canonical.as_bytes())
            .unwrap(),
    );
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields.iter())
        .finish();

    let payment = verify_alipay_payment_callback(&credential, body.as_bytes()).unwrap();
    assert_eq!(payment.provider_event_id, "notify-alipay-1");
    assert_eq!(payment.provider_transaction_id, "2026082722001001");
    assert_eq!(payment.order_number, "LS-ALIPAY-1");
    assert_eq!(payment.amount_minor, "1234");

    let duplicate_body = format!("{body}&trade_no=duplicate");
    assert_eq!(
        verify_alipay_payment_callback(&credential, duplicate_body.as_bytes()).unwrap_err(),
        AlipayCallbackError::InvalidEncoding
    );
    let tampered = body.replace("12.34", "12.35");
    assert_eq!(
        verify_alipay_payment_callback(&credential, tampered.as_bytes()).unwrap_err(),
        AlipayCallbackError::Authentication
    );
}

#[test]
fn wechat_native_checkout_builds_exact_v3_authorization() {
    let private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
    let public = RsaPublicKey::from(&private);
    let private_pem = private.to_pkcs8_pem(LineEnding::LF).unwrap();
    let public_pem = public.to_public_key_pem(LineEnding::LF).unwrap();
    let credential = monoize::store_billing::adapters::wechat::WechatCredential::from_json(
        serde_json::json!({
            "merchant_id":"1900000109",
            "app_id":"wx1234567890",
            "api_v3_key":"0123456789abcdef0123456789abcdef",
            "merchant_certificate_serial":"7777777777777777777777777777777777777777",
            "merchant_private_key_pem":private_pem.as_str(),
            "platform_certificate_serial":"PLATFORM-CERTIFICATE-1",
            "platform_public_key_pem":public_pem
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let checkout = monoize::store_billing::payment::CheckoutRequest {
        attempt_id: "attempt-wechat".to_string(),
        order_number: "LS-WECHAT-1".to_string(),
        amount_minor: "100".to_string(),
        currency: monoize::store_billing::money::Currency::CNY,
        success_url: Url::parse("https://lynshen.org/dashboard/store?checkout=success").unwrap(),
        cancel_url: Url::parse("https://lynshen.org/dashboard/store?checkout=cancel").unwrap(),
    };
    let prepared = monoize::store_billing::adapters::wechat::prepare_checkout_request(
        &credential,
        &checkout,
        monoize::store_billing::adapters::wechat::WechatProduct::Native,
        Url::parse("https://lynshen.org/api/store/callbacks/wechat-1").unwrap(),
        None,
        1_777_000_000,
        "nonce-1",
    )
    .unwrap();

    assert_eq!(prepared.canonical_path, "/v3/pay/transactions/native");
    assert_eq!(
        prepared.endpoint,
        "https://api.mch.weixin.qq.com/v3/pay/transactions/native"
    );
    assert!(
        prepared
            .authorization
            .starts_with("WECHATPAY2-SHA256-RSA2048 ")
    );
    assert!(prepared.authorization.contains("mchid=\"1900000109\""));
    assert_eq!(prepared.body["amount"]["total"], 100);
    assert_eq!(prepared.body["amount"]["currency"], "CNY");
    assert!(!format!("{credential:?}").contains("PRIVATE KEY"));
}

#[test]
fn wechat_checkout_response_returns_qr_or_h5_redirect() {
    let native = monoize::store_billing::adapters::wechat::parse_checkout_response(
        br#"{"code_url":"weixin://wxpay/bizpayurl?pr=test"}"#,
        monoize::store_billing::adapters::wechat::WechatProduct::Native,
        "LS-WECHAT-1",
        "2026-08-28T01:00:00Z",
    )
    .unwrap();
    assert!(matches!(
        native.action,
        CheckoutAction::Qr { payload, .. } if payload.starts_with("weixin://")
    ));

    let h5 = monoize::store_billing::adapters::wechat::parse_checkout_response(
        br#"{"h5_url":"https://wx.tenpay.com/cgi-bin/mmpayweb-bin/checkmweb?prepay_id=test"}"#,
        monoize::store_billing::adapters::wechat::WechatProduct::H5,
        "LS-WECHAT-2",
        "2026-08-28T01:00:00Z",
    )
    .unwrap();
    assert!(matches!(h5.action, CheckoutAction::Redirect { .. }));

    assert_eq!(
        monoize::store_billing::adapters::wechat::parse_checkout_response(
            br#"{"code_url":"https://example.com/not-wechat"}"#,
            monoize::store_billing::adapters::wechat::WechatProduct::Native,
            "LS-WECHAT-3",
            "2026-08-28T01:00:00Z",
        )
        .unwrap_err(),
        monoize::store_billing::payment::AdapterError::Ambiguous
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
