//! ST-D3: builtin template catalog — seeded at startup; admin price/enable
//! edits survive catalog revisions.

use crate::db::DbPool;
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};

const BUILTIN_TEMPLATES: &str = include_str!("../studio-templates.catalog.json");

pub async fn seed_builtin_templates(db: &DbPool) -> Result<(), String> {
    let catalog: Value = serde_json::from_str(BUILTIN_TEMPLATES).map_err(|e| e.to_string())?;
    let Some(templates) = catalog.get("templates").and_then(Value::as_array) else {
        return Ok(());
    };
    for template in templates {
        let id = template
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        let name = template.get("name").and_then(Value::as_str).unwrap_or(id);
        let description = template
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("");
        let graph = template
            .get("graph")
            .cloned()
            .unwrap_or_else(|| json!({"nodes": [], "edges": []}));
        let params = template.get("params").cloned().unwrap_or(json!({}));
        let prices = template
            .get("price_nano_usd_map")
            .cloned()
            .unwrap_or(json!({}));
        let now = chrono::Utc::now().to_rfc3339();
        // Seed-if-absent; refresh graph/params on revision, keep admin overrides.
        db.write()
            .await
            .execute(db.stmt(
                "INSERT INTO studio_templates (id, source, name, description, graph_json, params_json, price_nano_usd_map, enabled, created_at, updated_at)
                 VALUES ($1, 'builtin', $2, $3, $4, $5, $6, 1, $7, $7)
                 ON CONFLICT (id) DO UPDATE SET
                    graph_json = excluded.graph_json,
                    params_json = excluded.params_json,
                    name = CASE WHEN studio_templates.source = 'builtin' THEN excluded.name ELSE studio_templates.name END,
                    description = CASE WHEN studio_templates.source = 'builtin' THEN excluded.description ELSE studio_templates.description END",
                vec![
                    id.into(),
                    name.into(),
                    description.into(),
                    graph.to_string().into(),
                    params.to_string().into(),
                    prices.to_string().into(),
                    now.into(),
                ],
            ))
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TemplateRow {
    pub id: String,
    pub source: String,
    pub name: String,
    pub description: String,
    pub graph_json: String,
    pub params_json: String,
    pub price_nano_usd_map: String,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

fn template_from_row(row: &sea_orm::QueryResult) -> TemplateRow {
    let get = |col: &str| row.try_get::<String>("", col).unwrap_or_default();
    TemplateRow {
        id: get("id"),
        source: get("source"),
        name: get("name"),
        description: get("description"),
        graph_json: get("graph_json"),
        params_json: get("params_json"),
        price_nano_usd_map: get("price_nano_usd_map"),
        enabled: row
            .try_get::<i64>("", "enabled")
            .map(|v| v != 0)
            .unwrap_or(true),
        created_at: get("created_at"),
        updated_at: get("updated_at"),
    }
}

pub async fn list_templates(db: &DbPool, enabled_only: bool) -> Result<Vec<TemplateRow>, String> {
    let sql = if enabled_only {
        "SELECT id, source, name, description, graph_json, params_json, price_nano_usd_map, enabled, created_at, updated_at
         FROM studio_templates WHERE enabled = 1 ORDER BY source ASC, id ASC"
    } else {
        "SELECT id, source, name, description, graph_json, params_json, price_nano_usd_map, enabled, created_at, updated_at
         FROM studio_templates ORDER BY source ASC, id ASC"
    };
    let rows = db
        .read()
        .query_all(db.stmt(sql, vec![]))
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows.iter().map(template_from_row).collect())
}

pub async fn get_template(db: &DbPool, id: &str) -> Result<Option<TemplateRow>, String> {
    let rows = db
        .read()
        .query_all(db.stmt(
            "SELECT id, source, name, description, graph_json, params_json, price_nano_usd_map, enabled, created_at, updated_at
             FROM studio_templates WHERE id = $1",
            vec![id.into()],
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows.first().map(template_from_row))
}

pub async fn upsert_custom_template(
    db: &DbPool,
    id: &str,
    name: &str,
    description: &str,
    graph_json: &str,
    params_json: &str,
    price_map_json: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO studio_templates (id, source, name, description, graph_json, params_json, price_nano_usd_map, enabled, created_at, updated_at)
             VALUES ($1, 'custom', $2, $3, $4, $5, $6, 1, $7, $7)
             ON CONFLICT (id) DO UPDATE SET name = excluded.name, description = excluded.description,
                graph_json = excluded.graph_json, params_json = excluded.params_json,
                price_nano_usd_map = excluded.price_nano_usd_map, updated_at = excluded.updated_at",
            vec![
                id.into(),
                name.into(),
                description.into(),
                graph_json.into(),
                params_json.into(),
                price_map_json.into(),
                now.into(),
            ],
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn set_template_overrides(
    db: &DbPool,
    id: &str,
    enabled: Option<bool>,
    price_map_json: Option<&str>,
) -> Result<bool, String> {
    let now = chrono::Utc::now().to_rfc3339();
    if let Some(enabled) = enabled {
        let result = db
            .write()
            .await
            .execute(db.stmt(
                "UPDATE studio_templates SET enabled = $1, updated_at = $2 WHERE id = $3",
                vec![
                    (if enabled { 1 } else { 0 }).into(),
                    now.clone().into(),
                    id.into(),
                ],
            ))
            .await
            .map_err(|e| e.to_string())?;
        if result.rows_affected() == 0 {
            return Ok(false);
        }
    }
    if let Some(prices) = price_map_json {
        db.write()
            .await
            .execute(db.stmt(
                "UPDATE studio_templates SET price_nano_usd_map = $1, updated_at = $2 WHERE id = $3",
                vec![prices.into(), now.into(), id.into()],
            ))
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(true)
}

pub async fn delete_custom_template(db: &DbPool, id: &str) -> Result<bool, String> {
    let result = db
        .write()
        .await
        .execute(db.stmt(
            "DELETE FROM studio_templates WHERE id = $1 AND source = 'custom'",
            vec![id.into()],
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(result.rows_affected() == 1)
}
