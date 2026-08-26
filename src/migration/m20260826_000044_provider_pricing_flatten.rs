#[cfg(test)]
mod tests {
    use super::{ascii_fold_expression, target_provider_ddl};

    #[test]
    fn sqlite_target_schema_embeds_channel_and_removes_legacy_tables() {
        let ddl = target_provider_ddl();
        assert!(
            ddl.iter()
                .any(|statement| statement.contains("channel_id TEXT NOT NULL UNIQUE"))
        );
        assert!(
            ddl.iter()
                .any(|statement| statement.contains("group_id TEXT NOT NULL"))
        );
        assert!(
            ddl.iter()
                .any(|statement| statement.contains("monoize_provider_models"))
        );
        assert!(
            !ddl.iter()
                .any(|statement| statement.contains("monoize_channels"))
        );
        assert!(
            !ddl.iter()
                .any(|statement| statement.contains("monoize_channel_models"))
        );
    }

    #[test]
    fn model_search_key_uses_explicit_ascii_replacements() {
        let expression = ascii_fold_expression("model_name");
        assert!(expression.contains("replace("));
        assert!(expression.contains("'A', 'a'"));
        assert!(expression.contains("'Z', 'z'"));
        assert!(!expression.contains("lower("));
    }
}

pub(crate) fn ascii_fold_expression(column: &str) -> String {
    (b'A'..=b'Z').fold(column.to_owned(), |expression, upper| {
        format!(
            "replace({expression}, '{}', '{}')",
            char::from(upper),
            char::from(upper + 32)
        )
    })
}

pub(crate) fn target_provider_ddl() -> Vec<String> {
    let model_search = ascii_fold_expression("model_name");
    vec![
        format!(
            "CREATE TABLE monoize_provider_models (provider_id TEXT NOT NULL, model_name TEXT NOT NULL, model_name_key BLOB NOT NULL CHECK(model_name_key = CAST(model_name AS BLOB)), model_search_key BLOB NOT NULL CHECK(model_search_key = CAST({model_search} AS BLOB)), redirect TEXT, pricing_profile_mode TEXT NOT NULL CHECK(pricing_profile_mode IN ('inherit','override','unpriced')), pricing_profile_override TEXT, multiplier_override TEXT, created_at TEXT NOT NULL, PRIMARY KEY(provider_id, model_name_key))"
        ),
        "CREATE TABLE monoize_providers (id TEXT PRIMARY KEY NOT NULL, group_id TEXT NOT NULL, name TEXT NOT NULL, public_name TEXT NOT NULL, public_name_key BLOB NOT NULL CHECK(public_name_key = CAST(public_name AS BLOB)), priority INTEGER NOT NULL, enabled INTEGER NOT NULL, pricing_profile TEXT, multiplier TEXT NOT NULL, configuration_generation INTEGER NOT NULL, created_at TEXT NOT NULL, channel_id TEXT NOT NULL UNIQUE, channel_name TEXT NOT NULL, channel_public_name TEXT NOT NULL, channel_public_name_key BLOB NOT NULL CHECK(channel_public_name_key = CAST(channel_public_name AS BLOB)), channel_provider_type TEXT NOT NULL, channel_base_url TEXT NOT NULL, channel_api_key TEXT NOT NULL, channel_enabled INTEGER NOT NULL, channel_max_retries INTEGER NOT NULL)".to_string(),
        "CREATE INDEX idx_monoize_provider_route ON monoize_providers(group_id, priority, created_at, id)".to_string(),
        "CREATE INDEX idx_monoize_provider_models_lookup ON monoize_provider_models(model_name_key, provider_id)".to_string(),
        "CREATE UNIQUE INDEX uq_monoize_provider_public_name ON monoize_providers(group_id, public_name_key)".to_string(),
        "CREATE UNIQUE INDEX uq_monoize_channel_public_name ON monoize_providers(group_id, channel_public_name_key)".to_string(),
    ]
}
