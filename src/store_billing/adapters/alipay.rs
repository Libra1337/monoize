use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, FixedOffset, SecondsFormat, Utc};
use serde::Deserialize;
use url::Url;
use zeroize::{Zeroize, Zeroizing};

use crate::store_billing::crypto::{sign_rsa_sha256_base64, verify_rsa_sha256_base64};
use crate::store_billing::money::Currency;
use crate::store_billing::payment::{AdapterError, CheckoutAction, CheckoutRequest};

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
