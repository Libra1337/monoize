use super::crypto::{EncryptedSecret, PaymentKeyRing};
use rsa::rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

const V2_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const ACCEPTED_ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTUVWXYZ";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RedemptionAccessAction {
    Reveal,
    Copy,
    Export,
}

impl RedemptionAccessAction {
    pub(crate) const fn max_codes(self) -> usize {
        match self {
            Self::Reveal | Self::Copy => 20,
            Self::Export => 100,
        }
    }

    pub(crate) const fn audit_action(self) -> &'static str {
        match self {
            Self::Reveal => "redemption_reveal",
            Self::Copy => "redemption_copy",
            Self::Export => "redemption_export",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevealRedemptionInput {
    pub code_ids: Vec<String>,
    pub action: RedemptionAccessAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedemptionAuditContext {
    pub admin_user_id: String,
    pub source_ip: String,
    pub user_agent: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevealedRedemptionCode {
    pub id: String,
    pub code: String,
}

pub(crate) struct GeneratedCodeMaterial {
    pub code: String,
    pub normalized: String,
    pub hint: String,
    pub encrypted: EncryptedSecret,
}

pub(crate) fn generate_code_material(
    key_ring: &PaymentKeyRing,
    code_id: &str,
) -> Result<GeneratedCodeMaterial, ()> {
    let mut random = [0_u8; 16];
    OsRng.fill_bytes(&mut random);
    let normalized = random
        .iter()
        .map(|byte| V2_ALPHABET[usize::from(byte & 31)] as char)
        .collect::<String>();
    let code = format_normalized(&normalized).ok_or(())?;
    let hint = normalized[12..16].to_string();
    let encrypted = key_ring
        .encrypt(&redemption_aad(code_id), code.as_bytes())
        .map_err(|_| ())?;
    Ok(GeneratedCodeMaterial {
        code,
        normalized,
        hint,
        encrypted,
    })
}

pub(crate) fn decrypt_code(
    key_ring: &PaymentKeyRing,
    code_id: &str,
    encrypted: &EncryptedSecret,
) -> Result<String, ()> {
    let plaintext = key_ring
        .decrypt(&redemption_aad(code_id), encrypted)
        .map_err(|_| ())?;
    let code = std::str::from_utf8(&plaintext).map_err(|_| ())?.to_string();
    let normalized = normalize_code(&code).ok_or(())?;
    if format_normalized(&normalized).as_deref() != Some(code.as_str()) {
        return Err(());
    }
    Ok(code)
}

pub(crate) fn validate_reveal_input(input: &RevealRedemptionInput) -> bool {
    if input.code_ids.is_empty() || input.code_ids.len() > input.action.max_codes() {
        return false;
    }
    let mut unique = BTreeSet::new();
    input
        .code_ids
        .iter()
        .all(|id| !id.is_empty() && id.len() <= 100 && unique.insert(id.as_str()))
}

pub(crate) fn validate_audit_context(context: &RedemptionAuditContext) -> bool {
    !context.admin_user_id.trim().is_empty()
        && !context.source_ip.trim().is_empty()
        && context.source_ip.len() <= 128
        && !context.user_agent.trim().is_empty()
        && context.user_agent.len() <= 512
}

pub(crate) fn normalize_code(code: &str) -> Option<String> {
    if !code.is_ascii() {
        return None;
    }
    let normalized = code
        .bytes()
        .filter(|byte| *byte != b'-')
        .map(|byte| byte.to_ascii_uppercase())
        .collect::<Vec<_>>();
    if normalized.len() != 16
        || !normalized
            .iter()
            .all(|byte| ACCEPTED_ALPHABET.contains(byte))
    {
        return None;
    }
    String::from_utf8(normalized).ok()
}

pub(crate) fn code_digest(normalized: &str) -> String {
    lower_hex(&Sha256::digest(normalized.as_bytes()))
}

pub(crate) fn source_ip_digest(source_ip: &str) -> Option<String> {
    let source_ip = source_ip.trim();
    if source_ip.is_empty() || source_ip.len() > 128 || !source_ip.is_ascii() {
        return None;
    }
    Some(lower_hex(&Sha256::digest(source_ip.as_bytes())))
}

fn format_normalized(normalized: &str) -> Option<String> {
    if normalized.len() != 16 || !normalized.bytes().all(|byte| V2_ALPHABET.contains(&byte)) {
        return None;
    }
    Some(format!(
        "{}-{}-{}-{}",
        &normalized[0..4],
        &normalized[4..8],
        &normalized[8..12],
        &normalized[12..16]
    ))
}

fn redemption_aad(code_id: &str) -> String {
    format!("store_redemption_codes:{code_id}:code")
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[usize::from(byte >> 4)] as char);
        output.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    output
}
