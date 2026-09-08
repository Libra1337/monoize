use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, FixedOffset, SecondsFormat, Utc};
use serde::Deserialize;
use url::Url;
use zeroize::{Zeroize, Zeroizing};

use crate::store_billing::crypto::{sign_rsa_sha256_base64, verify_rsa_sha256_base64};
use crate::store_billing::money::Currency;
use crate::store_billing::payment::{
    AdapterError, CheckoutAction, CheckoutRequest, PaymentQuery, ProviderPaymentState,
    ProviderRefundState, RefundRequest, validate_payment_query,
};

const ALIPAY_PRODUCTION_GATEWAY: &str = "https://openapi.alipay.com/gateway.do";
const ALIPAY_SANDBOX_GATEWAY: &str = "https://openapi-sandbox.dl.alipaydev.com/gateway.do";

#[derive(Clone, Deserialize, Zeroize)]
#[serde(deny_unknown_fields)]
#[zeroize(drop)]
pub struct AlipayCredential {
    app_id: String,
    seller_id: String,
    merchant_private_key_pem: String,
    alipay_public_key_pem: String,
    environment: String,
}

impl AlipayCredential {
    pub fn from_json(raw: &[u8]) -> Result<Zeroizing<Self>, AdapterError> {
        let credential: Self =
            serde_json::from_slice(raw).map_err(|_| AdapterError::InvalidConfiguration)?;
        credential.validate()?;
        Ok(Zeroizing::new(credential))
    }

    pub fn seller_id(&self) -> &str {
        &self.seller_id
    }

    fn validate(&self) -> Result<(), AdapterError> {
        if [
            &self.app_id,
            &self.seller_id,
            &self.merchant_private_key_pem,
            &self.alipay_public_key_pem,
            &self.environment,
        ]
        .into_iter()
        .any(|value| value.trim().is_empty())
            || !matches!(self.environment.as_str(), "production" | "sandbox")
        {
            return Err(AdapterError::InvalidConfiguration);
        }
        Ok(())
    }
}

impl fmt::Debug for AlipayCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AlipayCredential")
            .field("app_id", &self.app_id)
            .field("seller_id", &self.seller_id)
            .field("merchant_private_key_pem", &"[REDACTED]")
            .field("alipay_public_key_pem", &"[REDACTED]")
            .field("environment", &self.environment)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlipayProduct {
    ComputerWeb,
    MobileWeb,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlipayCheckoutResult {
    pub provider_object_id: String,
    pub action: CheckoutAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedAlipayPaymentQuery {
    pub endpoint: String,
    pub fields: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlipayRefundResult {
    pub state: ProviderRefundState,
    pub provider_refund_id: Option<String>,
    pub not_found_is_definitive: bool,
}

pub fn prepare_refund_create(
    credential: &AlipayCredential,
    request: &RefundRequest,
    now: DateTime<Utc>,
) -> Result<PreparedAlipayPaymentQuery, AdapterError> {
    let amount = validate_refund_request(request)?;
    prepare_refund_request(
        credential,
        "alipay.trade.refund",
        serde_json::json!({
            "out_trade_no": request.merchant_order_number,
            "trade_no": request.provider_transaction_id,
            "refund_amount": format_minor_cny(amount),
            "out_request_no": request.idempotency_key,
        }),
        now,
    )
}

pub fn prepare_refund_query(
    credential: &AlipayCredential,
    request: &RefundRequest,
    now: DateTime<Utc>,
) -> Result<PreparedAlipayPaymentQuery, AdapterError> {
    validate_refund_request(request)?;
    prepare_refund_request(
        credential,
        "alipay.trade.fastpay.refund.query",
        serde_json::json!({
            "out_trade_no": request.merchant_order_number,
            "trade_no": request.provider_transaction_id,
            "out_request_no": request.idempotency_key,
        }),
        now,
    )
}

fn prepare_refund_request(
    credential: &AlipayCredential,
    method: &str,
    biz_content: serde_json::Value,
    now: DateTime<Utc>,
) -> Result<PreparedAlipayPaymentQuery, AdapterError> {
    credential.validate()?;
    let china = FixedOffset::east_opt(8 * 60 * 60).ok_or(AdapterError::InvalidConfiguration)?;
    let mut fields = BTreeMap::from([
        ("app_id".to_string(), credential.app_id.clone()),
        ("biz_content".to_string(), biz_content.to_string()),
        ("charset".to_string(), "utf-8".to_string()),
        ("format".to_string(), "JSON".to_string()),
        ("method".to_string(), method.to_string()),
        ("sign_type".to_string(), "RSA2".to_string()),
        (
            "timestamp".to_string(),
            now.with_timezone(&china)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
        ),
        ("version".to_string(), "1.0".to_string()),
    ]);
    let canonical = canonical_alipay_request_parameters(&fields);
    let signature =
        sign_rsa_sha256_base64(&credential.merchant_private_key_pem, canonical.as_bytes())
            .map_err(|_| AdapterError::InvalidConfiguration)?;
    fields.insert("sign".to_string(), signature);
    let endpoint = match credential.environment.as_str() {
        "production" => ALIPAY_PRODUCTION_GATEWAY,
        "sandbox" => ALIPAY_SANDBOX_GATEWAY,
        _ => return Err(AdapterError::InvalidConfiguration),
    };
    Ok(PreparedAlipayPaymentQuery {
        endpoint: endpoint.to_string(),
        fields,
    })
}

pub fn parse_refund_create_response(
    status: reqwest::StatusCode,
    body: &[u8],
    credential: &AlipayCredential,
    request: &RefundRequest,
) -> Result<AlipayRefundResult, AdapterError> {
    parse_refund_response(status, body, credential, request, false)
}

pub fn parse_refund_query_response(
    status: reqwest::StatusCode,
    body: &[u8],
    credential: &AlipayCredential,
    request: &RefundRequest,
) -> Result<AlipayRefundResult, AdapterError> {
    parse_refund_response(status, body, credential, request, true)
}

fn parse_refund_response(
    status: reqwest::StatusCode,
    body: &[u8],
    credential: &AlipayCredential,
    request: &RefundRequest,
    query: bool,
) -> Result<AlipayRefundResult, AdapterError> {
    use serde_json::value::RawValue;
    #[derive(Deserialize)]
    struct CreateEnvelope<'a> {
        #[serde(borrow)]
        alipay_trade_refund_response: &'a RawValue,
        sign: String,
    }
    #[derive(Deserialize)]
    struct QueryEnvelope<'a> {
        #[serde(borrow)]
        alipay_trade_fastpay_refund_query_response: &'a RawValue,
        sign: String,
    }
    #[derive(Deserialize)]
    struct Response {
        code: String,
        sub_code: Option<String>,
        out_trade_no: Option<String>,
        trade_no: Option<String>,
        out_request_no: Option<String>,
        refund_fee: Option<String>,
        refund_amount: Option<String>,
        refund_status: Option<String>,
    }
    credential.validate()?;
    let expected_amount = validate_refund_request(request)?;
    if !status.is_success() {
        return Err(AdapterError::Ambiguous);
    }
    let (raw, signature) = if query {
        let envelope: QueryEnvelope =
            serde_json::from_slice(body).map_err(|_| AdapterError::Verification)?;
        (
            envelope.alipay_trade_fastpay_refund_query_response,
            envelope.sign,
        )
    } else {
        let envelope: CreateEnvelope =
            serde_json::from_slice(body).map_err(|_| AdapterError::Verification)?;
        (envelope.alipay_trade_refund_response, envelope.sign)
    };
    verify_rsa_sha256_base64(
        &credential.alipay_public_key_pem,
        raw.get().as_bytes(),
        &signature,
    )
    .map_err(|_| AdapterError::Verification)?;
    let response: Response =
        serde_json::from_str(raw.get()).map_err(|_| AdapterError::Verification)?;
    if response.code != "10000" {
        let _ = response.sub_code;
        return Err(AdapterError::Ambiguous);
    }
    let amount = response
        .refund_fee
        .as_deref()
        .or(response.refund_amount.as_deref())
        .and_then(|value| parse_cny_minor(value).ok())
        .ok_or(AdapterError::Verification)?;
    if response.out_trade_no.as_deref() != Some(request.merchant_order_number.as_str())
        || response.trade_no.as_deref() != Some(request.provider_transaction_id.as_str())
        || response.out_request_no.as_deref() != Some(request.idempotency_key.as_str())
        || amount != expected_amount
    {
        return Err(AdapterError::Verification);
    }
    let state = if query {
        match response.refund_status.as_deref() {
            Some("REFUND_SUCCESS") => ProviderRefundState::Succeeded,
            Some("REFUND_PROCESSING") => ProviderRefundState::Pending,
            Some("REFUND_FAIL") => ProviderRefundState::Failed,
            _ => ProviderRefundState::Ambiguous,
        }
    } else {
        ProviderRefundState::Succeeded
    };
    Ok(AlipayRefundResult {
        state,
        provider_refund_id: Some(request.idempotency_key.clone()),
        not_found_is_definitive: false,
    })
}

fn validate_refund_request(request: &RefundRequest) -> Result<u64, AdapterError> {
    if request.currency != Currency::CNY
        || !valid_refund_value(&request.provider_transaction_id)
        || !valid_refund_value(&request.merchant_order_number)
        || !valid_refund_value(&request.idempotency_key)
    {
        return Err(AdapterError::InvalidRequest);
    }
    request
        .amount_minor
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or(AdapterError::InvalidRequest)
}

fn valid_refund_value(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && value.trim() == value
}

pub async fn create_refund(
    client: &reqwest::Client,
    credential: &AlipayCredential,
    request: &RefundRequest,
) -> Result<AlipayRefundResult, AdapterError> {
    let prepared = prepare_refund_create(credential, request, Utc::now())?;
    let response = client
        .post(&prepared.endpoint)
        .form(&prepared.fields)
        .send()
        .await
        .map_err(|_| AdapterError::Ambiguous)?;
    let status = response.status();
    let body = crate::bounded_response::read_response_body_with_limit(response, 65_536)
        .await
        .map_err(|_| AdapterError::Ambiguous)?;
    parse_refund_create_response(status, &body, credential, request)
}

pub async fn query_refund(
    client: &reqwest::Client,
    credential: &AlipayCredential,
    request: &RefundRequest,
) -> Result<AlipayRefundResult, AdapterError> {
    let prepared = prepare_refund_query(credential, request, Utc::now())?;
    let response = client
        .post(&prepared.endpoint)
        .form(&prepared.fields)
        .send()
        .await
        .map_err(|_| AdapterError::Ambiguous)?;
    let status = response.status();
    let body = crate::bounded_response::read_response_body_with_limit(response, 65_536)
        .await
        .map_err(|_| AdapterError::Ambiguous)?;
    parse_refund_query_response(status, &body, credential, request)
}

pub fn prepare_payment_query(
    credential: &AlipayCredential,
    query: &PaymentQuery,
    now: DateTime<Utc>,
) -> Result<PreparedAlipayPaymentQuery, AdapterError> {
    credential.validate()?;
    validate_payment_query(query)?;
    if query.currency != Currency::CNY || query.provider_object_id != query.merchant_order_number {
        return Err(AdapterError::InvalidRequest);
    }
    let china = FixedOffset::east_opt(8 * 60 * 60).ok_or(AdapterError::InvalidConfiguration)?;
    let mut fields = BTreeMap::from([
        ("app_id".to_string(), credential.app_id.clone()),
        (
            "biz_content".to_string(),
            serde_json::json!({"out_trade_no": query.merchant_order_number}).to_string(),
        ),
        ("charset".to_string(), "utf-8".to_string()),
        ("format".to_string(), "JSON".to_string()),
        ("method".to_string(), "alipay.trade.query".to_string()),
        ("sign_type".to_string(), "RSA2".to_string()),
        (
            "timestamp".to_string(),
            now.with_timezone(&china)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
        ),
        ("version".to_string(), "1.0".to_string()),
    ]);
    let canonical = canonical_alipay_request_parameters(&fields);
    let signature =
        sign_rsa_sha256_base64(&credential.merchant_private_key_pem, canonical.as_bytes())
            .map_err(|_| AdapterError::InvalidConfiguration)?;
    fields.insert("sign".to_string(), signature);
    let endpoint = match credential.environment.as_str() {
        "production" => ALIPAY_PRODUCTION_GATEWAY,
        "sandbox" => ALIPAY_SANDBOX_GATEWAY,
        _ => return Err(AdapterError::InvalidConfiguration),
    };
    Ok(PreparedAlipayPaymentQuery {
        endpoint: endpoint.to_string(),
        fields,
    })
}

pub fn parse_payment_query_response(
    status: reqwest::StatusCode,
    body: &[u8],
    credential: &AlipayCredential,
    query: &PaymentQuery,
) -> Result<ProviderPaymentState, AdapterError> {
    use serde_json::value::RawValue;

    #[derive(Deserialize)]
    struct Envelope<'a> {
        #[serde(borrow)]
        alipay_trade_query_response: &'a RawValue,
        sign: String,
    }
    #[derive(Deserialize)]
    struct QueryResponse {
        code: String,
        sub_code: Option<String>,
        out_trade_no: Option<String>,
        trade_no: Option<String>,
        trade_status: Option<String>,
        total_amount: Option<String>,
        seller_id: Option<String>,
    }

    credential.validate()?;
    validate_payment_query(query)?;
    if query.currency != Currency::CNY
        || query.provider_object_id != query.merchant_order_number
        || !status.is_success()
    {
        return Err(AdapterError::Ambiguous);
    }
    let envelope: Envelope =
        serde_json::from_slice(body).map_err(|_| AdapterError::Verification)?;
    verify_rsa_sha256_base64(
        &credential.alipay_public_key_pem,
        envelope.alipay_trade_query_response.get().as_bytes(),
        &envelope.sign,
    )
    .map_err(|_| AdapterError::Verification)?;
    let response: QueryResponse = serde_json::from_str(envelope.alipay_trade_query_response.get())
        .map_err(|_| AdapterError::Verification)?;
    if response.code == "40004" && response.sub_code.as_deref() == Some("ACQ.TRADE_NOT_EXIST") {
        return Ok(ProviderPaymentState::NotFound);
    }
    if response.code != "10000"
        || response.out_trade_no.as_deref() != Some(query.merchant_order_number.as_str())
        || response.seller_id.as_deref() != Some(credential.seller_id.as_str())
        || response
            .total_amount
            .as_deref()
            .and_then(|value| parse_cny_minor(value).ok())
            != Some(validate_payment_query(query)?)
    {
        return Err(AdapterError::Verification);
    }
    match response.trade_status.as_deref() {
        Some("WAIT_BUYER_PAY") => Ok(ProviderPaymentState::Unpaid),
        Some("TRADE_CLOSED") => Ok(ProviderPaymentState::Closed),
        Some("TRADE_SUCCESS" | "TRADE_FINISHED") => response
            .trade_no
            .filter(|value| !value.is_empty() && value.trim() == value)
            .map(|provider_transaction_id| ProviderPaymentState::Paid {
                provider_transaction_id,
            })
            .ok_or(AdapterError::Verification),
        _ => Ok(ProviderPaymentState::Ambiguous),
    }
}

pub async fn query_payment(
    client: &reqwest::Client,
    credential: &AlipayCredential,
    query: &PaymentQuery,
) -> Result<ProviderPaymentState, AdapterError> {
    let prepared = prepare_payment_query(credential, query, Utc::now())?;
    let response = client
        .post(&prepared.endpoint)
        .form(&prepared.fields)
        .send()
        .await
        .map_err(|_| AdapterError::Ambiguous)?;
    let status = response.status();
    let body = crate::bounded_response::read_response_body_with_limit(response, 65_536)
        .await
        .map_err(|_| AdapterError::Ambiguous)?;
    parse_payment_query_response(status, &body, credential, query)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedAlipayPayment {
    pub provider_event_id: String,
    pub provider_transaction_id: String,
    pub order_number: String,
    pub amount_minor: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AlipayCallbackError {
    #[error("Alipay callback encoding is invalid")]
    InvalidEncoding,
    #[error("Alipay callback authentication failed")]
    Authentication,
    #[error("Alipay callback payment fields are invalid")]
    InvalidPaymentEvent,
}

pub fn canonical_alipay_parameters(parameters: &BTreeMap<String, String>) -> String {
    parameters
        .iter()
        .filter(|(key, value)| {
            !value.is_empty() && key.as_str() != "sign" && key.as_str() != "sign_type"
        })
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

pub fn verify_alipay_payment_callback(
    credential: &AlipayCredential,
    body: &[u8],
) -> Result<VerifiedAlipayPayment, AlipayCallbackError> {
    credential
        .validate()
        .map_err(|_| AlipayCallbackError::Authentication)?;
    let encoded = std::str::from_utf8(body).map_err(|_| AlipayCallbackError::InvalidEncoding)?;
    let mut parameters = BTreeMap::new();
    for (key, value) in url::form_urlencoded::parse(encoded.as_bytes()) {
        if key.is_empty()
            || parameters
                .insert(key.into_owned(), value.into_owned())
                .is_some()
        {
            return Err(AlipayCallbackError::InvalidEncoding);
        }
    }
    let signature = required_callback_field(&parameters, "sign")?;
    if required_callback_field(&parameters, "sign_type")? != "RSA2" {
        return Err(AlipayCallbackError::InvalidPaymentEvent);
    }
    let canonical = canonical_alipay_parameters(&parameters);
    verify_rsa_sha256_base64(
        &credential.alipay_public_key_pem,
        canonical.as_bytes(),
        signature,
    )
    .map_err(|_| AlipayCallbackError::Authentication)?;
    if required_callback_field(&parameters, "app_id")? != credential.app_id
        || required_callback_field(&parameters, "seller_id")? != credential.seller_id
        || !matches!(
            required_callback_field(&parameters, "trade_status")?,
            "TRADE_SUCCESS" | "TRADE_FINISHED"
        )
        || parameters
            .get("charset")
            .is_some_and(|value| !value.eq_ignore_ascii_case("utf-8"))
    {
        return Err(AlipayCallbackError::InvalidPaymentEvent);
    }
    let amount_minor = parse_cny_minor(required_callback_field(&parameters, "total_amount")?)?;
    Ok(VerifiedAlipayPayment {
        provider_event_id: required_callback_field(&parameters, "notify_id")?.to_string(),
        provider_transaction_id: required_callback_field(&parameters, "trade_no")?.to_string(),
        order_number: required_callback_field(&parameters, "out_trade_no")?.to_string(),
        amount_minor: amount_minor.to_string(),
    })
}

fn required_callback_field<'a>(
    parameters: &'a BTreeMap<String, String>,
    name: &str,
) -> Result<&'a str, AlipayCallbackError> {
    parameters
        .get(name)
        .map(String::as_str)
        .filter(|value| !value.is_empty() && value.trim() == *value)
        .ok_or(AlipayCallbackError::InvalidPaymentEvent)
}

fn parse_cny_minor(value: &str) -> Result<u64, AlipayCallbackError> {
    let (whole, fraction) = value
        .split_once('.')
        .ok_or(AlipayCallbackError::InvalidPaymentEvent)?;
    if whole.is_empty()
        || whole.len() > 16
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() != 2
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(AlipayCallbackError::InvalidPaymentEvent);
    }
    let whole = whole
        .parse::<u64>()
        .map_err(|_| AlipayCallbackError::InvalidPaymentEvent)?;
    let fraction = fraction
        .parse::<u64>()
        .map_err(|_| AlipayCallbackError::InvalidPaymentEvent)?;
    whole
        .checked_mul(100)
        .and_then(|minor| minor.checked_add(fraction))
        .filter(|minor| *minor > 0)
        .ok_or(AlipayCallbackError::InvalidPaymentEvent)
}

pub fn prepare_checkout(
    credential: &AlipayCredential,
    request: &CheckoutRequest,
    product: AlipayProduct,
    notify_url: Url,
    now: DateTime<Utc>,
) -> Result<AlipayCheckoutResult, AdapterError> {
    credential.validate()?;
    if request.currency != Currency::CNY
        || request.success_url.scheme() != "https"
        || notify_url.scheme() != "https"
    {
        return Err(AdapterError::InvalidRequest);
    }
    let amount = request
        .amount_minor
        .parse::<u64>()
        .ok()
        .filter(|amount| *amount > 0)
        .ok_or(AdapterError::InvalidRequest)?;
    let (method, product_code) = match product {
        AlipayProduct::ComputerWeb => ("alipay.trade.page.pay", "FAST_INSTANT_TRADE_PAY"),
        AlipayProduct::MobileWeb => ("alipay.trade.wap.pay", "QUICK_WAP_WAY"),
    };
    let biz_content = serde_json::json!({
        "out_trade_no": request.order_number,
        "product_code": product_code,
        "subject": format!("LynShen Store {}", request.order_number),
        "total_amount": format_minor_cny(amount),
    })
    .to_string();
    let china = FixedOffset::east_opt(8 * 60 * 60).ok_or(AdapterError::InvalidConfiguration)?;
    let mut fields = BTreeMap::from([
        ("app_id".to_string(), credential.app_id.clone()),
        ("biz_content".to_string(), biz_content),
        ("charset".to_string(), "utf-8".to_string()),
        ("format".to_string(), "JSON".to_string()),
        ("method".to_string(), method.to_string()),
        ("notify_url".to_string(), notify_url.to_string()),
        ("return_url".to_string(), request.success_url.to_string()),
        ("sign_type".to_string(), "RSA2".to_string()),
        (
            "timestamp".to_string(),
            now.with_timezone(&china)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
        ),
        ("version".to_string(), "1.0".to_string()),
    ]);
    let canonical = canonical_alipay_request_parameters(&fields);
    let signature =
        sign_rsa_sha256_base64(&credential.merchant_private_key_pem, canonical.as_bytes())
            .map_err(|_| AdapterError::InvalidConfiguration)?;
    fields.insert("sign".to_string(), signature);
    let action = CheckoutAction::Form {
        action: match credential.environment.as_str() {
            "production" => ALIPAY_PRODUCTION_GATEWAY,
            "sandbox" => ALIPAY_SANDBOX_GATEWAY,
            _ => return Err(AdapterError::InvalidConfiguration),
        }
        .to_string(),
        fields: fields.into_iter().collect(),
        expires_at: (now + chrono::Duration::minutes(30))
            .to_rfc3339_opts(SecondsFormat::Secs, true),
    };
    Ok(AlipayCheckoutResult {
        provider_object_id: request.order_number.clone(),
        action,
    })
}

fn canonical_alipay_request_parameters(parameters: &BTreeMap<String, String>) -> String {
    parameters
        .iter()
        .filter(|(key, value)| !value.is_empty() && key.as_str() != "sign")
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

fn format_minor_cny(amount: u64) -> String {
    format!("{}.{:02}", amount / 100, amount % 100)
}
