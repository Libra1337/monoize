use super::{Classification, LegacyProvider, transform_provider};
use anyhow::Context;
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug)]
pub struct PreflightSource {
    pub providers: Vec<LegacyProvider>,
    pub public_names: Vec<PublicName>,
    pub fingerprint_secrets: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PublicName {
    pub entity: String,
    pub source_id: String,
    pub normalized_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PreflightReport {
    pub source_fingerprint: String,
    pub provider_count: usize,
    pub target_provider_count: usize,
    pub blockers: Vec<String>,
    pub providers: Vec<ProviderPreflight>,
    pub public_names: Vec<PublicName>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProviderPreflight {
    pub source_provider_digest: String,
    pub classification: Classification,
    pub group_ids: Vec<String>,
    pub old_channel_ids: Vec<String>,
    pub target_provider_ids: Vec<String>,
    pub target_channel_ids: Vec<String>,
}

pub fn build_preflight_report(source: &PreflightSource) -> anyhow::Result<PreflightReport> {
    let source_fingerprint = source_fingerprint(source)?;
    let mut providers = Vec::with_capacity(source.providers.len());
    let mut blockers = Vec::new();
    let mut target_provider_count = 0;

    for provider in &source.providers {
        match transform_provider(provider) {
            Ok(result) => {
                target_provider_count += result.targets.len();
                providers.push(ProviderPreflight {
                    source_provider_digest: opaque_digest(&provider.id),
                    classification: result.classification,
                    group_ids: provider.group_ids.clone(),
                    old_channel_ids: provider
                        .channels
                        .iter()
                        .map(|channel| opaque_digest(&channel.id))
                        .collect(),
                    target_provider_ids: result
                        .targets
                        .iter()
                        .map(|target| target.id.clone())
                        .collect(),
                    target_channel_ids: result
                        .targets
                        .iter()
                        .map(|target| target.channel.id.clone())
                        .collect(),
                });
            }
            Err(error) => {
                blockers.push(format!("{}:{}", opaque_digest(&provider.id), error.code()))
            }
        }
    }

    providers.sort_by(|left, right| {
        left.source_provider_digest
            .as_bytes()
            .cmp(right.source_provider_digest.as_bytes())
    });
    blockers.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    let mut public_names = source.public_names.clone();
    public_names.sort_by(|left, right| {
        left.entity
            .as_bytes()
            .cmp(right.entity.as_bytes())
            .then_with(|| {
                left.normalized_name
                    .as_bytes()
                    .cmp(right.normalized_name.as_bytes())
            })
            .then_with(|| left.source_id.as_bytes().cmp(right.source_id.as_bytes()))
    });

    Ok(PreflightReport {
        source_fingerprint,
        provider_count: source.providers.len(),
        target_provider_count,
        blockers,
        providers,
        public_names,
    })
}

fn source_fingerprint(source: &PreflightSource) -> anyhow::Result<String> {
    #[derive(Serialize)]
    struct Fingerprint<'a> {
        providers: &'a [LegacyProvider],
        public_names: &'a [PublicName],
        secrets: &'a [String],
    }
    let bytes = canonical_json(&Fingerprint {
        providers: &source.providers,
        public_names: &source.public_names,
        secrets: &source.fingerprint_secrets,
    })?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn opaque_digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

pub fn canonical_json<T: Serialize>(value: &T) -> anyhow::Result<Vec<u8>> {
    let value = serde_json::to_value(value).context("serialize canonical JSON source")?;
    let canonical = canonical_value(value);
    let mut bytes = serde_json::to_vec(&canonical).context("encode canonical JSON")?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn canonical_value(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonical_value).collect()),
        Value::Object(values) => {
            let mut keys = values.keys().cloned().collect::<Vec<_>>();
            keys.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
            let mut canonical = Map::new();
            for key in keys {
                let value = values
                    .get(&key)
                    .expect("key came from the same object")
                    .clone();
                canonical.insert(key, canonical_value(value));
            }
            Value::Object(canonical)
        }
        scalar => scalar,
    }
}
