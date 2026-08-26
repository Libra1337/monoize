use monoize_lynshen_rehearsal::marketplace::{Envelope, FixtureManifest, QueryKind};

const SEED: u64 = 0x004c_594e_5348_454e;

#[test]
fn smoke_manifest_is_deterministic_and_matches_its_envelope() {
    let first = FixtureManifest::generate(SEED, Envelope::Smoke).unwrap();
    let second = FixtureManifest::generate(SEED, Envelope::Smoke).unwrap();
    assert_eq!(first, second);
    assert_eq!(
        first.layout_version,
        "group-000-hot-fifty-and-seeded-cyclic-v2"
    );
    assert_eq!(first.groups, 8);
    assert_eq!(first.providers, 128);
    assert_eq!(first.provider_models, 4_096);
    assert_eq!(first.distinct_models, 2_048);
    assert_eq!(first.metadata_rows, 2_048);
    assert_eq!(first.rate_rows, 8_192);
    assert_eq!(first.offer_rate_entries, 32_768);
    assert_eq!(first.hot_model_offers, 128);
    assert_eq!(first.sha256.len(), 64);
    assert_eq!(
        first.sha256,
        "6649fd3d58489dd7d5fd44e91b5a5d97829e776c44d9c8a31e607e503c3c5af2"
    );
}

#[test]
fn qualification_manifest_matches_the_current_specification() {
    let manifest = FixtureManifest::generate(SEED, Envelope::Qualification).unwrap();
    assert_eq!(manifest.groups, 128);
    assert_eq!(manifest.providers, 5_000);
    assert_eq!(manifest.provider_models, 250_000);
    assert_eq!(manifest.distinct_models, 100_000);
    assert_eq!(manifest.metadata_rows, 250_000);
    assert_eq!(manifest.rate_rows, 1_000_000);
    assert_eq!(manifest.offer_rate_entries, 2_000_000);
    assert_eq!(manifest.hot_model_offers, 5_000);
}

#[test]
fn generated_rows_are_stable_and_the_hot_model_covers_every_provider() {
    let fixture = FixtureManifest::generate(SEED, Envelope::Smoke)
        .unwrap()
        .fixture();
    let groups = fixture.groups().collect::<Vec<_>>();
    let providers = fixture.providers().collect::<Vec<_>>();
    let models = fixture.provider_models().collect::<Vec<_>>();
    assert_eq!(groups.first().unwrap().id, "group-000");
    assert_eq!(groups.last().unwrap().id, "group-007");
    assert_eq!(providers.first().unwrap().id, "provider-00000");
    assert_eq!(providers.last().unwrap().id, "provider-00127");
    assert!(
        providers
            .iter()
            .all(|provider| provider.group_id == "group-000")
    );
    assert_eq!(models.len(), 4_096);
    assert_eq!(
        models
            .iter()
            .filter(|row| row.model_name == "hot-model")
            .count(),
        128
    );
    assert_eq!(
        models
            .iter()
            .filter(|row| row.model_name == "hot-model")
            .filter(|row| {
                providers
                    .iter()
                    .find(|provider| provider.id == row.provider_id)
                    .is_some_and(|provider| provider.group_id == "group-000")
            })
            .count(),
        128
    );
}

#[test]
fn smoke_rates_materialize_the_declared_offer_rate_entries() {
    use std::collections::BTreeMap;

    let fixture = FixtureManifest::generate(SEED, Envelope::Smoke)
        .unwrap()
        .fixture();
    let mut repeats_by_model = BTreeMap::<String, u64>::new();
    let rates = fixture.rates().collect::<Vec<_>>();
    for rate in &rates {
        *repeats_by_model.entry(rate.model_name.clone()).or_default() +=
            u64::from(rate.public_repeat_count);
    }
    let derived = fixture.provider_models().fold(0_u64, |total, mapping| {
        total + repeats_by_model[&mapping.model_name]
    });
    assert_eq!(rates.len(), 8_192);
    assert!(repeats_by_model.values().all(|count| *count == 8));
    assert_eq!(derived, 32_768);
    assert_eq!(fixture.metadata().count(), 2_048);
}

#[test]
fn canonical_query_set_has_more_than_256_weighted_cases() {
    let queries = FixtureManifest::generate(SEED, Envelope::Smoke)
        .unwrap()
        .query_set();
    assert!(queries.len() > 256);
    assert!(queries.iter().any(|query| query.kind == QueryKind::List));
    assert!(queries.iter().any(|query| query.kind == QueryKind::Offers));
    assert!(
        queries
            .iter()
            .filter(|query| query.kind == QueryKind::Offers)
            .all(|query| query.model.as_deref() == Some("hot-model"))
    );
    for search in ["missing-model", "hot-model", "fifty-model"] {
        assert!(
            queries
                .iter()
                .any(|query| query.query.as_deref() == Some(search))
        );
    }
    assert!(queries.iter().any(|query| query.query.is_none()));
    assert!(queries.iter().any(|query| query.cursor_position == 2));
}

#[test]
fn search_shapes_have_zero_one_fifty_and_broad_distinct_rows() {
    use std::collections::BTreeSet;

    let fixture = FixtureManifest::generate(SEED, Envelope::Smoke)
        .unwrap()
        .fixture();
    let models = fixture
        .provider_models()
        .map(|mapping| mapping.model_name)
        .collect::<BTreeSet<_>>();
    let count = |search: &str| models.iter().filter(|model| model.contains(search)).count();
    assert_eq!(count("missing-model"), 0);
    assert_eq!(count("hot-model"), 1);
    assert_eq!(count("fifty-model"), 50);
    assert_eq!(models.len(), 2_048);
}

#[test]
fn query_set_hashes_are_stable() {
    let smoke = FixtureManifest::generate(SEED, Envelope::Smoke).unwrap();
    let qualification = FixtureManifest::generate(SEED, Envelope::Qualification).unwrap();
    assert_eq!(
        smoke.query_set_sha256().unwrap(),
        "54c73d381b771906c948b44b066fc8c4004c1639d9d2e2178973f57b693bba19"
    );
    assert_eq!(
        qualification.query_set_sha256().unwrap(),
        "54c73d381b771906c948b44b066fc8c4004c1639d9d2e2178973f57b693bba19"
    );
}
