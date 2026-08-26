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
    pub layout_version: String,
    pub seed: u64,
    pub envelope: Envelope,
    pub groups: u64,
    pub providers: u64,
    pub provider_models: u64,
    pub distinct_models: u64,
    pub metadata_rows: u64,
    pub rate_rows: u64,
    pub offer_rate_entries: u64,
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateFixtureRow {
    pub id: String,
    pub model_name: String,
    pub usage_class: String,
    pub unit_price: String,
    pub public_repeat_count: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataFixtureRow {
    pub id: String,
    pub model_name: String,
    pub capability: String,
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
    pub cursor_position: u8,
    pub limit: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuerySetManifest {
    pub schema_version: u8,
    pub generator_version: u8,
    pub seed: u64,
    pub cases: u64,
    pub smoke_sha256: String,
    pub qualification_sha256: String,
}

#[derive(Serialize)]
struct HashMaterial {
    generator_version: u8,
    layout_version: &'static str,
    seed: u64,
    envelope: Envelope,
    groups: u64,
    providers: u64,
    provider_models: u64,
    distinct_models: u64,
    metadata_rows: u64,
    rate_rows: u64,
    offer_rate_entries: u64,
    hot_model_offers: u64,
    query_cases: u64,
}

impl FixtureManifest {
    pub fn generate(seed: u64, envelope: Envelope) -> anyhow::Result<Self> {
        let (
            groups,
            providers,
            provider_models,
            distinct_models,
            metadata_rows,
            rate_rows,
            offer_rate_entries,
        ) = match envelope {
            Envelope::Smoke => (8, 128, 4_096, 2_048, 2_048, 8_192, 32_768),
            Envelope::Qualification => {
                (128, 5_000, 250_000, 100_000, 250_000, 1_000_000, 2_000_000)
            }
        };
        let query_cases = 400;
        let material = HashMaterial {
            generator_version: 1,
            layout_version: "group-000-hot-fifty-and-seeded-cyclic-v2",
            seed,
            envelope,
            groups,
            providers,
            provider_models,
            distinct_models,
            metadata_rows,
            rate_rows,
            offer_rate_entries,
            hot_model_offers: providers,
            query_cases,
        };
        let encoded = serde_json::to_vec(&material).context("encode fixture manifest")?;
        let sha256 = hex::encode(Sha256::digest(encoded));
        Ok(Self {
            generator_version: material.generator_version,
            layout_version: material.layout_version.to_owned(),
            seed,
            envelope,
            groups,
            providers,
            provider_models,
            distinct_models,
            metadata_rows,
            rate_rows,
            offer_rate_entries,
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

    pub fn query_set_sha256(&self) -> anyhow::Result<String> {
        let encoded = serde_json::to_vec(&self.query_set()).context("encode query set")?;
        Ok(hex::encode(Sha256::digest(encoded)))
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
        (0..self.manifest.providers).map(|index| ProviderFixtureRow {
            id: format!("provider-{index:05}"),
            group_id: "group-000".to_owned(),
            public_name: format!("Provider {index:05}"),
            priority: i32::try_from(index).expect("supported envelope fits i32"),
            channel_public_name: format!("Channel {index:05}"),
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
                let seed_offset = self.manifest.seed % non_hot_count;
                let non_hot_index = (index - provider_index - 1 + seed_offset) % non_hot_count;
                fixture_model_name(non_hot_index + 1)
            };
            ProviderModelFixtureRow {
                provider_id: format!("provider-{provider_index:05}"),
                model_name,
            }
        })
    }

    pub fn rates(&self) -> impl Iterator<Item = RateFixtureRow> + '_ {
        let rates_per_model = self.manifest.rate_rows / self.manifest.distinct_models;
        (0..self.manifest.rate_rows).map(move |index| {
            let model_index = index / rates_per_model;
            let rate_index = index % rates_per_model;
            let public_repeat_count = match self.manifest.envelope {
                Envelope::Smoke => 2,
                Envelope::Qualification if rate_index < 8 => 1,
                Envelope::Qualification => 0,
            };
            RateFixtureRow {
                id: format!("rate-{index:07}"),
                model_name: fixture_model_name(model_index),
                usage_class: if rate_index.is_multiple_of(2) {
                    "input".to_owned()
                } else {
                    "output".to_owned()
                },
                unit_price: (rate_index + 1).to_string(),
                public_repeat_count,
            }
        })
    }

    pub fn metadata(&self) -> impl Iterator<Item = MetadataFixtureRow> + '_ {
        (0..self.manifest.metadata_rows).map(move |index| {
            let model_index = index % self.manifest.distinct_models;
            MetadataFixtureRow {
                id: format!("metadata-{index:07}"),
                model_name: fixture_model_name(model_index),
                capability: match index % 3 {
                    0 => "text",
                    1 => "vision",
                    _ => "tools",
                }
                .to_owned(),
            }
        })
    }

    pub fn query_set(&self) -> Vec<QueryCase> {
        let mut queries = Vec::with_capacity(400);
        let search_shapes = [
            Some("missing-model"),
            Some("hot-model"),
            Some("fifty-model"),
            None,
        ];
        for batch in 0..80_u64 {
            for offset in 0..4_u64 {
                let index = batch * 4 + offset;
                let query = search_shapes[usize::try_from(index % 4).expect("index fits usize")]
                    .map(str::to_owned);
                queries.push(QueryCase {
                    id: format!("list-{index:03}"),
                    kind: QueryKind::List,
                    query,
                    group: (index % 5 == 0).then(|| "Group 000".to_owned()),
                    model: None,
                    cursor_position: u8::try_from(index % 3).expect("cursor position fits u8"),
                    limit: 50,
                });
            }
            queries.push(QueryCase {
                id: format!("offers-{batch:03}"),
                kind: QueryKind::Offers,
                query: None,
                group: Some("Group 000".to_owned()),
                model: Some("hot-model".to_owned()),
                cursor_position: u8::try_from(batch % 3).expect("cursor position fits u8"),
                limit: 50,
            });
        }
        queries
    }
}

fn fixture_model_name(model_index: u64) -> String {
    if model_index == 0 {
        "hot-model".to_owned()
    } else if model_index <= 50 {
        format!("fifty-model-{:02}", model_index - 1)
    } else {
        format!("model-{:06}", model_index - 1)
    }
}

impl QuerySetManifest {
    pub fn read(path: &std::path::Path) -> anyhow::Result<Self> {
        let bytes = std::fs::read(path).context("read query-set manifest")?;
        serde_json::from_slice(&bytes).context("decode query-set manifest")
    }

    pub fn validate(&self, fixture: &FixtureManifest) -> anyhow::Result<()> {
        if self.schema_version != 1
            || self.generator_version != fixture.generator_version
            || self.seed != fixture.seed
            || self.cases != fixture.query_cases
        {
            anyhow::bail!("query_set_manifest_mismatch");
        }
        let expected = match fixture.envelope {
            Envelope::Smoke => &self.smoke_sha256,
            Envelope::Qualification => &self.qualification_sha256,
        };
        if fixture.query_set_sha256()? != *expected {
            anyhow::bail!("query_set_hash_mismatch");
        }
        Ok(())
    }
}
