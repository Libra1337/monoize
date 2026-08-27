use chrono::{DateTime, SecondsFormat, Utc};
use sea_orm::ConnectionTrait;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;
use zeroize::Zeroizing;

use super::adapters::alipay::AlipayCredential;
use super::adapters::stripe::StripeCredential;
use super::adapters::wechat::WechatCredential;
use super::crypto::EncryptedSecret;
use super::crypto::PaymentKeyRing;
use crate::db::DbPool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialStatus {
    Active,
    Retired,
}

#[derive(Clone)]
pub struct CredentialVersion {
    pub id: String,
    pub channel_id: String,
    pub adapter_kind: String,
    pub account_identity_digest: String,
    pub status: CredentialStatus,
    encrypted_secret: EncryptedSecret,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CredentialVersionView {
    pub id: String,
    pub channel_id: String,
    pub adapter_kind: String,
    pub account_identity_digest: String,
    pub status: &'static str,
    pub key_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SavedCredentialView {
    pub id: String,
    pub channel_id: String,
    pub adapter_kind: String,
    pub account_identity_digest: String,
    pub status: &'static str,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CredentialStoreError {
    #[error("payment Channel does not exist")]
    ChannelNotFound,
    #[error("payment credential is invalid")]
    InvalidCredential,
    #[error("payment credential encryption is unavailable")]
    EncryptionUnavailable,
    #[error("payment credential storage failed: {0}")]
    Storage(String),
}

#[derive(Clone)]
pub struct CredentialStore {
    db: DbPool,
    key_ring: Arc<PaymentKeyRing>,
}

impl CredentialStore {
    pub fn new(db: DbPool, key_ring: Arc<PaymentKeyRing>) -> Self {
        Self { db, key_ring }
    }

    pub async fn replace(
        &self,
        channel_id: &str,
        credential: Value,
    ) -> Result<SavedCredentialView, CredentialStoreError> {
        let tx = self.db.begin_write().await.map_err(storage)?;
        if self.db.is_sqlite() {
            let locked = tx
                .execute(self.db.stmt(
                    "UPDATE store_payment_channels SET revision = revision WHERE id = $1",
                    vec![channel_id.into()],
                ))
                .await
                .map_err(storage)?;
            if locked.rows_affected() != 1 {
                return Err(CredentialStoreError::ChannelNotFound);
            }
        }
        let lock_clause = if self.db.is_postgres() {
            " FOR UPDATE"
        } else {
            ""
        };
        let channel = tx
            .query_one(self.db.stmt(
                &format!(
                    "SELECT adapter_kind FROM store_payment_channels WHERE id = $1{lock_clause}"
                ),
                vec![channel_id.into()],
            ))
            .await
            .map_err(storage)?
            .ok_or(CredentialStoreError::ChannelNotFound)?;
        let adapter_kind: String = channel.try_get("", "adapter_kind").map_err(storage)?;
        let plaintext = Zeroizing::new(
            serde_json::to_vec(&credential).map_err(|_| CredentialStoreError::InvalidCredential)?,
        );
        let account_identity_digest =
            validate_credential_identity(&adapter_kind, plaintext.as_slice())?;
        let id = Uuid::new_v4().to_string();
        let encrypted = self
            .key_ring
            .encrypt(
                &format!("store_channel_credentials:{id}:secret"),
                plaintext.as_slice(),
            )
            .map_err(|_| CredentialStoreError::EncryptionUnavailable)?;
        let created_at = Utc::now();
        tx.execute(self.db.stmt(
            "UPDATE store_channel_credentials
             SET status = 'retired', retired_at = $2
             WHERE channel_id = $1 AND status = 'active'",
            vec![channel_id.into(), timestamp(created_at).into()],
        ))
        .await
        .map_err(storage)?;
        tx.execute(self.db.stmt(
            "INSERT INTO store_channel_credentials
                (id, channel_id, adapter_kind, format_version, key_id, nonce_base64,
                 ciphertext_base64, account_identity_digest, status, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'active', $9)",
            vec![
                id.clone().into(),
                channel_id.into(),
                adapter_kind.clone().into(),
                i32::from(encrypted.version).into(),
                encrypted.key_id.into(),
                encrypted.nonce_base64.into(),
                encrypted.ciphertext_base64.into(),
                account_identity_digest.clone().into(),
                timestamp(created_at).into(),
            ],
        ))
        .await
        .map_err(storage)?;
        tx.execute(self.db.stmt(
            "DELETE FROM store_merchant_capabilities WHERE channel_id = $1",
            vec![channel_id.into()],
        ))
        .await
        .map_err(storage)?;
        let changed = tx
            .execute(self.db.stmt(
                "UPDATE store_payment_channels
                 SET enabled = 0, revision = revision + 1, updated_at = $2
                 WHERE id = $1",
                vec![channel_id.into(), timestamp(created_at).into()],
            ))
            .await
            .map_err(storage)?;
        if changed.rows_affected() != 1 {
            return Err(CredentialStoreError::ChannelNotFound);
        }
        tx.commit().await.map_err(storage)?;
        Ok(SavedCredentialView {
            id,
            channel_id: channel_id.to_string(),
            adapter_kind,
            account_identity_digest,
            status: "active",
            created_at,
        })
    }
}

fn validate_credential_identity(
    adapter_kind: &str,
    plaintext: &[u8],
) -> Result<String, CredentialStoreError> {
    match adapter_kind {
        "stripe" => StripeCredential::from_json(plaintext)
            .map(|credential| digest(credential.account_id()))
            .map_err(|_| CredentialStoreError::InvalidCredential),
        "alipay" => AlipayCredential::from_json(plaintext)
            .map(|credential| digest(credential.seller_id()))
            .map_err(|_| CredentialStoreError::InvalidCredential),
        "wechat" => WechatCredential::from_json(plaintext)
            .map(|credential| credential.account_identity_digest())
            .map_err(|_| CredentialStoreError::InvalidCredential),
        _ => Err(CredentialStoreError::InvalidCredential),
    }
}

fn digest(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn storage(error: impl ToString) -> CredentialStoreError {
    CredentialStoreError::Storage(error.to_string())
}

impl CredentialVersion {
    pub fn new(
        id: impl Into<String>,
        channel_id: impl Into<String>,
        adapter_kind: impl Into<String>,
        account_identity_digest: impl Into<String>,
        encrypted_secret: EncryptedSecret,
    ) -> Self {
        Self {
            id: id.into(),
            channel_id: channel_id.into(),
            adapter_kind: adapter_kind.into(),
            account_identity_digest: account_identity_digest.into(),
            status: CredentialStatus::Active,
            encrypted_secret,
        }
    }

    pub fn public_view(&self) -> CredentialVersionView {
        CredentialVersionView {
            id: self.id.clone(),
            channel_id: self.channel_id.clone(),
            adapter_kind: self.adapter_kind.clone(),
            account_identity_digest: self.account_identity_digest.clone(),
            status: match self.status {
                CredentialStatus::Active => "active",
                CredentialStatus::Retired => "retired",
            },
            key_id: self.encrypted_secret.key_id.clone(),
        }
    }
}
