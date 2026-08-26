use anyhow::Context;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Envelope {
    Smoke,
    Qualification,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureManifest {
    pub generator_version: u8,
    pub seed: u64,
    pub envelope: Envelope,
    pub groups: u64,
    pub providers: u64,
    pub provider_models: u64,
    pub distinct_models: u64,
    pub metadata_rows: u64,
    pub rate_rows: u64,
    pub offers: u64,
    pub hot_model_offers: u64,
    pub query_cases: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarketplaceFixture {
    manifest: FixtureManifest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupFixtureRow {
    pub id: String,
    pub public_name: String,
    pub sort_order: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderFixtureRow {
    pub id: String,
    pub group_id: String,
    pub public_name: String,
    pub priority: i32,
    pub channel_public_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderModelFixtureRow {
    pub provider_id: String,
    pub model_name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryKind {
    List,
    Offers,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryCase {
    pub id: String,
    pub kind: QueryKind,
    pub query: Option<String>,
    pub group: Option<String>,
    pub model: Option<String>,
    pub cursor_page: u8,
    pub limit: u16,
}

#[derive(Serialize)]
struct HashMaterial {
    generator_version: u8,
    seed: u64,
    envelope: Envelope,
    groups: u64,
    providers: u64,
    provider_models: u64,
    distinct_models: u64,
    metadata_rows: u64,
    rate_rows: u64,
    offers: u64,
    hot_model_offers: u64,
    query_cases: u64,
}

impl FixtureManifest {
    pub fn generate(seed: u64, envelope: Envelope) -> anyhow::Result<Self> {
        let (groups, providers, provider_models, distinct_models, metadata_rows, rate_rows, offers) =
            match envelope {
                Envelope::Smoke => (8, 128, 4_096, 2_048, 2_048, 8_192, 32_768),
                Envelope::Qualification => {
                    (128, 5_000, 250_000, 100_000, 250_000, 1_000_000, 2_000_000)
                }
            };
        let query_cases = 400;
        let material = HashMaterial {
            generator_version: 1,
            seed,
            envelope,
            groups,
            providers,
            provider_models,
            distinct_models,
            metadata_rows,
            rate_rows,
            offers,
            hot_model_offers: providers,
            query_cases,
        };
        let encoded = serde_json::to_vec(&material).context("encode fixture manifest")?;
        let sha256 = hex::encode(Sha256::digest(encoded));
        Ok(Self {
            generator_version: material.generator_version,
            seed,
            envelope,
            groups,
            providers,
            provider_models,
            distinct_models,
            metadata_rows,
            rate_rows,
            offers,
            hot_model_offers: providers,
            query_cases,
            sha256,
        })
    }

    pub fn fixture(&self) -> MarketplaceFixture {
        MarketplaceFixture {
            manifest: self.clone(),
        }
    }

    pub fn query_set(&self) -> Vec<QueryCase> {
        self.fixture().query_set()
    }
}

impl MarketplaceFixture {
    pub fn groups(&self) -> impl Iterator<Item = GroupFixtureRow> + '_ {
        (0..self.manifest.groups).map(|index| GroupFixtureRow {
            id: format!("group-{index:03}"),
            public_name: format!("Group {index:03}"),
            sort_order: i64::try_from(index).expect("supported envelope fits i64"),
        })
    }

    pub fn providers(&self) -> impl Iterator<Item = ProviderFixtureRow> + '_ {
        (0..self.manifest.providers).map(|index| {
            let group_index = index % self.manifest.groups;
            ProviderFixtureRow {
                id: format!("provider-{index:05}"),
                group_id: format!("group-{group_index:03}"),
                public_name: format!("Provider {index:05}"),
                priority: i32::try_from(index / self.manifest.groups)
                    .expect("supported envelope fits i32"),
                channel_public_name: format!("Channel {index:05}"),
            }
        })
    }

    pub fn provider_models(&self) -> impl Iterator<Item = ProviderModelFixtureRow> + '_ {
        let mappings_per_provider = self.manifest.provider_models / self.manifest.providers;
        (0..self.manifest.provider_models).map(move |index| {
            let provider_index = index / mappings_per_provider;
            let provider_slot = index % mappings_per_provider;
            let model_name = if provider_slot == 0 {
                "hot-model".to_owned()
            } else {
                let non_hot_count = self.manifest.distinct_models - 1;
                let non_hot_index = (index - provider_index - 1) % non_hot_count;
                format!("model-{non_hot_index:06}")
            };
            ProviderModelFixtureRow {
                provider_id: format!("provider-{provider_index:05}"),
                model_name,
            }
        })
    }

    pub fn query_set(&self) -> Vec<QueryCase> {
        let mut queries = Vec::with_capacity(400);
        let search_shapes = [None, Some("m"), Some("model"), Some("model-000")];
        for index in 0..320_u64 {
            let group_index = index % self.manifest.groups;
            let query = search_shapes[usize::try_from(index % 4).expect("index fits usize")]
                .map(str::to_owned);
            queries.push(QueryCase {
                id: format!("list-{index:03}"),
                kind: QueryKind::List,
                query,
                group: (index % 5 == 0).then(|| format!("Group {group_index:03}")),
                model: None,
                cursor_page: u8::try_from(index % 3).expect("cursor page fits u8"),
                limit: 50,
            });
        }
        for index in 0..80_u64 {
            let group_index = index % self.manifest.groups;
            queries.push(QueryCase {
                id: format!("offers-{index:03}"),
                kind: QueryKind::Offers,
                query: None,
                group: Some(format!("Group {group_index:03}")),
                model: Some(if index % 2 == 0 {
                    "hot-model".to_owned()
                } else {
                    format!("model-{:06}", index % (self.manifest.distinct_models - 1))
                }),
                cursor_page: u8::try_from(index % 3).expect("cursor page fits u8"),
                limit: 50,
            });
        }
        queries
    }
}
