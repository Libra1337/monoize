use chrono::{Duration, Utc};
use monoize::db::DbPool;
use monoize::migration::Migrator;
use monoize::store_billing::crypto::{PaymentKey, PaymentKeyRing};
use monoize::store_billing::money::Currency;
use monoize::store_billing::redemption::{
    RedemptionAccessAction, RedemptionAuditContext, RevealRedemptionInput,
};
use monoize::store_billing::{
    GenerateRedemptionCodesInput, RedemptionCodeStatus, RedemptionRewardInput, StoreBillingError,
    StoreBillingStore,
};
use sea_orm::ConnectionTrait;
use sea_orm_migration::MigratorTrait;
use sha2::{Digest, Sha256};
use std::sync::Arc;

async fn setup() -> (DbPool, StoreBillingStore, Arc<PaymentKeyRing>) {
    let db = DbPool::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    Migrator::up(&*db.write().await, None)
        .await
        .expect("run migrations");
    db.write()
        .await
        .execute_unprepared(
            "INSERT INTO users
                (id, username, password_hash, role, created_at, updated_at, enabled,
                 balance_nano_usd, balance_unlimited, group_id)
             SELECT 'redemption-user', 'redemption-user', 'test', 'user',
                    '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z', 1, '0', 0, id
             FROM monoize_groups WHERE is_default = 1 LIMIT 1",
        )
        .await
        .unwrap();
    let keys = Arc::new(
        PaymentKeyRing::new(
            PaymentKey::new("redemption-key", [61_u8; 32]).unwrap(),
            vec![],
        )
        .unwrap(),
    );
    (db.clone(), StoreBillingStore::new(db), keys)
}

fn balance_reward(count: u32) -> GenerateRedemptionCodesInput {
    GenerateRedemptionCodesInput {
        reward: RedemptionRewardInput::Balance {
            currency: Currency::USD,
            amount_minor: "100".to_string(),
        },
        count,
        validity_days: 30,
    }
}

fn audit(action: RedemptionAccessAction) -> (RevealRedemptionInput, RedemptionAuditContext) {
    (
        RevealRedemptionInput {
            code_ids: Vec::new(),
            action,
        },
        RedemptionAuditContext {
            admin_user_id: "redemption-admin".to_string(),
            source_ip: "203.0.113.41".to_string(),
            user_agent: "Store security test".to_string(),
        },
    )
}

#[tokio::test]
async fn generated_v2_codes_are_returned_once_and_stored_as_bound_ciphertext() {
    let (db, store, keys) = setup().await;
    let generated = store
        .generate_redemption_codes(keys.as_ref(), "redemption-admin", balance_reward(2))
        .await
        .unwrap();

    assert_eq!(generated.len(), 2);
    for item in &generated {
        assert_eq!(item.code.len(), 19);
        assert_eq!(item.code.chars().filter(|value| *value == '-').count(), 3);
        assert!(
            item.code.bytes().all(|byte| {
                byte == b'-' || b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&byte)
            })
        );
    }
    let rows = db
        .read()
        .query_all(db.stmt(
            "SELECT code_format_version, encrypted_format_version, encrypted_key_id,
                    encrypted_nonce_base64, encrypted_ciphertext_base64
             FROM store_redemption_codes ORDER BY id",
            vec![],
        ))
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    for row in rows {
        assert_eq!(row.try_get::<i32>("", "code_format_version").unwrap(), 2);
        assert_eq!(
            row.try_get::<i32>("", "encrypted_format_version").unwrap(),
            1
        );
        assert_eq!(
            row.try_get::<String>("", "encrypted_key_id").unwrap(),
            "redemption-key"
        );
        let ciphertext = row
            .try_get::<String>("", "encrypted_ciphertext_base64")
            .unwrap();
        assert!(
            generated
                .iter()
                .all(|item| !ciphertext.contains(&item.code))
        );
    }
}

#[tokio::test]
async fn reveal_and_copy_require_v2_unused_ids_and_audit_without_plaintext() {
    let (db, store, keys) = setup().await;
    let generated = store
        .generate_redemption_codes(keys.as_ref(), "redemption-admin", balance_reward(2))
        .await
        .unwrap();
    let (mut input, context) = audit(RedemptionAccessAction::Reveal);
    input.code_ids = generated
        .iter()
        .map(|item| item.record.id.clone())
        .collect();

    let revealed = store
        .reveal_redemption_codes(keys.as_ref(), input, &context)
        .await
        .unwrap();
    assert_eq!(
        revealed
            .iter()
            .map(|item| item.code.as_str())
            .collect::<Vec<_>>(),
        generated
            .iter()
            .map(|item| item.code.as_str())
            .collect::<Vec<_>>()
    );
    let audit = db
        .read()
        .query_one(db.stmt(
            "SELECT action, scope_json FROM store_access_audits
             WHERE actor_id = 'redemption-admin'",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        audit.try_get::<String>("", "action").unwrap(),
        "redemption_reveal"
    );
    let scope = audit.try_get::<String>("", "scope_json").unwrap();
    assert!(scope.contains("203.0.113.41"));
    assert!(scope.contains("Store security test"));
    assert!(generated.iter().all(|item| !scope.contains(&item.code)));
}

#[tokio::test]
async fn reveal_limits_and_record_bound_aad_fail_closed() {
    let (db, store, keys) = setup().await;
    let generated = store
        .generate_redemption_codes(keys.as_ref(), "redemption-admin", balance_reward(20))
        .await
        .unwrap();
    let (mut input, context) = audit(RedemptionAccessAction::Reveal);
    input.code_ids = generated
        .iter()
        .map(|item| item.record.id.clone())
        .collect();
    input.code_ids.push("one-too-many".to_string());
    assert_eq!(
        store
            .reveal_redemption_codes(keys.as_ref(), input, &context)
            .await
            .unwrap_err(),
        StoreBillingError::InvalidInput
    );

    let first = &generated[0].record.id;
    let second = &generated[1].record.id;
    db.write()
        .await
        .execute(db.stmt(
            "UPDATE store_redemption_codes
             SET encrypted_nonce_base64 = (SELECT encrypted_nonce_base64
                                             FROM store_redemption_codes WHERE id = $2),
                 encrypted_ciphertext_base64 = (SELECT encrypted_ciphertext_base64
                                                  FROM store_redemption_codes WHERE id = $2)
             WHERE id = $1",
            vec![first.clone().into(), second.clone().into()],
        ))
        .await
        .unwrap();
    let (mut input, context) = audit(RedemptionAccessAction::Reveal);
    input.code_ids = vec![first.clone()];
    assert_eq!(
        store
            .reveal_redemption_codes(keys.as_ref(), input, &context)
            .await
            .unwrap_err(),
        StoreBillingError::EncryptionUnavailable
    );
}

#[tokio::test]
async fn redemption_and_revocation_delete_recoverable_ciphertext() {
    let (db, store, keys) = setup().await;
    let generated = store
        .generate_redemption_codes(keys.as_ref(), "redemption-admin", balance_reward(2))
        .await
        .unwrap();

    store
        .redeem(
            "redemption-user",
            &generated[0].code.to_ascii_lowercase(),
            None,
            "203.0.113.51",
        )
        .await
        .unwrap();
    store
        .revoke_redemption_code(&generated[1].record.id, "redemption-admin")
        .await
        .unwrap();

    let rows = db
        .read()
        .query_all(db.stmt(
            "SELECT status, encrypted_ciphertext_base64
             FROM store_redemption_codes ORDER BY status",
            vec![],
        ))
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| {
        row.try_get::<Option<String>>("", "encrypted_ciphertext_base64")
            .unwrap()
            .is_none()
    }));
    let records = store.list_redemption_codes_admin(10).await.unwrap();
    assert!(
        records
            .iter()
            .any(|record| record.status == RedemptionCodeStatus::Used)
    );
    assert!(
        records
            .iter()
            .any(|record| record.status == RedemptionCodeStatus::Revoked)
    );
}

#[tokio::test]
async fn cleanup_removes_only_ciphertext_expired_for_more_than_twenty_four_hours() {
    let (db, store, keys) = setup().await;
    let generated = store
        .generate_redemption_codes(keys.as_ref(), "redemption-admin", balance_reward(2))
        .await
        .unwrap();
    let now = Utc::now();
    let write = db.write().await;
    write
        .execute(db.stmt(
            "UPDATE store_redemption_codes SET expires_at = $2 WHERE id = $1",
            vec![
                generated[0].record.id.clone().into(),
                (now - Duration::hours(25)).to_rfc3339().into(),
            ],
        ))
        .await
        .unwrap();
    write
        .execute(db.stmt(
            "UPDATE store_redemption_codes SET expires_at = $2 WHERE id = $1",
            vec![
                generated[1].record.id.clone().into(),
                (now - Duration::hours(23)).to_rfc3339().into(),
            ],
        ))
        .await
        .unwrap();
    drop(write);

    assert_eq!(
        store
            .cleanup_expired_redemption_ciphertexts(now)
            .await
            .unwrap(),
        1
    );
    let rows = db
        .read()
        .query_all(db.stmt(
            "SELECT id, encrypted_ciphertext_base64 FROM store_redemption_codes ORDER BY id",
            vec![],
        ))
        .await
        .unwrap();
    assert_eq!(
        rows.iter()
            .filter(|row| row
                .try_get::<Option<String>>("", "encrypted_ciphertext_base64")
                .unwrap()
                .is_some())
            .count(),
        1
    );
}

#[tokio::test]
async fn failed_redemptions_enter_persistent_cooldown_before_code_lookup() {
    let (_db, store, _keys) = setup().await;
    for suffix in 0..5 {
        assert_eq!(
            store
                .redeem(
                    "redemption-user",
                    &format!("invalid code {suffix}"),
                    None,
                    "203.0.113.61",
                )
                .await
                .unwrap_err(),
            StoreBillingError::InvalidRedemptionCode
        );
    }
    assert_eq!(
        store
            .redeem(
                "redemption-user",
                "0000-0000-0000-9999",
                None,
                "203.0.113.61",
            )
            .await
            .unwrap_err(),
        StoreBillingError::RedemptionCooldown
    );
}

#[tokio::test]
async fn legacy_v1_digest_codes_remain_redeemable_but_never_recoverable() {
    let (db, store, keys) = setup().await;
    let normalized = "ABCD2345EFGH6789";
    let digest = Sha256::digest(normalized.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO store_redemption_codes
                (id, code_format_version, code_digest, code_hint,
                 encrypted_format_version, encrypted_key_id, encrypted_nonce_base64,
                 encrypted_ciphertext_base64, ciphertext_destroyed_at,
                 reward_kind, reward_json, status, expires_at,
                 redeemed_by_user_id, redeemed_at, revoked_at,
                 created_by_user_id, created_at)
             VALUES ('legacy-v1-code', 1, $1, '6789', NULL, NULL, NULL, NULL, NULL,
                     'balance', $2, 'unused', $3, NULL, NULL, NULL,
                     'legacy-admin', $4)",
            vec![
                digest.into(),
                serde_json::json!({
                    "kind":"balance","currency":"USD","amount_minor":"100"
                })
                .to_string()
                .into(),
                (Utc::now() + Duration::days(30)).to_rfc3339().into(),
                Utc::now().to_rfc3339().into(),
            ],
        ))
        .await
        .unwrap();

    let redeemed = store
        .redeem(
            "redemption-user",
            "abcd-2345-efgh-6789",
            None,
            "203.0.113.62",
        )
        .await
        .unwrap();
    assert_eq!(redeemed.status, RedemptionCodeStatus::Used);
    let row = db
        .read()
        .query_one(db.stmt(
            "SELECT ciphertext_destroyed_at FROM store_redemption_codes
             WHERE id = 'legacy-v1-code'",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.try_get::<Option<String>>("", "ciphertext_destroyed_at")
            .unwrap(),
        None
    );
    let (mut input, context) = audit(RedemptionAccessAction::Reveal);
    input.code_ids = vec!["legacy-v1-code".to_string()];
    assert_eq!(
        store
            .reveal_redemption_codes(keys.as_ref(), input, &context)
            .await
            .unwrap_err(),
        StoreBillingError::InvalidRedemptionCode
    );
}
