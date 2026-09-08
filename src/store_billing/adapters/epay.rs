use std::collections::BTreeMap;
use std::fmt;
use std::net::IpAddr;

use chrono::{DateTime, SecondsFormat, Utc};
use md5::{Digest as _, Md5};
use serde::Deserialize;
use sha2::Sha256;
use subtle::ConstantTimeEq as _;
use url::Url;
use zeroize::{Zeroize, Zeroizing};

use crate::store_billing::money::Currency;
use crate::store_billing::payment::{
    AdapterError, CheckoutAction, CheckoutRequest, PaymentQuery, ProviderPaymentState,
    ProviderRefundState, RefundRequest, validate_payment_query,
};

const RESPONSE_LIMIT_BYTES: usize = 65_536;
const CHECKOUT_VALID_MINUTES: i64 = 30;

/// One EPay payment method. The protocol calls WeChat Pay `wxpay`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EpayMethod {
    Alipay,
    Wxpay,
}

impl EpayMethod {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Alipay => "alipay",
            Self::Wxpay => "wxpay",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "alipay" => Some(Self::Alipay),
            "wxpay" => Some(Self::Wxpay),
            _ => None,
        }
    }
}

/// One EPay device type. The gateway defaults to `pc` when the field is absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpayDevice {
    Pc,
    Mobile,
    Qq,
    Wechat,
    Alipay,
}

impl EpayDevice {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pc => "pc",
            Self::Mobile => "mobile",
            Self::Qq => "qq",
            Self::Wechat => "wechat",
            Self::Alipay => "alipay",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "pc" => Some(Self::Pc),
            "mobile" => Some(Self::Mobile),
            "qq" => Some(Self::Qq),
            "wechat" => Some(Self::Wechat),
            "alipay" => Some(Self::Alipay),
            _ => None,
        }
    }

    /// In-app browsers are checked before the generic mobile markers because every in-app
    /// user agent also contains a mobile marker.
    pub fn from_user_agent(user_agent: Option<&str>) -> Self {
        let Some(user_agent) = user_agent else {
            return Self::Pc;
        };
        let lowered = user_agent.to_ascii_lowercase();
        if lowered.contains("alipayclient") {
            Self::Alipay
        } else if lowered.contains("micromessenger") {
            Self::Wechat
        } else if lowered.contains(" qq/") || lowered.contains("mqqbrowser") {
            Self::Qq
        } else if ["mobile", "android", "iphone", "ipad", "ipod"]
            .iter()
            .any(|marker| lowered.contains(marker))
        {
            Self::Mobile
        } else {
            Self::Pc
        }
    }
}

#[derive(Clone, Deserialize, Zeroize)]
#[serde(deny_unknown_fields)]
#[zeroize(drop)]
pub struct EpayCredential {
    gateway_base_url: String,
    merchant_id: String,
    merchant_key: String,
    #[serde(default)]
    alipay_enabled: bool,
    #[serde(default)]
    wxpay_enabled: bool,
}

impl EpayCredential {
    pub fn from_json(raw: &[u8]) -> Result<Zeroizing<Self>, AdapterError> {
        let credential: Self =
            serde_json::from_slice(raw).map_err(|_| AdapterError::InvalidConfiguration)?;
        credential.validate()?;
        Ok(Zeroizing::new(credential))
    }

    /// The account identity is the gateway host plus merchant ID. One merchant ID is only
    /// unique inside one gateway deployment, so the host must participate in the identity.
    pub fn account_identity(&self) -> String {
        let host = validate_gateway_url(&self.gateway_base_url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_string))
            .unwrap_or_default();
        format!("epay:{host}:{}", self.merchant_id)
    }

    pub fn account_identity_digest(&self) -> String {
        hex_sha256(self.account_identity().as_bytes())
    }

    pub fn merchant_id(&self) -> &str {
        &self.merchant_id
    }

    pub fn method_enabled(&self, method: EpayMethod) -> bool {
        match method {
            EpayMethod::Alipay => self.alipay_enabled,
            EpayMethod::Wxpay => self.wxpay_enabled,
        }
    }

    pub fn enabled_methods(&self) -> Vec<EpayMethod> {
        [EpayMethod::Alipay, EpayMethod::Wxpay]
            .into_iter()
            .filter(|method| self.method_enabled(*method))
            .collect()
    }

    pub fn validate(&self) -> Result<(), AdapterError> {
        if [
            &self.gateway_base_url,
            &self.merchant_id,
            &self.merchant_key,
        ]
        .into_iter()
        .any(|value| value.trim().is_empty() || value.trim() != value)
            || self.merchant_id.len() > 64
            || self.merchant_key.len() > 256
            || !self
                .merchant_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric())
            || !self
                .merchant_key
                .bytes()
                .all(|byte| byte.is_ascii_graphic())
            || (!self.alipay_enabled && !self.wxpay_enabled)
        {
            return Err(AdapterError::InvalidConfiguration);
        }
        validate_gateway_url(&self.gateway_base_url)?;
        Ok(())
    }

    fn endpoint(&self, path: &str) -> Result<Url, AdapterError> {
        let base = validate_gateway_url(&self.gateway_base_url)?;
        base.join(path)
            .map_err(|_| AdapterError::InvalidConfiguration)
    }
}

impl fmt::Debug for EpayCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EpayCredential")
            .field("gateway_base_url", &self.gateway_base_url)
            .field("merchant_id", &self.merchant_id)
            .field("merchant_key", &"[REDACTED]")
            .field("alipay_enabled", &self.alipay_enabled)
            .field("wxpay_enabled", &self.wxpay_enabled)
            .finish()
    }
}

/// Accepts an explicit gateway deployment over HTTP or HTTPS. Rejects credential-bearing URLs
/// and destinations that a request must never reach from the server.
pub fn validate_gateway_url(raw: &str) -> Result<Url, AdapterError> {
    let url = Url::parse(raw).map_err(|_| AdapterError::InvalidConfiguration)?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.path().ends_with('/')
    {
        return Err(AdapterError::InvalidConfiguration);
    }
    // `host_str` keeps the brackets around an IPv6 literal, so the typed host is required to
    // recognize an address literal.
    let literal = match url.host().ok_or(AdapterError::InvalidConfiguration)? {
        url::Host::Domain(domain) => {
            if domain.is_empty() || domain.eq_ignore_ascii_case("localhost") {
                return Err(AdapterError::InvalidConfiguration);
            }
            None
        }
        url::Host::Ipv4(address) => Some(IpAddr::V4(address)),
        url::Host::Ipv6(address) => Some(IpAddr::V6(address)),
    };
    if literal.is_some_and(|address| !is_public_address(address)) {
        return Err(AdapterError::InvalidConfiguration);
    }
    Ok(url)
}

/// Rejects loopback, link-local, private, unspecified, multicast, and reserved destinations.
pub fn is_public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(value) => {
            !(value.is_loopback()
                || value.is_private()
                || value.is_link_local()
                || value.is_unspecified()
                || value.is_multicast()
                || value.is_broadcast()
                || value.is_documentation()
                // 100.64.0.0/10 carrier NAT.
                || (value.octets()[0] == 100 && (64..128).contains(&value.octets()[1]))
                // 192.0.0.0/24 IETF protocol assignments.
                || value.octets()[0..3] == [192, 0, 0]
                // 198.18.0.0/15 benchmarking.
                || (value.octets()[0] == 198 && (18..20).contains(&value.octets()[1]))
                // 240.0.0.0/4 reserved.
                || value.octets()[0] >= 240)
        }
        IpAddr::V6(value) => {
            let segments = value.segments();
            !(value.is_loopback()
                || value.is_unspecified()
                || value.is_multicast()
                // fe80::/10 link-local.
                || (segments[0] & 0xffc0) == 0xfe80
                // fc00::/7 unique local.
                || (segments[0] & 0xfe00) == 0xfc00
                // ::ffff:0:0/96 IPv4-mapped destinations bypass the IPv4 rules otherwise.
                || value.to_ipv4_mapped().is_some_and(|mapped| {
                    !is_public_address(IpAddr::V4(mapped))
                }))
        }
    }
}

/// Removes `sign`, `sign_type`, and empty values, orders remaining names by ascending ASCII
/// bytes, and joins unencoded `key=value` pairs with `&`. `BTreeMap` already orders by bytes.
pub fn canonical_epay_parameters(parameters: &BTreeMap<String, String>) -> String {
    parameters
        .iter()
        .filter(|(key, value)| {
            !value.is_empty() && key.as_str() != "sign" && key.as_str() != "sign_type"
        })
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// `sign = md5(canonical + KEY)`. The merchant key is appended without a separator.
pub fn sign_epay_parameters(parameters: &BTreeMap<String, String>, merchant_key: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(canonical_epay_parameters(parameters).as_bytes());
    hasher.update(merchant_key.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn verify_epay_signature(
    parameters: &BTreeMap<String, String>,
    merchant_key: &str,
    provided: &str,
) -> bool {
    let expected = sign_epay_parameters(parameters, merchant_key);
    expected
        .as_bytes()
        .ct_eq(provided.to_ascii_lowercase().as_bytes())
        .into()
}

/// EPay money is CNY yuan with exactly two decimal places derived from integer fen.
pub fn format_fen_as_yuan(fen: u64) -> String {
    format!("{}.{:02}", fen / 100, fen % 100)
}

pub fn parse_yuan_as_fen(value: &str) -> Option<u64> {
    let (whole, fraction) = value.split_once('.')?;
    if whole.is_empty()
        || whole.len() > 16
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() != 2
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    whole
        .parse::<u64>()
        .ok()?
        .checked_mul(100)?
        .checked_add(fraction.parse::<u64>().ok()?)
        .filter(|fen| *fen > 0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedEpayForm {
    pub endpoint: String,
    pub fields: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedEpayQuery {
    pub endpoint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpayCheckoutResult {
    pub provider_object_id: String,
    pub action: CheckoutAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpayRefundResult {
    pub state: ProviderRefundState,
    pub provider_refund_id: Option<String>,
    pub not_found_is_definitive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpayMerchantInfo {
    pub merchant_id: String,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedEpayPayment {
    pub provider_event_id: String,
    pub provider_transaction_id: String,
    pub order_number: String,
    pub amount_minor: String,
    pub method: EpayMethod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EpayCallbackError {
    #[error("EPay callback encoding is invalid")]
    InvalidEncoding,
    #[error("EPay callback authentication failed")]
    Authentication,
    #[error("EPay callback payment fields are invalid")]
    InvalidPaymentEvent,
}

/// Builds the `POST {gateway}/mapi.php` form for one CNY order.
pub fn prepare_checkout(
    credential: &EpayCredential,
    request: &CheckoutRequest,
    method: EpayMethod,
    notify_url: &Url,
    device: EpayDevice,
    client_ip: Option<IpAddr>,
    now: DateTime<Utc>,
) -> Result<PreparedEpayForm, AdapterError> {
    credential.validate()?;
    if !credential.method_enabled(method) {
        return Err(AdapterError::InvalidConfiguration);
    }
    if request.currency != Currency::CNY
        || request.success_url.scheme() != "https"
        || !matches!(notify_url.scheme(), "http" | "https")
        || !valid_protocol_value(&request.order_number)
    {
        return Err(AdapterError::InvalidRequest);
    }
    let fen = request
        .amount_minor
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or(AdapterError::InvalidRequest)?;
    let mut fields = BTreeMap::from([
        ("pid".to_string(), credential.merchant_id.clone()),
        ("type".to_string(), method.as_str().to_string()),
        ("out_trade_no".to_string(), request.order_number.clone()),
        ("notify_url".to_string(), notify_url.to_string()),
        ("return_url".to_string(), request.success_url.to_string()),
        (
            "name".to_string(),
            truncate_utf8_bytes(&format!("Monoize Store {}", request.order_number), 127),
        ),
        ("money".to_string(), format_fen_as_yuan(fen)),
        ("device".to_string(), device.as_str().to_string()),
        ("param".to_string(), request.attempt_id.clone()),
    ]);
    if let Some(client_ip) = client_ip {
        fields.insert("clientip".to_string(), client_ip.to_string());
    }
    let signature = sign_epay_parameters(&fields, &credential.merchant_key);
    fields.insert("sign".to_string(), signature);
    fields.insert("sign_type".to_string(), "MD5".to_string());
    let _ = now;
    Ok(PreparedEpayForm {
        endpoint: credential.endpoint("mapi.php")?.to_string(),
        fields,
    })
}

#[derive(Deserialize)]
struct MapiResponse {
    code: i64,
    #[serde(default)]
    trade_no: Option<String>,
    #[serde(default)]
    payurl: Option<String>,
    #[serde(default)]
    qrcode: Option<String>,
    #[serde(default)]
    urlscheme: Option<String>,
}

/// A usable create response carries `code = 1` and exactly one action. An absent or ambiguous
/// action fails creation so no order can be fulfilled without a payable artifact.
pub fn parse_checkout_response(
    status: reqwest::StatusCode,
    body: &[u8],
    request: &CheckoutRequest,
    now: DateTime<Utc>,
) -> Result<EpayCheckoutResult, AdapterError> {
    if status.is_server_error() {
        return Err(AdapterError::Ambiguous);
    }
    if !status.is_success() {
        return Err(AdapterError::Rejected);
    }
    let response: MapiResponse =
        serde_json::from_slice(body).map_err(|_| AdapterError::Verification)?;
    if response.code != 1 {
        return Err(AdapterError::Rejected);
    }
    let expires_at = (now + chrono::Duration::minutes(CHECKOUT_VALID_MINUTES))
        .to_rfc3339_opts(SecondsFormat::Secs, true);
    let qrcode = response.qrcode.filter(|value| valid_protocol_value(value));
    let payurl = response.payurl.filter(|value| valid_protocol_value(value));
    let urlscheme = response
        .urlscheme
        .filter(|value| valid_protocol_value(value));
    let actions = [qrcode.is_some(), payurl.is_some(), urlscheme.is_some()]
        .into_iter()
        .filter(|present| *present)
        .count();
    if actions != 1 {
        return Err(AdapterError::Verification);
    }
    let action = if let Some(payload) = qrcode {
        CheckoutAction::Qr {
            payload,
            expires_at,
        }
    } else {
        let url = payurl.or(urlscheme).expect("one action exists");
        CheckoutAction::Redirect { url, expires_at }
    };
    // The gateway trade number is informational at creation time. The merchant order number is
    // the stable provider object because every later query and callback is keyed by it.
    let _ = response.trade_no;
    Ok(EpayCheckoutResult {
        provider_object_id: request.order_number.clone(),
        action,
    })
}

pub async fn create_checkout(
    client: &reqwest::Client,
    credential: &EpayCredential,
    request: &CheckoutRequest,
    method: EpayMethod,
    notify_url: &Url,
    device: EpayDevice,
    client_ip: Option<IpAddr>,
) -> Result<EpayCheckoutResult, AdapterError> {
    let prepared = prepare_checkout(
        credential,
        request,
        method,
        notify_url,
        device,
        client_ip,
        Utc::now(),
    )?;
    let response = client
        .post(&prepared.endpoint)
        .form(&prepared.fields)
        .send()
        .await
        .map_err(|_| AdapterError::Ambiguous)?;
    let status = response.status();
    let body =
        crate::bounded_response::read_response_body_with_limit(response, RESPONSE_LIMIT_BYTES)
            .await
            .map_err(|_| AdapterError::Ambiguous)?;
    parse_checkout_response(status, &body, request, Utc::now())
}

/// Verifies one documented GET callback query string. Applies success only for
/// `trade_status = TRADE_SUCCESS`.
pub fn verify_payment_callback(
    credential: &EpayCredential,
    query: &str,
) -> Result<VerifiedEpayPayment, EpayCallbackError> {
    credential
        .validate()
        .map_err(|_| EpayCallbackError::Authentication)?;
    let mut parameters = BTreeMap::new();
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if key.is_empty()
            || parameters
                .insert(key.into_owned(), value.into_owned())
                .is_some()
        {
            return Err(EpayCallbackError::InvalidEncoding);
        }
    }
    let signature = required_field(&parameters, "sign")?.to_string();
    if !parameters
        .get("sign_type")
        .is_some_and(|value| value.eq_ignore_ascii_case("MD5"))
    {
        return Err(EpayCallbackError::InvalidPaymentEvent);
    }
    if !verify_epay_signature(&parameters, &credential.merchant_key, &signature) {
        return Err(EpayCallbackError::Authentication);
    }
    if required_field(&parameters, "pid")? != credential.merchant_id
        || required_field(&parameters, "trade_status")? != "TRADE_SUCCESS"
    {
        return Err(EpayCallbackError::InvalidPaymentEvent);
    }
    let method = EpayMethod::from_str(required_field(&parameters, "type")?)
        .filter(|method| credential.method_enabled(*method))
        .ok_or(EpayCallbackError::InvalidPaymentEvent)?;
    let provider_transaction_id = required_field(&parameters, "trade_no")?.to_string();
    let amount_minor = parse_yuan_as_fen(required_field(&parameters, "money")?)
        .ok_or(EpayCallbackError::InvalidPaymentEvent)?;
    Ok(VerifiedEpayPayment {
        // EPay sends no separate notification identifier. The gateway trade number is unique
        // per merchant order and is the only stable event identity available.
        provider_event_id: provider_transaction_id.clone(),
        provider_transaction_id,
        order_number: required_field(&parameters, "out_trade_no")?.to_string(),
        amount_minor: amount_minor.to_string(),
        method,
    })
}

pub fn prepare_payment_query(
    credential: &EpayCredential,
    query: &PaymentQuery,
) -> Result<PreparedEpayQuery, AdapterError> {
    credential.validate()?;
    validate_payment_query(query)?;
    if query.currency != Currency::CNY || query.provider_object_id != query.merchant_order_number {
        return Err(AdapterError::InvalidRequest);
    }
    let mut endpoint = credential.endpoint("api.php")?;
    endpoint
        .query_pairs_mut()
        .append_pair("act", "order")
        .append_pair("pid", &credential.merchant_id)
        .append_pair("key", &credential.merchant_key)
        .append_pair("out_trade_no", &query.merchant_order_number);
    Ok(PreparedEpayQuery {
        endpoint: endpoint.to_string(),
    })
}

#[derive(Deserialize)]
struct OrderQueryResponse {
    code: i64,
    #[serde(default)]
    trade_no: Option<String>,
    #[serde(default)]
    out_trade_no: Option<String>,
    #[serde(default)]
    #[serde(rename = "type")]
    method: Option<String>,
    #[serde(default)]
    pid: Option<serde_json::Value>,
    #[serde(default)]
    money: Option<String>,
    #[serde(default)]
    status: Option<i64>,
}

pub fn parse_payment_query_response(
    status: reqwest::StatusCode,
    body: &[u8],
    credential: &EpayCredential,
    query: &PaymentQuery,
) -> Result<ProviderPaymentState, AdapterError> {
    credential.validate()?;
    let expected_fen = validate_payment_query(query)?;
    if status.is_server_error() {
        return Err(AdapterError::Ambiguous);
    }
    if !status.is_success() {
        return Err(AdapterError::Ambiguous);
    }
    let response: OrderQueryResponse =
        serde_json::from_slice(body).map_err(|_| AdapterError::Verification)?;
    if response.code != 1 {
        // The gateway reports an unknown merchant order with a non-success code. Treating it as
        // definitively absent is only safe when it also returns no order identity.
        if response.out_trade_no.is_none() && response.trade_no.is_none() {
            return Ok(ProviderPaymentState::NotFound);
        }
        return Err(AdapterError::Verification);
    }
    if response.out_trade_no.as_deref() != Some(query.merchant_order_number.as_str())
        || json_identity(response.pid.as_ref()).as_deref() != Some(credential.merchant_id.as_str())
        || response.money.as_deref().and_then(parse_yuan_as_fen) != Some(expected_fen)
        || !response
            .method
            .as_deref()
            .is_some_and(|value| EpayMethod::from_str(value).is_some())
    {
        return Err(AdapterError::Verification);
    }
    match response.status {
        Some(0) => Ok(ProviderPaymentState::Unpaid),
        Some(1) => response
            .trade_no
            .filter(|value| valid_protocol_value(value))
            .map(|provider_transaction_id| ProviderPaymentState::Paid {
                provider_transaction_id,
            })
            .ok_or(AdapterError::Verification),
        _ => Ok(ProviderPaymentState::Ambiguous),
    }
}

pub async fn query_payment(
    client: &reqwest::Client,
    credential: &EpayCredential,
    query: &PaymentQuery,
) -> Result<ProviderPaymentState, AdapterError> {
    let prepared = prepare_payment_query(credential, query)?;
    let response = client
        .get(&prepared.endpoint)
        .send()
        .await
        .map_err(|_| AdapterError::Ambiguous)?;
    let status = response.status();
    let body =
        crate::bounded_response::read_response_body_with_limit(response, RESPONSE_LIMIT_BYTES)
            .await
            .map_err(|_| AdapterError::Ambiguous)?;
    parse_payment_query_response(status, &body, credential, query)
}

/// EPay refunds are full-amount only. The gateway accepts one refund per merchant order.
pub fn prepare_refund(
    credential: &EpayCredential,
    request: &RefundRequest,
) -> Result<PreparedEpayForm, AdapterError> {
    credential.validate()?;
    let fen = validate_refund_request(request)?;
    let endpoint = {
        let mut endpoint = credential.endpoint("api.php")?;
        endpoint.query_pairs_mut().append_pair("act", "refund");
        endpoint.to_string()
    };
    Ok(PreparedEpayForm {
        endpoint,
        fields: BTreeMap::from([
            ("pid".to_string(), credential.merchant_id.clone()),
            ("key".to_string(), credential.merchant_key.clone()),
            (
                "trade_no".to_string(),
                request.provider_transaction_id.clone(),
            ),
            (
                "out_trade_no".to_string(),
                request.merchant_order_number.clone(),
            ),
            ("money".to_string(), format_fen_as_yuan(fen)),
        ]),
    })
}

#[derive(Deserialize)]
struct RefundResponse {
    code: i64,
}

pub fn parse_refund_response(
    status: reqwest::StatusCode,
    body: &[u8],
    credential: &EpayCredential,
    request: &RefundRequest,
) -> Result<EpayRefundResult, AdapterError> {
    credential.validate()?;
    validate_refund_request(request)?;
    if !status.is_success() {
        return Err(AdapterError::Ambiguous);
    }
    let response: RefundResponse =
        serde_json::from_slice(body).map_err(|_| AdapterError::Verification)?;
    let state = if response.code == 1 {
        ProviderRefundState::Succeeded
    } else {
        ProviderRefundState::Failed
    };
    Ok(EpayRefundResult {
        state,
        provider_refund_id: Some(request.idempotency_key.clone()),
        not_found_is_definitive: false,
    })
}

pub async fn create_refund(
    client: &reqwest::Client,
    credential: &EpayCredential,
    request: &RefundRequest,
) -> Result<EpayRefundResult, AdapterError> {
    let prepared = prepare_refund(credential, request)?;
    let response = client
        .post(&prepared.endpoint)
        .form(&prepared.fields)
        .send()
        .await
        .map_err(|_| AdapterError::Ambiguous)?;
    let status = response.status();
    let body =
        crate::bounded_response::read_response_body_with_limit(response, RESPONSE_LIMIT_BYTES)
            .await
            .map_err(|_| AdapterError::Ambiguous)?;
    parse_refund_response(status, &body, credential, request)
}

/// The documented protocol has no refund-status query. Refund state is derived from the payment
/// order query instead, so a refund query is unsupported rather than ambiguous.
pub async fn query_refund(
    _client: &reqwest::Client,
    credential: &EpayCredential,
    request: &RefundRequest,
) -> Result<EpayRefundResult, AdapterError> {
    credential.validate()?;
    validate_refund_request(request)?;
    Err(AdapterError::Unsupported)
}

pub fn prepare_merchant_query(
    credential: &EpayCredential,
) -> Result<PreparedEpayQuery, AdapterError> {
    credential.validate()?;
    let mut endpoint = credential.endpoint("api.php")?;
    endpoint
        .query_pairs_mut()
        .append_pair("act", "query")
        .append_pair("pid", &credential.merchant_id)
        .append_pair("key", &credential.merchant_key);
    Ok(PreparedEpayQuery {
        endpoint: endpoint.to_string(),
    })
}

#[derive(Deserialize)]
struct MerchantQueryResponse {
    code: i64,
    #[serde(default)]
    pid: Option<serde_json::Value>,
    #[serde(default)]
    active: Option<i64>,
}

pub fn parse_merchant_query_response(
    status: reqwest::StatusCode,
    body: &[u8],
    credential: &EpayCredential,
) -> Result<EpayMerchantInfo, AdapterError> {
    credential.validate()?;
    if !status.is_success() {
        return Err(AdapterError::Ambiguous);
    }
    let response: MerchantQueryResponse =
        serde_json::from_slice(body).map_err(|_| AdapterError::Verification)?;
    if response.code != 1
        || json_identity(response.pid.as_ref()).as_deref() != Some(credential.merchant_id.as_str())
    {
        return Err(AdapterError::Verification);
    }
    Ok(EpayMerchantInfo {
        merchant_id: credential.merchant_id.clone(),
        active: response.active == Some(1),
    })
}

pub async fn validate_merchant(
    client: &reqwest::Client,
    credential: &EpayCredential,
) -> Result<EpayMerchantInfo, AdapterError> {
    let prepared = prepare_merchant_query(credential)?;
    let response = client
        .get(&prepared.endpoint)
        .send()
        .await
        .map_err(|_| AdapterError::Ambiguous)?;
    let status = response.status();
    let body =
        crate::bounded_response::read_response_body_with_limit(response, RESPONSE_LIMIT_BYTES)
            .await
            .map_err(|_| AdapterError::Ambiguous)?;
    parse_merchant_query_response(status, &body, credential)
}

pub fn prepare_method_discovery(
    credential: &EpayCredential,
) -> Result<PreparedEpayQuery, AdapterError> {
    credential.validate()?;
    let mut endpoint = credential.endpoint("api.php")?;
    endpoint
        .query_pairs_mut()
        .append_pair("act", "paytype")
        .append_pair("pid", &credential.merchant_id)
        .append_pair("key", &credential.merchant_key);
    Ok(PreparedEpayQuery {
        endpoint: endpoint.to_string(),
    })
}

#[derive(Deserialize)]
struct PaytypeResponse {
    code: i64,
    #[serde(default)]
    data: Vec<PaytypeItem>,
}

#[derive(Deserialize)]
struct PaytypeItem {
    #[serde(default)]
    name: Option<String>,
}

/// Returns the intersection of gateway-reported methods and the Channel method switches, so a
/// disabled switch never produces a user-visible method.
pub fn parse_method_discovery_response(
    status: reqwest::StatusCode,
    body: &[u8],
    credential: &EpayCredential,
) -> Result<Vec<EpayMethod>, AdapterError> {
    credential.validate()?;
    if !status.is_success() {
        return Err(AdapterError::Ambiguous);
    }
    let response: PaytypeResponse =
        serde_json::from_slice(body).map_err(|_| AdapterError::Verification)?;
    if response.code != 1 {
        return Err(AdapterError::Verification);
    }
    let mut methods = response
        .data
        .iter()
        .filter_map(|item| item.name.as_deref())
        .filter_map(EpayMethod::from_str)
        .filter(|method| credential.method_enabled(*method))
        .collect::<Vec<_>>();
    methods.sort_unstable();
    methods.dedup();
    Ok(methods)
}

pub async fn available_methods(
    client: &reqwest::Client,
    credential: &EpayCredential,
) -> Result<Vec<EpayMethod>, AdapterError> {
    let prepared = prepare_method_discovery(credential)?;
    let response = client
        .get(&prepared.endpoint)
        .send()
        .await
        .map_err(|_| AdapterError::Ambiguous)?;
    let status = response.status();
    let body =
        crate::bounded_response::read_response_body_with_limit(response, RESPONSE_LIMIT_BYTES)
            .await
            .map_err(|_| AdapterError::Ambiguous)?;
    parse_method_discovery_response(status, &body, credential)
}

pub fn prepare_settlement_query(
    credential: &EpayCredential,
) -> Result<PreparedEpayQuery, AdapterError> {
    credential.validate()?;
    let mut endpoint = credential.endpoint("api.php")?;
    endpoint
        .query_pairs_mut()
        .append_pair("act", "settle")
        .append_pair("pid", &credential.merchant_id)
        .append_pair("key", &credential.merchant_key);
    Ok(PreparedEpayQuery {
        endpoint: endpoint.to_string(),
    })
}

fn validate_refund_request(request: &RefundRequest) -> Result<u64, AdapterError> {
    if request.currency != Currency::CNY
        || !valid_protocol_value(&request.provider_transaction_id)
        || !valid_protocol_value(&request.merchant_order_number)
        || !valid_protocol_value(&request.idempotency_key)
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

fn required_field<'a>(
    parameters: &'a BTreeMap<String, String>,
    name: &str,
) -> Result<&'a str, EpayCallbackError> {
    parameters
        .get(name)
        .map(String::as_str)
        .filter(|value| valid_protocol_value(value))
        .ok_or(EpayCallbackError::InvalidPaymentEvent)
}

/// The gateway documents `pid` as an integer but real deployments also answer with a string.
fn json_identity(value: Option<&serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn valid_protocol_value(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512 && value.trim() == value
}

/// The gateway truncates `name` at 127 bytes. Truncating on a character boundary keeps the
/// signed value byte-identical to the transmitted value.
fn truncate_utf8_bytes(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn hex_sha256(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credential_json(gateway: &str) -> Vec<u8> {
        serde_json::json!({
            "gateway_base_url": gateway,
            "merchant_id": "1001",
            "merchant_key": "89unJUB8HZ54Hj7x4nUj56HN4nUzUJ8i",
            "alipay_enabled": true,
            "wxpay_enabled": true,
        })
        .to_string()
        .into_bytes()
    }

    fn credential() -> Zeroizing<EpayCredential> {
        EpayCredential::from_json(&credential_json("https://pay.example.com/")).expect("credential")
    }

    fn checkout_request() -> CheckoutRequest {
        CheckoutRequest {
            attempt_id: "attempt-1".to_string(),
            order_number: "20160806151343349".to_string(),
            amount_minor: "100".to_string(),
            currency: Currency::CNY,
            success_url: Url::parse("https://store.example.com/dashboard/store").expect("url"),
            cancel_url: Url::parse("https://store.example.com/dashboard/store").expect("url"),
        }
    }

    fn notify_url() -> Url {
        Url::parse("https://store.example.com/api/store/callbacks/channel-1").expect("url")
    }

    #[test]
    fn credential_requires_a_public_gateway_and_one_enabled_method() {
        assert!(EpayCredential::from_json(&credential_json("https://pay.example.com/")).is_ok());
        for gateway in [
            "https://user:pass@pay.example.com/",
            "https://127.0.0.1/",
            "https://localhost/",
            "https://10.1.2.3/",
            "https://192.168.0.5/",
            "https://169.254.169.254/",
            "https://[::1]/",
            "https://[fd00::1]/",
            "https://[fe80::1]/",
            "https://[::ffff:127.0.0.1]/",
            "ftp://pay.example.com/",
            "https://pay.example.com/?a=b",
            "https://pay.example.com/path",
        ] {
            assert!(
                EpayCredential::from_json(&credential_json(gateway)).is_err(),
                "gateway {gateway} must be rejected"
            );
        }
        let no_method = serde_json::json!({
            "gateway_base_url": "https://pay.example.com/",
            "merchant_id": "1001",
            "merchant_key": "k",
            "alipay_enabled": false,
            "wxpay_enabled": false,
        })
        .to_string();
        assert!(EpayCredential::from_json(no_method.as_bytes()).is_err());
    }

    #[test]
    fn canonicalization_excludes_sign_fields_and_empty_values_in_ascii_order() {
        let parameters = BTreeMap::from([
            ("money".to_string(), "1.00".to_string()),
            ("Name".to_string(), "VIP".to_string()),
            ("pid".to_string(), "1001".to_string()),
            ("param".to_string(), String::new()),
            ("sign".to_string(), "ignored".to_string()),
            ("sign_type".to_string(), "MD5".to_string()),
        ]);
        assert_eq!(
            canonical_epay_parameters(&parameters),
            "Name=VIP&money=1.00&pid=1001"
        );
    }

    #[test]
    fn signature_matches_the_documented_md5_vector() {
        // md5("a=b&c=d&e=f" + "123") == "202cb962ac59075b964b07152d234b70" is the documented
        // example shape; this vector pins the exact concatenation order.
        let parameters = BTreeMap::from([
            ("a".to_string(), "b".to_string()),
            ("c".to_string(), "d".to_string()),
            ("e".to_string(), "f".to_string()),
        ]);
        let expected = {
            let mut hasher = Md5::new();
            hasher.update(b"a=b&c=d&e=f123");
            hasher
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        };
        let signature = sign_epay_parameters(&parameters, "123");
        assert_eq!(signature, expected);
        assert_eq!(signature, signature.to_ascii_lowercase());
        assert!(verify_epay_signature(&parameters, "123", &signature));
        assert!(verify_epay_signature(
            &parameters,
            "123",
            &signature.to_ascii_uppercase()
        ));
        assert!(!verify_epay_signature(&parameters, "124", &signature));
    }

    #[test]
    fn fen_formatting_is_exact_and_reversible() {
        assert_eq!(format_fen_as_yuan(1), "0.01");
        assert_eq!(format_fen_as_yuan(100), "1.00");
        assert_eq!(format_fen_as_yuan(123_456), "1234.56");
        assert_eq!(parse_yuan_as_fen("1234.56"), Some(123_456));
        assert_eq!(parse_yuan_as_fen("1.0"), None);
        assert_eq!(parse_yuan_as_fen("1"), None);
        assert_eq!(parse_yuan_as_fen("0.00"), None);
        assert_eq!(parse_yuan_as_fen("-1.00"), None);
    }

    #[test]
    fn mapi_form_carries_the_documented_signed_fields() {
        let credential = credential();
        let request = checkout_request();
        let prepared = prepare_checkout(
            &credential,
            &request,
            EpayMethod::Wxpay,
            &notify_url(),
            EpayDevice::Pc,
            Some("203.0.113.9".parse().expect("ip")),
            Utc::now(),
        )
        .expect("prepared");
        assert_eq!(prepared.endpoint, "https://pay.example.com/mapi.php");
        assert_eq!(prepared.fields.get("pid").map(String::as_str), Some("1001"));
        assert_eq!(
            prepared.fields.get("type").map(String::as_str),
            Some("wxpay")
        );
        assert_eq!(
            prepared.fields.get("money").map(String::as_str),
            Some("1.00")
        );
        assert_eq!(
            prepared.fields.get("out_trade_no").map(String::as_str),
            Some("20160806151343349")
        );
        assert_eq!(
            prepared.fields.get("clientip").map(String::as_str),
            Some("203.0.113.9")
        );
        assert_eq!(
            prepared.fields.get("device").map(String::as_str),
            Some("pc")
        );
        assert_eq!(
            prepared.fields.get("sign_type").map(String::as_str),
            Some("MD5")
        );
        let mut signed = prepared.fields.clone();
        let signature = signed.remove("sign").expect("sign");
        assert!(verify_epay_signature(
            &signed,
            "89unJUB8HZ54Hj7x4nUj56HN4nUzUJ8i",
            &signature
        ));
    }

    #[test]
    fn checkout_rejects_non_cny_and_disabled_methods() {
        let credential = credential();
        let mut request = checkout_request();
        request.currency = Currency::USD;
        assert_eq!(
            prepare_checkout(
                &credential,
                &request,
                EpayMethod::Alipay,
                &notify_url(),
                EpayDevice::Pc,
                None,
                Utc::now(),
            ),
            Err(AdapterError::InvalidRequest)
        );
        let alipay_only = EpayCredential::from_json(
            serde_json::json!({
                "gateway_base_url": "https://pay.example.com/",
                "merchant_id": "1001",
                "merchant_key": "k",
                "alipay_enabled": true,
                "wxpay_enabled": false,
            })
            .to_string()
            .as_bytes(),
        )
        .expect("credential");
        assert_eq!(
            prepare_checkout(
                &alipay_only,
                &checkout_request(),
                EpayMethod::Wxpay,
                &notify_url(),
                EpayDevice::Pc,
                None,
                Utc::now(),
            ),
            Err(AdapterError::InvalidConfiguration)
        );
    }

    #[test]
    fn create_response_accepts_exactly_one_action() {
        let request = checkout_request();
        let now = Utc::now();
        let qr = parse_checkout_response(
            reqwest::StatusCode::OK,
            br#"{"code":1,"trade_no":"20160806151343349021","qrcode":"weixin://wxpay/bizpayurl?pr=04IPMKM"}"#,
            &request,
            now,
        )
        .expect("qr");
        assert_eq!(qr.provider_object_id, "20160806151343349");
        assert!(matches!(qr.action, CheckoutAction::Qr { .. }));
        let redirect = parse_checkout_response(
            reqwest::StatusCode::OK,
            br#"{"code":1,"payurl":"https://pay.example.com/pay/wxpay/202010903/"}"#,
            &request,
            now,
        )
        .expect("redirect");
        assert!(matches!(redirect.action, CheckoutAction::Redirect { .. }));
        assert_eq!(
            parse_checkout_response(reqwest::StatusCode::OK, br#"{"code":1}"#, &request, now),
            Err(AdapterError::Verification)
        );
        assert_eq!(
            parse_checkout_response(
                reqwest::StatusCode::OK,
                br#"{"code":1,"qrcode":"a","payurl":"https://b.example.com/"}"#,
                &request,
                now,
            ),
            Err(AdapterError::Verification)
        );
        assert_eq!(
            parse_checkout_response(
                reqwest::StatusCode::OK,
                br#"{"code":0,"msg":"merchant disabled"}"#,
                &request,
                now,
            ),
            Err(AdapterError::Rejected)
        );
        assert_eq!(
            parse_checkout_response(
                reqwest::StatusCode::BAD_GATEWAY,
                b"upstream failure",
                &request,
                now,
            ),
            Err(AdapterError::Ambiguous)
        );
    }

    fn signed_callback(overrides: &[(&str, &str)]) -> String {
        let mut parameters = BTreeMap::from([
            ("pid".to_string(), "1001".to_string()),
            ("trade_no".to_string(), "20160806151343349021".to_string()),
            ("out_trade_no".to_string(), "20160806151343349".to_string()),
            ("type".to_string(), "alipay".to_string()),
            ("name".to_string(), "VIP".to_string()),
            ("money".to_string(), "1.00".to_string()),
            ("trade_status".to_string(), "TRADE_SUCCESS".to_string()),
        ]);
        for (key, value) in overrides {
            if value.is_empty() {
                parameters.remove(*key);
            } else {
                parameters.insert((*key).to_string(), (*value).to_string());
            }
        }
        let signature = sign_epay_parameters(&parameters, "89unJUB8HZ54Hj7x4nUj56HN4nUzUJ8i");
        parameters.insert("sign".to_string(), signature);
        parameters.insert("sign_type".to_string(), "MD5".to_string());
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        for (key, value) in &parameters {
            serializer.append_pair(key, value);
        }
        serializer.finish()
    }

    #[test]
    fn callback_verification_accepts_the_documented_success_notification() {
        let credential = credential();
        let verified =
            verify_payment_callback(&credential, &signed_callback(&[])).expect("verified");
        assert_eq!(verified.order_number, "20160806151343349");
        assert_eq!(verified.provider_transaction_id, "20160806151343349021");
        assert_eq!(verified.amount_minor, "100");
        assert_eq!(verified.method, EpayMethod::Alipay);
    }

    #[test]
    fn callback_verification_rejects_tampered_and_unpaid_notifications() {
        let credential = credential();
        let tampered = {
            let query = signed_callback(&[]);
            query.replace("money=1.00", "money=9.00")
        };
        assert_eq!(
            verify_payment_callback(&credential, &tampered),
            Err(EpayCallbackError::Authentication)
        );
        assert_eq!(
            verify_payment_callback(
                &credential,
                &signed_callback(&[("trade_status", "TRADE_CLOSED")])
            ),
            Err(EpayCallbackError::InvalidPaymentEvent)
        );
        assert_eq!(
            verify_payment_callback(&credential, &signed_callback(&[("pid", "2002")])),
            Err(EpayCallbackError::InvalidPaymentEvent)
        );
        assert_eq!(
            verify_payment_callback(&credential, &signed_callback(&[("type", "qqpay")])),
            Err(EpayCallbackError::InvalidPaymentEvent)
        );
        assert_eq!(
            verify_payment_callback(&credential, "pid=1001&out_trade_no=x"),
            Err(EpayCallbackError::InvalidPaymentEvent)
        );
        assert_eq!(
            verify_payment_callback(&credential, "pid=1001&pid=1002&sign=x&sign_type=MD5"),
            Err(EpayCallbackError::InvalidEncoding)
        );
    }

    #[test]
    fn order_query_maps_documented_status_values() {
        let credential = credential();
        let query = PaymentQuery {
            provider_object_id: "20160806151343349".to_string(),
            merchant_order_number: "20160806151343349".to_string(),
            amount_minor: "100".to_string(),
            currency: Currency::CNY,
        };
        let prepared = prepare_payment_query(&credential, &query).expect("prepared");
        assert!(
            prepared
                .endpoint
                .starts_with("https://pay.example.com/api.php?act=order")
        );
        assert!(prepared.endpoint.contains("out_trade_no=20160806151343349"));
        let paid = parse_payment_query_response(
            reqwest::StatusCode::OK,
            br#"{"code":1,"trade_no":"2016080622555342651","out_trade_no":"20160806151343349","type":"alipay","pid":1001,"money":"1.00","status":1}"#,
            &credential,
            &query,
        )
        .expect("paid");
        assert_eq!(
            paid,
            ProviderPaymentState::Paid {
                provider_transaction_id: "2016080622555342651".to_string()
            }
        );
        let unpaid = parse_payment_query_response(
            reqwest::StatusCode::OK,
            br#"{"code":1,"trade_no":"2016080622555342651","out_trade_no":"20160806151343349","type":"alipay","pid":"1001","money":"1.00","status":0}"#,
            &credential,
            &query,
        )
        .expect("unpaid");
        assert_eq!(unpaid, ProviderPaymentState::Unpaid);
        let missing = parse_payment_query_response(
            reqwest::StatusCode::OK,
            br#"{"code":-1,"msg":"order not found"}"#,
            &credential,
            &query,
        )
        .expect("missing");
        assert_eq!(missing, ProviderPaymentState::NotFound);
        assert_eq!(
            parse_payment_query_response(
                reqwest::StatusCode::OK,
                br#"{"code":1,"trade_no":"t","out_trade_no":"20160806151343349","type":"alipay","pid":1001,"money":"9.00","status":1}"#,
                &credential,
                &query,
            ),
            Err(AdapterError::Verification)
        );
        assert_eq!(
            parse_payment_query_response(
                reqwest::StatusCode::BAD_GATEWAY,
                b"gateway down",
                &credential,
                &query,
            ),
            Err(AdapterError::Ambiguous)
        );
    }

    #[test]
    fn refund_uses_the_full_original_amount_and_reports_terminal_state() {
        let credential = credential();
        let request = RefundRequest {
            provider_transaction_id: "20160806151343349021".to_string(),
            merchant_order_number: "20160806151343349".to_string(),
            amount_minor: "150".to_string(),
            currency: Currency::CNY,
            idempotency_key: "refund-1".to_string(),
        };
        let prepared = prepare_refund(&credential, &request).expect("prepared");
        assert_eq!(
            prepared.endpoint,
            "https://pay.example.com/api.php?act=refund"
        );
        assert_eq!(
            prepared.fields.get("money").map(String::as_str),
            Some("1.50")
        );
        let succeeded = parse_refund_response(
            reqwest::StatusCode::OK,
            r#"{"code":1,"msg":"refund ok"}"#.as_bytes(),
            &credential,
            &request,
        )
        .expect("succeeded");
        assert_eq!(succeeded.state, ProviderRefundState::Succeeded);
        let failed = parse_refund_response(
            reqwest::StatusCode::OK,
            br#"{"code":0,"msg":"refund disabled"}"#,
            &credential,
            &request,
        )
        .expect("failed");
        assert_eq!(failed.state, ProviderRefundState::Failed);
        assert_eq!(
            parse_refund_response(
                reqwest::StatusCode::GATEWAY_TIMEOUT,
                b"",
                &credential,
                &request
            ),
            Err(AdapterError::Ambiguous)
        );
        let mut usd = request.clone();
        usd.currency = Currency::USD;
        assert_eq!(
            prepare_refund(&credential, &usd),
            Err(AdapterError::InvalidRequest)
        );
    }

    #[test]
    fn merchant_and_method_discovery_respect_the_channel_switches() {
        let credential = credential();
        let merchant = parse_merchant_query_response(
            reqwest::StatusCode::OK,
            br#"{"code":1,"pid":1001,"active":1,"money":"0.00"}"#,
            &credential,
        )
        .expect("merchant");
        assert_eq!(
            merchant,
            EpayMerchantInfo {
                merchant_id: "1001".to_string(),
                active: true,
            }
        );
        assert_eq!(
            parse_merchant_query_response(
                reqwest::StatusCode::OK,
                br#"{"code":1,"pid":2002,"active":1}"#,
                &credential,
            ),
            Err(AdapterError::Verification)
        );
        let methods = parse_method_discovery_response(
            reqwest::StatusCode::OK,
            r#"{"code":1,"data":[{"name":"wxpay","showname":"WeChat"},{"name":"alipay","showname":"Alipay"},{"name":"qqpay","showname":"QQ"}]}"#.as_bytes(),
            &credential,
        )
        .expect("methods");
        assert_eq!(methods, vec![EpayMethod::Alipay, EpayMethod::Wxpay]);
        let alipay_only = EpayCredential::from_json(
            serde_json::json!({
                "gateway_base_url": "https://pay.example.com/",
                "merchant_id": "1001",
                "merchant_key": "k",
                "alipay_enabled": true,
                "wxpay_enabled": false,
            })
            .to_string()
            .as_bytes(),
        )
        .expect("credential");
        let filtered = parse_method_discovery_response(
            reqwest::StatusCode::OK,
            br#"{"code":1,"data":[{"name":"wxpay"},{"name":"alipay"}]}"#,
            &alipay_only,
        )
        .expect("methods");
        assert_eq!(filtered, vec![EpayMethod::Alipay]);
    }

    #[test]
    fn device_detection_prefers_in_app_browsers_over_generic_mobile_markers() {
        assert_eq!(EpayDevice::from_user_agent(None), EpayDevice::Pc);
        assert_eq!(
            EpayDevice::from_user_agent(Some(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/140.0"
            )),
            EpayDevice::Pc
        );
        assert_eq!(
            EpayDevice::from_user_agent(Some("Mozilla/5.0 (iPhone; CPU iPhone OS 18_0) Safari")),
            EpayDevice::Mobile
        );
        assert_eq!(
            EpayDevice::from_user_agent(Some("Mozilla/5.0 (iPhone) MicroMessenger/8.0.50")),
            EpayDevice::Wechat
        );
        assert_eq!(
            EpayDevice::from_user_agent(Some("Mozilla/5.0 (iPhone) AlipayClient/10.5.0")),
            EpayDevice::Alipay
        );
        assert_eq!(
            EpayDevice::from_user_agent(Some("Mozilla/5.0 (Linux; Android 15) MQQBrowser/14.0")),
            EpayDevice::Qq
        );
    }

    #[test]
    fn account_identity_binds_the_gateway_host_to_the_merchant_id() {
        let first = credential();
        let second = EpayCredential::from_json(&credential_json("https://other.example.com/"))
            .expect("credential");
        assert_ne!(
            first.account_identity_digest(),
            second.account_identity_digest()
        );
        assert_eq!(first.account_identity(), "epay:pay.example.com:1001");
    }

    #[test]
    fn debug_output_never_contains_the_merchant_key() {
        let credential = credential();
        let rendered = format!("{credential:?}");
        assert!(!rendered.contains("89unJUB8HZ54Hj7x4nUj56HN4nUzUJ8i"));
        assert!(rendered.contains("[REDACTED]"));
    }
}
