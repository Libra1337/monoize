use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use monoize::store_billing::credentials::CredentialVersion;
use monoize::store_billing::crypto::{
    CryptoError, Ed25519KeyPair, PaymentKey, PaymentKeyRing, sign_rsa_sha256_base64,
    verify_hmac_sha256_hex, verify_rsa_sha256_base64, wechat_decrypt_resource,
};
use rand_core::OsRng;
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::{RsaPrivateKey, RsaPublicKey};

fn key(id: &str, byte: u8) -> PaymentKey {
    PaymentKey::new(id, [byte; 32]).unwrap()
}

#[test]
fn payment_ciphertext_is_bound_to_record_identity() {
    let ring = PaymentKeyRing::new(key("active", 7), vec![]).unwrap();
    let encrypted = ring
        .encrypt(
            "store_channel_credentials:channel-a:secret",
            b"merchant-secret",
        )
        .unwrap();

    assert_eq!(
        ring.decrypt("store_channel_credentials:channel-b:secret", &encrypted)
            .unwrap_err(),
        CryptoError::Authentication
    );
    assert_eq!(
        ring.decrypt("store_channel_credentials:channel-a:secret", &encrypted)
            .unwrap()
            .as_slice(),
        b"merchant-secret"
    );
}

#[test]
fn payment_key_ring_decrypts_with_a_prior_key() {
    let old_ring = PaymentKeyRing::new(key("old", 3), vec![]).unwrap();
    let encrypted = old_ring
        .encrypt("store_redemption_codes:code-a:full_code", b"ABCD-EFGH")
        .unwrap();
    let rotated = PaymentKeyRing::new(key("new", 9), vec![key("old", 3)]).unwrap();

    assert_eq!(
        rotated
            .decrypt("store_redemption_codes:code-a:full_code", &encrypted)
            .unwrap()
            .as_slice(),
        b"ABCD-EFGH"
    );
    assert_eq!(encrypted.key_id, "old");
}

#[test]
fn stripe_hmac_verification_rejects_changed_payloads() {
    let secret = b"whsec_test";
    let payload = b"1710000000.{\"id\":\"evt_1\"}";
    let signature = "9aa6c76983dca6e776444eb182125d8bccacdda762f1172107a4a7a3ab4fca1e";

    assert!(verify_hmac_sha256_hex(secret, payload, signature).is_ok());
    assert_eq!(
        verify_hmac_sha256_hex(secret, b"changed", signature).unwrap_err(),
        CryptoError::Authentication
    );
}

#[test]
fn rsa2_signature_verifies_the_exact_canonical_bytes() {
    let private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
    let public = RsaPublicKey::from(&private);
    let private_pem = private.to_pkcs8_pem(LineEnding::LF).unwrap();
    let public_pem = public.to_public_key_pem(LineEnding::LF).unwrap();
    let message = b"app_id=lynshen&out_trade_no=order-1&total_amount=1.00";

    let signature = sign_rsa_sha256_base64(private_pem.as_str(), message).unwrap();
    assert!(verify_rsa_sha256_base64(&public_pem, message, &signature).is_ok());
    assert_eq!(
        verify_rsa_sha256_base64(&public_pem, b"changed", &signature).unwrap_err(),
        CryptoError::Authentication
    );
}

#[test]
fn wechat_resource_decryption_authenticates_associated_data() {
    let key = [11_u8; 32];
    let nonce = *b"0123456789ab";
    let aad = b"transaction";
    let plaintext = br#"{\"out_trade_no\":\"order-1\"}"#;
    let encrypted = Aes256Gcm::new_from_slice(&key)
        .unwrap()
        .encrypt(
            &Nonce::try_from(nonce.as_slice()).unwrap(),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .unwrap();

    assert_eq!(
        wechat_decrypt_resource(&key, &nonce, aad, &STANDARD.encode(&encrypted)).unwrap(),
        plaintext
    );
    assert_eq!(
        wechat_decrypt_resource(&key, &nonce, b"changed", &STANDARD.encode(&encrypted))
            .unwrap_err(),
        CryptoError::Authentication
    );
}

#[test]
fn admission_signature_binds_key_id_and_payload() {
    let key_pair = Ed25519KeyPair::from_seed("admission-2026-08", [13_u8; 32]).unwrap();
    let signature = key_pair.sign(b"reservation-1");

    assert_eq!(signature.key_id, "admission-2026-08");
    assert!(key_pair.verify(b"reservation-1", &signature).is_ok());
    assert_eq!(
        key_pair.verify(b"reservation-2", &signature).unwrap_err(),
        CryptoError::Authentication
    );
}

#[test]
fn credential_public_view_omits_encrypted_secret_material() {
    let ring = PaymentKeyRing::new(key("active", 7), vec![]).unwrap();
    let encrypted = ring
        .encrypt(
            "store_channel_credentials:credential-1:secret",
            b"merchant-secret",
        )
        .unwrap();
    let credential = CredentialVersion::new(
        "credential-1",
        "channel-1",
        "stripe",
        "account-digest",
        encrypted.clone(),
    );

    let encoded = serde_json::to_string(&credential.public_view()).unwrap();
    assert!(!encoded.contains("merchant-secret"));
    assert!(!encoded.contains(&encrypted.ciphertext_base64));
    assert!(encoded.contains("account-digest"));
    assert!(encoded.contains("active"));
}
