use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, FixedOffset, SecondsFormat, Utc};
use serde::Deserialize;
use url::Url;
use zeroize::{Zeroize, Zeroizing};

use crate::store_billing::crypto::sign_rsa_sha256_base64;
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
