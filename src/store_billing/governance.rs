use chrono::{DateTime, Duration, SecondsFormat, Utc};
use sea_orm::{ConnectionTrait, DbErr, QueryResult};
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use uuid::Uuid;

use super::models::{
    CheckoutActionKind, ConfirmStoreComplianceInput, CreateStorePrivacyRecordInput,
    MerchantCapabilityKind, PutStoreChannelReadinessInput, PutStoreMerchantCapabilityInput,
    StoreAmountLimit, StoreChannelAvailability, StoreChannelReadinessProfile,
    StoreChannelReadinessView, StoreComplianceView, StoreMerchantCapabilitiesView,
    StoreMerchantCapability, StorePaymentCompliance, StorePrivacyRecord, StorePrivacyRecordsView,
};
use super::money::{Currency, parse_minor};
use super::store::StoreBillingError;
use crate::db::DbPool;

pub const CURRENT_STORE_PAYMENT_TERMS_VERSION: &str = "2026-08-28";

const REQUIRED_CAPABILITIES: [&str; 4] = [
    "payment_query",
    "refund",
    "refund_query",
    "settlement_report",
];

#[derive(Debug, Clone)]
pub struct PaymentGovernanceStore {
    db: DbPool,
}

impl PaymentGovernanceStore {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }

    pub async fn compliance(
        &self,
        channel_id: &str,
    ) -> Result<StoreComplianceView, StoreBillingError> {
        self.require_channel(channel_id).await?;
        let row = self
            .db
            .read()
            .query_one(self.db.stmt(
                "SELECT id, channel_id, terms_version, admin_user_id, source_ip,
                        confirmed_at, invalidated_at
                 FROM store_payment_compliance
                 WHERE channel_id = $1 AND invalidated_at IS NULL
                 ORDER BY confirmed_at DESC, id DESC LIMIT 1",
                vec![channel_id.into()],
            ))
            .await
            .map_err(storage)?;
        Ok(StoreComplianceView {
            current_terms_version: CURRENT_STORE_PAYMENT_TERMS_VERSION.to_string(),
            compliance: row.map(compliance_from_row).transpose()?,
        })
    }

    pub async fn confirm_compliance(
        &self,
        channel_id: &str,
        input: ConfirmStoreComplianceInput,
        admin_user_id: &str,
        source_ip: &str,
    ) -> Result<StorePaymentCompliance, StoreBillingError> {
        if !input.confirmed || input.terms_version != CURRENT_STORE_PAYMENT_TERMS_VERSION {
            return Err(StoreBillingError::InvalidInput);
        }
        let tx = self.db.begin_write().await.map_err(storage)?;
        if !lock_channel(&self.db, &*tx, channel_id)
            .await
            .map_err(storage)?
        {
            return Err(StoreBillingError::NotFound);
        }
        let confirmed_at = Utc::now();
        tx.execute(self.db.stmt(
            "UPDATE store_payment_compliance SET invalidated_at = $2
             WHERE channel_id = $1 AND invalidated_at IS NULL",
            vec![channel_id.into(), timestamp(confirmed_at).into()],
        ))
        .await
        .map_err(storage)?;
        let compliance = StorePaymentCompliance {
            id: Uuid::new_v4().to_string(),
            channel_id: channel_id.to_string(),
            terms_version: CURRENT_STORE_PAYMENT_TERMS_VERSION.to_string(),
            admin_user_id: admin_user_id.to_string(),
            source_ip: source_ip.to_string(),
            confirmed_at,
            invalidated_at: None,
        };
        tx.execute(self.db.stmt(
            "INSERT INTO store_payment_compliance
                (id, channel_id, terms_version, admin_user_id, source_ip,
                 confirmed_at, invalidated_at)
             VALUES ($1, $2, $3, $4, $5, $6, NULL)",
            vec![
                compliance.id.clone().into(),
                compliance.channel_id.clone().into(),
                compliance.terms_version.clone().into(),
                compliance.admin_user_id.clone().into(),
                compliance.source_ip.clone().into(),
                timestamp(compliance.confirmed_at).into(),
            ],
        ))
        .await
        .map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok(compliance)
    }

    pub async fn capabilities(
        &self,
        channel_id: &str,
    ) -> Result<StoreMerchantCapabilitiesView, StoreBillingError> {
        self.require_channel(channel_id).await?;
        let rows = self
            .db
            .read()
            .query_all(self.db.stmt(
                "SELECT id, channel_id, capability, state, environment,
                        merchant_account_digest, provider_product, evidence_digest,
                        controlled_transaction_id, verifier_admin_id, verified_at, expires_at
                 FROM store_merchant_capabilities WHERE channel_id = $1
                 ORDER BY capability ASC, id ASC",
                vec![channel_id.into()],
            ))
            .await
            .map_err(storage)?;
        Ok(StoreMerchantCapabilitiesView {
            capabilities: rows
                .into_iter()
                .map(capability_from_row)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }

    pub async fn put_capability(
        &self,
        channel_id: &str,
        capability: MerchantCapabilityKind,
        input: PutStoreMerchantCapabilityInput,
        verifier_admin_id: &str,
    ) -> Result<StoreMerchantCapability, StoreBillingError> {
        validate_capability_input(&input)?;
        let tx = self.db.begin_write().await.map_err(storage)?;
        if !lock_channel(&self.db, &*tx, channel_id)
            .await
            .map_err(storage)?
        {
            return Err(StoreBillingError::NotFound);
        }
        let credential = tx
            .query_one(self.db.stmt(
                "SELECT account_identity_digest FROM store_channel_credentials
                 WHERE channel_id = $1 AND status = 'active'
                 ORDER BY created_at DESC, id DESC LIMIT 1",
                vec![channel_id.into()],
            ))
            .await
            .map_err(storage)?
            .ok_or(StoreBillingError::Conflict)?;
        let verified_at = Utc::now();
        let record = StoreMerchantCapability {
            id: Uuid::new_v4().to_string(),
            channel_id: channel_id.to_string(),
            capability,
            state: input.state,
            environment: input.environment.trim().to_string(),
            merchant_account_digest: credential
                .try_get("", "account_identity_digest")
                .map_err(storage)?,
            provider_product: input.provider_product.trim().to_string(),
            evidence_digest: input.evidence_digest,
            controlled_transaction_id: input
                .controlled_transaction_id
                .map(|value| value.trim().to_string()),
            verifier_admin_id: verifier_admin_id.to_string(),
            verified_at,
            expires_at: verified_at + Duration::days(90),
        };
        tx.execute(self.db.stmt(
            "INSERT INTO store_merchant_capabilities
                (id, channel_id, capability, state, environment, merchant_account_digest,
                 provider_product, evidence_digest, controlled_transaction_id,
                 verifier_admin_id, verified_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
             ON CONFLICT (channel_id, capability) DO UPDATE SET
                id = excluded.id, state = excluded.state, environment = excluded.environment,
                merchant_account_digest = excluded.merchant_account_digest,
                provider_product = excluded.provider_product,
                evidence_digest = excluded.evidence_digest,
                controlled_transaction_id = excluded.controlled_transaction_id,
                verifier_admin_id = excluded.verifier_admin_id,
                verified_at = excluded.verified_at, expires_at = excluded.expires_at",
            vec![
                record.id.clone().into(),
                record.channel_id.clone().into(),
                record.capability.as_str().into(),
                record.state.as_str().into(),
                record.environment.clone().into(),
                record.merchant_account_digest.clone().into(),
                record.provider_product.clone().into(),
                record.evidence_digest.clone().into(),
                record.controlled_transaction_id.clone().into(),
                record.verifier_admin_id.clone().into(),
                timestamp(record.verified_at).into(),
                timestamp(record.expires_at).into(),
            ],
        ))
        .await
        .map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok(record)
    }

    pub async fn privacy_records(&self) -> Result<StorePrivacyRecordsView, StoreBillingError> {
        let rows = self
            .db
            .read()
            .query_all(self.db.stmt(
                "SELECT id, policy_version, jurisdiction, allowed_regions_json,
                        retention_json, legal_basis, reviewer_id, evidence_digest,
                        approved_at, next_review_at, accepted
                 FROM store_privacy_records
                 ORDER BY approved_at DESC, id DESC",
                vec![],
            ))
            .await
            .map_err(storage)?;
        Ok(StorePrivacyRecordsView {
            records: rows
                .into_iter()
                .map(privacy_record_from_row)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }

    pub async fn create_privacy_record(
        &self,
        mut input: CreateStorePrivacyRecordInput,
        reviewer_id: &str,
    ) -> Result<StorePrivacyRecord, StoreBillingError> {
        validate_privacy_record_input(&input)?;
        input.allowed_regions.sort();
        let allowed_regions_json =
            serde_json::to_string(&input.allowed_regions).map_err(storage)?;
        let retention_json = serde_json::to_string(&input.retention).map_err(storage)?;
        let tx = self.db.begin_write().await.map_err(storage)?;
        let approved_at = Utc::now();
        let next_review_at = approved_at
            .checked_add_signed(Duration::days(input.review_after_days))
            .ok_or(StoreBillingError::InvalidInput)?;
        let record = StorePrivacyRecord {
            id: Uuid::new_v4().to_string(),
            policy_version: input.policy_version,
            jurisdiction: input.jurisdiction,
            allowed_regions: input.allowed_regions,
            retention: input.retention,
            legal_basis: input.legal_basis,
            reviewer_id: reviewer_id.to_string(),
            evidence_digest: input.evidence_digest,
            approved_at,
            next_review_at,
            accepted: true,
        };
        tx.execute(self.db.stmt(
            "INSERT INTO store_privacy_records
                (id, policy_version, jurisdiction, allowed_regions_json, retention_json,
                 legal_basis, reviewer_id, evidence_digest, approved_at, next_review_at, accepted)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 1)",
            vec![
                record.id.clone().into(),
                record.policy_version.clone().into(),
                record.jurisdiction.clone().into(),
                allowed_regions_json.into(),
                retention_json.into(),
                record.legal_basis.clone().into(),
                record.reviewer_id.clone().into(),
                record.evidence_digest.clone().into(),
                timestamp(record.approved_at).into(),
                timestamp(record.next_review_at).into(),
            ],
        ))
        .await
        .map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok(record)
    }

    pub async fn readiness(
        &self,
        channel_id: &str,
    ) -> Result<StoreChannelReadinessView, StoreBillingError> {
        let row = self
            .db
            .read()
            .query_one(self.db.stmt(
                "SELECT r.channel_id AS channel_id, r.active_credential_digest,
                        r.privacy_record_id, r.callback_verification_passed,
                        r.supported_currencies_json, r.amount_limits_json,
                        r.checkout_action_kinds_json, r.license_evidence_digest,
                        r.runtime_evidence_digest, r.availability_evidence_digest,
                        r.verifier_admin_id, r.verified_at, r.expires_at
                 FROM store_payment_channels c
                 LEFT JOIN store_channel_readiness_profiles r ON r.channel_id = c.id
                 WHERE c.id = $1",
                vec![channel_id.into()],
            ))
            .await
            .map_err(storage)?
            .ok_or(StoreBillingError::NotFound)?;
        let has_readiness = row
            .try_get::<Option<String>>("", "channel_id")
            .map_err(storage)?
            .is_some();
        Ok(StoreChannelReadinessView {
            readiness: has_readiness.then(|| readiness_profile_from_row(row)).transpose()?,
        })
    }

    pub async fn put_readiness(
        &self,
        channel_id: &str,
        mut input: PutStoreChannelReadinessInput,
        verifier_admin_id: &str,
    ) -> Result<StoreChannelReadinessProfile, StoreBillingError> {
        validate_readiness_input(&input)?;
        input
            .supported_currencies
            .sort_by_key(|currency| currency_string(*currency));
        input
            .checkout_action_kinds
            .sort_by_key(|action| action.as_str());
        let supported_currencies_json =
            serde_json::to_string(&input.supported_currencies).map_err(storage)?;
        let amount_limits_json = serde_json::to_string(&input.amount_limits).map_err(storage)?;
        let checkout_action_kinds_json =
            serde_json::to_string(&input.checkout_action_kinds).map_err(storage)?;

        let tx = self.db.begin_write().await.map_err(storage)?;
        if !lock_channel(&self.db, &*tx, channel_id)
            .await
            .map_err(storage)?
        {
            return Err(StoreBillingError::NotFound);
        }
        let channel = tx
            .query_one(self.db.stmt(
                "SELECT adapter_kind FROM store_payment_channels WHERE id = $1",
                vec![channel_id.into()],
            ))
            .await
            .map_err(storage)?
            .ok_or(StoreBillingError::NotFound)?;
        let adapter_kind = channel
            .try_get::<String>("", "adapter_kind")
            .map_err(storage)?;
        parse_readiness_metadata(
            &adapter_kind,
            &supported_currencies_json,
            &amount_limits_json,
            &checkout_action_kinds_json,
        )
        .map_err(|_| StoreBillingError::InvalidInput)?;

        let credential = tx
            .query_one(self.db.stmt(
                "SELECT account_identity_digest FROM store_channel_credentials
                 WHERE channel_id = $1 AND status = 'active'
                 ORDER BY created_at DESC, id DESC LIMIT 1",
                vec![channel_id.into()],
            ))
            .await
            .map_err(storage)?
            .ok_or(StoreBillingError::Conflict)?;
        let active_credential_digest = credential
            .try_get::<String>("", "account_identity_digest")
            .map_err(storage)?;
        let verified_at = Utc::now();
        let privacy = tx
            .query_one(self.db.stmt(
                "SELECT evidence_digest, approved_at, next_review_at, accepted
                 FROM store_privacy_records WHERE id = $1",
                vec![input.privacy_record_id.clone().into()],
            ))
            .await
            .map_err(storage)?
            .ok_or(StoreBillingError::Conflict)?;
        let privacy_approved_at =
            parse_timestamp(privacy.try_get("", "approved_at").map_err(storage)?)?;
        let privacy_next_review_at =
            parse_timestamp(privacy.try_get("", "next_review_at").map_err(storage)?)?;
        let privacy_accepted = privacy.try_get::<i32>("", "accepted").map_err(storage)? == 1;
        let privacy_digest = privacy
            .try_get::<String>("", "evidence_digest")
            .map_err(storage)?;
        if !privacy_accepted
            || privacy_approved_at > verified_at
            || verified_at >= privacy_next_review_at
            || !valid_digest(&privacy_digest)
        {
            return Err(StoreBillingError::Conflict);
        }
        let expires_at = verified_at
            .checked_add_signed(Duration::days(input.valid_for_days))
            .ok_or(StoreBillingError::InvalidInput)?;
        let record = StoreChannelReadinessProfile {
            channel_id: channel_id.to_string(),
            active_credential_digest,
            privacy_record_id: input.privacy_record_id,
            callback_verification_passed: input.callback_verification_passed,
            supported_currencies: input.supported_currencies,
            amount_limits: input.amount_limits,
            checkout_action_kinds: input.checkout_action_kinds,
            license_evidence_digest: input.license_evidence_digest,
            runtime_evidence_digest: input.runtime_evidence_digest,
            availability_evidence_digest: input.availability_evidence_digest,
            verifier_admin_id: verifier_admin_id.to_string(),
            verified_at,
            expires_at,
        };
        tx.execute(self.db.stmt(
            "INSERT INTO store_channel_readiness_profiles
                (channel_id, active_credential_digest, privacy_record_id,
                 callback_verification_passed, supported_currencies_json,
                 amount_limits_json, checkout_action_kinds_json, license_evidence_digest,
                 runtime_evidence_digest, availability_evidence_digest, verifier_admin_id,
                 verified_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
             ON CONFLICT (channel_id) DO UPDATE SET
                active_credential_digest = excluded.active_credential_digest,
                privacy_record_id = excluded.privacy_record_id,
                callback_verification_passed = excluded.callback_verification_passed,
                supported_currencies_json = excluded.supported_currencies_json,
                amount_limits_json = excluded.amount_limits_json,
                checkout_action_kinds_json = excluded.checkout_action_kinds_json,
                license_evidence_digest = excluded.license_evidence_digest,
                runtime_evidence_digest = excluded.runtime_evidence_digest,
                availability_evidence_digest = excluded.availability_evidence_digest,
                verifier_admin_id = excluded.verifier_admin_id,
                verified_at = excluded.verified_at, expires_at = excluded.expires_at",
            vec![
                record.channel_id.clone().into(),
                record.active_credential_digest.clone().into(),
                record.privacy_record_id.clone().into(),
                i64::from(record.callback_verification_passed).into(),
                supported_currencies_json.into(),
                amount_limits_json.into(),
                checkout_action_kinds_json.into(),
                record.license_evidence_digest.clone().into(),
                record.runtime_evidence_digest.clone().into(),
                record.availability_evidence_digest.clone().into(),
                record.verifier_admin_id.clone().into(),
                timestamp(record.verified_at).into(),
                timestamp(record.expires_at).into(),
            ],
        ))
        .await
        .map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok(record)
    }

    pub async fn availability(
        &self,
        channel_id: &str,
    ) -> Result<StoreChannelAvailability, StoreBillingError> {
        let availability = evaluate_channel(&self.db, self.db.read(), channel_id, Utc::now())
            .await
            .map_err(storage)?;
        if availability
            .unavailable_reasons
            .iter()
            .any(|reason| reason == "channel_not_found")
        {
            return Err(StoreBillingError::NotFound);
        }
        Ok(availability)
    }

    async fn require_channel(&self, channel_id: &str) -> Result<(), StoreBillingError> {
        self.db
            .read()
            .query_one(self.db.stmt(
                "SELECT id FROM store_payment_channels WHERE id = $1",
                vec![channel_id.into()],
            ))
            .await
            .map_err(storage)?
            .ok_or(StoreBillingError::NotFound)?;
        Ok(())
    }
}

pub async fn evaluate_channel<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    channel_id: &str,
    now: DateTime<Utc>,
) -> Result<StoreChannelAvailability, DbErr> {
    let snapshot = load_scoped_governance_snapshot(db, connection, channel_id).await?;
    Ok(evaluate_snapshot_channel(&snapshot, channel_id, now))
}

pub async fn evaluate_channels<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    now: DateTime<Utc>,
) -> Result<BTreeMap<String, StoreChannelAvailability>, DbErr> {
    let snapshot = load_governance_snapshot(db, connection).await?;
    Ok(snapshot
        .channels
        .keys()
        .map(|channel_id| {
            (
                channel_id.clone(),
                evaluate_snapshot_channel(&snapshot, channel_id, now),
            )
        })
        .collect())
}

#[derive(Default)]
struct GovernanceSnapshot {
    channels: BTreeMap<String, RawChannel>,
    credentials: BTreeMap<String, RawCredential>,
    compliance: BTreeMap<String, Option<String>>,
    capabilities: BTreeMap<(String, String), RawCapability>,
    readiness: BTreeMap<String, RawReadiness>,
    privacy: BTreeMap<String, RawPrivacy>,
}

struct RawChannel {
    adapter_kind: Option<String>,
    enabled: Option<i32>,
}

struct RawCredential {
    adapter_kind: Option<String>,
    account_identity_digest: Option<String>,
}

struct RawCapability {
    capability: Option<String>,
    state: Option<String>,
    environment: Option<String>,
    merchant_account_digest: Option<String>,
    provider_product: Option<String>,
    evidence_digest: Option<String>,
    controlled_transaction_id: Option<Option<String>>,
    verified_at: Option<String>,
    expires_at: Option<String>,
}

struct RawReadiness {
    active_credential_digest: Option<String>,
    privacy_record_id: Option<String>,
    callback_verification_passed: Option<i32>,
    supported_currencies_json: Option<String>,
    amount_limits_json: Option<String>,
    checkout_action_kinds_json: Option<String>,
    license_evidence_digest: Option<String>,
    runtime_evidence_digest: Option<String>,
    availability_evidence_digest: Option<String>,
    verified_at: Option<String>,
    expires_at: Option<String>,
}

struct RawPrivacy {
    evidence_digest: Option<String>,
    approved_at: Option<String>,
    next_review_at: Option<String>,
    accepted: Option<i32>,
}

async fn load_scoped_governance_snapshot<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    channel_id: &str,
) -> Result<GovernanceSnapshot, DbErr> {
    let channel_rows = connection
        .query_all(db.stmt(
            "SELECT id, adapter_kind, enabled FROM store_payment_channels WHERE id = $1",
            vec![channel_id.into()],
        ))
        .await?;
    let credential_rows = connection
        .query_all(db.stmt(
            "SELECT channel_id, adapter_kind, account_identity_digest
             FROM store_channel_credentials
             WHERE status = 'active' AND channel_id = $1
             ORDER BY created_at DESC, id DESC",
            vec![channel_id.into()],
        ))
        .await?;
    let compliance_rows = connection
        .query_all(db.stmt(
            "SELECT channel_id, terms_version FROM store_payment_compliance
             WHERE invalidated_at IS NULL AND channel_id = $1
             ORDER BY confirmed_at DESC, id DESC",
            vec![channel_id.into()],
        ))
        .await?;
    let capability_rows = connection
        .query_all(db.stmt(
            "SELECT channel_id, capability, state, environment, merchant_account_digest,
                    provider_product, evidence_digest, controlled_transaction_id,
                    verified_at, expires_at
             FROM store_merchant_capabilities WHERE channel_id = $1",
            vec![channel_id.into()],
        ))
        .await?;
    let readiness_rows = connection
        .query_all(db.stmt(
            "SELECT channel_id, active_credential_digest, privacy_record_id,
                    callback_verification_passed, supported_currencies_json,
                    amount_limits_json, checkout_action_kinds_json,
                    license_evidence_digest, runtime_evidence_digest,
                    availability_evidence_digest, verified_at, expires_at
             FROM store_channel_readiness_profiles WHERE channel_id = $1",
            vec![channel_id.into()],
        ))
        .await?;
    let privacy_rows = connection
        .query_all(db.stmt(
            "SELECT id, evidence_digest, approved_at, next_review_at, accepted
             FROM store_privacy_records
             WHERE id = (SELECT privacy_record_id FROM store_channel_readiness_profiles WHERE channel_id = $1)",
            vec![channel_id.into()],
        ))
        .await?;
    Ok(governance_snapshot_from_rows(
        channel_rows,
        credential_rows,
        compliance_rows,
        capability_rows,
        readiness_rows,
        privacy_rows,
    ))
}

async fn load_governance_snapshot<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
) -> Result<GovernanceSnapshot, DbErr> {
    let mut snapshot = GovernanceSnapshot::default();
    for row in connection
        .query_all(db.stmt(
            "SELECT id, adapter_kind, enabled FROM store_payment_channels",
            vec![],
        ))
        .await?
    {
        insert_channel_row(&mut snapshot, &row);
    }
    for row in connection
        .query_all(db.stmt(
            "SELECT channel_id, adapter_kind, account_identity_digest
             FROM store_channel_credentials WHERE status = 'active'
             ORDER BY channel_id ASC, created_at DESC, id DESC",
            vec![],
        ))
        .await?
    {
        insert_credential_row(&mut snapshot, &row);
    }
    for row in connection
        .query_all(db.stmt(
            "SELECT channel_id, terms_version FROM store_payment_compliance
             WHERE invalidated_at IS NULL
             ORDER BY channel_id ASC, confirmed_at DESC, id DESC",
            vec![],
        ))
        .await?
    {
        insert_compliance_row(&mut snapshot, &row);
    }
    for row in connection
        .query_all(db.stmt(
            "SELECT channel_id, capability, state, environment, merchant_account_digest,
                    provider_product, evidence_digest, controlled_transaction_id,
                    verified_at, expires_at
             FROM store_merchant_capabilities",
            vec![],
        ))
        .await?
    {
        insert_capability_row(&mut snapshot, &row);
    }
    for row in connection
        .query_all(db.stmt(
            "SELECT channel_id, active_credential_digest, privacy_record_id,
                    callback_verification_passed, supported_currencies_json,
                    amount_limits_json, checkout_action_kinds_json,
                    license_evidence_digest, runtime_evidence_digest,
                    availability_evidence_digest, verified_at, expires_at
             FROM store_channel_readiness_profiles",
            vec![],
        ))
        .await?
    {
        insert_readiness_row(&mut snapshot, &row);
    }
    for row in connection
        .query_all(db.stmt(
            "SELECT id, evidence_digest, approved_at, next_review_at, accepted
             FROM store_privacy_records",
            vec![],
        ))
        .await?
    {
        insert_privacy_row(&mut snapshot, &row);
    }
    Ok(snapshot)
}

fn governance_snapshot_from_rows(
    channel_rows: Vec<QueryResult>,
    credential_rows: Vec<QueryResult>,
    compliance_rows: Vec<QueryResult>,
    capability_rows: Vec<QueryResult>,
    readiness_rows: Vec<QueryResult>,
    privacy_rows: Vec<QueryResult>,
) -> GovernanceSnapshot {
    let mut snapshot = GovernanceSnapshot::default();
    for row in channel_rows {
        insert_channel_row(&mut snapshot, &row);
    }
    for row in credential_rows {
        insert_credential_row(&mut snapshot, &row);
    }
    for row in compliance_rows {
        insert_compliance_row(&mut snapshot, &row);
    }
    for row in capability_rows {
        insert_capability_row(&mut snapshot, &row);
    }
    for row in readiness_rows {
        insert_readiness_row(&mut snapshot, &row);
    }
    for row in privacy_rows {
        insert_privacy_row(&mut snapshot, &row);
    }
    snapshot
}

fn insert_channel_row(snapshot: &mut GovernanceSnapshot, row: &QueryResult) {
    if let Ok(channel_id) = row.try_get::<String>("", "id") {
        snapshot.channels.insert(
            channel_id,
            RawChannel {
                adapter_kind: row.try_get("", "adapter_kind").ok(),
                enabled: row.try_get("", "enabled").ok(),
            },
        );
    }
}

fn insert_credential_row(snapshot: &mut GovernanceSnapshot, row: &QueryResult) {
    if let Ok(channel_id) = row.try_get::<String>("", "channel_id") {
        snapshot
            .credentials
            .entry(channel_id)
            .or_insert_with(|| RawCredential {
                adapter_kind: row.try_get("", "adapter_kind").ok(),
                account_identity_digest: row.try_get("", "account_identity_digest").ok(),
            });
    }
}

fn insert_compliance_row(snapshot: &mut GovernanceSnapshot, row: &QueryResult) {
    if let Ok(channel_id) = row.try_get::<String>("", "channel_id") {
        snapshot
            .compliance
            .entry(channel_id)
            .or_insert_with(|| row.try_get("", "terms_version").ok());
    }
}

fn insert_capability_row(snapshot: &mut GovernanceSnapshot, row: &QueryResult) {
    let channel_id = row.try_get::<String>("", "channel_id").ok();
    let capability = row.try_get::<String>("", "capability").ok();
    if let (Some(channel_id), Some(raw_capability)) = (channel_id, capability.clone()) {
        let capability_key = canonical_required_capability(&raw_capability)
            .unwrap_or(raw_capability.as_str())
            .to_string();
        let map_key = (channel_id, capability_key.clone());
        let record = RawCapability {
            capability,
            state: row.try_get("", "state").ok(),
            environment: row.try_get("", "environment").ok(),
            merchant_account_digest: row.try_get("", "merchant_account_digest").ok(),
            provider_product: row.try_get("", "provider_product").ok(),
            evidence_digest: row.try_get("", "evidence_digest").ok(),
            controlled_transaction_id: row
                .try_get::<Option<String>>("", "controlled_transaction_id")
                .ok(),
            verified_at: row.try_get("", "verified_at").ok(),
            expires_at: row.try_get("", "expires_at").ok(),
        };
        let existing_is_exact = snapshot
            .capabilities
            .get(&map_key)
            .is_some_and(|existing| existing.capability.as_deref() == Some(&capability_key));
        if !existing_is_exact || raw_capability == capability_key {
            snapshot.capabilities.insert(map_key, record);
        }
    }
}

fn insert_readiness_row(snapshot: &mut GovernanceSnapshot, row: &QueryResult) {
    if let Ok(channel_id) = row.try_get::<String>("", "channel_id") {
        snapshot.readiness.insert(
            channel_id,
            RawReadiness {
                active_credential_digest: row.try_get("", "active_credential_digest").ok(),
                privacy_record_id: row.try_get("", "privacy_record_id").ok(),
                callback_verification_passed: row
                    .try_get("", "callback_verification_passed")
                    .ok(),
                supported_currencies_json: row.try_get("", "supported_currencies_json").ok(),
                amount_limits_json: row.try_get("", "amount_limits_json").ok(),
                checkout_action_kinds_json: row
                    .try_get("", "checkout_action_kinds_json")
                    .ok(),
                license_evidence_digest: row.try_get("", "license_evidence_digest").ok(),
                runtime_evidence_digest: row.try_get("", "runtime_evidence_digest").ok(),
                availability_evidence_digest: row
                    .try_get("", "availability_evidence_digest")
                    .ok(),
                verified_at: row.try_get("", "verified_at").ok(),
                expires_at: row.try_get("", "expires_at").ok(),
            },
        );
    }
}

fn insert_privacy_row(snapshot: &mut GovernanceSnapshot, row: &QueryResult) {
    if let Ok(privacy_id) = row.try_get::<String>("", "id") {
        snapshot.privacy.insert(
            privacy_id,
            RawPrivacy {
                evidence_digest: row.try_get("", "evidence_digest").ok(),
                approved_at: row.try_get("", "approved_at").ok(),
                next_review_at: row.try_get("", "next_review_at").ok(),
                accepted: row.try_get("", "accepted").ok(),
            },
        );
    }
}

fn canonical_required_capability(value: &str) -> Option<&'static str> {
    let lowercase = value.to_ascii_lowercase();
    REQUIRED_CAPABILITIES
        .iter()
        .copied()
        .find(|required| *required == lowercase)
}

fn evaluate_snapshot_channel(
    snapshot: &GovernanceSnapshot,
    channel_id: &str,
    now: DateTime<Utc>,
) -> StoreChannelAvailability {
    let Some(channel) = snapshot.channels.get(channel_id) else {
        return availability(
            channel_id,
            vec!["channel_not_found".to_string()],
            ReadinessMetadata::default(),
        );
    };
    let mut reasons = Vec::new();
    let adapter_kind = channel.adapter_kind.as_deref().unwrap_or("");
    if channel.enabled != Some(1) {
        reasons.push("channel_disabled".to_string());
    }
    if !matches!(adapter_kind, "alipay" | "wechat" | "stripe") {
        reasons.push("adapter_milestone_unavailable".to_string());
    }
    let credential_digest = if let Some(credential) = snapshot.credentials.get(channel_id) {
        if credential.adapter_kind.as_deref() != Some(adapter_kind) {
            reasons.push("active_credential_mismatch".to_string());
        }
        credential.account_identity_digest.as_deref()
    } else {
        reasons.push("active_credential_missing".to_string());
        None
    };
    match snapshot.compliance.get(channel_id) {
        None => reasons.push("compliance_missing".to_string()),
        Some(Some(version)) if version == CURRENT_STORE_PAYMENT_TERMS_VERSION => {}
        Some(_) => reasons.push("compliance_terms_outdated".to_string()),
    }
    for capability in REQUIRED_CAPABILITIES {
        let key = (channel_id.to_string(), capability.to_string());
        let Some(row) = snapshot.capabilities.get(&key) else {
            reasons.push(format!("capability_{capability}_missing"));
            continue;
        };
        let Some(record) = validated_capability(row, capability, now) else {
            reasons.push(format!("capability_{capability}_invalid"));
            continue;
        };
        if record.state != "supported" {
            reasons.push(format!("capability_{capability}_not_supported"));
        }
        if record.expires_at <= now {
            reasons.push(format!("capability_{capability}_expired"));
        }
        if credential_digest != Some(record.merchant_account_digest.as_str()) {
            reasons.push(format!("capability_{capability}_credential_mismatch"));
        }
    }
    let metadata = evaluate_readiness_snapshot(
        snapshot,
        channel_id,
        adapter_kind,
        credential_digest,
        now,
        &mut reasons,
    );
    availability(channel_id, reasons, metadata)
}

struct ValidatedCapability {
    state: String,
    merchant_account_digest: String,
    expires_at: DateTime<Utc>,
}

fn validated_capability(
    row: &RawCapability,
    expected_capability: &str,
    now: DateTime<Utc>,
) -> Option<ValidatedCapability> {
    let capability = row.capability.as_deref()?;
    let state = row.state.as_deref()?;
    let environment = row.environment.as_deref()?;
    let merchant_account_digest = row.merchant_account_digest.as_deref()?;
    let provider_product = row.provider_product.as_deref()?;
    let evidence_digest = row.evidence_digest.as_deref()?;
    let controlled_transaction_id = row.controlled_transaction_id.as_ref()?;
    let verified_at = parse_rfc3339(row.verified_at.clone()?)?;
    let expires_at = parse_rfc3339(row.expires_at.clone()?)?;
    if capability != expected_capability
        || !matches!(state, "supported" | "unsupported" | "manual")
        || !valid_trimmed(environment, 128)
        || !valid_digest(merchant_account_digest)
        || !valid_trimmed(provider_product, 128)
        || !valid_digest(evidence_digest)
        || controlled_transaction_id
            .as_ref()
            .is_some_and(|value| !valid_trimmed(value, 256))
        || verified_at > now
        || verified_at >= expires_at
    {
        return None;
    }
    Some(ValidatedCapability {
        state: state.to_string(),
        merchant_account_digest: merchant_account_digest.to_string(),
        expires_at,
    })
}

pub async fn evaluate_channel_for_payment<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    channel_id: &str,
    now: DateTime<Utc>,
    currency: Currency,
    payment_minor: &str,
) -> Result<StoreChannelAvailability, DbErr> {
    let mut result = evaluate_channel(db, connection, channel_id, now).await?;
    if !result.supported_currencies.contains(&currency) {
        result
            .unavailable_reasons
            .push("payment_currency_unsupported".to_string());
    } else {
        let currency_key = currency_string(currency);
        let amount = parse_minor(payment_minor).ok();
        let in_range = result.amount_limits.get(currency_key).is_some_and(|limit| {
            let min = parse_minor(&limit.min_minor).ok();
            let max = parse_minor(&limit.max_minor).ok();
            matches!((amount, min, max), (Some(amount), Some(min), Some(max)) if min <= amount && amount <= max)
        });
        if !in_range {
            result
                .unavailable_reasons
                .push("payment_amount_out_of_range".to_string());
        }
    }
    result.unavailable_reasons.sort();
    result.unavailable_reasons.dedup();
    result.effective_available = result.unavailable_reasons.is_empty();
    Ok(result)
}

#[derive(Default)]
struct ReadinessMetadata {
    supported_currencies: Vec<Currency>,
    amount_limits: BTreeMap<String, StoreAmountLimit>,
    checkout_action_kinds: Vec<CheckoutActionKind>,
}

struct UniqueAmountLimits(BTreeMap<String, StoreAmountLimit>);

impl<'de> Deserialize<'de> for UniqueAmountLimits {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct UniqueAmountLimitsVisitor;

        impl<'de> Visitor<'de> for UniqueAmountLimitsVisitor {
            type Value = UniqueAmountLimits;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object with unique currency keys")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut limits = BTreeMap::new();
                while let Some((currency, limit)) = map.next_entry::<String, StoreAmountLimit>()? {
                    if limits.insert(currency.clone(), limit).is_some() {
                        return Err(de::Error::custom(format_args!(
                            "duplicate amount limit currency `{currency}`"
                        )));
                    }
                }
                Ok(UniqueAmountLimits(limits))
            }
        }

        deserializer.deserialize_map(UniqueAmountLimitsVisitor)
    }
}

fn evaluate_readiness_snapshot(
    snapshot: &GovernanceSnapshot,
    channel_id: &str,
    adapter_kind: &str,
    credential_digest: Option<&str>,
    now: DateTime<Utc>,
    reasons: &mut Vec<String>,
) -> ReadinessMetadata {
    let Some(row) = snapshot.readiness.get(channel_id) else {
        reasons.push("readiness_profile_missing".to_string());
        return ReadinessMetadata::default();
    };
    let mut profile_current = true;
    if credential_digest != row.active_credential_digest.as_deref() {
        reasons.push("readiness_profile_credential_mismatch".to_string());
        profile_current = false;
    }
    let verified_at = row.verified_at.clone().and_then(parse_rfc3339);
    let expires_at = row.expires_at.clone().and_then(parse_rfc3339);
    if !matches!((verified_at, expires_at), (Some(verified), Some(expires)) if verified <= now && now < expires && verified < expires)
    {
        reasons.push("readiness_profile_expired".to_string());
        profile_current = false;
    }
    if row.callback_verification_passed != Some(1) {
        reasons.push("callback_verification_pending".to_string());
        profile_current = false;
    }
    for (digest, reason) in [
        (
            row.license_evidence_digest.as_deref(),
            "license_gate_pending",
        ),
        (row.runtime_evidence_digest.as_deref(), "runtime_gate_pending"),
        (
            row.availability_evidence_digest.as_deref(),
            "availability_evidence_pending",
        ),
    ] {
        if !digest.is_some_and(valid_digest) {
            reasons.push(reason.to_string());
            profile_current = false;
        }
    }
    let privacy_current = row
        .privacy_record_id
        .as_deref()
        .and_then(|privacy_id| snapshot.privacy.get(privacy_id))
        .is_some_and(|privacy| {
            let approved_at = privacy.approved_at.clone().and_then(parse_rfc3339);
            let next_review_at = privacy.next_review_at.clone().and_then(parse_rfc3339);
            privacy.accepted == Some(1)
            && privacy.evidence_digest.as_deref().is_some_and(valid_digest)
            && matches!((approved_at, next_review_at), (Some(approved), Some(review)) if approved <= now && now < review && approved < review)
        });
    if !privacy_current {
        reasons.push("privacy_gate_pending".to_string());
        profile_current = false;
    }

    let metadata = match (
        row.supported_currencies_json.as_deref(),
        row.amount_limits_json.as_deref(),
        row.checkout_action_kinds_json.as_deref(),
    ) {
        (Some(currencies), Some(limits), Some(actions)) => {
            parse_readiness_metadata(adapter_kind, currencies, limits, actions)
        }
        _ => Err("readiness_metadata_invalid"),
    };
    match metadata {
        Ok(metadata) if profile_current => metadata,
        Ok(_) => ReadinessMetadata::default(),
        Err(reason) => {
            reasons.push(reason.to_string());
            ReadinessMetadata::default()
        }
    }
}

fn parse_readiness_metadata(
    adapter_kind: &str,
    currencies_json: &str,
    limits_json: &str,
    actions_json: &str,
) -> Result<ReadinessMetadata, &'static str> {
    let currency_values = serde_json::from_str::<Vec<String>>(currencies_json)
        .map_err(|_| "readiness_metadata_invalid")?;
    if currency_values.is_empty() {
        return Err("readiness_metadata_invalid");
    }
    let mut currency_names = BTreeSet::new();
    let mut supported_currencies = Vec::with_capacity(currency_values.len());
    for value in currency_values {
        if !currency_names.insert(value.clone()) {
            return Err("readiness_metadata_invalid");
        }
        supported_currencies.push(match value.as_str() {
            "CNY" => Currency::CNY,
            "USD" => Currency::USD,
            _ => return Err("readiness_metadata_invalid"),
        });
    }
    if matches!(adapter_kind, "alipay" | "wechat")
        && currency_names != BTreeSet::from(["CNY".to_string()])
    {
        return Err("readiness_metadata_invalid");
    }

    let UniqueAmountLimits(amount_limits) = serde_json::from_str::<UniqueAmountLimits>(limits_json)
        .map_err(|_| "readiness_metadata_invalid")?;
    if amount_limits.keys().cloned().collect::<BTreeSet<_>>() != currency_names {
        return Err("readiness_metadata_invalid");
    }
    for limit in amount_limits.values() {
        let min = parse_minor(&limit.min_minor).map_err(|_| "readiness_metadata_invalid")?;
        let max = parse_minor(&limit.max_minor).map_err(|_| "readiness_metadata_invalid")?;
        if min <= 0 || min > max {
            return Err("readiness_metadata_invalid");
        }
    }

    let action_values = serde_json::from_str::<Vec<String>>(actions_json)
        .map_err(|_| "readiness_metadata_invalid")?;
    if action_values.is_empty() {
        return Err("checkout_action_incompatible");
    }
    let mut action_names = BTreeSet::new();
    let mut checkout_action_kinds = Vec::with_capacity(action_values.len());
    for value in action_values {
        if !action_names.insert(value.clone()) {
            return Err("checkout_action_incompatible");
        }
        checkout_action_kinds
            .push(CheckoutActionKind::from_str(&value).ok_or("checkout_action_incompatible")?);
    }
    let actions_valid = match adapter_kind {
        "stripe" => action_names == BTreeSet::from(["redirect".to_string()]),
        "alipay" => action_names == BTreeSet::from(["form".to_string()]),
        "wechat" => action_names
            .iter()
            .all(|value| matches!(value.as_str(), "qr" | "redirect")),
        _ => false,
    };
    if !actions_valid {
        return Err("checkout_action_incompatible");
    }
    Ok(ReadinessMetadata {
        supported_currencies,
        amount_limits,
        checkout_action_kinds,
    })
}

fn availability(
    channel_id: &str,
    mut unavailable_reasons: Vec<String>,
    metadata: ReadinessMetadata,
) -> StoreChannelAvailability {
    unavailable_reasons.sort();
    unavailable_reasons.dedup();
    StoreChannelAvailability {
        channel_id: channel_id.to_string(),
        effective_available: unavailable_reasons.is_empty(),
        unavailable_reasons,
        supported_currencies: metadata.supported_currencies,
        amount_limits: metadata.amount_limits,
        checkout_action_kinds: metadata.checkout_action_kinds,
    }
}

fn parse_rfc3339(value: String) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn currency_string(currency: Currency) -> &'static str {
    match currency {
        Currency::CNY => "CNY",
        Currency::USD => "USD",
    }
}

pub(crate) async fn lock_channel<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    channel_id: &str,
) -> Result<bool, DbErr> {
    let row = if db.is_postgres() {
        connection
            .query_one(db.stmt(
                "SELECT id FROM store_payment_channels WHERE id = $1 FOR UPDATE",
                vec![channel_id.into()],
            ))
            .await?
    } else {
        let result = connection
            .execute(db.stmt(
                "UPDATE store_payment_channels SET revision = revision WHERE id = $1",
                vec![channel_id.into()],
            ))
            .await?;
        if result.rows_affected() == 0 {
            None
        } else {
            connection
                .query_one(db.stmt(
                    "SELECT id FROM store_payment_channels WHERE id = $1",
                    vec![channel_id.into()],
                ))
                .await?
        }
    };
    Ok(row.is_some())
}

fn validate_capability_input(
    input: &PutStoreMerchantCapabilityInput,
) -> Result<(), StoreBillingError> {
    if !valid_trimmed(&input.environment, 128)
        || !valid_trimmed(&input.provider_product, 128)
        || input.evidence_digest.len() != 64
        || !input
            .evidence_digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || input
            .controlled_transaction_id
            .as_ref()
            .is_some_and(|value| !valid_trimmed(value, 256))
    {
        return Err(StoreBillingError::InvalidInput);
    }
    Ok(())
}

fn validate_privacy_record_input(
    input: &CreateStorePrivacyRecordInput,
) -> Result<(), StoreBillingError> {
    if !valid_exact_trimmed(&input.policy_version, 64)
        || !valid_exact_trimmed(&input.jurisdiction, 128)
        || !valid_exact_trimmed(&input.legal_basis, 512)
        || !valid_digest(&input.evidence_digest)
        || !input.accepted
        || !(1..=365).contains(&input.review_after_days)
        || !(1..=32).contains(&input.allowed_regions.len())
        || input.retention.raw_callback_days != 30
        || input.retention.network_metadata_days != 90
        || !(1..=36_500).contains(&input.retention.financial_records_days)
        || input.retention.redemption_audit_days != 730
        || !(1..=24).contains(&input.retention.expired_reauth_grant_hours)
    {
        return Err(StoreBillingError::InvalidInput);
    }
    let mut regions = BTreeSet::new();
    for region in &input.allowed_regions {
        if !valid_exact_trimmed(region, 64)
            || !region
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            || !regions.insert(region)
        {
            return Err(StoreBillingError::InvalidInput);
        }
    }
    Ok(())
}

fn validate_readiness_input(
    input: &PutStoreChannelReadinessInput,
) -> Result<(), StoreBillingError> {
    if !valid_exact_trimmed(&input.privacy_record_id, 255)
        || !valid_digest(&input.license_evidence_digest)
        || !valid_digest(&input.runtime_evidence_digest)
        || !valid_digest(&input.availability_evidence_digest)
        || !(1..=90).contains(&input.valid_for_days)
    {
        return Err(StoreBillingError::InvalidInput);
    }
    Ok(())
}

fn valid_exact_trimmed(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && value.trim() == value
}

fn valid_trimmed(value: &str, max: usize) -> bool {
    let value = value.trim();
    !value.is_empty() && value.len() <= max
}

fn compliance_from_row(row: QueryResult) -> Result<StorePaymentCompliance, StoreBillingError> {
    Ok(StorePaymentCompliance {
        id: row.try_get("", "id").map_err(storage)?,
        channel_id: row.try_get("", "channel_id").map_err(storage)?,
        terms_version: row.try_get("", "terms_version").map_err(storage)?,
        admin_user_id: row.try_get("", "admin_user_id").map_err(storage)?,
        source_ip: row.try_get("", "source_ip").map_err(storage)?,
        confirmed_at: parse_timestamp(row.try_get("", "confirmed_at").map_err(storage)?)?,
        invalidated_at: row
            .try_get::<Option<String>>("", "invalidated_at")
            .map_err(storage)?
            .map(parse_timestamp)
            .transpose()?,
    })
}

fn capability_from_row(row: QueryResult) -> Result<StoreMerchantCapability, StoreBillingError> {
    let capability = row.try_get::<String>("", "capability").map_err(storage)?;
    let state = row.try_get::<String>("", "state").map_err(storage)?;
    Ok(StoreMerchantCapability {
        id: row.try_get("", "id").map_err(storage)?,
        channel_id: row.try_get("", "channel_id").map_err(storage)?,
        capability: MerchantCapabilityKind::from_str(&capability)
            .ok_or(StoreBillingError::InvalidInput)?,
        state: super::models::MerchantCapabilityState::from_str(&state)
            .ok_or(StoreBillingError::InvalidInput)?,
        environment: row.try_get("", "environment").map_err(storage)?,
        merchant_account_digest: row
            .try_get("", "merchant_account_digest")
            .map_err(storage)?,
        provider_product: row.try_get("", "provider_product").map_err(storage)?,
        evidence_digest: row.try_get("", "evidence_digest").map_err(storage)?,
        controlled_transaction_id: row
            .try_get("", "controlled_transaction_id")
            .map_err(storage)?,
        verifier_admin_id: row.try_get("", "verifier_admin_id").map_err(storage)?,
        verified_at: parse_timestamp(row.try_get("", "verified_at").map_err(storage)?)?,
        expires_at: parse_timestamp(row.try_get("", "expires_at").map_err(storage)?)?,
    })
}

fn privacy_record_from_row(row: QueryResult) -> Result<StorePrivacyRecord, StoreBillingError> {
    let accepted = row.try_get::<i32>("", "accepted").map_err(storage)?;
    if !matches!(accepted, 0 | 1) {
        return Err(StoreBillingError::Storage(
            "privacy accepted value is invalid".to_string(),
        ));
    }
    Ok(StorePrivacyRecord {
        id: row.try_get("", "id").map_err(storage)?,
        policy_version: row.try_get("", "policy_version").map_err(storage)?,
        jurisdiction: row.try_get("", "jurisdiction").map_err(storage)?,
        allowed_regions: serde_json::from_str(
            &row.try_get::<String>("", "allowed_regions_json")
                .map_err(storage)?,
        )
        .map_err(storage)?,
        retention: serde_json::from_str(
            &row.try_get::<String>("", "retention_json")
                .map_err(storage)?,
        )
        .map_err(storage)?,
        legal_basis: row.try_get("", "legal_basis").map_err(storage)?,
        reviewer_id: row.try_get("", "reviewer_id").map_err(storage)?,
        evidence_digest: row.try_get("", "evidence_digest").map_err(storage)?,
        approved_at: parse_timestamp(row.try_get("", "approved_at").map_err(storage)?)?,
        next_review_at: parse_timestamp(row.try_get("", "next_review_at").map_err(storage)?)?,
        accepted: accepted == 1,
    })
}

fn readiness_profile_from_row(
    row: QueryResult,
) -> Result<StoreChannelReadinessProfile, StoreBillingError> {
    let callback = row
        .try_get::<i32>("", "callback_verification_passed")
        .map_err(storage)?;
    if !matches!(callback, 0 | 1) {
        return Err(StoreBillingError::Storage(
            "readiness callback value is invalid".to_string(),
        ));
    }
    Ok(StoreChannelReadinessProfile {
        channel_id: row.try_get("", "channel_id").map_err(storage)?,
        active_credential_digest: row
            .try_get("", "active_credential_digest")
            .map_err(storage)?,
        privacy_record_id: row.try_get("", "privacy_record_id").map_err(storage)?,
        callback_verification_passed: callback == 1,
        supported_currencies: serde_json::from_str(
            &row.try_get::<String>("", "supported_currencies_json")
                .map_err(storage)?,
        )
        .map_err(storage)?,
        amount_limits: serde_json::from_str(
            &row.try_get::<String>("", "amount_limits_json")
                .map_err(storage)?,
        )
        .map_err(storage)?,
        checkout_action_kinds: serde_json::from_str(
            &row.try_get::<String>("", "checkout_action_kinds_json")
                .map_err(storage)?,
        )
        .map_err(storage)?,
        license_evidence_digest: row
            .try_get("", "license_evidence_digest")
            .map_err(storage)?,
        runtime_evidence_digest: row
            .try_get("", "runtime_evidence_digest")
            .map_err(storage)?,
        availability_evidence_digest: row
            .try_get("", "availability_evidence_digest")
            .map_err(storage)?,
        verifier_admin_id: row.try_get("", "verifier_admin_id").map_err(storage)?,
        verified_at: parse_timestamp(row.try_get("", "verified_at").map_err(storage)?)?,
        expires_at: parse_timestamp(row.try_get("", "expires_at").map_err(storage)?)?,
    })
}

fn parse_timestamp(value: String) -> Result<DateTime<Utc>, StoreBillingError> {
    DateTime::parse_from_rfc3339(&value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| StoreBillingError::Storage(error.to_string()))
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn storage(error: impl ToString) -> StoreBillingError {
    StoreBillingError::Storage(error.to_string())
}
