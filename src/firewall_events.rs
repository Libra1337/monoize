//! Persistence and aggregation for content-firewall rejections
//! (`spec/content-firewall.spec.md` §6–§7).

use crate::db::DbPool;
use crate::entity::firewall_events;
use chrono::Utc;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Set, Statement};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

/// CF-23: hard ceiling on retained rows; the trim runs after every insert.
const RETAINED_ROW_CAP: i64 = 50_000;

pub struct NewFirewallEvent {
    pub user_id: Option<String>,
    pub username: Option<String>,
    pub api_key_id: Option<String>,
    pub api_key_name: Option<String>,
    pub endpoint: String,
    pub model: String,
    pub term: String,
    pub content: String,
    /// CF-20: `blocked` (request rejected) or `marked` (allowed but recorded).
    pub action: &'static str,
    /// CF-20: the judge's stated evidence; empty when no judge verdict exists.
    pub reason: String,
}

pub const ACTION_BLOCKED: &str = "blocked";
pub const ACTION_MARKED: &str = "marked";

/// CF-21: content is the matched scanned string truncated to at most 8000
/// Unicode scalar values so an adversarial oversized request cannot bloat the
/// event store.
pub fn content_of(text: &str) -> String {
    text.chars().take(8000).collect()
}

/// CF-22: persistence failures are logged by the caller and never change the
/// rejection outcome.
pub async fn record_event(db: &DbPool, event: NewFirewallEvent) -> Result<(), String> {
    let now = Utc::now();
    let row = firewall_events::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        user_id: Set(event.user_id),
        username: Set(event.username),
        api_key_id: Set(event.api_key_id),
        api_key_name: Set(event.api_key_name),
        endpoint: Set(event.endpoint),
        model: Set(event.model),
        term: Set(event.term),
        content: Set(event.content),
        action: Set(event.action.to_string()),
        reason: Set(event.reason),
        created_at: Set(now.to_rfc3339()),
        created_at_unix_ms: Set(now.timestamp_millis()),
    };
    let _write_guard = db.write().await;
    firewall_events::Entity::insert(row)
        .exec(&*_write_guard)
        .await
        .map_err(|e| e.to_string())?;
    // CF-23: keep only the newest RETAINED_ROW_CAP rows. LIMIT inside the
    // subquery is valid on both SQLite and PostgreSQL.
    let trim_sql = format!(
        "DELETE FROM firewall_events WHERE id NOT IN \
         (SELECT id FROM firewall_events ORDER BY created_at_unix_ms DESC LIMIT {RETAINED_ROW_CAP})"
    );
    ConnectionTrait::execute(&*_write_guard, Statement::from_string(db.backend(), trim_sql))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub struct FirewallEventFilter {
    pub term: Option<String>,
    pub action: Option<&'static str>,
    pub since_ms: Option<i64>,
    pub until_ms: Option<i64>,
}

fn filtered_events(filter: &FirewallEventFilter) -> sea_orm::Select<firewall_events::Entity> {
    let mut query = firewall_events::Entity::find();
    if let Some(term) = filter.term.as_deref().filter(|t| !t.is_empty()) {
        query = query.filter(firewall_events::Column::Term.contains(term));
    }
    if let Some(action) = filter.action {
        query = query.filter(firewall_events::Column::Action.eq(action));
    }
    if let Some(since_ms) = filter.since_ms {
        query = query.filter(firewall_events::Column::CreatedAtUnixMs.gte(since_ms));
    }
    if let Some(until_ms) = filter.until_ms {
        query = query.filter(firewall_events::Column::CreatedAtUnixMs.lte(until_ms));
    }
    query
}

pub async fn list_events(
    db: &DbPool,
    filter: &FirewallEventFilter,
    limit: u64,
    offset: u64,
) -> Result<(Vec<firewall_events::Model>, u64), String> {
    let read = db.read();
    let total = filtered_events(filter)
        .count(read)
        .await
        .map_err(|e| e.to_string())?;
    let rows = filtered_events(filter)
        .order_by_desc(firewall_events::Column::CreatedAtUnixMs)
        .offset(offset)
        .limit(limit)
        .all(read)
        .await
        .map_err(|e| e.to_string())?;
    Ok((rows, total))
}

#[derive(Debug, Serialize)]
pub struct FirewallStats {
    /// CF-24: blocked rows only; marked rows are reported separately.
    pub total: u64,
    pub marked: u64,
    pub last_24h: u64,
    pub last_7d: u64,
    pub distinct_users: u64,
    /// CF-24: exactly 14 UTC-day buckets, oldest first, ending with the
    /// current UTC day.
    pub daily: Vec<DailyCount>,
    pub top_terms: Vec<TermCount>,
}

#[derive(Debug, Serialize)]
pub struct DailyCount {
    pub date: String,
    pub count: u64,
}

#[derive(Debug, Serialize)]
pub struct TermCount {
    pub term: String,
    pub count: u64,
}

/// CF-24: aggregation over retained rows in Rust. The row cap bounds the scan,
/// so no SQL-side date dialect handling is needed.
pub async fn compute_stats(db: &DbPool) -> Result<FirewallStats, String> {
    let read = db.read();
    let rows = firewall_events::Entity::find()
        .all(read)
        .await
        .map_err(|e| e.to_string())?;

    let now_ms = Utc::now().timestamp_millis();
    let day_ms: i64 = 24 * 60 * 60 * 1000;
    let today_start = Utc::now().date_naive();
    let mut day_counts: BTreeMap<chrono::NaiveDate, u64> = (0..14)
        .map(|offset| (today_start - chrono::Days::new(offset), 0))
        .collect();

    let mut term_counts: HashMap<String, u64> = HashMap::new();
    let mut users: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut last_24h = 0_u64;
    let mut last_7d = 0_u64;
    let mut blocked = 0_u64;
    let mut marked = 0_u64;

    for row in &rows {
        if row.action == ACTION_MARKED {
            marked += 1;
            // CF-24: aggregate the marked count only; every other statistic
            // describes blocked requests.
            continue;
        }
        blocked += 1;
        if now_ms - row.created_at_unix_ms <= day_ms {
            last_24h += 1;
        }
        if now_ms - row.created_at_unix_ms <= 7 * day_ms {
            last_7d += 1;
        }
        if let Some(user_id) = row.user_id.as_deref() {
            users.insert(user_id);
        }
        *term_counts.entry(row.term.clone()).or_default() += 1;
        if let Some(ts) = chrono::DateTime::from_timestamp_millis(row.created_at_unix_ms)
            && let Some(count) = day_counts.get_mut(&ts.date_naive())
        {
            *count += 1;
        }
    }

    let mut top_terms: Vec<TermCount> = term_counts
        .into_iter()
        .map(|(term, count)| TermCount { term, count })
        .collect();
    top_terms.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.term.cmp(&b.term)));
    top_terms.truncate(10);

    Ok(FirewallStats {
        total: blocked,
        marked,
        last_24h,
        last_7d,
        distinct_users: users.len() as u64,
        daily: day_counts
            .into_iter()
            .map(|(date, count)| DailyCount {
                date: date.format("%Y-%m-%d").to_string(),
                count,
            })
            .collect(),
        top_terms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_truncates_to_8000_scalar_values() {
        let long: String = "a".repeat(9000);
        assert_eq!(content_of(&long).chars().count(), 8000);
        let multi_byte: String = "字".repeat(9000);
        let content = content_of(&multi_byte);
        assert_eq!(content.chars().count(), 8000);
        assert!(content.chars().all(|c| c == '字'));
        assert_eq!(content_of("短"), "短");
    }
}
