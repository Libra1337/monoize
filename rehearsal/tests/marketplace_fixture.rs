use monoize_lynshen_rehearsal::marketplace::{Envelope, FixtureManifest, QueryKind};

const SEED: u64 = 0x004c_594e_5348_454e;

#[test]
fn smoke_manifest_is_deterministic_and_matches_its_envelope() {
    let first = FixtureManifest::generate(SEED, Envelope::Smoke).unwrap();
    let second = FixtureManifest::generate(SEED, Envelope::Smoke).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.groups, 8);
    assert_eq!(first.providers, 128);
    assert_eq!(first.provider_models, 4_096);
    assert_eq!(first.distinct_models, 2_048);
    assert_eq!(first.metadata_rows, 2_048);
    assert_eq!(first.rate_rows, 8_192);
    assert_eq!(first.offers, 32_768);
    assert_eq!(first.hot_model_offers, 128);
    assert_eq!(first.sha256.len(), 64);
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
    assert_eq!(manifest.offers, 2_000_000);
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
    assert_eq!(models.len(), 4_096);
    assert_eq!(
        models
            .iter()
            .filter(|row| row.model_name == "hot-model")
            .count(),
        128
    );
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
            .any(|query| query.query.as_deref() == Some("model"))
    );
    assert!(queries.iter().any(|query| query.cursor_page == 2));
}
