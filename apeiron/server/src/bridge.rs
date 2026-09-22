//! Platform bridge client: exchange handoff tokens, read balances, settle
//! debits/refunds (studio-bridge.spec.md SB-7..SB-11).

use serde_json::{json, Value};

#[derive(Clone, Debug)]
pub struct BridgeUser {
    pub user_id: String,
    pub username: String,
    pub display_name: String,
    pub role: String,
    pub balance_nano_usd: String,
    pub balance_unlimited: bool,
}

#[derive(Debug)]
pub enum BridgeError {
    /// The platform rejected the operation permanently (4xx other than below).
    Rejected(String),
    Insufficient,
    InProgress,
    Transport(String),
}

impl BridgeError {
    pub fn message(&self) -> String {
        match self {
            Self::Rejected(m) => m.clone(),
            Self::Insufficient => "insufficient_balance".to_string(),
            Self::InProgress => "operation_in_progress".to_string(),
            Self::Transport(m) => m.clone(),
        }
    }
}

pub struct BridgeClient<'a> {
    http: &'a reqwest::Client,
    base: &'a str,
    token: Option<&'a str>,
}

async fn parse_error(response: reqwest::Response) -> BridgeError {
    let status = response.status().as_u16();
    let body = response
        .json::<Value>()
        .await
        .unwrap_or(Value::Null);
    let code = body
        .pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let message = body
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or("bridge call failed")
        .to_string();
    match (status, code.as_str()) {
        (409, "insufficient_balance") => BridgeError::Insufficient,
        (409, "operation_in_progress") => BridgeError::InProgress,
        _ => BridgeError::Rejected(format!("bridge {status} {code}: {message}")),
    }
}

impl<'a> BridgeClient<'a> {
    pub fn new(
        http: &'a reqwest::Client,
        base: &'a str,
        token: Option<&'a str>,
    ) -> Self {
        Self { http, base, token }
    }

    fn bearer(&self) -> Result<String, BridgeError> {
        self.token
            .map(|token| format!("Bearer {token}"))
            .ok_or_else(|| {
                BridgeError::Transport(
                    "APEIRON_BRIDGE_SERVICE_TOKEN is not configured".to_string(),
                )
            })
    }

    pub async fn exchange(&self, handoff_token: &str) -> Result<BridgeUser, BridgeError> {
        let response = self
            .http
            .post(format!("{}/api/studio-bridge/exchange", self.base))
            .header("authorization", self.bearer()?)
            .json(&json!({ "token": handoff_token }))
            .send()
            .await
            .map_err(|e| BridgeError::Transport(e.to_string()))?;
        if !response.status().is_success() {
            return Err(parse_error(response).await);
        }
        let value: Value = response
            .json()
            .await
            .map_err(|e| BridgeError::Transport(e.to_string()))?;
        Ok(BridgeUser {
            user_id: value
                .get("user_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            username: value
                .get("username")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            display_name: value
                .get("display_name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            role: value
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("user")
                .to_string(),
            balance_nano_usd: value
                .get("balance_nano_usd")
                .and_then(Value::as_str)
                .unwrap_or("0")
                .to_string(),
            balance_unlimited: value
                .get("balance_unlimited")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }

    pub async fn balance(
        &self,
        user_id: &str,
    ) -> Result<(String, bool), BridgeError> {
        let response = self
            .http
            .get(format!("{}/api/studio-bridge/balance", self.base))
            .header("authorization", self.bearer()?)
            .query(&[("user_id", user_id)])
            .send()
            .await
            .map_err(|e| BridgeError::Transport(e.to_string()))?;
        if !response.status().is_success() {
            return Err(parse_error(response).await);
        }
        let value: Value = response
            .json()
            .await
            .map_err(|e| BridgeError::Transport(e.to_string()))?;
        Ok((
            value
                .get("balance_nano_usd")
                .and_then(Value::as_str)
                .unwrap_or("0")
                .to_string(),
            value
                .get("balance_unlimited")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        ))
    }

    /// SB-9/SB-10. Returns Ok(true) when a fresh settlement was applied and
    /// Ok(false) for an idempotent replay of an already-applied key.
    pub async fn settle(
        &self,
        kind: &str,
        user_id: &str,
        amount_nano: i128,
        idempotency_key: &str,
        meta: Value,
    ) -> Result<bool, BridgeError> {
        if amount_nano <= 0 {
            return Ok(false);
        }
        let response = self
            .http
            .post(format!("{}/api/studio-bridge/{kind}", self.base))
            .header("authorization", self.bearer()?)
            .json(&json!({
                "user_id": user_id,
                "amount_nano_usd": amount_nano,
                "idempotency_key": idempotency_key,
                "reason": if kind == "debit" { "apeiron_charge" } else { "apeiron_refund" },
                "meta": meta,
            }))
            .send()
            .await
            .map_err(|e| BridgeError::Transport(e.to_string()))?;
        if !response.status().is_success() {
            return Err(parse_error(response).await);
        }
        let value: Value = response
            .json()
            .await
            .map_err(|e| BridgeError::Transport(e.to_string()))?;
        Ok(value
            .get("applied")
            .and_then(Value::as_bool)
            .unwrap_or(true))
    }
}
