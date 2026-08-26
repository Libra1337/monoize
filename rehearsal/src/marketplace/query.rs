use std::collections::BTreeMap;

use sqlx::{Connection, Executor, PgConnection, QueryBuilder, Row, Sqlite, SqliteConnection};

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
    pub statement_count: u64,
}

struct ListCandidate {
    group_id: String,
    group_ordinal: i64,
    group_name: String,
    model: String,
    model_name_key: Vec<u8>,
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
            r#"SELECT group_id, group_sort_order AS sort_order,
                      group_public_name AS group_name, model_name, model_name_key
               FROM marketplace_group_models
               WHERE (?1 = X'' OR instr(model_search_key, ?1) > 0)
                 AND (?2 = '' OR group_public_name = ?2)
                 AND (group_sort_order > ?3
                      OR (group_sort_order = ?3 AND model_name_key > ?4))
               ORDER BY group_sort_order ASC, model_name_key ASC
               LIMIT ?5"#,
        )
        .bind(query)
        .bind(group)
        .bind(after_group)
        .bind(after_model)
        .bind(i64::from(input.limit) + 1)
        .fetch_all(&mut *db)
        .await?;
        let candidates = decode_list_candidates(rows)?;
        if candidates.is_empty() {
            return Ok(empty_list_page());
        }
        let offer_counts = count_sqlite_offers(db, &candidates).await?;
        build_list_page(candidates, offer_counts, input.limit, 2)
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
                  AND EXISTS (
                      SELECT 1 FROM billing_rate_records br
                      WHERE br.model_name = pm.model_name AND br.public_repeat_count > 0
                  )
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
        .fetch_all(&mut *db)
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

    pub async fn list_postgres(
        db: &mut PgConnection,
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
            r#"SELECT group_id, group_sort_order AS sort_order,
                      group_public_name AS group_name, model_name, model_name_key
               FROM marketplace_group_models
               WHERE (octet_length($1::bytea) = 0 OR position($1::bytea in model_search_key) > 0)
                 AND ($2 = '' OR group_public_name = $2)
                 AND (group_sort_order > $3
                      OR (group_sort_order = $3 AND model_name_key > $4::bytea))
               ORDER BY group_sort_order ASC, model_name_key ASC
               LIMIT $5"#,
        )
        .bind(query)
        .bind(group)
        .bind(after_group)
        .bind(after_model)
        .bind(i64::from(input.limit) + 1)
        .fetch_all(&mut *db)
        .await?;
        let candidates = decode_list_candidates(rows)?;
        if candidates.is_empty() {
            return Ok(empty_list_page());
        }
        let offer_counts = count_postgres_offers(db, &candidates).await?;
        build_list_page(candidates, offer_counts, input.limit, 2)
    }

    pub async fn offers_postgres(
        db: &mut PgConnection,
        input: OfferQueryInput,
    ) -> Result<OfferPage, sqlx::Error> {
        if !(1..=50).contains(&input.limit) {
            return Err(sqlx::Error::Protocol("invalid limit".to_owned()));
        }
        let (after_priority, after_provider, after_channel, has_after) = offer_after(input.after);
        let rows = sqlx::query(
            r#"SELECT p.priority, p.public_name, p.channel_public_name
               FROM monoize_provider_models pm
               JOIN monoize_providers p ON p.id = pm.provider_id
               JOIN monoize_groups g ON g.id = p.group_id
               WHERE p.enabled = 1 AND p.channel_enabled = 1
                  AND g.public_name = $1 AND pm.model_name = $2
                  AND EXISTS (
                      SELECT 1 FROM billing_rate_records br
                      WHERE br.model_name = pm.model_name AND br.public_repeat_count > 0
                  )
                 AND ($6 = 0 OR p.priority > $3
                      OR (p.priority = $3 AND p.public_name_key > $4::bytea)
                      OR (p.priority = $3 AND p.public_name_key = $4::bytea AND p.channel_public_name_key > $5::bytea))
               ORDER BY p.priority ASC, p.public_name_key ASC, p.channel_public_name_key ASC
               LIMIT $7"#,
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
        decode_offer_rows(rows, input.limit)
    }
}

pub async fn create_sqlite_query_fixture(db: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    for statement in [
        "CREATE TABLE monoize_groups (id TEXT PRIMARY KEY, public_name TEXT NOT NULL, sort_order INTEGER NOT NULL)",
        "CREATE TABLE monoize_providers (id TEXT PRIMARY KEY, group_id TEXT NOT NULL, public_name TEXT NOT NULL, public_name_key BLOB NOT NULL, priority INTEGER NOT NULL, enabled INTEGER NOT NULL, channel_public_name TEXT NOT NULL, channel_public_name_key BLOB NOT NULL, channel_enabled INTEGER NOT NULL)",
        "CREATE TABLE monoize_provider_models (provider_id TEXT NOT NULL, model_name TEXT NOT NULL, model_name_key BLOB NOT NULL, model_search_key BLOB NOT NULL)",
        "CREATE TABLE marketplace_group_models (group_id TEXT NOT NULL, group_sort_order INTEGER NOT NULL, group_public_name TEXT NOT NULL, model_name TEXT NOT NULL, model_name_key BLOB NOT NULL, model_search_key BLOB NOT NULL, PRIMARY KEY(group_id, model_name_key), UNIQUE(group_sort_order, model_name_key))",
        "CREATE TABLE billing_rate_records (model_name TEXT NOT NULL, public_repeat_count INTEGER NOT NULL)",
        "INSERT INTO monoize_groups VALUES ('ga', 'Alpha', 0), ('gb', 'Beta', 1)",
        "INSERT INTO monoize_providers VALUES ('pa1', 'ga', 'Provider B', CAST('Provider B' AS BLOB), 1, 1, 'Channel A', CAST('Channel A' AS BLOB), 1), ('pa2', 'ga', 'Provider A', CAST('Provider A' AS BLOB), 1, 1, 'Channel Z', CAST('Channel Z' AS BLOB), 1), ('pb1', 'gb', 'Provider C', CAST('Provider C' AS BLOB), 0, 1, 'Channel C', CAST('Channel C' AS BLOB), 1), ('disabled', 'ga', 'Hidden', CAST('Hidden' AS BLOB), 0, 0, 'Hidden', CAST('Hidden' AS BLOB), 1)",
        "INSERT INTO monoize_provider_models VALUES ('pa1', 'GPT-4o', CAST('GPT-4o' AS BLOB), CAST('gpt-4o' AS BLOB)), ('pa2', 'GPT-4o', CAST('GPT-4o' AS BLOB), CAST('gpt-4o' AS BLOB)), ('pa1', '模型-A', CAST('模型-A' AS BLOB), CAST('模型-a' AS BLOB)), ('pb1', 'GPT-4o', CAST('GPT-4o' AS BLOB), CAST('gpt-4o' AS BLOB)), ('disabled', 'hidden', CAST('hidden' AS BLOB), CAST('hidden' AS BLOB))",
        "INSERT INTO billing_rate_records VALUES ('GPT-4o', 1), ('模型-A', 1)",
        "INSERT INTO marketplace_group_models SELECT g.id, g.sort_order, g.public_name, pm.model_name, pm.model_name_key, pm.model_search_key FROM monoize_provider_models pm JOIN monoize_providers p ON p.id = pm.provider_id JOIN monoize_groups g ON g.id = p.group_id WHERE p.enabled = 1 AND p.channel_enabled = 1 AND EXISTS (SELECT 1 FROM billing_rate_records br WHERE br.model_name = pm.model_name AND br.public_repeat_count > 0) GROUP BY g.id, g.sort_order, g.public_name, pm.model_name, pm.model_name_key, pm.model_search_key",
    ] {
        db.execute(statement).await?;
    }
    Ok(())
}

pub async fn create_postgres_query_fixture(db: &mut PgConnection) -> Result<(), sqlx::Error> {
    db.execute("DROP SCHEMA IF EXISTS public CASCADE; CREATE SCHEMA public")
        .await?;
    for statement in [
        "CREATE TABLE monoize_groups (id TEXT PRIMARY KEY, public_name TEXT NOT NULL, sort_order BIGINT NOT NULL)",
        "CREATE TABLE monoize_providers (id TEXT PRIMARY KEY, group_id TEXT NOT NULL, public_name TEXT NOT NULL, public_name_key BYTEA NOT NULL, priority INTEGER NOT NULL, enabled INTEGER NOT NULL, channel_public_name TEXT NOT NULL, channel_public_name_key BYTEA NOT NULL, channel_enabled INTEGER NOT NULL)",
        "CREATE TABLE monoize_provider_models (provider_id TEXT NOT NULL, model_name TEXT NOT NULL, model_name_key BYTEA NOT NULL, model_search_key BYTEA NOT NULL)",
        "CREATE TABLE marketplace_group_models (group_id TEXT NOT NULL, group_sort_order BIGINT NOT NULL, group_public_name TEXT NOT NULL, model_name TEXT NOT NULL, model_name_key BYTEA NOT NULL, model_search_key BYTEA NOT NULL, PRIMARY KEY(group_id, model_name_key), UNIQUE(group_sort_order, model_name_key))",
        "CREATE TABLE billing_rate_records (model_name TEXT NOT NULL, public_repeat_count INTEGER NOT NULL)",
        "INSERT INTO monoize_groups VALUES ('ga', 'Alpha', 0), ('gb', 'Beta', 1)",
        "INSERT INTO monoize_providers VALUES ('pa1', 'ga', 'Provider B', convert_to('Provider B', 'UTF8'), 1, 1, 'Channel A', convert_to('Channel A', 'UTF8'), 1), ('pa2', 'ga', 'Provider A', convert_to('Provider A', 'UTF8'), 1, 1, 'Channel Z', convert_to('Channel Z', 'UTF8'), 1), ('pb1', 'gb', 'Provider C', convert_to('Provider C', 'UTF8'), 0, 1, 'Channel C', convert_to('Channel C', 'UTF8'), 1), ('disabled', 'ga', 'Hidden', convert_to('Hidden', 'UTF8'), 0, 0, 'Hidden', convert_to('Hidden', 'UTF8'), 1)",
        "INSERT INTO monoize_provider_models VALUES ('pa1', 'GPT-4o', convert_to('GPT-4o', 'UTF8'), convert_to('gpt-4o', 'UTF8')), ('pa2', 'GPT-4o', convert_to('GPT-4o', 'UTF8'), convert_to('gpt-4o', 'UTF8')), ('pa1', '模型-A', convert_to('模型-A', 'UTF8'), convert_to('模型-a', 'UTF8')), ('pb1', 'GPT-4o', convert_to('GPT-4o', 'UTF8'), convert_to('gpt-4o', 'UTF8')), ('disabled', 'hidden', convert_to('hidden', 'UTF8'), convert_to('hidden', 'UTF8'))",
        "INSERT INTO billing_rate_records VALUES ('GPT-4o', 1), ('模型-A', 1)",
        "INSERT INTO marketplace_group_models SELECT g.id, g.sort_order, g.public_name, pm.model_name, pm.model_name_key, pm.model_search_key FROM monoize_provider_models pm JOIN monoize_providers p ON p.id = pm.provider_id JOIN monoize_groups g ON g.id = p.group_id WHERE p.enabled = 1 AND p.channel_enabled = 1 AND EXISTS (SELECT 1 FROM billing_rate_records br WHERE br.model_name = pm.model_name AND br.public_repeat_count > 0) GROUP BY g.id, g.sort_order, g.public_name, pm.model_name, pm.model_name_key, pm.model_search_key",
    ] {
        db.execute(statement).await?;
    }
    Ok(())
}

pub async fn rebuild_sqlite_group_models(db: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    let mut transaction = db.begin().await?;
    transaction
        .execute("DELETE FROM marketplace_group_models")
        .await?;
    insert_sqlite_group_models(&mut transaction).await?;
    transaction.commit().await
}

pub async fn rebuild_postgres_group_models(db: &mut PgConnection) -> Result<(), sqlx::Error> {
    let mut transaction = db.begin().await?;
    transaction
        .execute("DELETE FROM marketplace_group_models")
        .await?;
    insert_postgres_group_models(&mut transaction).await?;
    transaction.commit().await
}

pub(crate) async fn insert_sqlite_group_models(
    connection: &mut SqliteConnection,
) -> Result<(), sqlx::Error> {
    connection
        .execute(
            "INSERT INTO marketplace_group_models \
             SELECT g.id, g.sort_order, g.public_name, pm.model_name, pm.model_name_key, pm.model_search_key \
             FROM monoize_provider_models pm \
             JOIN monoize_providers p ON p.id = pm.provider_id \
             JOIN monoize_groups g ON g.id = p.group_id \
             WHERE p.enabled = 1 AND p.channel_enabled = 1 \
               AND EXISTS (SELECT 1 FROM billing_rate_records br \
                           WHERE br.model_name = pm.model_name AND br.public_repeat_count > 0) \
             GROUP BY g.id, g.sort_order, g.public_name, pm.model_name, pm.model_name_key, pm.model_search_key",
        )
        .await?;
    Ok(())
}

pub(crate) async fn insert_postgres_group_models(
    connection: &mut PgConnection,
) -> Result<(), sqlx::Error> {
    connection
        .execute(
            "INSERT INTO marketplace_group_models \
             SELECT g.id, g.sort_order, g.public_name, pm.model_name, pm.model_name_key, pm.model_search_key \
             FROM monoize_provider_models pm \
             JOIN monoize_providers p ON p.id = pm.provider_id \
             JOIN monoize_groups g ON g.id = p.group_id \
             WHERE p.enabled = 1 AND p.channel_enabled = 1 \
               AND EXISTS (SELECT 1 FROM billing_rate_records br \
                           WHERE br.model_name = pm.model_name AND br.public_repeat_count > 0) \
             GROUP BY g.id, g.sort_order, g.public_name, pm.model_name, pm.model_name_key, pm.model_search_key",
        )
        .await?;
    Ok(())
}

fn offer_after(after: Option<OfferKey>) -> (i32, Vec<u8>, Vec<u8>, i64) {
    after.map_or((0, Vec::new(), Vec::new(), 0), |key| {
        (
            key.priority,
            key.provider_public_name.into_bytes(),
            key.channel_public_name.into_bytes(),
            1,
        )
    })
}

fn decode_list_candidates<DB>(rows: Vec<DB>) -> Result<Vec<ListCandidate>, sqlx::Error>
where
    DB: Row,
    for<'a> &'a str: sqlx::ColumnIndex<DB>,
    i64: for<'r> sqlx::Decode<'r, DB::Database> + sqlx::Type<DB::Database>,
    String: for<'r> sqlx::Decode<'r, DB::Database> + sqlx::Type<DB::Database>,
    Vec<u8>: for<'r> sqlx::Decode<'r, DB::Database> + sqlx::Type<DB::Database>,
{
    rows.into_iter()
        .map(|row| {
            Ok(ListCandidate {
                group_id: row.try_get("group_id")?,
                group_ordinal: row.try_get("sort_order")?,
                group_name: row.try_get("group_name")?,
                model: row.try_get("model_name")?,
                model_name_key: row.try_get("model_name_key")?,
            })
        })
        .collect()
}

fn empty_list_page() -> ListPage {
    ListPage {
        items: Vec::new(),
        next_key: None,
        statement_count: 1,
    }
}

async fn count_sqlite_offers(
    db: &mut SqliteConnection,
    candidates: &[ListCandidate],
) -> Result<BTreeMap<(String, Vec<u8>), u64>, sqlx::Error> {
    let mut query = QueryBuilder::<Sqlite>::new("WITH selected(group_id, model_name_key) AS (");
    query.push_values(candidates, |mut values, candidate| {
        values
            .push_bind(candidate.group_id.clone())
            .push_bind(candidate.model_name_key.clone());
    });
    query.push(
        ") SELECT selected.group_id, selected.model_name_key, COUNT(*) AS offer_count \
         FROM selected \
         JOIN monoize_providers p \
           ON p.group_id = selected.group_id AND p.enabled = 1 AND p.channel_enabled = 1 \
         JOIN monoize_provider_models pm \
           ON pm.provider_id = p.id AND pm.model_name_key = selected.model_name_key \
         WHERE EXISTS (SELECT 1 FROM billing_rate_records br \
                       WHERE br.model_name = pm.model_name AND br.public_repeat_count > 0) \
         GROUP BY selected.group_id, selected.model_name_key",
    );
    decode_offer_counts(query.build().fetch_all(db).await?)
}

async fn count_postgres_offers(
    db: &mut PgConnection,
    candidates: &[ListCandidate],
) -> Result<BTreeMap<(String, Vec<u8>), u64>, sqlx::Error> {
    let mut query =
        QueryBuilder::<sqlx::Postgres>::new("WITH selected(group_id, model_name_key) AS (");
    query.push_values(candidates, |mut values, candidate| {
        values
            .push_bind(candidate.group_id.clone())
            .push_bind(candidate.model_name_key.clone());
    });
    query.push(
        ") SELECT selected.group_id, selected.model_name_key, COUNT(*) AS offer_count \
         FROM selected \
         JOIN monoize_providers p \
           ON p.group_id = selected.group_id AND p.enabled = 1 AND p.channel_enabled = 1 \
         JOIN monoize_provider_models pm \
           ON pm.provider_id = p.id AND pm.model_name_key = selected.model_name_key \
         WHERE EXISTS (SELECT 1 FROM billing_rate_records br \
                       WHERE br.model_name = pm.model_name AND br.public_repeat_count > 0) \
         GROUP BY selected.group_id, selected.model_name_key",
    );
    decode_offer_counts(query.build().fetch_all(db).await?)
}

fn decode_offer_counts<DB>(rows: Vec<DB>) -> Result<BTreeMap<(String, Vec<u8>), u64>, sqlx::Error>
where
    DB: Row,
    for<'a> &'a str: sqlx::ColumnIndex<DB>,
    i64: for<'r> sqlx::Decode<'r, DB::Database> + sqlx::Type<DB::Database>,
    String: for<'r> sqlx::Decode<'r, DB::Database> + sqlx::Type<DB::Database>,
    Vec<u8>: for<'r> sqlx::Decode<'r, DB::Database> + sqlx::Type<DB::Database>,
{
    rows.into_iter()
        .map(|row| {
            let count = u64::try_from(row.try_get::<i64, _>("offer_count")?)
                .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
            Ok((
                (row.try_get("group_id")?, row.try_get("model_name_key")?),
                count,
            ))
        })
        .collect()
}

fn build_list_page(
    candidates: Vec<ListCandidate>,
    offer_counts: BTreeMap<(String, Vec<u8>), u64>,
    limit: u16,
    statement_count: u64,
) -> Result<ListPage, sqlx::Error> {
    let has_more = candidates.len() > usize::from(limit);
    let mut items = Vec::with_capacity(candidates.len().min(usize::from(limit)));
    let mut keys = Vec::with_capacity(items.capacity());
    for candidate in candidates.into_iter().take(usize::from(limit)) {
        let count_key = (candidate.group_id, candidate.model_name_key);
        let offer_count = offer_counts
            .get(&count_key)
            .copied()
            .ok_or_else(|| sqlx::Error::Protocol("missing candidate offer count".to_owned()))?;
        keys.push(ListKey {
            group_ordinal: candidate.group_ordinal,
            model_name: candidate.model.clone(),
        });
        items.push(MarketplaceRow {
            group: candidate.group_name,
            model: candidate.model,
            offer_count,
        });
    }
    Ok(ListPage {
        next_key: has_more.then(|| keys.last().expect("non-empty page with more rows").clone()),
        items,
        statement_count,
    })
}

fn decode_offer_rows<DB>(rows: Vec<DB>, limit: u16) -> Result<OfferPage, sqlx::Error>
where
    DB: Row,
    for<'a> &'a str: sqlx::ColumnIndex<DB>,
    i32: for<'r> sqlx::Decode<'r, DB::Database> + sqlx::Type<DB::Database>,
    String: for<'r> sqlx::Decode<'r, DB::Database> + sqlx::Type<DB::Database>,
{
    let has_more = rows.len() > usize::from(limit);
    let mut items = Vec::with_capacity(rows.len().min(usize::from(limit)));
    let mut keys = Vec::with_capacity(items.capacity());
    for row in rows.into_iter().take(usize::from(limit)) {
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
        next_key: has_more.then(|| keys.last().expect("non-empty offers").clone()),
        items,
    })
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
