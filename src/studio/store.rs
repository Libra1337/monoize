//! ST-D1/D2: raw-SQL persistence for studio projects, runs, steps, and assets.

use crate::db::DbPool;
use sea_orm::ConnectionTrait;
use serde_json::Value;

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProjectRow {
    pub id: String,
    pub user_id: String,
    pub title: String,
    pub graph_json: String,
    pub version: i64,
    pub template_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RunRow {
    pub id: String,
    pub user_id: String,
    pub project_id: Option<String>,
    pub api_key_id: Option<String>,
    pub kind: String,
    pub status: String,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StepRow {
    pub id: String,
    pub run_id: String,
    pub kind: String,
    pub status: String,
    pub node_id: Option<String>,
    pub payload_json: String,
    pub result_json: Option<String>,
    pub error: Option<String>,
    pub attempts: i64,
    pub charge_nano_usd: i64,
    pub refund_nano_usd: i64,
    pub created_at: String,
    pub updated_at: String,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AssetRow {
    pub id: String,
    pub user_id: String,
    pub run_id: String,
    pub step_id: String,
    pub kind: String,
    pub upstream_kind: String,
    pub provider_id: Option<String>,
    pub remote_ref_json: String,
    pub mime_type: String,
    pub bytes: Option<i64>,
    pub status: String,
    pub created_at: String,
}

fn get_opt_string(row: &sea_orm::QueryResult, col: &str) -> Option<String> {
    row.try_get::<String>("", col).ok()
}

pub async fn create_project(
    db: &DbPool,
    id: &str,
    user_id: &str,
    title: &str,
    graph_json: &str,
    template_id: Option<&str>,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO studio_projects (id, user_id, title, graph_json, version, template_id, created_at, updated_at)
             VALUES ($1, $2, $3, $4, 1, $5, $6, $6)",
            vec![
                id.into(),
                user_id.into(),
                title.into(),
                graph_json.into(),
                template_id.map(sea_orm::Value::from).unwrap_or(sea_orm::Value::String(None)),
                now.into(),
            ],
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn list_projects(db: &DbPool, user_id: &str) -> Result<Vec<ProjectRow>, String> {
    let rows = db
        .read()
        .query_all(db.stmt(
            "SELECT id, user_id, title, graph_json, version, template_id, created_at, updated_at
             FROM studio_projects WHERE user_id = $1 ORDER BY updated_at DESC LIMIT 200",
            vec![user_id.into()],
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows.iter().map(project_from_row).collect())
}

pub async fn get_project(
    db: &DbPool,
    user_id: &str,
    id: &str,
) -> Result<Option<ProjectRow>, String> {
    let rows = db
        .read()
        .query_all(db.stmt(
            "SELECT id, user_id, title, graph_json, version, template_id, created_at, updated_at
             FROM studio_projects WHERE id = $1 AND user_id = $2",
            vec![id.into(), user_id.into()],
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows.first().map(project_from_row))
}

#[derive(Debug)]
pub enum SaveGraphOutcome {
    Saved,
    Stale { current_version: i64 },
    Missing,
}

pub async fn save_graph(
    db: &DbPool,
    user_id: &str,
    id: &str,
    base_version: i64,
    graph_json: &str,
) -> Result<SaveGraphOutcome, String> {
    let now = chrono::Utc::now().to_rfc3339();
    let result = db
        .write()
        .await
        .execute(db.stmt(
            "UPDATE studio_projects SET graph_json = $1, version = version + 1, updated_at = $2
             WHERE id = $3 AND user_id = $4 AND version = $5",
            vec![
                graph_json.into(),
                now.into(),
                id.into(),
                user_id.into(),
                base_version.into(),
            ],
        ))
        .await
        .map_err(|e| e.to_string())?;
    if result.rows_affected() == 1 {
        return Ok(SaveGraphOutcome::Saved);
    }
    match get_project(db, user_id, id).await? {
        Some(project) => Ok(SaveGraphOutcome::Stale {
            current_version: project.version,
        }),
        None => Ok(SaveGraphOutcome::Missing),
    }
}

/// Engine-side graph mutation: bumps version and persists unconditionally
/// (server mutations are authoritative; the client reconciles via SSE patch).
pub async fn engine_save_graph(db: &DbPool, id: &str, graph_json: &str) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    db.write()
        .await
        .execute(db.stmt(
            "UPDATE studio_projects SET graph_json = $1, version = version + 1, updated_at = $2 WHERE id = $3",
            vec![graph_json.into(), now.into(), id.into()],
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn delete_project(db: &DbPool, user_id: &str, id: &str) -> Result<bool, String> {
    let result = db
        .write()
        .await
        .execute(db.stmt(
            "DELETE FROM studio_projects WHERE id = $1 AND user_id = $2",
            vec![id.into(), user_id.into()],
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(result.rows_affected() == 1)
}

fn project_from_row(row: &sea_orm::QueryResult) -> ProjectRow {
    ProjectRow {
        id: row.try_get("", "id").unwrap_or_default(),
        user_id: row.try_get("", "user_id").unwrap_or_default(),
        title: row.try_get("", "title").unwrap_or_default(),
        graph_json: row.try_get("", "graph_json").unwrap_or_default(),
        version: row.try_get("", "version").unwrap_or(1),
        template_id: get_opt_string(row, "template_id"),
        created_at: row.try_get("", "created_at").unwrap_or_default(),
        updated_at: row.try_get("", "updated_at").unwrap_or_default(),
    }
}

pub async fn create_run(
    db: &DbPool,
    id: &str,
    user_id: &str,
    project_id: Option<&str>,
    api_key_id: Option<&str>,
    kind: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO studio_runs (id, user_id, project_id, api_key_id, kind, status, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, 'queued', $6, $6)",
            vec![
                id.into(),
                user_id.into(),
                project_id.map(sea_orm::Value::from).unwrap_or(sea_orm::Value::String(None)),
                api_key_id.map(sea_orm::Value::from).unwrap_or(sea_orm::Value::String(None)),
                kind.into(),
                now.into(),
            ],
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn run_from_row(row: &sea_orm::QueryResult) -> RunRow {
    RunRow {
        id: row.try_get("", "id").unwrap_or_default(),
        user_id: row.try_get("", "user_id").unwrap_or_default(),
        project_id: get_opt_string(row, "project_id"),
        api_key_id: get_opt_string(row, "api_key_id"),
        kind: row.try_get("", "kind").unwrap_or_default(),
        status: row.try_get("", "status").unwrap_or_default(),
        error: get_opt_string(row, "error"),
        created_at: row.try_get("", "created_at").unwrap_or_default(),
        updated_at: row.try_get("", "updated_at").unwrap_or_default(),
        finished_at: get_opt_string(row, "finished_at"),
    }
}

pub async fn get_run(db: &DbPool, id: &str) -> Result<Option<RunRow>, String> {
    let rows = db
        .read()
        .query_all(db.stmt(
            "SELECT id, user_id, project_id, api_key_id, kind, status, error, created_at, updated_at, finished_at
             FROM studio_runs WHERE id = $1",
            vec![id.into()],
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows.first().map(run_from_row))
}

pub async fn list_runs(
    db: &DbPool,
    user_id: Option<&str>,
    limit: u64,
) -> Result<Vec<RunRow>, String> {
    let (sql, params) = match user_id {
        Some(user_id) => (
            "SELECT id, user_id, project_id, api_key_id, kind, status, error, created_at, updated_at, finished_at
             FROM studio_runs WHERE user_id = $1 ORDER BY created_at DESC LIMIT $2".to_string(),
            vec![user_id.into(), (limit as i64).into()],
        ),
        None => (
            "SELECT id, user_id, project_id, api_key_id, kind, status, error, created_at, updated_at, finished_at
             FROM studio_runs ORDER BY created_at DESC LIMIT $1".to_string(),
            vec![(limit as i64).into()],
        ),
    };
    let rows = db
        .read()
        .query_all(db.stmt(&sql, params))
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows.iter().map(run_from_row).collect())
}

pub async fn active_run_counts(db: &DbPool, user_id: Option<&str>) -> Result<i64, String> {
    let (sql, params) = match user_id {
        Some(user_id) => (
            "SELECT COUNT(*) AS n FROM studio_runs WHERE user_id = $1 AND status IN ('queued','running')"
                .to_string(),
            vec![user_id.into()],
        ),
        None => (
            "SELECT COUNT(*) AS n FROM studio_runs WHERE status IN ('queued','running')".to_string(),
            vec![],
        ),
    };
    let rows = db
        .read()
        .query_one(db.stmt(&sql, params))
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows
        .and_then(|row| row.try_get::<i64>("", "n").ok())
        .unwrap_or(0))
}

pub async fn update_run_status(
    db: &DbPool,
    id: &str,
    status: &str,
    error: Option<&str>,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    let finished = matches!(status, "succeeded" | "failed" | "canceled" | "partial");
    db.write()
        .await
        .execute(db.stmt(
            "UPDATE studio_runs SET status = $1, error = $2, updated_at = $3, finished_at = CASE WHEN $4 THEN $3 ELSE finished_at END WHERE id = $5",
            vec![
                status.into(),
                error.map(sea_orm::Value::from).unwrap_or(sea_orm::Value::String(None)),
                now.into(),
                (if finished { 1 } else { 0 }).into(),
                id.into(),
            ],
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn create_step(
    db: &DbPool,
    id: &str,
    run_id: &str,
    kind: &str,
    node_id: Option<&str>,
    payload_json: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO studio_steps (id, run_id, kind, status, node_id, payload_json, attempts, charge_nano_usd, refund_nano_usd, created_at, updated_at)
             VALUES ($1, $2, $3, 'pending', $4, $5, 0, 0, 0, $6, $6)",
            vec![
                id.into(),
                run_id.into(),
                kind.into(),
                node_id.map(sea_orm::Value::from).unwrap_or(sea_orm::Value::String(None)),
                payload_json.into(),
                now.into(),
            ],
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn step_from_row(row: &sea_orm::QueryResult) -> StepRow {
    StepRow {
        id: row.try_get("", "id").unwrap_or_default(),
        run_id: row.try_get("", "run_id").unwrap_or_default(),
        kind: row.try_get("", "kind").unwrap_or_default(),
        status: row.try_get("", "status").unwrap_or_default(),
        node_id: get_opt_string(row, "node_id"),
        payload_json: row.try_get("", "payload_json").unwrap_or_default(),
        result_json: get_opt_string(row, "result_json"),
        error: get_opt_string(row, "error"),
        attempts: row.try_get("", "attempts").unwrap_or(0),
        charge_nano_usd: row.try_get("", "charge_nano_usd").unwrap_or(0),
        refund_nano_usd: row.try_get("", "refund_nano_usd").unwrap_or(0),
        created_at: row.try_get("", "created_at").unwrap_or_default(),
        updated_at: row.try_get("", "updated_at").unwrap_or_default(),
        finished_at: get_opt_string(row, "finished_at"),
    }
}

const STEP_COLUMNS: &str = "id, run_id, kind, status, node_id, payload_json, result_json, error, attempts, charge_nano_usd, refund_nano_usd, created_at, updated_at, finished_at";

pub async fn get_step(db: &DbPool, id: &str) -> Result<Option<StepRow>, String> {
    let rows = db
        .read()
        .query_all(db.stmt(
            &format!("SELECT {STEP_COLUMNS} FROM studio_steps WHERE id = $1"),
            vec![id.into()],
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows.first().map(step_from_row))
}

pub async fn list_steps_for_run(db: &DbPool, run_id: &str) -> Result<Vec<StepRow>, String> {
    let rows = db
        .read()
        .query_all(db.stmt(
            &format!(
                "SELECT {STEP_COLUMNS} FROM studio_steps WHERE run_id = $1 ORDER BY created_at ASC"
            ),
            vec![run_id.into()],
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows.iter().map(step_from_row).collect())
}

pub async fn list_steps_in_status(
    db: &DbPool,
    status: &str,
    limit: u64,
) -> Result<Vec<StepRow>, String> {
    let rows = db
        .read()
        .query_all(db.stmt(
            &format!(
                "SELECT {STEP_COLUMNS} FROM studio_steps WHERE status = $1 ORDER BY created_at ASC LIMIT $2"
            ),
            vec![status.into(), (limit as i64).into()],
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows.iter().map(step_from_row).collect())
}

/// Returns true when the transition happened (ST-S2 guards the legal set).
pub async fn transition_step(
    db: &DbPool,
    id: &str,
    from: &[&str],
    to: &str,
    error: Option<&str>,
    result_json: Option<&str>,
) -> Result<bool, String> {
    let from_list = from
        .iter()
        .map(|f| format!("'{f}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let terminal = matches!(to, "succeeded" | "failed" | "canceled" | "skipped");
    let now = chrono::Utc::now().to_rfc3339();
    let sets = match result_json {
        Some(_) => "result_json = COALESCE($3, result_json), ".to_string(),
        None => String::new(),
    };
    let sql = format!(
        "UPDATE studio_steps SET status = $1, error = $2, {sets}attempts = attempts + 1, updated_at = $4, finished_at = CASE WHEN $5 THEN $4 ELSE finished_at END
         WHERE id = $6 AND status IN ({from_list})"
    );
    let result = db
        .write()
        .await
        .execute(db.stmt(
            &sql,
            vec![
                to.into(),
                error.map(sea_orm::Value::from).unwrap_or(sea_orm::Value::String(None)),
                result_json
                    .map(sea_orm::Value::from)
                    .unwrap_or(sea_orm::Value::String(None)),
                now.into(),
                (if terminal { 1 } else { 0 }).into(),
                id.into(),
            ],
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(result.rows_affected() == 1)
}

pub async fn add_step_charge(db: &DbPool, id: &str, charge_nano: i64) -> Result<(), String> {
    db.write()
        .await
        .execute(db.stmt(
            "UPDATE studio_steps SET charge_nano_usd = charge_nano_usd + $1 WHERE id = $2",
            vec![charge_nano.into(), id.into()],
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn mark_step_refunded(db: &DbPool, id: &str, refund_nano: i64) -> Result<(), String> {
    db.write()
        .await
        .execute(db.stmt(
            "UPDATE studio_steps SET refund_nano_usd = refund_nano_usd + $1 WHERE id = $2",
            vec![refund_nano.into(), id.into()],
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn insert_asset(
    db: &DbPool,
    id: &str,
    user_id: &str,
    run_id: &str,
    step_id: &str,
    kind: &str,
    upstream_kind: &str,
    provider_id: Option<&str>,
    remote_ref_json: &str,
    mime_type: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO studio_assets (id, user_id, run_id, step_id, kind, upstream_kind, provider_id, remote_ref_json, mime_type, status, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'ready', $10)",
            vec![
                id.into(),
                user_id.into(),
                run_id.into(),
                step_id.into(),
                kind.into(),
                upstream_kind.into(),
                provider_id
                    .map(sea_orm::Value::from)
                    .unwrap_or(sea_orm::Value::String(None)),
                remote_ref_json.into(),
                mime_type.into(),
                now.into(),
            ],
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn asset_from_row(row: &sea_orm::QueryResult) -> AssetRow {
    AssetRow {
        id: row.try_get("", "id").unwrap_or_default(),
        user_id: row.try_get("", "user_id").unwrap_or_default(),
        run_id: row.try_get("", "run_id").unwrap_or_default(),
        step_id: row.try_get("", "step_id").unwrap_or_default(),
        kind: row.try_get("", "kind").unwrap_or_default(),
        upstream_kind: row.try_get("", "upstream_kind").unwrap_or_default(),
        provider_id: get_opt_string(row, "provider_id"),
        remote_ref_json: row.try_get("", "remote_ref_json").unwrap_or_default(),
        mime_type: row.try_get("", "mime_type").unwrap_or_default(),
        bytes: row.try_get("", "bytes").ok(),
        status: row.try_get("", "status").unwrap_or_default(),
        created_at: row.try_get("", "created_at").unwrap_or_default(),
    }
}

const ASSET_COLUMNS: &str = "id, user_id, run_id, step_id, kind, upstream_kind, provider_id, remote_ref_json, mime_type, bytes, status, created_at";

pub async fn list_assets(
    db: &DbPool,
    user_id: Option<&str>,
    kind: Option<&str>,
    limit: u64,
) -> Result<Vec<AssetRow>, String> {
    let mut sql = format!("SELECT {ASSET_COLUMNS} FROM studio_assets WHERE 1=1");
    let mut params: Vec<sea_orm::Value> = Vec::new();
    if let Some(user_id) = user_id {
        params.push(user_id.into());
        sql.push_str(&format!(" AND user_id = ${}", params.len()));
    }
    if let Some(kind) = kind {
        params.push(kind.into());
        sql.push_str(&format!(" AND kind = ${}", params.len()));
    }
    params.push((limit as i64).into());
    sql.push_str(&format!(
        " ORDER BY created_at DESC LIMIT ${}",
        params.len()
    ));
    let rows = db
        .read()
        .query_all(db.stmt(&sql, params))
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows.iter().map(asset_from_row).collect())
}

pub async fn get_asset(db: &DbPool, id: &str) -> Result<Option<AssetRow>, String> {
    let rows = db
        .read()
        .query_all(db.stmt(
            &format!("SELECT {ASSET_COLUMNS} FROM studio_assets WHERE id = $1"),
            vec![id.into()],
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows.first().map(asset_from_row))
}

/// Graph helpers: mutate a node's data object and bump the project version.
pub fn graph_mutate_node(graph: &mut Value, node_id: &str, data_patch: &Value) -> bool {
    let Some(nodes) = graph.get_mut("nodes").and_then(Value::as_array_mut) else {
        return false;
    };
    for node in nodes.iter_mut() {
        if node.get("id").and_then(Value::as_str) == Some(node_id) {
            let data = node.as_object_mut().map(|obj| {
                obj.entry("data")
                    .or_insert(Value::Object(Default::default()))
            });
            if let Some(data) = data {
                if let (Some(target), Some(patch)) = (data.as_object_mut(), data_patch.as_object())
                {
                    for (key, value) in patch {
                        target.insert(key.clone(), value.clone());
                    }
                    return true;
                }
            }
        }
    }
    false
}

pub fn graph_next_node_position(graph: &Value, seed: usize) -> (f64, f64) {
    let count = graph
        .get("nodes")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let column = (seed % 4) as f64;
    let row = (seed / 4) as f64;
    (
        80.0 + column * 320.0,
        80.0 + row * 300.0 + count as f64 * 10.0,
    )
}
