use crate::app::AppState;
use crate::beijing_time::{beijing_day_id, beijing_day_shift, is_valid_beijing_day_id};
use crate::dashboard_handlers::session_helpers::require_admin;
use crate::error::{AppError, AppResult};
use crate::users::{
    RevenueDayRow, RevenueExclusionRow, aggregate_revenue_day, list_persisted_revenue_days,
    list_revenue_exclusions, recompute_persisted_days,
};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use chrono::Utc;
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};

#[derive(Debug, serde::Deserialize)]
pub struct RevenueDailyQuery {
    pub from: Option<String>,
    pub to: Option<String>,
}

const REVENUE_RANGE_MAX_DAYS: i64 = 366;

/// Resolve and validate the `from`/`to` day-id range per AR-10.
fn resolve_revenue_range(query: &RevenueDailyQuery, today: &str) -> AppResult<(String, String)> {
    let default_from =
        beijing_day_shift(today, -30).ok_or_else(|| invalid_range("day arithmetic failed"))?;
    let from = query.from.clone().unwrap_or(default_from);
    let to = query.to.clone().unwrap_or_else(|| today.to_string());
    if !is_valid_beijing_day_id(&from) || !is_valid_beijing_day_id(&to) {
        return Err(invalid_range("from and to must be YYYY-MM-DD day ids"));
    }
    if from > to {
        return Err(invalid_range("from must not be after to"));
    }
    let distance = crate::beijing_time::beijing_day_distance(&from, &to)
        .ok_or_else(|| invalid_range("invalid day range"))?;
    if distance + 1 > REVENUE_RANGE_MAX_DAYS {
        return Err(invalid_range("range must span at most 366 days"));
    }
    Ok((from, to))
}

fn invalid_range(message: &str) -> AppError {
    AppError::new(StatusCode::BAD_REQUEST, "invalid_request", message)
}

fn internal(message: String) -> AppError {
    AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
}

fn render_revenue_day(day: &RevenueDayRow) -> Value {
    json!({
        "day": day.day,
        "total_charge_nano_usd": day.total_charge_nano_usd,
        "total_calls": day.total_calls,
        "total_input_tokens": day.total_input_tokens,
        "total_output_tokens": day.total_output_tokens,
        "models": day.models.iter().map(|row| json!({
            "model": row.model,
            "charge_nano_usd": row.charge_nano_usd,
            "calls": row.calls,
            "input_tokens": row.input_tokens,
            "output_tokens": row.output_tokens,
        })).collect::<Vec<_>>(),
    })
}

/// AR-10: persisted days plus the live current day.
pub async fn get_admin_revenue_daily(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<RevenueDailyQuery>,
) -> AppResult<Json<Value>> {
    require_admin(&headers, &state).await?;
    let today = beijing_day_id(Utc::now());
    let (from, to) = resolve_revenue_range(&query, &today)?;

    let mut days = list_persisted_revenue_days(&state.db_pool, &from, &to)
        .await
        .map_err(internal)?;
    if to >= today && from <= today {
        let excluded: Vec<String> = list_revenue_exclusions(&state.db_pool)
            .await
            .map_err(internal)?
            .into_iter()
            .map(|row| row.user_id)
            .collect();
        if let Some(current) = aggregate_revenue_day(&state.db_pool, &today, &excluded)
            .await
            .map_err(internal)?
        {
            if !days.iter().any(|day| day.day == current.day) {
                days.push(current);
            }
        }
    }
    days.sort_by(|left, right| right.day.cmp(&left.day));

    Ok(Json(json!({
        "from": from,
        "to": to,
        "days": days.iter().map(render_revenue_day).collect::<Vec<_>>(),
    })))
}

/// AR-15: Excel export of the same rows as `get_admin_revenue_daily`.
pub async fn export_admin_revenue_daily(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<RevenueDailyQuery>,
) -> AppResult<Response> {
    require_admin(&headers, &state).await?;
    let today = beijing_day_id(Utc::now());
    let (from, to) = resolve_revenue_range(&query, &today)?;

    let mut days = list_persisted_revenue_days(&state.db_pool, &from, &to)
        .await
        .map_err(internal)?;
    if to >= today && from <= today {
        let excluded: Vec<String> = list_revenue_exclusions(&state.db_pool)
            .await
            .map_err(internal)?
            .into_iter()
            .map(|row| row.user_id)
            .collect();
        if let Some(current) = aggregate_revenue_day(&state.db_pool, &today, &excluded)
            .await
            .map_err(internal)?
        {
            if !days.iter().any(|day| day.day == current.day) {
                days.push(current);
            }
        }
    }
    days.sort_by(|left, right| left.day.cmp(&right.day));

    let mut workbook = build_revenue_workbook(&days)
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;
    let bytes = workbook.save_to_buffer().map_err(|e| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            e.to_string(),
        )
    })?;

    let filename = format!("monoize-revenue-{from}-{to}.xlsx");
    let mut response = Response::new(axum::body::Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static(
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        header::HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .map_err(|_| internal("invalid filename".to_string()))?,
    );
    Ok(response)
}

/// Exact nano-USD to USD with 6 fractional digits, via i128 integer math only.
fn nano_usd_to_usd_string(nano: &str) -> Result<String, String> {
    let value: i128 = nano
        .parse()
        .map_err(|_| format!("invalid charge value: {nano}"))?;
    let sign = if value < 0 { "-" } else { "" };
    let abs = value.unsigned_abs();
    let units = abs / 1_000_000_000;
    let frac = abs % 1_000_000_000;
    Ok(format!("{sign}{units}.{frac:06}"))
}

fn build_revenue_workbook(days: &[RevenueDayRow]) -> Result<rust_xlsxwriter::Workbook, String> {
    use rust_xlsxwriter::{Format, FormatAlign, Workbook};

    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("Revenue").map_err(|e| e.to_string())?;

    let header_format = Format::new().set_bold().set_align(FormatAlign::Center);
    let headers = [
        "Day",
        "Revenue (USD)",
        "Calls",
        "Input Tokens",
        "Output Tokens",
        "Top Model",
        "Top Model Revenue (USD)",
    ];
    for (column, header) in headers.iter().enumerate() {
        worksheet
            .write_with_format(0, column as u16, *header, &header_format)
            .map_err(|e| e.to_string())?;
    }

    for (index, day) in days.iter().enumerate() {
        let row = (index + 1) as u32;
        worksheet
            .write(row, 0, &day.day)
            .map_err(|e| e.to_string())?;
        worksheet
            .write(row, 1, nano_usd_to_usd_string(&day.total_charge_nano_usd)?)
            .map_err(|e| e.to_string())?;
        worksheet
            .write(row, 2, day.total_calls)
            .map_err(|e| e.to_string())?;
        worksheet
            .write(row, 3, day.total_input_tokens)
            .map_err(|e| e.to_string())?;
        worksheet
            .write(row, 4, day.total_output_tokens)
            .map_err(|e| e.to_string())?;
        if let Some(top) = day.models.first() {
            worksheet
                .write(row, 5, &top.model)
                .map_err(|e| e.to_string())?;
            worksheet
                .write(row, 6, nano_usd_to_usd_string(&top.charge_nano_usd)?)
                .map_err(|e| e.to_string())?;
        }
    }
    // autofit returns the worksheet for chaining, not a Result.
    worksheet.autofit();
    Ok(workbook)
}

fn render_exclusion(row: &RevenueExclusionRow) -> Value {
    json!({
        "user_id": row.user_id,
        "username": row.username,
        "created_at": row.created_at,
    })
}

/// AR-11.
pub async fn list_admin_revenue_exclusions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    require_admin(&headers, &state).await?;
    let exclusions = list_revenue_exclusions(&state.db_pool)
        .await
        .map_err(internal)?;
    Ok(Json(json!({
        "exclusions": exclusions.iter().map(render_exclusion).collect::<Vec<_>>(),
    })))
}

#[derive(Debug, serde::Deserialize)]
pub struct AddRevenueExclusionRequest {
    pub user_id: String,
}

/// AR-12.
pub async fn add_admin_revenue_exclusion(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<AddRevenueExclusionRequest>,
) -> AppResult<Json<Value>> {
    require_admin(&headers, &state).await?;
    let user_id = body.user_id.trim().to_string();
    if user_id.is_empty() {
        return Err(invalid_range("user_id must not be empty"));
    }
    let user = state
        .user_store
        .get_user_by_id(&user_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| {
            AppError::new(
                StatusCode::NOT_FOUND,
                "user_not_found",
                "user does not exist",
            )
        })?;

    let existing = list_revenue_exclusions(&state.db_pool)
        .await
        .map_err(internal)?;
    if existing.iter().any(|row| row.user_id == user_id) {
        return Ok(Json(json!({
            "exclusions": existing.iter().map(render_exclusion).collect::<Vec<_>>(),
        })));
    }

    state
        .db_pool
        .write()
        .await
        .execute(state.db_pool.stmt(
            "INSERT INTO admin_revenue_exclusions (user_id, username, created_at) VALUES ($1, $2, $3)",
            vec![
                user_id.clone().into(),
                user.username.clone().into(),
                Utc::now().to_rfc3339().into(),
            ],
        ))
        .await
        .map_err(|e| e.to_string())
        .map_err(internal)?;

    // AR-8: recompute retroactively before returning.
    recompute_persisted_days(&state.db_pool, Utc::now())
        .await
        .map_err(internal)?;

    let exclusions = list_revenue_exclusions(&state.db_pool)
        .await
        .map_err(internal)?;
    Ok(Json(json!({
        "exclusions": exclusions.iter().map(render_exclusion).collect::<Vec<_>>(),
    })))
}

/// AR-13.
pub async fn remove_admin_revenue_exclusion(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> AppResult<Json<Value>> {
    require_admin(&headers, &state).await?;
    let result = state
        .db_pool
        .write()
        .await
        .execute(state.db_pool.stmt(
            "DELETE FROM admin_revenue_exclusions WHERE user_id = $1",
            vec![user_id.clone().into()],
        ))
        .await
        .map_err(|e| e.to_string())
        .map_err(internal)?;
    if result.rows_affected() == 0 {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "exclusion not found",
        ));
    }

    // AR-8: recompute retroactively before returning.
    recompute_persisted_days(&state.db_pool, Utc::now())
        .await
        .map_err(internal)?;

    let exclusions = list_revenue_exclusions(&state.db_pool)
        .await
        .map_err(internal)?;
    Ok(Json(json!({
        "exclusions": exclusions.iter().map(render_exclusion).collect::<Vec<_>>(),
    })))
}
