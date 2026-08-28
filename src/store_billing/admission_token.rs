use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, TimeZone, Utc};
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use thiserror::Error;
use uuid::Uuid;

use crate::replica::metering::MeteringSpoolCapacity;

const TOKEN_TTL_SECONDS: i64 = 30;
const CLOCK_SKEW_SECONDS: i64 = 5;
const PRIOR_KEY_SECONDS: i64 = 300;
const MAX_TOKEN_BYTES: usize = 16 * 1024;
pub const ADMISSION_ISSUER: &str = "lynshen-primary";
const TERMINAL_RESERVED_BYTES: u64 = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionTokenInput {
    pub audience: String,
    pub token_id: String,
    pub reservation_id: String,
    pub request_id: String,
    pub entitlement_id: String,
    pub generation: i64,
    pub maximum_nano_usd: i128,
    pub reserved_fen_cny: i128,
    pub pricing_revision: String,
    pub issued_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionBinding {
    pub audience: String,
    pub reservation_id: String,
    pub request_id: String,
    pub entitlement_id: String,
    pub generation: i64,
    pub maximum_nano_usd: i128,
    pub reserved_fen_cny: i128,
    pub pricing_revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionVerificationBinding {
    pub audience: String,
    pub token_id: String,
    pub reservation_id: String,
    pub request_id: String,
    pub maximum_nano_usd: i128,
    pub pricing_revision: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionVerifierKey {
    pub key_id: String,
    pub public_key_base64: String,
    pub state: String,
    pub activated_at: DateTime<Utc>,
    pub verify_until: Option<DateTime<Utc>>,
}

#[derive(Clone, Default)]
pub struct AdmissionVerifierRing {
    inner: Arc<RwLock<VerifierSnapshot>>,
}

#[derive(Default)]
struct VerifierSnapshot {
    keys: BTreeMap<String, VerifierEntry>,
    refreshed_at: Option<DateTime<Utc>>,
}

struct VerifierEntry {
    key: VerifyingKey,
    verify_until: Option<DateTime<Utc>>,
}

impl AdmissionVerifierRing {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn replace_snapshot(
        &self,
        keys: Vec<AdmissionVerifierKey>,
        now: DateTime<Utc>,
    ) -> Result<(), AdmissionError> {
        let mut replacement = BTreeMap::new();
        let mut active_count = 0usize;
        for input in keys {
            validate_identifier(&input.key_id)
                .map_err(|_| AdmissionError::VerifierSnapshotInvalid)?;
            if replacement.contains_key(&input.key_id) || input.public_key_base64.contains('=') {
                return Err(AdmissionError::VerifierSnapshotInvalid);
            }
            let decoded = URL_SAFE_NO_PAD
                .decode(&input.public_key_base64)
                .map_err(|_| AdmissionError::VerifierSnapshotInvalid)?;
            if URL_SAFE_NO_PAD.encode(&decoded) != input.public_key_base64 {
                return Err(AdmissionError::VerifierSnapshotInvalid);
            }
            let bytes = <[u8; 32]>::try_from(decoded.as_slice())
                .map_err(|_| AdmissionError::VerifierSnapshotInvalid)?;
            let key = VerifyingKey::from_bytes(&bytes)
                .map_err(|_| AdmissionError::VerifierSnapshotInvalid)?;
            let verify_until = match input.state.as_str() {
                "active" if input.verify_until.is_none() => {
                    active_count += 1;
                    None
                }
                "retired" if input.verify_until.is_some_and(|until| until > now) => {
                    input.verify_until
                }
                _ => return Err(AdmissionError::VerifierSnapshotInvalid),
            };
            replacement.insert(input.key_id, VerifierEntry { key, verify_until });
        }
        if active_count > 1 {
            return Err(AdmissionError::VerifierSnapshotInvalid);
        }
        let mut snapshot = self.inner.write().map_err(|_| AdmissionError::KeyRing)?;
        *snapshot = VerifierSnapshot {
            keys: replacement,
            refreshed_at: Some(now),
        };
        Ok(())
    }

    pub fn key_ids(&self, now: DateTime<Utc>) -> Vec<String> {
        self.inner
            .read()
            .map(|snapshot| {
                snapshot
                    .keys
                    .iter()
                    .filter(|(_, entry)| entry.verify_until.is_none_or(|until| now < until))
                    .map(|(key_id, _)| key_id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn refreshed_at(&self) -> Option<DateTime<Utc>> {
        self.inner
            .read()
            .ok()
            .and_then(|snapshot| snapshot.refreshed_at)
    }

    pub fn verify(
        &self,
        token: &str,
        binding: &AdmissionVerificationBinding,
        now: DateTime<Utc>,
    ) -> Result<VerifiedAdmission, AdmissionError> {
        if token.len() > MAX_TOKEN_BYTES {
            return Err(AdmissionError::NonCanonicalToken);
        }
        let segments = token.split('.').collect::<Vec<_>>();
        if segments.len() != 3 || segments.iter().any(|segment| segment.is_empty()) {
            return Err(AdmissionError::NonCanonicalToken);
        }
        let header_bytes = decode_canonical(segments[0])?;
        let header: HeaderOwned =
            serde_json::from_slice(&header_bytes).map_err(|_| AdmissionError::NonCanonicalToken)?;
        if header.alg != "EdDSA" || header.typ != "lynshen-plan-admission" {
            return Err(AdmissionError::NonCanonicalToken);
        }
        validate_identifier(&header.kid)?;
        let key = {
            let snapshot = self.inner.read().map_err(|_| AdmissionError::KeyRing)?;
            let entry = snapshot
                .keys
                .get(&header.kid)
                .ok_or(AdmissionError::UnknownKey)?;
            if entry.verify_until.is_some_and(|until| now >= until) {
                return Err(AdmissionError::UnknownKey);
            }
            entry.key
        };
        let signature_bytes = decode_canonical(segments[2])?;
        let signature = Signature::from_slice(&signature_bytes)
            .map_err(|_| AdmissionError::InvalidSignature)?;
        let signing_input = format!("{}.{}", segments[0], segments[1]);
        key.verify(signing_input.as_bytes(), &signature)
            .map_err(|_| AdmissionError::InvalidSignature)?;

        let claims_bytes = decode_canonical(segments[1])?;
        let claims: ClaimsOwned =
            serde_json::from_slice(&claims_bytes).map_err(|_| AdmissionError::NonCanonicalToken)?;
        if claims.v != 1 || claims.nbf != claims.iat || claims.exp != claims.iat + TOKEN_TTL_SECONDS
        {
            return Err(AdmissionError::NonCanonicalToken);
        }
        if claims.iss != ADMISSION_ISSUER {
            return Err(AdmissionError::WrongIssuer);
        }
        if claims.aud != binding.audience {
            return Err(AdmissionError::WrongAudience);
        }
        let now_seconds = now.timestamp();
        if now_seconds < claims.nbf - CLOCK_SKEW_SECONDS {
            return Err(AdmissionError::NotYetValid);
        }
        if now_seconds >= claims.exp + CLOCK_SKEW_SECONDS {
            return Err(AdmissionError::Expired);
        }
        let maximum_nano_usd = parse_canonical_amount(&claims.maximum_nano_usd)?;
        let reserved_fen_cny = parse_canonical_amount(&claims.reserved_fen_cny)?;
        let issued_at = unix_time(claims.iat)?;
        let expires_at = unix_time(claims.exp)?;
        if claims.jti != binding.token_id
            || claims.reservation_id != binding.reservation_id
            || claims.request_id != binding.request_id
            || maximum_nano_usd != binding.maximum_nano_usd
            || claims.pricing_revision != binding.pricing_revision
            || issued_at != binding.issued_at
            || expires_at != binding.expires_at
        {
            return Err(AdmissionError::BindingMismatch);
        }
        Ok(VerifiedAdmission {
            issuer: claims.iss,
            key_id: header.kid,
            audience: claims.aud,
            token_id: claims.jti,
            reservation_id: claims.reservation_id,
            request_id: claims.request_id,
            entitlement_id: claims.entitlement_id,
            generation: claims.generation,
            maximum_nano_usd,
            reserved_fen_cny,
            pricing_revision: claims.pricing_revision,
            issued_at,
            expires_at,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedAdmission {
    pub issuer: String,
    pub key_id: String,
    pub audience: String,
    pub token_id: String,
    pub reservation_id: String,
    pub request_id: String,
    pub entitlement_id: String,
    pub generation: i64,
    pub maximum_nano_usd: i128,
    pub reserved_fen_cny: i128,
    pub pricing_revision: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct AdmissionSigningKey {
    key_id: String,
    signing_key: SigningKey,
    activated_at: DateTime<Utc>,
}

impl AdmissionSigningKey {
    pub fn from_seed(
        key_id: impl Into<String>,
        seed: [u8; 32],
        activated_at: DateTime<Utc>,
    ) -> Result<Self, AdmissionError> {
        let key_id = key_id.into();
        validate_identifier(&key_id)?;
        Ok(Self {
            key_id,
            signing_key: SigningKey::from_bytes(&seed),
            activated_at,
        })
    }
}

#[derive(Clone)]
pub struct PriorAdmissionSigningKey {
    pub key: AdmissionSigningKey,
    pub deactivated_at: DateTime<Utc>,
    pub last_issued_expires_at: Option<DateTime<Utc>>,
    pub verify_until: Option<DateTime<Utc>>,
}

#[derive(Clone)]
pub struct AdmissionKeyRing {
    inner: Arc<RwLock<KeyRingInner>>,
}

struct KeyRingInner {
    issuer: String,
    active_key_id: String,
    keys: BTreeMap<String, KeyEntry>,
}

struct KeyEntry {
    key: AdmissionSigningKey,
    state: KeyState,
    last_issued_expires_at: Option<DateTime<Utc>>,
    verify_until: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum KeyState {
    Published,
    Active,
    Retired,
}

impl AdmissionKeyRing {
    pub fn new(
        issuer: impl Into<String>,
        active: AdmissionSigningKey,
        prior: Vec<PriorAdmissionSigningKey>,
    ) -> Result<Self, AdmissionError> {
        let issuer = issuer.into();
        validate_identifier(&issuer)?;
        let active_key_id = active.key_id.clone();
        let mut keys = BTreeMap::new();
        keys.insert(
            active.key_id.clone(),
            KeyEntry {
                key: active,
                state: KeyState::Active,
                last_issued_expires_at: None,
                verify_until: None,
            },
        );
        for prior_key in prior {
            if prior_key.deactivated_at < prior_key.key.activated_at
                || keys.contains_key(&prior_key.key.key_id)
            {
                return Err(AdmissionError::InvalidInput);
            }
            let minimum = prior_key
                .deactivated_at
                .checked_add_signed(chrono::Duration::seconds(PRIOR_KEY_SECONDS))
                .ok_or(AdmissionError::InvalidInput)?;
            let issued_limit = prior_key
                .last_issued_expires_at
                .map(|expires| {
                    expires
                        .checked_add_signed(chrono::Duration::seconds(CLOCK_SKEW_SECONDS))
                        .ok_or(AdmissionError::InvalidInput)
                })
                .transpose()?;
            let verify_until = prior_key
                .verify_until
                .into_iter()
                .chain(issued_limit)
                .fold(minimum, DateTime::max);
            keys.insert(
                prior_key.key.key_id.clone(),
                KeyEntry {
                    key: prior_key.key,
                    state: KeyState::Retired,
                    last_issued_expires_at: prior_key.last_issued_expires_at,
                    verify_until: Some(verify_until),
                },
            );
        }
        Ok(Self {
            inner: Arc::new(RwLock::new(KeyRingInner {
                issuer,
                active_key_id,
                keys,
            })),
        })
    }

    pub fn issue(&self, input: AdmissionTokenInput) -> Result<String, AdmissionError> {
        validate_token_input(&input)?;
        let mut inner = self.inner.write().map_err(|_| AdmissionError::KeyRing)?;
        let issuer = inner.issuer.clone();
        let active_key_id = inner.active_key_id.clone();
        let entry = inner
            .keys
            .get_mut(&active_key_id)
            .ok_or(AdmissionError::UnknownKey)?;
        if entry.state != KeyState::Active || input.issued_at < entry.key.activated_at {
            return Err(AdmissionError::UnknownKey);
        }
        let expires_at = input.issued_at + chrono::Duration::seconds(TOKEN_TTL_SECONDS);
        let header = HeaderWire {
            alg: "EdDSA",
            kid: &entry.key.key_id,
            typ: "lynshen-plan-admission",
        };
        let claims = ClaimsWire {
            v: 1,
            iss: &issuer,
            aud: &input.audience,
            jti: &input.token_id,
            reservation_id: &input.reservation_id,
            request_id: &input.request_id,
            entitlement_id: &input.entitlement_id,
            generation: input.generation,
            maximum_nano_usd: input.maximum_nano_usd.to_string(),
            reserved_fen_cny: input.reserved_fen_cny.to_string(),
            pricing_revision: &input.pricing_revision,
            iat: input.issued_at.timestamp(),
            nbf: input.issued_at.timestamp(),
            exp: expires_at.timestamp(),
        };
        let header_segment = encode_json(&header)?;
        let claims_segment = encode_json(&claims)?;
        let signing_input = format!("{header_segment}.{claims_segment}");
        let signature = entry.key.signing_key.sign(signing_input.as_bytes());
        entry.last_issued_expires_at = Some(
            entry
                .last_issued_expires_at
                .map_or(expires_at, |existing| existing.max(expires_at)),
        );
        Ok(format!(
            "{signing_input}.{}",
            URL_SAFE_NO_PAD.encode(signature.to_bytes())
        ))
    }

    pub fn verify(
        &self,
        token: &str,
        binding: &AdmissionBinding,
        now: DateTime<Utc>,
    ) -> Result<VerifiedAdmission, AdmissionError> {
        if token.len() > MAX_TOKEN_BYTES {
            return Err(AdmissionError::NonCanonicalToken);
        }
        let segments = token.split('.').collect::<Vec<_>>();
        if segments.len() != 3 || segments.iter().any(|segment| segment.is_empty()) {
            return Err(AdmissionError::NonCanonicalToken);
        }
        let header_bytes = decode_canonical(segments[0])?;
        let header: HeaderOwned =
            serde_json::from_slice(&header_bytes).map_err(|_| AdmissionError::NonCanonicalToken)?;
        if header.alg != "EdDSA" || header.typ != "lynshen-plan-admission" {
            return Err(AdmissionError::NonCanonicalToken);
        }
        validate_identifier(&header.kid)?;

        let inner = self.inner.read().map_err(|_| AdmissionError::KeyRing)?;
        let entry = inner
            .keys
            .get(&header.kid)
            .ok_or(AdmissionError::UnknownKey)?;
        if entry.state == KeyState::Published
            || (entry.state == KeyState::Retired
                && entry.verify_until.is_some_and(|until| now >= until))
        {
            return Err(AdmissionError::UnknownKey);
        }
        let signature_bytes = decode_canonical(segments[2])?;
        let signature = Signature::from_slice(&signature_bytes)
            .map_err(|_| AdmissionError::InvalidSignature)?;
        let signing_input = format!("{}.{}", segments[0], segments[1]);
        entry
            .key
            .signing_key
            .verifying_key()
            .verify(signing_input.as_bytes(), &signature)
            .map_err(|_| AdmissionError::InvalidSignature)?;

        let claims_bytes = decode_canonical(segments[1])?;
        let claims: ClaimsOwned =
            serde_json::from_slice(&claims_bytes).map_err(|_| AdmissionError::NonCanonicalToken)?;
        if claims.v != 1 || claims.nbf != claims.iat || claims.exp != claims.iat + TOKEN_TTL_SECONDS
        {
            return Err(AdmissionError::NonCanonicalToken);
        }
        if claims.iss != inner.issuer {
            return Err(AdmissionError::WrongIssuer);
        }
        if claims.aud != binding.audience {
            return Err(AdmissionError::WrongAudience);
        }
        let now_seconds = now.timestamp();
        if now_seconds < claims.nbf - CLOCK_SKEW_SECONDS {
            return Err(AdmissionError::NotYetValid);
        }
        if now_seconds >= claims.exp + CLOCK_SKEW_SECONDS {
            return Err(AdmissionError::Expired);
        }
        let maximum_nano_usd = parse_canonical_amount(&claims.maximum_nano_usd)?;
        let reserved_fen_cny = parse_canonical_amount(&claims.reserved_fen_cny)?;
        if claims.reservation_id != binding.reservation_id
            || claims.request_id != binding.request_id
            || claims.entitlement_id != binding.entitlement_id
            || claims.generation != binding.generation
            || maximum_nano_usd != binding.maximum_nano_usd
            || reserved_fen_cny != binding.reserved_fen_cny
            || claims.pricing_revision != binding.pricing_revision
        {
            return Err(AdmissionError::BindingMismatch);
        }
        let issued_at = unix_time(claims.iat)?;
        let expires_at = unix_time(claims.exp)?;
        Ok(VerifiedAdmission {
            issuer: claims.iss,
            key_id: header.kid,
            audience: claims.aud,
            token_id: claims.jti,
            reservation_id: claims.reservation_id,
            request_id: claims.request_id,
            entitlement_id: claims.entitlement_id,
            generation: claims.generation,
            maximum_nano_usd,
            reserved_fen_cny,
            pricing_revision: claims.pricing_revision,
            issued_at,
            expires_at,
        })
    }

    pub fn publish(&self, key: AdmissionSigningKey) -> Result<(), AdmissionError> {
        let mut inner = self.inner.write().map_err(|_| AdmissionError::KeyRing)?;
        if inner.keys.contains_key(&key.key_id) {
            return Err(AdmissionError::InvalidInput);
        }
        inner.keys.insert(
            key.key_id.clone(),
            KeyEntry {
                key,
                state: KeyState::Published,
                last_issued_expires_at: None,
                verify_until: None,
            },
        );
        Ok(())
    }

    pub fn activate(&self, key_id: &str, now: DateTime<Utc>) -> Result<(), AdmissionError> {
        let mut inner = self.inner.write().map_err(|_| AdmissionError::KeyRing)?;
        let next = inner.keys.get(key_id).ok_or(AdmissionError::UnknownKey)?;
        if next.state != KeyState::Published || now < next.key.activated_at {
            return Err(AdmissionError::InvalidInput);
        }
        let prior_id = inner.active_key_id.clone();
        let prior = inner
            .keys
            .get_mut(&prior_id)
            .ok_or(AdmissionError::UnknownKey)?;
        let minimum = now + chrono::Duration::seconds(PRIOR_KEY_SECONDS);
        let issued_limit = prior
            .last_issued_expires_at
            .map(|expires| expires + chrono::Duration::seconds(CLOCK_SKEW_SECONDS));
        prior.verify_until = Some(issued_limit.map_or(minimum, |limit| limit.max(minimum)));
        prior.state = KeyState::Retired;
        inner.keys.get_mut(key_id).unwrap().state = KeyState::Active;
        inner.active_key_id = key_id.to_string();
        Ok(())
    }

    pub fn prune_retired(&self, now: DateTime<Utc>) {
        if let Ok(mut inner) = self.inner.write() {
            inner.keys.retain(|_, entry| {
                entry.state != KeyState::Retired
                    || entry.verify_until.is_none_or(|until| now < until)
            });
        }
    }

    pub fn verification_key_ids(&self, now: DateTime<Utc>) -> Vec<String> {
        self.inner
            .read()
            .map(|inner| {
                inner
                    .keys
                    .iter()
                    .filter(|(_, entry)| {
                        entry.state != KeyState::Published
                            && entry.verify_until.is_none_or(|until| now < until)
                    })
                    .map(|(id, _)| id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn active_key_id(&self) -> String {
        self.inner
            .read()
            .map(|inner| inner.active_key_id.clone())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TerminalKind {
    Settlement,
    Release,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSpoolInput {
    pub token_id: String,
    pub reservation_id: String,
    pub request_id: String,
    pub audience: String,
    pub kind: TerminalKind,
    pub actual_nano_usd: Option<i128>,
    pub created_at: DateTime<Utc>,
}

impl TerminalSpoolInput {
    pub fn settlement(
        admission: &VerifiedAdmission,
        actual_nano_usd: i128,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            token_id: admission.token_id.clone(),
            reservation_id: admission.reservation_id.clone(),
            request_id: admission.request_id.clone(),
            audience: admission.audience.clone(),
            kind: TerminalKind::Settlement,
            actual_nano_usd: Some(actual_nano_usd),
            created_at,
        }
    }

    pub fn release(admission: &VerifiedAdmission, created_at: DateTime<Utc>) -> Self {
        Self {
            token_id: admission.token_id.clone(),
            reservation_id: admission.reservation_id.clone(),
            request_id: admission.request_id.clone(),
            audience: admission.audience.clone(),
            kind: TerminalKind::Release,
            actual_nano_usd: None,
            created_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalSpoolRecord {
    pub path: PathBuf,
    pub input: TerminalSpoolInput,
    pub canonical_digest: String,
    pub encoded_len: u64,
}

impl TerminalSpoolRecord {
    pub fn wire(&self) -> PlanTerminalWire {
        PlanTerminalWire {
            version: 1,
            token_id: self.input.token_id.clone(),
            reservation_id: self.input.reservation_id.clone(),
            request_id: self.input.request_id.clone(),
            audience: self.input.audience.clone(),
            kind: self.input.kind,
            actual_nano_usd: self.input.actual_nano_usd.map(|value| value.to_string()),
            canonical_digest: self.canonical_digest.clone(),
            created_at: self.input.created_at,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TerminalAcknowledgementResult {
    Applied,
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanTerminalAcknowledgement {
    pub token_id: String,
    pub canonical_digest: String,
    pub result: TerminalAcknowledgementResult,
}

pub fn validate_terminal_acknowledgements(
    acknowledgements: &[PlanTerminalAcknowledgement],
    token_id: &str,
    canonical_digest: &str,
) -> bool {
    acknowledgements.len() == 1
        && acknowledgements[0].token_id == token_id
        && acknowledgements[0].canonical_digest == canonical_digest
}

#[cfg(test)]
#[derive(Debug, Default)]
struct AcknowledgementFaults {
    claims_sync: AtomicBool,
    terminal_sync: AtomicBool,
}

#[cfg(test)]
impl AcknowledgementFaults {
    fn fail_claims_sync_once(&self) {
        self.claims_sync.store(true, Ordering::Release);
    }

    fn fail_terminal_sync_once(&self) {
        self.terminal_sync.store(true, Ordering::Release);
    }
}

#[derive(Debug, Clone)]
pub struct AdmissionClaimStore {
    claims_dir: PathBuf,
    terminal_dir: PathBuf,
    capacity: Arc<MeteringSpoolCapacity>,
    #[cfg(test)]
    acknowledgement_faults: Option<Arc<AcknowledgementFaults>>,
}

impl AdmissionClaimStore {
    pub async fn new(root: impl AsRef<Path>) -> Result<Self, AdmissionError> {
        Self::new_with_capacity(root, Arc::new(MeteringSpoolCapacity::new(u64::MAX))).await
    }

    pub async fn new_with_capacity(
        root: impl AsRef<Path>,
        capacity: Arc<MeteringSpoolCapacity>,
    ) -> Result<Self, AdmissionError> {
        Self::open_with_capacity(root, capacity)
    }

    pub fn open_with_capacity(
        root: impl AsRef<Path>,
        capacity: Arc<MeteringSpoolCapacity>,
    ) -> Result<Self, AdmissionError> {
        let root = root.as_ref().to_path_buf();
        let claims_dir = root.join("claims");
        let terminal_dir = root.join("terminal");
        std::fs::create_dir_all(&claims_dir).map_err(io_error)?;
        std::fs::create_dir_all(&terminal_dir).map_err(io_error)?;
        cleanup_residual_temps(&claims_dir, "claim")?;
        cleanup_residual_temps(&terminal_dir, "terminal")?;
        sync_directory(&root)?;
        let store = Self {
            claims_dir,
            terminal_dir,
            capacity,
            #[cfg(test)]
            acknowledgement_faults: None,
        };
        let reconstructed = store.validate_and_measure()?;
        store.capacity.add_reconstructed(reconstructed);
        store.recover_startup_before_publish(Utc::now())?;
        Ok(store)
    }

    pub fn capacity(&self) -> &Arc<MeteringSpoolCapacity> {
        &self.capacity
    }

    #[cfg(test)]
    fn with_acknowledgement_faults(mut self, faults: Arc<AcknowledgementFaults>) -> Self {
        self.acknowledgement_faults = Some(faults);
        self
    }

    fn sync_acknowledgement_claims_dir(&self) -> Result<(), AdmissionError> {
        #[cfg(test)]
        if self
            .acknowledgement_faults
            .as_ref()
            .is_some_and(|faults| faults.claims_sync.swap(false, Ordering::AcqRel))
        {
            return Err(AdmissionError::Io(
                "injected claims directory sync failure".to_string(),
            ));
        }
        sync_directory(&self.claims_dir)
    }

    fn sync_acknowledgement_terminal_dir(&self) -> Result<(), AdmissionError> {
        #[cfg(test)]
        if self
            .acknowledgement_faults
            .as_ref()
            .is_some_and(|faults| faults.terminal_sync.swap(false, Ordering::AcqRel))
        {
            return Err(AdmissionError::Io(
                "injected terminal directory sync failure".to_string(),
            ));
        }
        sync_directory(&self.terminal_dir)
    }

    pub async fn claim(&self, admission: &VerifiedAdmission) -> Result<PathBuf, AdmissionError> {
        validate_identifier(&admission.token_id)?;
        let path = self.marker_path(&admission.token_id);
        let marker = ClaimMarker {
            version: 1,
            token_id: admission.token_id.clone(),
            reservation_id: admission.reservation_id.clone(),
            request_id: admission.request_id.clone(),
            audience: admission.audience.clone(),
            maximum_nano_usd: admission.maximum_nano_usd.to_string(),
            expires_at: admission.expires_at,
            state: ClaimState::Claimed,
            routed_at: None,
            acknowledged_at: None,
            terminal_reserved_bytes: TERMINAL_RESERVED_BYTES,
        };
        validate_claim(&marker)?;
        let bytes = serde_json::to_vec(&marker).map_err(serialize_error)?;
        let accounted = bytes.len() as u64 + TERMINAL_RESERVED_BYTES;
        let _capacity_guard = self.capacity.lock().await;
        self.capacity
            .ensure_add(accounted)
            .map_err(|_| AdmissionError::SpoolQuotaExhausted)?;
        match durable_publish_new(&path, &bytes) {
            Ok(()) => {
                self.capacity.add(accounted);
                sync_directory(&self.claims_dir)?;
                Ok(path)
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => Err(AdmissionError::Replay),
            Err(error) => Err(io_error(error)),
        }
    }

    pub async fn marker_exists(&self, token_id: &str) -> Result<bool, AdmissionError> {
        validate_identifier(token_id)?;
        Ok(self.marker_path(token_id).exists())
    }

    pub async fn spool_terminal(
        &self,
        input: TerminalSpoolInput,
    ) -> Result<TerminalSpoolRecord, AdmissionError> {
        validate_terminal(&input)?;
        let wire = terminal_wire(&input)?;
        let bytes = serde_json::to_vec(&wire).map_err(serialize_error)?;
        ensure_terminal_size(&bytes)?;
        let _capacity_guard = self.capacity.lock().await;
        let marker_path = self.marker_path(&input.token_id);
        let marker = read_claim(&marker_path)?;
        if marker.acknowledged_at.is_some()
            || marker.reservation_id != input.reservation_id
            || marker.request_id != input.request_id
            || marker.audience != input.audience
        {
            return Err(AdmissionError::ClaimMissing);
        }
        let path = self.terminal_path(&input.token_id);
        match durable_publish_new(&path, &bytes) {
            Ok(()) => {
                self.capacity
                    .replace(TERMINAL_RESERVED_BYTES, bytes.len() as u64);
                sync_directory(&self.terminal_dir)?;
                Ok(TerminalSpoolRecord {
                    path,
                    input,
                    canonical_digest: wire.canonical_digest,
                    encoded_len: bytes.len() as u64,
                })
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                let existing = read_terminal(&path)?;
                confirm_existing_terminal(existing.input.clone(), &input, || {
                    sync_directory(&self.terminal_dir)
                })?;
                if existing.canonical_digest != wire.canonical_digest {
                    return Err(AdmissionError::TerminalConflict);
                }
                Ok(existing)
            }
            Err(error) => Err(io_error(error)),
        }
    }

    pub async fn acknowledge_terminal(
        &self,
        token_id: &str,
        canonical_digest: &str,
        now: DateTime<Utc>,
    ) -> Result<(), AdmissionError> {
        validate_identifier(token_id)?;
        validate_digest(canonical_digest)?;
        let _capacity_guard = self.capacity.lock().await;
        let path = self.marker_path(token_id);
        let mut marker = read_claim(&path)?;
        let terminal_path = self.terminal_path(token_id);
        if !terminal_path.exists() {
            if marker.acknowledged_at.is_some() {
                return self.sync_acknowledgement_terminal_dir();
            }
            return Err(AdmissionError::TerminalAcknowledgementMismatch);
        }
        let terminal = read_terminal(&terminal_path)?;
        if terminal.input.token_id != token_id || terminal.canonical_digest != canonical_digest {
            return Err(AdmissionError::TerminalAcknowledgementMismatch);
        }
        if marker.acknowledged_at.is_none() {
            let old_len = std::fs::metadata(&path).map_err(io_error)?.len();
            marker.acknowledged_at = Some(now);
            let bytes = serde_json::to_vec(&marker).map_err(serialize_error)?;
            durable_replace(&path, &bytes)?;
            self.capacity.replace(old_len, bytes.len() as u64);
            self.sync_acknowledgement_claims_dir()?;
        }
        match std::fs::remove_file(&terminal_path) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(AdmissionError::TerminalAcknowledgementMismatch);
            }
            Err(error) => return Err(io_error(error)),
        }
        self.capacity.subtract(terminal.encoded_len);
        self.sync_acknowledgement_terminal_dir()?;
        Ok(())
    }

    pub async fn cleanup_acknowledged(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<PathBuf>, AdmissionError> {
        let _capacity_guard = self.capacity.lock().await;
        let mut removed = Vec::new();
        let mut removed_bytes = 0u64;
        for (path, marker, encoded_len) in self.load_claims()? {
            if marker.acknowledged_at.is_none()
                || now < marker.expires_at + chrono::Duration::seconds(PRIOR_KEY_SECONDS)
                || self.terminal_path(&marker.token_id).exists()
            {
                continue;
            }
            std::fs::remove_file(&path).map_err(io_error)?;
            removed_bytes += encoded_len;
            removed.push(path);
        }
        if !removed.is_empty() {
            sync_directory(&self.claims_dir)?;
            self.capacity.subtract(removed_bytes);
        }
        Ok(removed)
    }

    pub async fn mark_confirmed(&self, token_id: &str) -> Result<(), AdmissionError> {
        self.replace_claim_state(token_id, |marker| match marker.state {
            ClaimState::Claimed => {
                marker.state = ClaimState::Confirmed;
                Ok(())
            }
            ClaimState::Confirmed => Ok(()),
            ClaimState::Routed | ClaimState::ReleasePending => {
                Err(AdmissionError::InvalidClaimTransition)
            }
        })
        .await
    }

    pub async fn mark_routed(
        &self,
        token_id: &str,
        routed_at: DateTime<Utc>,
    ) -> Result<(), AdmissionError> {
        self.replace_claim_state(token_id, |marker| match marker.state {
            ClaimState::Confirmed => {
                marker.state = ClaimState::Routed;
                marker.routed_at = Some(routed_at);
                Ok(())
            }
            ClaimState::Routed if marker.routed_at == Some(routed_at) => Ok(()),
            ClaimState::Claimed | ClaimState::Routed | ClaimState::ReleasePending => {
                Err(AdmissionError::InvalidClaimTransition)
            }
        })
        .await
    }

    pub async fn mark_release_pending(&self, token_id: &str) -> Result<(), AdmissionError> {
        self.replace_claim_state(token_id, |marker| {
            if marker.state == ClaimState::ReleasePending {
                return Ok(());
            }
            marker.state = ClaimState::ReleasePending;
            marker.routed_at = None;
            Ok(())
        })
        .await
    }

    pub async fn publish_release_pending(
        &self,
        created_at: DateTime<Utc>,
    ) -> Result<usize, AdmissionError> {
        let pending = self
            .load_claims()?
            .into_iter()
            .filter(|(_, marker, _)| {
                marker.acknowledged_at.is_none()
                    && marker.state == ClaimState::ReleasePending
                    && !self.terminal_path(&marker.token_id).exists()
            })
            .map(|(_, marker, _)| marker)
            .collect::<Vec<_>>();
        let count = pending.len();
        for marker in pending {
            self.spool_terminal(terminal_from_claim(
                &marker,
                TerminalKind::Release,
                created_at,
            )?)
            .await?;
        }
        Ok(count)
    }

    pub async fn load_pending_terminals(
        &self,
        limit: usize,
    ) -> Result<Vec<TerminalSpoolRecord>, AdmissionError> {
        let _capacity_guard = self.capacity.lock().await;
        let mut terminals = self.load_terminals()?;
        terminals.sort_by(|left, right| {
            left.input
                .created_at
                .cmp(&right.input.created_at)
                .then_with(|| {
                    left.input
                        .token_id
                        .as_bytes()
                        .cmp(right.input.token_id.as_bytes())
                })
        });
        terminals.truncate(limit);
        Ok(terminals)
    }

    async fn replace_claim_state<F>(&self, token_id: &str, mutate: F) -> Result<(), AdmissionError>
    where
        F: FnOnce(&mut ClaimMarker) -> Result<(), AdmissionError>,
    {
        validate_identifier(token_id)?;
        let _capacity_guard = self.capacity.lock().await;
        let path = self.marker_path(token_id);
        let mut marker = read_claim(&path)?;
        let before = marker.clone();
        mutate(&mut marker)?;
        validate_claim(&marker)?;
        if marker == before {
            return Ok(());
        }
        let old_len = std::fs::metadata(&path).map_err(io_error)?.len();
        let bytes = serde_json::to_vec(&marker).map_err(serialize_error)?;
        durable_replace(&path, &bytes)?;
        sync_directory(&self.claims_dir)?;
        self.capacity.replace(old_len, bytes.len() as u64);
        Ok(())
    }

    fn recover_startup_before_publish(
        &self,
        created_at: DateTime<Utc>,
    ) -> Result<(), AdmissionError> {
        let abandoned = self
            .load_claims()?
            .into_iter()
            .filter(|(_, marker, _)| {
                marker.acknowledged_at.is_none() && !self.terminal_path(&marker.token_id).exists()
            })
            .map(|(_, marker, _)| marker)
            .collect::<Vec<_>>();
        for marker in abandoned {
            let kind = if marker.state == ClaimState::Routed {
                TerminalKind::Settlement
            } else {
                TerminalKind::Release
            };
            let input = terminal_from_claim(&marker, kind, created_at)?;
            let wire = terminal_wire(&input)?;
            let bytes = serde_json::to_vec(&wire).map_err(serialize_error)?;
            ensure_terminal_size(&bytes)?;
            let path = self.terminal_path(&input.token_id);
            durable_publish_new(&path, &bytes).map_err(io_error)?;
            sync_directory(&self.terminal_dir)?;
            self.capacity
                .replace(TERMINAL_RESERVED_BYTES, bytes.len() as u64);
        }
        Ok(())
    }

    fn validate_and_measure(&self) -> Result<u64, AdmissionError> {
        let claims = self.load_claims()?;
        let terminals = self.load_terminals()?;
        let terminal_by_token = terminals
            .iter()
            .map(|terminal| (terminal.input.token_id.as_str(), terminal))
            .collect::<BTreeMap<_, _>>();
        for terminal in &terminals {
            let Some((_, claim, _)) = claims
                .iter()
                .find(|(_, claim, _)| claim.token_id == terminal.input.token_id)
            else {
                return Err(AdmissionError::SpoolCorrupt);
            };
            if claim.reservation_id != terminal.input.reservation_id
                || claim.request_id != terminal.input.request_id
                || claim.audience != terminal.input.audience
            {
                return Err(AdmissionError::SpoolCorrupt);
            }
        }
        let mut total = 0u64;
        for (_, marker, encoded_len) in claims {
            total = total
                .checked_add(encoded_len)
                .ok_or(AdmissionError::SpoolCorrupt)?;
            if let Some(terminal) = terminal_by_token.get(marker.token_id.as_str()) {
                total = total
                    .checked_add(terminal.encoded_len)
                    .ok_or(AdmissionError::SpoolCorrupt)?;
            } else if marker.acknowledged_at.is_none() {
                total = total
                    .checked_add(TERMINAL_RESERVED_BYTES)
                    .ok_or(AdmissionError::SpoolCorrupt)?;
            }
        }
        Ok(total)
    }

    fn load_claims(&self) -> Result<Vec<(PathBuf, ClaimMarker, u64)>, AdmissionError> {
        let mut claims = Vec::new();
        for (path, digest) in final_files(&self.claims_dir, "claim")? {
            let marker = read_claim(&path)?;
            if digest_name(&marker.token_id) != digest {
                return Err(AdmissionError::SpoolCorrupt);
            }
            let encoded_len = std::fs::metadata(&path)
                .map_err(|_| AdmissionError::SpoolCorrupt)?
                .len();
            claims.push((path, marker, encoded_len));
        }
        Ok(claims)
    }

    fn load_terminals(&self) -> Result<Vec<TerminalSpoolRecord>, AdmissionError> {
        let mut terminals = Vec::new();
        for (path, digest) in final_files(&self.terminal_dir, "terminal")? {
            let terminal = read_terminal(&path)?;
            if digest_name(&terminal.input.token_id) != digest {
                return Err(AdmissionError::SpoolCorrupt);
            }
            terminals.push(terminal);
        }
        Ok(terminals)
    }

    fn marker_path(&self, token_id: &str) -> PathBuf {
        self.claims_dir
            .join(format!("claim-{}.json", digest_name(token_id)))
    }

    fn terminal_path(&self, token_id: &str) -> PathBuf {
        self.terminal_dir
            .join(format!("terminal-{}.json", digest_name(token_id)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ClaimState {
    Claimed,
    Confirmed,
    Routed,
    ReleasePending,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaimMarker {
    version: u8,
    token_id: String,
    reservation_id: String,
    request_id: String,
    audience: String,
    maximum_nano_usd: String,
    expires_at: DateTime<Utc>,
    state: ClaimState,
    routed_at: Option<DateTime<Utc>>,
    acknowledged_at: Option<DateTime<Utc>>,
    terminal_reserved_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanTerminalWire {
    pub version: u8,
    pub token_id: String,
    pub reservation_id: String,
    pub request_id: String,
    pub audience: String,
    pub kind: TerminalKind,
    pub actual_nano_usd: Option<String>,
    pub canonical_digest: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct HeaderWire<'a> {
    alg: &'a str,
    kid: &'a str,
    typ: &'a str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HeaderOwned {
    alg: String,
    kid: String,
    typ: String,
}

#[derive(Serialize)]
struct ClaimsWire<'a> {
    v: u8,
    iss: &'a str,
    aud: &'a str,
    jti: &'a str,
    reservation_id: &'a str,
    request_id: &'a str,
    entitlement_id: &'a str,
    generation: i64,
    maximum_nano_usd: String,
    reserved_fen_cny: String,
    pricing_revision: &'a str,
    iat: i64,
    nbf: i64,
    exp: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaimsOwned {
    v: u8,
    iss: String,
    aud: String,
    jti: String,
    reservation_id: String,
    request_id: String,
    entitlement_id: String,
    generation: i64,
    maximum_nano_usd: String,
    reserved_fen_cny: String,
    pricing_revision: String,
    iat: i64,
    nbf: i64,
    exp: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AdmissionError {
    #[error("admission input is invalid")]
    InvalidInput,
    #[error("admission token is not canonical compact JWS")]
    NonCanonicalToken,
    #[error("admission key is unknown")]
    UnknownKey,
    #[error("admission verifier snapshot is invalid")]
    VerifierSnapshotInvalid,
    #[error("admission signature is invalid")]
    InvalidSignature,
    #[error("admission issuer is wrong")]
    WrongIssuer,
    #[error("admission audience is wrong")]
    WrongAudience,
    #[error("admission binding does not match")]
    BindingMismatch,
    #[error("admission token is not yet valid")]
    NotYetValid,
    #[error("admission token expired")]
    Expired,
    #[error("admission token replay")]
    Replay,
    #[error("admission terminal record conflicts")]
    TerminalConflict,
    #[error("admission terminal acknowledgement does not match")]
    TerminalAcknowledgementMismatch,
    #[error("admission terminal record exceeds its reserved bytes")]
    TerminalTooLarge,
    #[error("admission claim marker is missing")]
    ClaimMissing,
    #[error("admission claim transition is invalid")]
    InvalidClaimTransition,
    #[error("admission_spool_corrupt")]
    SpoolCorrupt,
    #[error("metering_spool_quota_exhausted")]
    SpoolQuotaExhausted,
    #[error("admission key ring is unavailable")]
    KeyRing,
    #[error("admission I/O failed: {0}")]
    Io(String),
    #[error("admission serialization failed: {0}")]
    Serialize(String),
}

fn validate_token_input(input: &AdmissionTokenInput) -> Result<(), AdmissionError> {
    validate_identifier(&input.audience)?;
    validate_identifier(&input.token_id)?;
    validate_identifier(&input.reservation_id)?;
    validate_identifier(&input.request_id)?;
    validate_identifier(&input.entitlement_id)?;
    validate_identifier(&input.pricing_revision)?;
    if input.generation <= 0 || input.maximum_nano_usd < 0 || input.reserved_fen_cny < 0 {
        return Err(AdmissionError::InvalidInput);
    }
    Ok(())
}

fn validate_terminal(input: &TerminalSpoolInput) -> Result<(), AdmissionError> {
    validate_identifier(&input.token_id)?;
    validate_identifier(&input.reservation_id)?;
    validate_identifier(&input.request_id)?;
    validate_identifier(&input.audience)?;
    match (input.kind, input.actual_nano_usd) {
        (TerminalKind::Settlement, Some(value)) if value >= 0 => Ok(()),
        (TerminalKind::Release, None) => Ok(()),
        _ => Err(AdmissionError::InvalidInput),
    }
}

fn terminal_wire(input: &TerminalSpoolInput) -> Result<PlanTerminalWire, AdmissionError> {
    validate_terminal(input)?;
    let canonical_digest = terminal_digest(input);
    Ok(PlanTerminalWire {
        version: 1,
        token_id: input.token_id.clone(),
        reservation_id: input.reservation_id.clone(),
        request_id: input.request_id.clone(),
        audience: input.audience.clone(),
        kind: input.kind,
        actual_nano_usd: input.actual_nano_usd.map(|value| value.to_string()),
        canonical_digest,
        created_at: input.created_at,
    })
}

fn terminal_digest(input: &TerminalSpoolInput) -> String {
    let kind = match input.kind {
        TerminalKind::Settlement => "settlement",
        TerminalKind::Release => "release",
    };
    let actual = input
        .actual_nano_usd
        .map(|value| value.to_string())
        .unwrap_or_default();
    digest_name(&format!(
        "v=1\ntoken_id={}\nreservation_id={}\nrequest_id={}\naudience={}\nkind={}\nactual_nano_usd={}\n",
        input.token_id, input.reservation_id, input.request_id, input.audience, kind, actual
    ))
}

fn terminal_from_claim(
    marker: &ClaimMarker,
    kind: TerminalKind,
    created_at: DateTime<Utc>,
) -> Result<TerminalSpoolInput, AdmissionError> {
    let maximum =
        parse_positive_canonical(&marker.maximum_nano_usd).ok_or(AdmissionError::SpoolCorrupt)?;
    Ok(TerminalSpoolInput {
        token_id: marker.token_id.clone(),
        reservation_id: marker.reservation_id.clone(),
        request_id: marker.request_id.clone(),
        audience: marker.audience.clone(),
        kind,
        actual_nano_usd: (kind == TerminalKind::Settlement).then_some(maximum),
        created_at,
    })
}

fn ensure_terminal_size(bytes: &[u8]) -> Result<(), AdmissionError> {
    if bytes.len() as u64 > TERMINAL_RESERVED_BYTES {
        Err(AdmissionError::TerminalTooLarge)
    } else {
        Ok(())
    }
}

fn claim_is_valid(marker: &ClaimMarker) -> bool {
    marker.version == 1
        && marker.terminal_reserved_bytes == TERMINAL_RESERVED_BYTES
        && validate_identifier(&marker.token_id).is_ok()
        && validate_identifier(&marker.reservation_id).is_ok()
        && validate_identifier(&marker.request_id).is_ok()
        && validate_identifier(&marker.audience).is_ok()
        && parse_positive_canonical(&marker.maximum_nano_usd).is_some()
        && (marker.state == ClaimState::Routed) == marker.routed_at.is_some()
}

fn validate_claim(marker: &ClaimMarker) -> Result<(), AdmissionError> {
    claim_is_valid(marker)
        .then_some(())
        .ok_or(AdmissionError::InvalidInput)
}

fn parse_positive_canonical(value: &str) -> Option<i128> {
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || value.starts_with('0')
    {
        return None;
    }
    value.parse::<i128>().ok().filter(|value| *value > 0)
}

fn validate_digest(value: &str) -> Result<(), AdmissionError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(AdmissionError::TerminalAcknowledgementMismatch)
    }
}

fn read_claim(path: &Path) -> Result<ClaimMarker, AdmissionError> {
    if !path.exists() {
        return Err(AdmissionError::ClaimMissing);
    }
    let marker: ClaimMarker = read_json(path).map_err(|_| AdmissionError::SpoolCorrupt)?;
    if !claim_is_valid(&marker) {
        return Err(AdmissionError::SpoolCorrupt);
    }
    Ok(marker)
}

fn read_terminal(path: &Path) -> Result<TerminalSpoolRecord, AdmissionError> {
    if !path.exists() {
        return Err(AdmissionError::ClaimMissing);
    }
    let encoded_len = std::fs::metadata(path)
        .map_err(|_| AdmissionError::SpoolCorrupt)?
        .len();
    if encoded_len > TERMINAL_RESERVED_BYTES {
        return Err(AdmissionError::SpoolCorrupt);
    }
    let wire: PlanTerminalWire = read_json(path).map_err(|_| AdmissionError::SpoolCorrupt)?;
    if wire.version != 1
        || validate_identifier(&wire.token_id).is_err()
        || validate_identifier(&wire.reservation_id).is_err()
        || validate_identifier(&wire.request_id).is_err()
        || validate_identifier(&wire.audience).is_err()
    {
        return Err(AdmissionError::SpoolCorrupt);
    }
    let actual_nano_usd = match (&wire.kind, &wire.actual_nano_usd) {
        (TerminalKind::Settlement, Some(value)) => parse_nonnegative_canonical(value),
        (TerminalKind::Release, None) => Some(None),
        _ => None,
    }
    .ok_or(AdmissionError::SpoolCorrupt)?;
    let input = TerminalSpoolInput {
        token_id: wire.token_id,
        reservation_id: wire.reservation_id,
        request_id: wire.request_id,
        audience: wire.audience,
        kind: wire.kind,
        actual_nano_usd,
        created_at: wire.created_at,
    };
    if terminal_digest(&input) != wire.canonical_digest {
        return Err(AdmissionError::SpoolCorrupt);
    }
    Ok(TerminalSpoolRecord {
        path: path.to_path_buf(),
        input,
        canonical_digest: wire.canonical_digest,
        encoded_len,
    })
}

fn parse_nonnegative_canonical(value: &str) -> Option<Option<i128>> {
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return None;
    }
    value.parse::<i128>().ok().map(Some)
}

fn final_files(directory: &Path, kind: &str) -> Result<Vec<(PathBuf, String)>, AdmissionError> {
    let prefix = format!("{kind}-");
    let mut files = Vec::new();
    for entry in std::fs::read_dir(directory).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(digest) = name
            .strip_prefix(&prefix)
            .and_then(|value| value.strip_suffix(".json"))
        else {
            continue;
        };
        if !entry.file_type().map_err(io_error)?.is_file() || validate_digest(digest).is_err() {
            return Err(AdmissionError::SpoolCorrupt);
        }
        files.push((entry.path(), digest.to_string()));
    }
    Ok(files)
}

fn validate_identifier(value: &str) -> Result<(), AdmissionError> {
    if value.is_empty()
        || value.len() > 256
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(AdmissionError::InvalidInput);
    }
    Ok(())
}

fn parse_canonical_amount(value: &str) -> Result<i128, AdmissionError> {
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(AdmissionError::NonCanonicalToken);
    }
    value.parse().map_err(|_| AdmissionError::NonCanonicalToken)
}

fn encode_json(value: &impl Serialize) -> Result<String, AdmissionError> {
    serde_json::to_vec(value)
        .map(|bytes| URL_SAFE_NO_PAD.encode(bytes))
        .map_err(serialize_error)
}

fn decode_canonical(segment: &str) -> Result<Vec<u8>, AdmissionError> {
    if segment.contains('=') {
        return Err(AdmissionError::NonCanonicalToken);
    }
    let decoded = URL_SAFE_NO_PAD
        .decode(segment)
        .map_err(|_| AdmissionError::NonCanonicalToken)?;
    if URL_SAFE_NO_PAD.encode(&decoded) != segment {
        return Err(AdmissionError::NonCanonicalToken);
    }
    Ok(decoded)
}

fn unix_time(value: i64) -> Result<DateTime<Utc>, AdmissionError> {
    Utc.timestamp_opt(value, 0)
        .single()
        .ok_or(AdmissionError::NonCanonicalToken)
}

fn digest_name(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    let mut name = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(name, "{byte:02x}");
    }
    name
}

fn durable_temp(path: &Path, bytes: &[u8]) -> std::io::Result<PathBuf> {
    durable_temp_with(
        path,
        bytes,
        |file, bytes| file.write_all(bytes),
        File::sync_all,
    )
}

fn durable_temp_with<W, S>(path: &Path, bytes: &[u8], write: W, sync: S) -> std::io::Result<PathBuf>
where
    W: FnOnce(&mut File, &[u8]) -> std::io::Result<()>,
    S: FnOnce(&File) -> std::io::Result<()>,
{
    let file_name = path.file_name().ok_or_else(|| {
        std::io::Error::new(ErrorKind::InvalidInput, "durable path has no file name")
    })?;
    let temp_path = path.with_file_name(format!(
        ".{}.{}.tmp",
        file_name.to_string_lossy(),
        Uuid::new_v4()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)?;
    let result = write(&mut file, bytes).and_then(|()| sync(&file));
    drop(file);
    match result {
        Ok(()) => Ok(temp_path),
        Err(error) => {
            let _ = std::fs::remove_file(&temp_path);
            Err(error)
        }
    }
}

fn durable_publish_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    durable_publish_new_with(
        path,
        bytes,
        |source, destination| std::fs::hard_link(source, destination),
        |temp| std::fs::remove_file(temp),
    )
}

fn durable_publish_new_with<L, R>(
    path: &Path,
    bytes: &[u8],
    link: L,
    remove: R,
) -> std::io::Result<()>
where
    L: FnOnce(&Path, &Path) -> std::io::Result<()>,
    R: FnOnce(&Path) -> std::io::Result<()>,
{
    let temp_path = durable_temp(path, bytes)?;
    match link(&temp_path, path) {
        Ok(()) => {
            let _ = remove(&temp_path);
            Ok(())
        }
        Err(error) => {
            let _ = remove(&temp_path);
            Err(error)
        }
    }
}

fn confirm_existing_terminal<F>(
    existing: TerminalSpoolInput,
    input: &TerminalSpoolInput,
    sync: F,
) -> Result<(), AdmissionError>
where
    F: FnOnce() -> Result<(), AdmissionError>,
{
    if existing != *input {
        return Err(AdmissionError::TerminalConflict);
    }
    sync()
}

fn cleanup_residual_temps(directory: &Path, kind: &str) -> Result<(), AdmissionError> {
    let mut removed = false;
    for entry in std::fs::read_dir(directory).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        if !entry.file_type().map_err(io_error)?.is_file()
            || !is_residual_temp_name(&entry.file_name(), kind)
        {
            continue;
        }
        std::fs::remove_file(entry.path()).map_err(io_error)?;
        removed = true;
    }
    if removed {
        sync_directory(directory)?;
    }
    Ok(())
}

fn is_residual_temp_name(name: &std::ffi::OsStr, kind: &str) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let prefix = format!(".{kind}-");
    let Some(body) = name
        .strip_prefix(&prefix)
        .and_then(|value| value.strip_suffix(".tmp"))
    else {
        return false;
    };
    let Some((final_name, uuid_text)) = body.rsplit_once('.') else {
        return false;
    };
    let Some(digest) = final_name.strip_suffix(".json") else {
        return false;
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return false;
    }
    let Ok(uuid) = Uuid::parse_str(uuid_text) else {
        return false;
    };
    uuid.get_version_num() == 4
        && uuid.get_variant() == uuid::Variant::RFC4122
        && uuid.hyphenated().to_string() == uuid_text
}

fn durable_replace(path: &Path, bytes: &[u8]) -> Result<(), AdmissionError> {
    let temp_path = durable_temp(path, bytes).map_err(io_error)?;
    let replaced = atomic_replace(&temp_path, path);
    if replaced.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    replaced.map_err(io_error)
}

#[cfg(unix)]
fn atomic_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt as _;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, AdmissionError> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(io_error)?;
    serde_json::from_slice(&bytes).map_err(serialize_error)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), AdmissionError> {
    File::open(path)?.sync_all().map_err(io_error)
}

#[cfg(windows)]
fn sync_directory(path: &Path) -> Result<(), AdmissionError> {
    use std::os::windows::fs::OpenOptionsExt as _;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    OpenOptions::new()
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(io_error)
}

fn io_error(error: std::io::Error) -> AdmissionError {
    AdmissionError::Io(error.to_string())
}

fn serialize_error(error: serde_json::Error) -> AdmissionError {
    AdmissionError::Serialize(error.to_string())
}

impl From<std::io::Error> for AdmissionError {
    fn from(value: std::io::Error) -> Self {
        io_error(value)
    }
}

#[cfg(test)]
mod durable_tests {
    use std::cell::Cell;
    use std::io::{Error, ErrorKind, Write as _};
    use std::sync::Arc;

    use chrono::{Duration, TimeZone, Utc};
    use tempfile::tempdir;

    use super::{
        AcknowledgementFaults, AdmissionClaimStore, AdmissionError, TerminalKind,
        TerminalSpoolInput, VerifiedAdmission, confirm_existing_terminal, durable_publish_new_with,
        durable_temp_with, ensure_terminal_size,
    };
    use crate::replica::metering::MeteringSpoolCapacity;

    fn verified_admission(now: chrono::DateTime<Utc>) -> VerifiedAdmission {
        VerifiedAdmission {
            issuer: "lynshen-primary".to_string(),
            key_id: "key".to_string(),
            audience: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_string(),
            token_id: "fault-token".to_string(),
            reservation_id: "fault-reservation".to_string(),
            request_id: "fault-request".to_string(),
            entitlement_id: "fault-entitlement".to_string(),
            generation: 1,
            maximum_nano_usd: 100,
            reserved_fen_cny: 1,
            pricing_revision: "pricing-v1".to_string(),
            issued_at: now,
            expires_at: now + Duration::seconds(30),
        }
    }

    #[tokio::test]
    async fn claims_directory_sync_failure_is_retryable_with_exact_capacity() {
        let now = Utc.with_ymd_and_hms(2026, 8, 28, 11, 0, 0).unwrap();
        let admission = verified_admission(now);
        let temp = tempdir().unwrap();
        let capacity = Arc::new(MeteringSpoolCapacity::new(1024 * 1024));
        let faults = Arc::new(AcknowledgementFaults::default());
        let claims = AdmissionClaimStore::new_with_capacity(temp.path(), capacity.clone())
            .await
            .unwrap()
            .with_acknowledgement_faults(faults.clone());
        let marker_path = claims.claim(&admission).await.unwrap();
        let terminal = claims
            .spool_terminal(TerminalSpoolInput::release(&admission, now))
            .await
            .unwrap();

        faults.fail_claims_sync_once();
        assert!(
            claims
                .acknowledge_terminal(&admission.token_id, &terminal.canonical_digest, now)
                .await
                .is_err()
        );
        assert!(terminal.path.exists());
        assert_eq!(
            capacity.accounted_bytes(),
            std::fs::metadata(&marker_path).unwrap().len()
                + std::fs::metadata(&terminal.path).unwrap().len()
        );

        claims
            .acknowledge_terminal(
                &admission.token_id,
                &terminal.canonical_digest,
                now + Duration::seconds(1),
            )
            .await
            .unwrap();
        assert!(!terminal.path.exists());
        assert_eq!(
            capacity.accounted_bytes(),
            std::fs::metadata(marker_path).unwrap().len()
        );
    }

    #[tokio::test]
    async fn terminal_directory_sync_failure_is_retryable_with_exact_capacity() {
        let now = Utc.with_ymd_and_hms(2026, 8, 28, 11, 1, 0).unwrap();
        let admission = verified_admission(now);
        let temp = tempdir().unwrap();
        let capacity = Arc::new(MeteringSpoolCapacity::new(1024 * 1024));
        let faults = Arc::new(AcknowledgementFaults::default());
        let claims = AdmissionClaimStore::new_with_capacity(temp.path(), capacity.clone())
            .await
            .unwrap()
            .with_acknowledgement_faults(faults.clone());
        let marker_path = claims.claim(&admission).await.unwrap();
        let terminal = claims
            .spool_terminal(TerminalSpoolInput::release(&admission, now))
            .await
            .unwrap();

        faults.fail_terminal_sync_once();
        assert!(
            claims
                .acknowledge_terminal(&admission.token_id, &terminal.canonical_digest, now)
                .await
                .is_err()
        );
        assert!(!terminal.path.exists());
        let expected = std::fs::metadata(&marker_path).unwrap().len();
        assert_eq!(capacity.accounted_bytes(), expected);

        claims
            .acknowledge_terminal(
                &admission.token_id,
                &terminal.canonical_digest,
                now + Duration::seconds(1),
            )
            .await
            .unwrap();
        assert_eq!(capacity.accounted_bytes(), expected);
    }

    #[test]
    fn durable_temp_removes_partial_file_after_write_or_sync_failure() {
        for fail_during_write in [true, false] {
            let directory = tempdir().unwrap();
            let final_path = directory.path().join("claim-final.json");
            let error = durable_temp_with(
                &final_path,
                b"payload",
                |file, bytes| {
                    file.write_all(&bytes[..1])?;
                    if fail_during_write {
                        Err(Error::new(ErrorKind::WriteZero, "injected write failure"))
                    } else {
                        file.write_all(&bytes[1..])
                    }
                },
                |_| Err(Error::other("injected sync failure")),
            )
            .unwrap_err();
            assert_eq!(
                error.kind(),
                if fail_during_write {
                    ErrorKind::WriteZero
                } else {
                    ErrorKind::Other
                }
            );
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        }
    }

    #[test]
    fn durable_publish_keeps_link_result_and_best_effort_cleans_temp() {
        let failed = tempdir().unwrap();
        let failed_path = failed.path().join("claim-final.json");
        let cleanup_called = Cell::new(false);
        let error = durable_publish_new_with(
            &failed_path,
            b"payload",
            |_, _| Err(Error::new(ErrorKind::PermissionDenied, "link denied")),
            |temp| {
                cleanup_called.set(true);
                std::fs::remove_file(temp)
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert!(cleanup_called.get());
        assert_eq!(std::fs::read_dir(failed.path()).unwrap().count(), 0);

        let published = tempdir().unwrap();
        let published_path = published.path().join("claim-final.json");
        durable_publish_new_with(
            &published_path,
            b"payload",
            |source, destination| std::fs::hard_link(source, destination),
            |_| Err(Error::new(ErrorKind::PermissionDenied, "remove denied")),
        )
        .unwrap();
        assert_eq!(std::fs::read(published_path).unwrap(), b"payload");
        assert_eq!(std::fs::read_dir(published.path()).unwrap().count(), 2);
    }

    #[test]
    fn exact_terminal_retry_requires_directory_sync() {
        let input = TerminalSpoolInput {
            token_id: "token".to_string(),
            reservation_id: "reservation".to_string(),
            request_id: "request".to_string(),
            audience: "replica".to_string(),
            kind: TerminalKind::Release,
            actual_nano_usd: None,
            created_at: chrono::Utc::now(),
        };
        let error = confirm_existing_terminal(input.clone(), &input, || {
            Err(AdmissionError::Io("directory sync failed".to_string()))
        })
        .unwrap_err();
        assert_eq!(
            error,
            AdmissionError::Io("directory sync failed".to_string())
        );

        let mut changed = input.clone();
        changed.request_id = "changed".to_string();
        let sync_called = Cell::new(false);
        assert_eq!(
            confirm_existing_terminal(changed, &input, || {
                sync_called.set(true);
                Ok(())
            })
            .unwrap_err(),
            AdmissionError::TerminalConflict
        );
        assert!(!sync_called.get());
    }

    #[test]
    fn terminal_payload_larger_than_the_reserved_bytes_is_rejected() {
        assert_eq!(
            ensure_terminal_size(&vec![b'x'; 4097]).unwrap_err(),
            AdmissionError::TerminalTooLarge
        );
        ensure_terminal_size(&vec![b'x'; 4096]).unwrap();
    }
}
