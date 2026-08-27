use std::fmt;
use std::net::IpAddr;

use chrono::{SecondsFormat, Utc};
use serde::Deserialize;
use serde_json::Value;
use url::Url;
use zeroize::{Zeroize, Zeroizing};

use crate::store_billing::crypto::{
    sign_rsa_sha256_base64, verify_rsa_sha256_base64, wechat_decrypt_resource,
};
use crate::store_billing::money::Currency;
use crate::store_billing::payment::CheckoutAction;
use crate::store_billing::payment::{AdapterError, CheckoutRequest};

const WECHAT_API_ORIGIN: &str = "https://api.mch.weixin.qq.com";

#[derive(Clone, Deserialize, Zeroize)]
#[serde(deny_unknown_fields)]
#[zeroize(drop)]
pub struct WechatCredential {
    merchant_id: String,
    app_id: String,
    api_v3_key: String,
    merchant_certificate_serial: String,
    merchant_private_key_pem: String,
    platform_certificate_serial: String,
    platform_public_key_pem: String,
}

impl WechatCredential {
    pub fn from_json(raw: &[u8]) -> Result<Zeroizing<Self>, AdapterError> {
        let credential: Self =
            serde_json::from_slice(raw).map_err(|_| AdapterError::InvalidConfiguration)?;
        credential.validate()?;
        Ok(Zeroizing::new(credential))
    }

    pub fn merchant_id(&self) -> &str {
        &self.merchant_id
    }

    fn validate(&self) -> Result<(), AdapterError> {
        if [
            &self.merchant_id,
            &self.app_id,
            &self.api_v3_key,
            &self.merchant_certificate_serial,
            &self.merchant_private_key_pem,
            &self.platform_certificate_serial,
            &self.platform_public_key_pem,
        ]
        .into_iter()
        .any(|value| value.trim().is_empty())
            || self.api_v3_key.as_bytes().len() != 32
        {
            return Err(AdapterError::InvalidConfiguration);
        }
        Ok(())
    }
}

impl fmt::Debug for WechatCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WechatCredential")
            .field("merchant_id", &self.merchant_id)
            .field("app_id", &self.app_id)
            .field("api_v3_key", &"[REDACTED]")
            .field(
                "merchant_certificate_serial",
                &self.merchant_certificate_serial,
            )
            .field("merchant_private_key_pem", &"[REDACTED]")
            .field(
                "platform_certificate_serial",
                &self.platform_certificate_serial,
            )
            .field("platform_public_key_pem", &"[PUBLIC KEY]")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WechatProduct {
    Native,
    H5,
}

#[derive(Clone, PartialEq, Eq)]
pub struct PreparedWechatCheckout {
    pub endpoint: String,
    pub canonical_path: String,
    pub authorization: Zeroizing<String>,
    pub body: Value,
    pub body_text: String,
    pub expires_at: String,
}

impl fmt::Debug for PreparedWechatCheckout {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedWechatCheckout")
            .field("endpoint", &self.endpoint)
            .field("canonical_path", &self.canonical_path)
            .field("authorization", &"[REDACTED]")
            .field("body", &self.body)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WechatCheckoutResult {
    pub provider_object_id: String,
    pub action: CheckoutAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedWechatPayment {
    pub provider_event_id: String,
    pub provider_transaction_id: String,
    pub order_number: String,
    pub amount_minor: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WechatCallbackError {
    #[error("WeChat callback headers are invalid")]
    InvalidHeaders,
    #[error("WeChat callback timestamp is outside tolerance")]
    TimestampOutsideTolerance,
    #[error("WeChat callback authentication failed")]
    Authentication,
    #[error("WeChat callback resource is invalid")]
    InvalidResource,
    #[error("WeChat callback payment fields are invalid")]
    InvalidPaymentEvent,
}

pub fn wechat_signature_message(
    method: &str,
    canonical_url: &str,
    timestamp: &str,
    nonce: &str,
    body: &str,
) -> String {
    format!("{method}\n{canonical_url}\n{timestamp}\n{nonce}\n{body}\n")
}

pub fn wechat_callback_signature_message(timestamp: &str, nonce: &str, body: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(timestamp.len() + nonce.len() + body.len() + 3);
    message.extend_from_slice(timestamp.as_bytes());
    message.push(b'\n');
    message.extend_from_slice(nonce.as_bytes());
    message.push(b'\n');
    message.extend_from_slice(body);
    message.push(b'\n');
    message
}

pub fn verify_wechat_payment_callback(
    credential: &WechatCredential,
    timestamp: &str,
    nonce: &str,
    certificate_serial: &str,
    signature: &str,
    body: &[u8],
    now_timestamp: i64,
    tolerance_seconds: i64,
) -> Result<VerifiedWechatPayment, WechatCallbackError> {
    #[derive(Deserialize)]
    struct Callback {
        id: String,
        event_type: String,
        resource: EncryptedResource,
    }

    #[derive(Deserialize)]
    struct EncryptedResource {
        original_type: String,
        algorithm: String,
        ciphertext: String,
        associated_data: String,
        nonce: String,
    }

    #[derive(Deserialize)]
    struct PaymentResource {
        appid: String,
        mchid: String,
        out_trade_no: String,
        transaction_id: String,
        trade_state: String,
        amount: PaymentAmount,
    }

    #[derive(Deserialize)]
    struct PaymentAmount {
        total: u64,
        currency: String,
    }

    credential
        .validate()
        .map_err(|_| WechatCallbackError::Authentication)?;
    if timestamp.is_empty()
        || timestamp.len() > 20
        || !timestamp.bytes().all(|byte| byte.is_ascii_digit())
        || nonce.is_empty()
        || nonce.len() > 256
        || nonce.trim() != nonce
        || certificate_serial != credential.platform_certificate_serial
        || signature.is_empty()
        || tolerance_seconds < 0
    {
        return Err(WechatCallbackError::Authentication);
    }
    let timestamp_value = timestamp
        .parse::<i64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or(WechatCallbackError::InvalidHeaders)?;
    if now_timestamp.abs_diff(timestamp_value) > tolerance_seconds as u64 {
        return Err(WechatCallbackError::TimestampOutsideTolerance);
    }
    let message = wechat_callback_signature_message(timestamp, nonce, body);
    verify_rsa_sha256_base64(&credential.platform_public_key_pem, &message, signature)
        .map_err(|_| WechatCallbackError::Authentication)?;

    let callback: Callback =
        serde_json::from_slice(body).map_err(|_| WechatCallbackError::InvalidResource)?;
    if !valid_required(&callback.id) || callback.event_type != "TRANSACTION.SUCCESS" {
        return Err(WechatCallbackError::InvalidPaymentEvent);
    }
    if callback.resource.original_type != "transaction"
        || callback.resource.algorithm != "AEAD_AES_256_GCM"
        || callback.resource.nonce.as_bytes().len() != 12
        || callback.resource.ciphertext.is_empty()
    {
        return Err(WechatCallbackError::InvalidResource);
    }
    let api_v3_key: &[u8; 32] = credential
        .api_v3_key
        .as_bytes()
        .try_into()
        .map_err(|_| WechatCallbackError::Authentication)?;
    let resource_nonce: &[u8; 12] = callback
        .resource
        .nonce
        .as_bytes()
        .try_into()
        .map_err(|_| WechatCallbackError::InvalidResource)?;
    let plaintext = wechat_decrypt_resource(
        api_v3_key,
        resource_nonce,
        callback.resource.associated_data.as_bytes(),
        &callback.resource.ciphertext,
    )
    .map_err(|_| WechatCallbackError::InvalidResource)?;
    let payment: PaymentResource =
        serde_json::from_slice(&plaintext).map_err(|_| WechatCallbackError::InvalidPaymentEvent)?;
    if payment.mchid != credential.merchant_id
        || payment.appid != credential.app_id
        || !valid_required(&payment.out_trade_no)
        || !valid_required(&payment.transaction_id)
        || payment.trade_state != "SUCCESS"
        || payment.amount.total == 0
        || payment.amount.currency != "CNY"
    {
        return Err(WechatCallbackError::InvalidPaymentEvent);
    }
    Ok(VerifiedWechatPayment {
        provider_event_id: callback.id,
        provider_transaction_id: payment.transaction_id,
        order_number: payment.out_trade_no,
        amount_minor: payment.amount.total.to_string(),
    })
}

fn valid_required(value: &str) -> bool {
    !value.is_empty() && value.trim() == value
}

pub fn prepare_checkout_request(
    credential: &WechatCredential,
    request: &CheckoutRequest,
    product: WechatProduct,
    notify_url: Url,
    client_ip: Option<IpAddr>,
    timestamp: i64,
    nonce: &str,
) -> Result<PreparedWechatCheckout, AdapterError> {
    credential.validate()?;
    if request.currency != Currency::CNY
        || notify_url.scheme() != "https"
        || timestamp <= 0
        || nonce.is_empty()
    {
        return Err(AdapterError::InvalidRequest);
    }
    let amount = request
        .amount_minor
        .parse::<u64>()
        .ok()
        .filter(|amount| *amount > 0 && *amount <= i64::MAX as u64)
        .ok_or(AdapterError::InvalidRequest)?;
    let (canonical_path, scene_info) = match product {
        WechatProduct::Native => ("/v3/pay/transactions/native", None),
        WechatProduct::H5 => {
            let client_ip = client_ip.ok_or(AdapterError::InvalidRequest)?;
            (
                "/v3/pay/transactions/h5",
                Some(serde_json::json!({
                    "payer_client_ip": client_ip.to_string(),
                    "h5_info": {"type": "Wap"},
                })),
            )
        }
    };
    let mut body = serde_json::json!({
        "appid": credential.app_id,
        "mchid": credential.merchant_id,
        "description": format!("LynShen Store {}", request.order_number),
        "out_trade_no": request.order_number,
        "notify_url": notify_url.as_str(),
        "amount": {"total": amount, "currency": "CNY"},
    });
    if let Some(scene_info) = scene_info {
        body["scene_info"] = scene_info;
    }
    let body_text = serde_json::to_string(&body).map_err(|_| AdapterError::InvalidRequest)?;
    let timestamp_text = timestamp.to_string();
    let message =
        wechat_signature_message("POST", canonical_path, &timestamp_text, nonce, &body_text);
    let signature =
        sign_rsa_sha256_base64(&credential.merchant_private_key_pem, message.as_bytes())
            .map_err(|_| AdapterError::InvalidConfiguration)?;
    let authorization = Zeroizing::new(format!(
        "WECHATPAY2-SHA256-RSA2048 mchid=\"{}\",nonce_str=\"{}\",timestamp=\"{}\",serial_no=\"{}\",signature=\"{}\"",
        credential.merchant_id,
        nonce,
        timestamp_text,
        credential.merchant_certificate_serial,
        signature,
    ));
    let expires_at = chrono::DateTime::<Utc>::from_timestamp(timestamp + 30 * 60, 0)
        .ok_or(AdapterError::InvalidRequest)?
        .to_rfc3339_opts(SecondsFormat::Secs, true);
    Ok(PreparedWechatCheckout {
        endpoint: format!("{WECHAT_API_ORIGIN}{canonical_path}"),
        canonical_path: canonical_path.to_string(),
        authorization,
        body,
        body_text,
        expires_at,
    })
}

pub fn parse_checkout_response(
    body: &[u8],
    product: WechatProduct,
    order_number: &str,
    expires_at: &str,
) -> Result<WechatCheckoutResult, AdapterError> {
    #[derive(Deserialize)]
    struct Response {
        code_url: Option<String>,
        h5_url: Option<String>,
    }
    let response: Response = serde_json::from_slice(body).map_err(|_| AdapterError::Ambiguous)?;
    if order_number.is_empty() || chrono::DateTime::parse_from_rfc3339(expires_at).is_err() {
        return Err(AdapterError::Ambiguous);
    }
    let action = match product {
        WechatProduct::Native => {
            let payload = response.code_url.ok_or(AdapterError::Ambiguous)?;
            if !payload.starts_with("weixin://") {
                return Err(AdapterError::Ambiguous);
            }
            CheckoutAction::Qr {
                payload,
                expires_at: expires_at.to_string(),
            }
        }
        WechatProduct::H5 => {
            let url = response.h5_url.ok_or(AdapterError::Ambiguous)?;
            let parsed = Url::parse(&url).map_err(|_| AdapterError::Ambiguous)?;
            if parsed.scheme() != "https" || parsed.host_str().is_none() {
                return Err(AdapterError::Ambiguous);
            }
            CheckoutAction::Redirect {
                url,
                expires_at: expires_at.to_string(),
            }
        }
    };
    Ok(WechatCheckoutResult {
        provider_object_id: order_number.to_string(),
        action,
    })
}

pub async fn create_checkout(
    client: &reqwest::Client,
    credential: &WechatCredential,
    request: &CheckoutRequest,
    product: WechatProduct,
    notify_url: Url,
    client_ip: Option<IpAddr>,
) -> Result<WechatCheckoutResult, AdapterError> {
    let timestamp = Utc::now().timestamp();
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let prepared = prepare_checkout_request(
        credential, request, product, notify_url, client_ip, timestamp, &nonce,
    )?;
    let response = client
        .post(&prepared.endpoint)
        .header(
            reqwest::header::AUTHORIZATION,
            prepared.authorization.as_str(),
        )
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::ACCEPT, "application/json")
        .body(prepared.body_text.clone())
        .send()
        .await
        .map_err(|_| AdapterError::Ambiguous)?;
    let status = response.status();
    let body = crate::bounded_response::read_response_body_with_limit(response, 65_536)
        .await
        .map_err(|_| AdapterError::Ambiguous)?;
    if !status.is_success() {
        return Err(classify_checkout_error_response(status, &body));
    }
    parse_checkout_response(&body, product, &request.order_number, &prepared.expires_at)
}

pub fn classify_checkout_error_response(status: reqwest::StatusCode, body: &[u8]) -> AdapterError {
    #[derive(Deserialize)]
    struct ErrorResponse {
        code: String,
        message: String,
    }
    let recognized = serde_json::from_slice::<ErrorResponse>(body)
        .ok()
        .is_some_and(|error| !error.code.trim().is_empty() && !error.message.trim().is_empty());
    if status.is_client_error() && recognized {
        AdapterError::Rejected
    } else {
        AdapterError::Ambiguous
    }
}
