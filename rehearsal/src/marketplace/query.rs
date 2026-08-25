use sqlx::{Executor, Row, SqliteConnection};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryInput {
    pub query: Option<String>,
    pub group: Option<String>,
    pub after: Option<ListKey>,
    pub limit: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListKey {
    pub group_ordinal: i64,
    pub model_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarketplaceRow {
    pub group: String,
    pub model: String,
    pub offer_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListPage {
    pub items: Vec<MarketplaceRow>,
    pub next_key: Option<ListKey>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfferQueryInput {
    pub group: String,
    pub model: String,
    pub after: Option<OfferKey>,
    pub limit: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfferKey {
    pub priority: i32,
    pub provider_public_name: String,
    pub channel_public_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfferItem {
    pub provider_public_name: String,
    pub channel_public_name: String,
    pub priority: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfferPage {
    pub items: Vec<OfferItem>,
    pub next_key: Option<OfferKey>,
}

pub struct MarketplaceQuery;

impl MarketplaceQuery {
    pub async fn list_sqlite(
        db: &mut SqliteConnection,
        input: QueryInput,
    ) -> Result<ListPage, sqlx::Error> {
        if !(1..=50).contains(&input.limit) {
            return Err(sqlx::Error::Protocol("invalid limit".to_owned()));
        }
        let query = input.query.as_deref().map(ascii_fold).unwrap_or_default();
        let group = input.group.unwrap_or_default();
        let (after_group, after_model) = input.after.map_or((-1, Vec::new()), |key| {
            (key.group_ordinal, key.model_name.into_bytes())
        });
        let rows = sqlx::query(
            r#"SELECT g.sort_order, g.public_name AS group_name, pm.model_name,
                      COUNT(*) AS offer_count
               FROM monoize_provider_models pm
               JOIN monoize_providers p ON p.id = pm.provider_id
               JOIN monoize_groups g ON g.id = p.group_id
               WHERE p.enabled = 1
                 AND p.channel_enabled = 1
                 AND (?1 = X'' OR instr(pm.model_search_key, ?1) > 0)
                 AND (?2 = '' OR g.public_name = ?2)
                 AND (g.sort_order > ?3 OR (g.sort_order = ?3 AND pm.model_name_key > ?4))
               GROUP BY g.sort_order, g.public_name, pm.model_name, pm.model_name_key
               ORDER BY g.sort_order ASC, pm.model_name_key ASC
               LIMIT ?5"#,
        )
        .bind(query)
        .bind(group)
        .bind(after_group)
        .bind(after_model)
        .bind(i64::from(input.limit) + 1)
        .fetch_all(db)
        .await?;

        let has_more = rows.len() > usize::from(input.limit);
        let mut items = Vec::with_capacity(rows.len().min(usize::from(input.limit)));
        let mut keys = Vec::with_capacity(items.capacity());
        for row in rows.into_iter().take(usize::from(input.limit)) {
            let group_ordinal = row.try_get::<i64, _>("sort_order")?;
            let model = row.try_get::<String, _>("model_name")?;
            let offer_count = u64::try_from(row.try_get::<i64, _>("offer_count")?)
                .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
            keys.push(ListKey {
                group_ordinal,
                model_name: model.clone(),
            });
            items.push(MarketplaceRow {
                group: row.try_get("group_name")?,
                model,
                offer_count,
            });
        }
        Ok(ListPage {
            next_key: has_more.then(|| keys.last().expect("non-empty page with more rows").clone()),
            items,
        })
    }

    pub async fn offers_sqlite(
        db: &mut SqliteConnection,
        input: OfferQueryInput,
    ) -> Result<OfferPage, sqlx::Error> {
        if !(1..=50).contains(&input.limit) {
            return Err(sqlx::Error::Protocol("invalid limit".to_owned()));
        }
        let (after_priority, after_provider, after_channel, has_after) =
            input
                .after
                .map_or((0, Vec::new(), Vec::new(), 0_i64), |key| {
                    (
                        key.priority,
                        key.provider_public_name.into_bytes(),
                        key.channel_public_name.into_bytes(),
                        1,
                    )
                });
        let rows = sqlx::query(
            r#"SELECT p.priority, p.public_name, p.channel_public_name
               FROM monoize_provider_models pm
               JOIN monoize_providers p ON p.id = pm.provider_id
               JOIN monoize_groups g ON g.id = p.group_id
               WHERE p.enabled = 1 AND p.channel_enabled = 1
                 AND g.public_name = ?1 AND pm.model_name = ?2
                 AND (?6 = 0 OR p.priority > ?3
                      OR (p.priority = ?3 AND p.public_name_key > ?4)
                      OR (p.priority = ?3 AND p.public_name_key = ?4 AND p.channel_public_name_key > ?5))
               ORDER BY p.priority ASC, p.public_name_key ASC, p.channel_public_name_key ASC
               LIMIT ?7"#,
        )
        .bind(input.group)
        .bind(input.model)
        .bind(after_priority)
        .bind(after_provider)
        .bind(after_channel)
        .bind(has_after)
        .bind(i64::from(input.limit) + 1)
        .fetch_all(db)
        .await?;
        let has_more = rows.len() > usize::from(input.limit);
        let mut items = Vec::with_capacity(rows.len().min(usize::from(input.limit)));
        let mut keys = Vec::with_capacity(items.capacity());
        for row in rows.into_iter().take(usize::from(input.limit)) {
            let priority = row.try_get::<i32, _>("priority")?;
            let provider_public_name = row.try_get::<String, _>("public_name")?;
            let channel_public_name = row.try_get::<String, _>("channel_public_name")?;
            keys.push(OfferKey {
                priority,
                provider_public_name: provider_public_name.clone(),
                channel_public_name: channel_public_name.clone(),
            });
            items.push(OfferItem {
                provider_public_name,
                channel_public_name,
                priority,
            });
        }
        Ok(OfferPage {
            next_key: has_more.then(|| keys.last().expect("non-empty offer page").clone()),
            items,
        })
    }
}

pub async fn create_sqlite_query_fixture(db: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    for statement in [
        "CREATE TABLE monoize_groups (id TEXT PRIMARY KEY, public_name TEXT NOT NULL, sort_order INTEGER NOT NULL)",
        "CREATE TABLE monoize_providers (id TEXT PRIMARY KEY, group_id TEXT NOT NULL, public_name TEXT NOT NULL, public_name_key BLOB NOT NULL, priority INTEGER NOT NULL, enabled INTEGER NOT NULL, channel_public_name TEXT NOT NULL, channel_public_name_key BLOB NOT NULL, channel_enabled INTEGER NOT NULL)",
        "CREATE TABLE monoize_provider_models (provider_id TEXT NOT NULL, model_name TEXT NOT NULL, model_name_key BLOB NOT NULL, model_search_key BLOB NOT NULL)",
        "INSERT INTO monoize_groups VALUES ('ga', 'Alpha', 0), ('gb', 'Beta', 1)",
        "INSERT INTO monoize_providers VALUES ('pa1', 'ga', 'Provider B', CAST('Provider B' AS BLOB), 1, 1, 'Channel A', CAST('Channel A' AS BLOB), 1), ('pa2', 'ga', 'Provider A', CAST('Provider A' AS BLOB), 1, 1, 'Channel Z', CAST('Channel Z' AS BLOB), 1), ('pb1', 'gb', 'Provider C', CAST('Provider C' AS BLOB), 0, 1, 'Channel C', CAST('Channel C' AS BLOB), 1), ('disabled', 'ga', 'Hidden', CAST('Hidden' AS BLOB), 0, 0, 'Hidden', CAST('Hidden' AS BLOB), 1)",
        "INSERT INTO monoize_provider_models VALUES ('pa1', 'GPT-4o', CAST('GPT-4o' AS BLOB), CAST('gpt-4o' AS BLOB)), ('pa2', 'GPT-4o', CAST('GPT-4o' AS BLOB), CAST('gpt-4o' AS BLOB)), ('pa1', '模型-A', CAST('模型-A' AS BLOB), CAST('模型-a' AS BLOB)), ('pb1', 'GPT-4o', CAST('GPT-4o' AS BLOB), CAST('gpt-4o' AS BLOB)), ('disabled', 'hidden', CAST('hidden' AS BLOB), CAST('hidden' AS BLOB))",
    ] {
        db.execute(statement).await?;
    }
    Ok(())
}

fn ascii_fold(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .iter()
        .map(|byte| {
            if byte.is_ascii_uppercase() {
                byte + 32
            } else {
                *byte
            }
        })
        .collect()
}
