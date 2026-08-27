use serde::Serialize;

use super::crypto::EncryptedSecret;

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
