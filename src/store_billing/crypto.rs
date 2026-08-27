use std::collections::HashMap;
use std::fmt;

use aes_gcm::aead::{Aead as _, KeyInit as _, Payload as AesPayload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chacha20poly1305::aead::{Generate as _, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use hmac::{Hmac, Mac as _};
use rsa::pkcs8::{DecodePrivateKey as _, DecodePublicKey as _};
use rsa::sha2::{Digest as _, Sha256 as RsaSha256};
use rsa::{Pkcs1v15Sign, RsaPrivateKey, RsaPublicKey};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq as _;
use zeroize::Zeroizing;

const SECRET_FORMAT_VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CryptoError {
    #[error("invalid payment key")]
    InvalidKey,
    #[error("duplicate payment key id")]
    DuplicateKey,
    #[error("payment key id is unavailable")]
    UnknownKey,
    #[error("invalid encrypted value")]
    InvalidEncoding,
    #[error("authentication failed")]
    Authentication,
    #[error("invalid signature key")]
    InvalidSignatureKey,
}

#[derive(Clone)]
pub struct PaymentKey {
    id: String,
    bytes: [u8; 32],
}

impl PaymentKey {
    pub fn new(id: impl Into<String>, bytes: [u8; 32]) -> Result<Self, CryptoError> {
        let id = id.into();
        if id.is_empty()
            || id.len() > 64
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(CryptoError::InvalidKey);
        }
        Ok(Self { id, bytes })
    }
}

impl fmt::Debug for PaymentKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PaymentKey")
            .field("id", &self.id)
            .field("bytes", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptedSecret {
    pub version: u8,
    pub key_id: String,
    pub nonce_base64: String,
    pub ciphertext_base64: String,
}

#[derive(Clone)]
pub struct PaymentKeyRing {
    active_key_id: String,
    keys: HashMap<String, PaymentKey>,
}

impl PaymentKeyRing {
    pub fn new(active: PaymentKey, prior: Vec<PaymentKey>) -> Result<Self, CryptoError> {
        let active_key_id = active.id.clone();
        let mut keys = HashMap::with_capacity(prior.len() + 1);
        keys.insert(active.id.clone(), active);
        for key in prior {
            if keys.insert(key.id.clone(), key).is_some() {
                return Err(CryptoError::DuplicateKey);
            }
        }
        Ok(Self {
            active_key_id,
            keys,
        })
    }

    pub fn encrypt(&self, aad: &str, plaintext: &[u8]) -> Result<EncryptedSecret, CryptoError> {
        let key = self
            .keys
            .get(&self.active_key_id)
            .ok_or(CryptoError::UnknownKey)?;
        let cipher =
            XChaCha20Poly1305::new_from_slice(&key.bytes).map_err(|_| CryptoError::InvalidKey)?;
        let nonce = XNonce::generate();
        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| CryptoError::Authentication)?;
        Ok(EncryptedSecret {
            version: SECRET_FORMAT_VERSION,
            key_id: key.id.clone(),
            nonce_base64: STANDARD.encode(nonce),
            ciphertext_base64: STANDARD.encode(ciphertext),
        })
    }

    pub fn decrypt(
        &self,
        aad: &str,
        encrypted: &EncryptedSecret,
    ) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
        if encrypted.version != SECRET_FORMAT_VERSION {
            return Err(CryptoError::InvalidEncoding);
        }
        let key = self
            .keys
            .get(&encrypted.key_id)
            .ok_or(CryptoError::UnknownKey)?;
        let nonce = STANDARD
            .decode(&encrypted.nonce_base64)
            .map_err(|_| CryptoError::InvalidEncoding)?;
        let nonce = XNonce::try_from(nonce.as_slice()).map_err(|_| CryptoError::InvalidEncoding)?;
        let ciphertext = STANDARD
            .decode(&encrypted.ciphertext_base64)
            .map_err(|_| CryptoError::InvalidEncoding)?;
        let cipher =
            XChaCha20Poly1305::new_from_slice(&key.bytes).map_err(|_| CryptoError::InvalidKey)?;
        cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: &ciphertext,
                    aad: aad.as_bytes(),
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| CryptoError::Authentication)
    }
}

impl fmt::Debug for PaymentKeyRing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut key_ids = self.keys.keys().cloned().collect::<Vec<_>>();
        key_ids.sort();
        formatter
            .debug_struct("PaymentKeyRing")
            .field("active_key_id", &self.active_key_id)
            .field("key_ids", &key_ids)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ed25519Signature {
    pub key_id: String,
    pub signature_base64: String,
}

#[derive(Clone)]
pub struct Ed25519KeyPair {
    key_id: String,
    signing_key: SigningKey,
}

impl Ed25519KeyPair {
    pub fn from_seed(key_id: impl Into<String>, seed: [u8; 32]) -> Result<Self, CryptoError> {
        let key_id = key_id.into();
        validate_key_id(&key_id)?;
        Ok(Self {
            key_id,
            signing_key: SigningKey::from_bytes(&seed),
        })
    }

    pub fn sign(&self, payload: &[u8]) -> Ed25519Signature {
        Ed25519Signature {
            key_id: self.key_id.clone(),
            signature_base64: STANDARD.encode(self.signing_key.sign(payload).to_bytes()),
        }
    }

    pub fn verify(&self, payload: &[u8], signed: &Ed25519Signature) -> Result<(), CryptoError> {
        if signed.key_id != self.key_id {
            return Err(CryptoError::Authentication);
        }
        verify_ed25519(
            &self.signing_key.verifying_key(),
            payload,
            &signed.signature_base64,
        )
    }

    pub fn verifying_key_bytes(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }
}

pub fn verify_ed25519(
    verifying_key: &VerifyingKey,
    payload: &[u8],
    signature_base64: &str,
) -> Result<(), CryptoError> {
    let signature = STANDARD
        .decode(signature_base64)
        .map_err(|_| CryptoError::InvalidEncoding)?;
    let signature = Signature::from_slice(&signature).map_err(|_| CryptoError::InvalidEncoding)?;
    verifying_key
        .verify(payload, &signature)
        .map_err(|_| CryptoError::Authentication)
}

pub fn sign_rsa_sha256_base64(
    private_key_pem: &str,
    payload: &[u8],
) -> Result<String, CryptoError> {
    let private_key = RsaPrivateKey::from_pkcs8_pem(private_key_pem)
        .map_err(|_| CryptoError::InvalidSignatureKey)?;
    let digest = RsaSha256::digest(payload);
    let signature = private_key
        .sign(Pkcs1v15Sign::new::<RsaSha256>(), &digest)
        .map_err(|_| CryptoError::InvalidSignatureKey)?;
    Ok(STANDARD.encode(signature))
}

pub fn verify_rsa_sha256_base64(
    public_key_pem: &str,
    payload: &[u8],
    signature_base64: &str,
) -> Result<(), CryptoError> {
    let public_key = RsaPublicKey::from_public_key_pem(public_key_pem)
        .map_err(|_| CryptoError::InvalidSignatureKey)?;
    let signature = STANDARD
        .decode(signature_base64)
        .map_err(|_| CryptoError::InvalidEncoding)?;
    let digest = RsaSha256::digest(payload);
    public_key
        .verify(Pkcs1v15Sign::new::<RsaSha256>(), &digest, &signature)
        .map_err(|_| CryptoError::Authentication)
}

pub fn verify_hmac_sha256_hex(
    secret: &[u8],
    payload: &[u8],
    expected_hex: &str,
) -> Result<(), CryptoError> {
    if expected_hex.len() != 64 || !expected_hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CryptoError::InvalidEncoding);
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).map_err(|_| CryptoError::InvalidKey)?;
    mac.update(payload);
    let actual = mac.finalize().into_bytes();
    let actual_hex = lower_hex(actual.as_slice());
    if actual_hex
        .as_bytes()
        .ct_eq(expected_hex.to_ascii_lowercase().as_bytes())
        .into()
    {
        Ok(())
    } else {
        Err(CryptoError::Authentication)
    }
}

pub fn wechat_decrypt_resource(
    api_v3_key: &[u8; 32],
    nonce: &[u8; 12],
    associated_data: &[u8],
    ciphertext_base64: &str,
) -> Result<Vec<u8>, CryptoError> {
    let ciphertext = STANDARD
        .decode(ciphertext_base64)
        .map_err(|_| CryptoError::InvalidEncoding)?;
    let nonce = Nonce::try_from(nonce.as_slice()).map_err(|_| CryptoError::InvalidEncoding)?;
    Aes256Gcm::new_from_slice(api_v3_key)
        .map_err(|_| CryptoError::InvalidKey)?
        .decrypt(
            &nonce,
            AesPayload {
                msg: &ciphertext,
                aad: associated_data,
            },
        )
        .map_err(|_| CryptoError::Authentication)
}

fn validate_key_id(key_id: &str) -> Result<(), CryptoError> {
    if key_id.is_empty()
        || key_id.len() > 64
        || !key_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(CryptoError::InvalidKey);
    }
    Ok(())
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
