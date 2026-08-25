use super::CanonicalDecimal;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::str::FromStr;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyProvider {
    pub id: String,
    pub name: String,
    pub priority: i32,
    pub group_ids: Vec<String>,
    pub channels: Vec<LegacyChannel>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyChannel {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub enabled: bool,
    pub weight: i32,
    pub models: Vec<LegacyModel>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyModel {
    pub name: String,
    pub redirect: Option<String>,
    pub resolved_profile: Option<String>,
    pub multiplier: CanonicalDecimal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetProvider {
    pub id: String,
    pub source_provider_id: String,
    pub group_id: String,
    pub priority: i32,
    pub pricing_profile: Option<String>,
    pub multiplier: CanonicalDecimal,
    pub channel: TargetChannel,
    pub models: Vec<TargetModel>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetChannel {
    pub id: String,
    pub source_channel_id: String,
    pub name: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetModel {
    pub name: String,
    pub redirect: Option<String>,
    pub pricing: PricingMode,
    pub multiplier_override: Option<CanonicalDecimal>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "profile", rename_all = "snake_case")]
pub enum PricingMode {
    Inherit,
    Override(String),
    Unpriced,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    RouteSafe,
    SemanticChange,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransformResult {
    pub classification: Classification,
    pub targets: Vec<TargetProvider>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransformError {
    code: &'static str,
}

impl TransformError {
    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for TransformError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for TransformError {}

pub fn transform_provider(source: &LegacyProvider) -> Result<TransformResult, TransformError> {
    if source.group_ids.is_empty() {
        return Err(TransformError {
            code: "provider_has_no_group",
        });
    }
    if source.channels.is_empty() {
        return Err(TransformError {
            code: "provider_has_no_channel",
        });
    }

    let positive_enabled = source
        .channels
        .iter()
        .filter(|channel| channel.enabled && channel.weight > 0)
        .count();
    let classification = if source.group_ids.len() == 1 && positive_enabled <= 1 {
        Classification::RouteSafe
    } else {
        Classification::SemanticChange
    };

    let mut channels = source.channels.iter().collect::<Vec<_>>();
    channels.sort_by(|left, right| {
        left.created_at
            .as_bytes()
            .cmp(right.created_at.as_bytes())
            .then_with(|| left.id.as_bytes().cmp(right.id.as_bytes()))
    });

    let mut targets = Vec::with_capacity(source.group_ids.len() * channels.len());
    for (group_index, group_id) in source.group_ids.iter().enumerate() {
        for (channel_index, channel) in channels.iter().enumerate() {
            let pricing_profile = infer_default_profile(&channel.models);
            let multiplier = infer_default_multiplier(&channel.models);
            let models = channel
                .models
                .iter()
                .map(|model| TargetModel {
                    name: model.name.clone(),
                    redirect: model.redirect.clone(),
                    pricing: match (&model.resolved_profile, &pricing_profile) {
                        (None, _) => PricingMode::Unpriced,
                        (Some(profile), Some(default)) if profile == default => {
                            PricingMode::Inherit
                        }
                        (Some(profile), _) => PricingMode::Override(profile.clone()),
                    },
                    multiplier_override: (model.multiplier != multiplier)
                        .then(|| model.multiplier.clone()),
                })
                .collect();
            let pair_index = group_index * channels.len() + channel_index;
            targets.push(TargetProvider {
                id: if pair_index == 0 {
                    source.id.clone()
                } else {
                    deterministic_id("provider", &source.id, group_id, &channel.id)
                },
                source_provider_id: source.id.clone(),
                group_id: group_id.clone(),
                priority: source.priority,
                pricing_profile,
                multiplier,
                channel: TargetChannel {
                    id: if group_index == 0 {
                        channel.id.clone()
                    } else {
                        deterministic_id("channel", &source.id, group_id, &channel.id)
                    },
                    source_channel_id: channel.id.clone(),
                    name: channel.name.clone(),
                    enabled: channel.enabled && channel.weight > 0,
                },
                models,
            });
        }
    }

    Ok(TransformResult {
        classification,
        targets,
    })
}

pub fn deterministic_id(kind: &str, provider_id: &str, group_id: &str, channel_id: &str) -> String {
    let mut digest = Sha256::new();
    for component in [
        "lynshen-provider-migration-v1",
        kind,
        provider_id,
        group_id,
        channel_id,
    ] {
        let bytes = component.as_bytes();
        digest.update((bytes.len() as u64).to_be_bytes());
        digest.update(bytes);
    }
    let prefix = if kind == "provider" { "p_" } else { "c_" };
    format!("{prefix}{}", &hex::encode(digest.finalize())[..32])
}

fn infer_default_profile(models: &[LegacyModel]) -> Option<String> {
    let mut counts = BTreeMap::<&str, usize>::new();
    for profile in models
        .iter()
        .filter_map(|model| model.resolved_profile.as_deref())
    {
        *counts.entry(profile).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|(left_profile, left_count), (right_profile, right_count)| {
            left_count
                .cmp(right_count)
                .then_with(|| right_profile.as_bytes().cmp(left_profile.as_bytes()))
        })
        .map(|(profile, _)| profile.to_owned())
}

fn infer_default_multiplier(models: &[LegacyModel]) -> CanonicalDecimal {
    if models.is_empty() {
        return CanonicalDecimal::parse("1").expect("one is a valid multiplier");
    }
    let mut counts = BTreeMap::<&str, usize>::new();
    for model in models {
        *counts.entry(model.multiplier.as_str()).or_default() += 1;
    }
    let selected = counts
        .into_iter()
        .max_by(|(left_value, left_count), (right_value, right_count)| {
            left_count
                .cmp(right_count)
                .then_with(|| numeric_decimal_cmp(right_value, left_value))
        })
        .expect("non-empty model list has one multiplier")
        .0;
    CanonicalDecimal::parse(selected).expect("legacy multiplier was already validated")
}

fn numeric_decimal_cmp(left: &str, right: &str) -> Ordering {
    let left = Decimal::from_str(left).expect("canonical decimal");
    let right = Decimal::from_str(right).expect("canonical decimal");
    left.cmp(&right)
}
