use crate::provider::{CanonicalDecimal, canonical_json};
use anyhow::{Context, bail};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::str::FromStr;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineItem {
    pub usage_class: String,
    pub quantity: u64,
    pub unit_price_nano_usd: u64,
}

impl LineItem {
    pub fn new(usage_class: &str, quantity: u64, unit_price_nano_usd: u64) -> Self {
        Self {
            usage_class: usage_class.to_owned(),
            quantity,
            unit_price_nano_usd,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PricingScenario {
    pub id: String,
    pub line_items: Vec<LineItem>,
}

impl PricingScenario {
    pub fn new(id: &str, line_items: Vec<LineItem>) -> Self {
        Self {
            id: id.to_owned(),
            line_items,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PricingMapping {
    pub provider_id: String,
    pub group_id: String,
    pub logical_model: String,
    pub upstream_model: String,
    pub resolved_profile: String,
    pub multiplier: CanonicalDecimal,
}

impl PricingMapping {
    pub fn new(
        provider_id: &str,
        group_id: &str,
        logical_model: &str,
        upstream_model: &str,
        resolved_profile: &str,
        multiplier: CanonicalDecimal,
    ) -> Self {
        Self {
            provider_id: provider_id.to_owned(),
            group_id: group_id.to_owned(),
            logical_model: logical_model.to_owned(),
            upstream_model: upstream_model.to_owned(),
            resolved_profile: resolved_profile.to_owned(),
            multiplier,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotRow {
    pub mapping_digest: String,
    pub logical_model: String,
    pub upstream_model: String,
    pub resolved_profile: String,
    pub multiplier: CanonicalDecimal,
    pub scenario_id: String,
    pub base_charge_nano_usd: String,
    pub final_charge_nano_usd: String,
    pub line_items: Vec<LineItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PricingSnapshot {
    pub rows: Vec<SnapshotRow>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotComparison {
    pub equal: bool,
    pub before_sha256: String,
    pub after_sha256: String,
}

pub fn charge(line_items: &[LineItem], multiplier: &CanonicalDecimal) -> anyhow::Result<u128> {
    let base = base_charge(line_items)?;
    let decimal = Decimal::from_str(multiplier.as_str()).context("parse canonical multiplier")?;
    let mantissa = u128::try_from(decimal.mantissa()).context("positive multiplier mantissa")?;
    let divisor = 10_u128
        .checked_pow(decimal.scale())
        .context("multiplier scale overflow")?;
    base.checked_mul(mantissa)
        .context("scaled charge overflow")
        .map(|scaled| scaled / divisor)
}

fn base_charge(line_items: &[LineItem]) -> anyhow::Result<u128> {
    line_items.iter().try_fold(0_u128, |total, item| {
        let subtotal = u128::from(item.quantity)
            .checked_mul(u128::from(item.unit_price_nano_usd))
            .context("line-item charge overflow")?;
        total.checked_add(subtotal).context("base charge overflow")
    })
}

pub fn snapshot(
    mappings: &[PricingMapping],
    scenarios: &[PricingScenario],
) -> anyhow::Result<Vec<u8>> {
    let mut rows = Vec::with_capacity(mappings.len().saturating_mul(scenarios.len()));
    for mapping in mappings {
        let mapping_digest = mapping_digest(mapping)?;
        for scenario in scenarios {
            let base = base_charge(&scenario.line_items)?;
            let final_charge = charge(&scenario.line_items, &mapping.multiplier)?;
            rows.push(SnapshotRow {
                mapping_digest: mapping_digest.clone(),
                logical_model: mapping.logical_model.clone(),
                upstream_model: mapping.upstream_model.clone(),
                resolved_profile: mapping.resolved_profile.clone(),
                multiplier: mapping.multiplier.clone(),
                scenario_id: scenario.id.clone(),
                base_charge_nano_usd: base.to_string(),
                final_charge_nano_usd: final_charge.to_string(),
                line_items: scenario.line_items.clone(),
            });
        }
    }
    rows.sort_by(|left, right| {
        left.mapping_digest
            .as_bytes()
            .cmp(right.mapping_digest.as_bytes())
            .then_with(|| {
                left.scenario_id
                    .as_bytes()
                    .cmp(right.scenario_id.as_bytes())
            })
    });
    canonical_json(&PricingSnapshot { rows })
}

pub fn compare_snapshots(before: &[u8], after: &[u8]) -> anyhow::Result<SnapshotComparison> {
    if serde_json::from_slice::<PricingSnapshot>(before).is_err()
        || serde_json::from_slice::<PricingSnapshot>(after).is_err()
    {
        bail!("invalid pricing snapshot")
    }
    Ok(SnapshotComparison {
        equal: before == after,
        before_sha256: hex::encode(Sha256::digest(before)),
        after_sha256: hex::encode(Sha256::digest(after)),
    })
}

fn mapping_digest(mapping: &PricingMapping) -> anyhow::Result<String> {
    #[derive(Serialize)]
    struct Identity<'a> {
        provider_id: &'a str,
        group_id: &'a str,
        logical_model: &'a str,
        upstream_model: &'a str,
    }
    let bytes = canonical_json(&Identity {
        provider_id: &mapping.provider_id,
        group_id: &mapping.group_id,
        logical_model: &mapping.logical_model,
        upstream_model: &mapping.upstream_model,
    })?;
    Ok(hex::encode(Sha256::digest(bytes)))
}
