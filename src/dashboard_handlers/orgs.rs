//! Organization spaces (`orgs.spec.md`).
//!
//! An org is a `users` row with `is_org = 1` (the wallet) plus metadata and membership
//! rows. Keys always belong to and bill their human owner; the org wallet exists for the
//! creation deposit and owner distributions. Sharing a key exposes its material inside the
//! space without changing billing.

use crate::app::AppState;
use crate::dashboard_handlers::session_helpers::get_current_user;
use crate::error::{AppError, AppResult};
use crate::store_billing::money::{Currency, ExchangeRateRational, cny_fen_to_nano_usd};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use chrono::{DateTime, Duration, Utc};
use sea_orm::{ConnectionTrait, Statement, TransactionTrait};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// ORG-21: compile-time limits for this release.
pub const MAX_ORGS_PER_USER: i64 = 2;
pub const MAX_ORG_MEMBERS: i64 = 15;
/// ORG-6: creation deposit, 1000 CNY in fen.
const CREATION_DEPOSIT_MINOR: i128 = 100_000;

fn storage(error: impl std::fmt::Display) -> AppError {
    AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", error.to_string())
}

fn bad_request(message: &str) -> AppError {
    AppError::new(StatusCode::BAD_REQUEST, "invalid_request", message)
}

fn forbidden(message: &str) -> AppError {
    AppError::new(StatusCode::FORBIDDEN, "org_forbidden", message)
}

fn invite_invalid() -> AppError {
    AppError::new(StatusCode::NOT_FOUND, "invite_invalid", "invite link is invalid")
}

fn parse_expiry(raw: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    match raw {
        "24h" => Some(now + Duration::hours(24)),
        "3d" => Some(now + Duration::days(3)),
        "7d" => Some(now + Duration::days(7)),
        "30d" => Some(now + Duration::days(30)),
        "never" => None,
        _ => None,
    }
}

fn expiry_is_valid(raw: &str) -> bool {
    matches!(raw, "24h" | "3d" | "7d" | "30d" | "never")
}

/// ORG-5: only an enterprise-class main account may create an org.
async fn creation_eligible(state: &AppState, user: &crate::users::User) -> AppResult<()> {
    if user.parent_user_id.is_some() {
        return Err(forbidden("sub-accounts cannot create organizations"));
    }
    if user.account_class != crate::users::AccountClass::Enterprise {
        return Err(forbidden("only enterprise accounts can create organizations"));
    }
    let sales = crate::store_billing::sales_store::SalesStore::new(state.db_pool.clone())
        .list_agents()
        .await
        .map_err(storage)?
        .iter()
        .any(|agent| agent.user_id == user.id);
    if sales {
        return Err(forbidden("sales agent accounts cannot create organizations"));
    }
    Ok(())
}

/// Membership check; returns the role.
async fn member_role<C: ConnectionTrait>(
    tx: &C,
    backend: sea_orm::DbBackend,
    org_id: &str,
    user_id: &str,
) -> Result<Option<String>, AppError> {
    let row = tx
        .query_one(Statement::from_sql_and_values(
            backend,
            "SELECT role FROM org_members WHERE org_id = $1 AND user_id = $2",
            [org_id.into(), user_id.into()],
        ))
        .await
        .map_err(storage)?;
    row.map(|row| row.try_get("", "role").map_err(storage))
        .transpose()
}

async fn org_member_count<C: ConnectionTrait>(
    tx: &C,
    backend: sea_orm::DbBackend,
    org_id: &str,
) -> Result<i64, AppError> {
    let row = tx
        .query_one(Statement::from_sql_and_values(
            backend,
            "SELECT COUNT(*) AS value FROM org_members WHERE org_id = $1",
            [org_id.into()],
        ))
        .await
        .map_err(storage)?;
    row.ok_or_else(|| storage("count returned no row"))?
        .try_get::<i64>("", "value")
        .map_err(storage)
}

async fn write_ledger_row<C: ConnectionTrait>(
    tx: &C,
    backend: sea_orm::DbBackend,
    entry_id: &str,
    user_id: &str,
    kind: &str,
    delta: i128,
    balance_after: i128,
    meta: serde_json::Value,
    now: &str,
) -> Result<(), AppError> {
    tx.execute(Statement::from_sql_and_values(
        backend,
        "INSERT INTO billing_ledger
            (id, user_id, kind, delta_nano_usd, balance_after_nano_usd,
             meta_json, created_at, idempotency_key)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        [
            entry_id.into(),
            user_id.into(),
            kind.into(),
            delta.to_string().into(),
            balance_after.to_string().into(),
            meta.to_string().into(),
            now.into(),
            format!("ledger:{entry_id}").into(),
        ],
    ))
    .await
    .map_err(storage)?;
    Ok(())
}

/// One wallet-to-wallet move in a single transaction; both ledger sides written.
async fn move_wallet_balance(
    state: &AppState,
    from_user_id: &str,
    to_user_id: &str,
    amount_nano: i128,
    kind_out: &str,
    kind_in: &str,
    meta_extra: serde_json::Value,
) -> AppResult<(i128, i128)> {
    if amount_nano <= 0 {
        return Err(bad_request("amount must be positive"));
    }
    let backend = state.db_pool.read().get_database_backend();
    let tx = state.db_pool.write().await.begin().await.map_err(storage)?;
    let lock_suffix = if state.db_pool.is_postgres() { " FOR UPDATE" } else { "" };

    let read_balance = |user_id: &str| {
        Statement::from_string(
            backend,
            format!("SELECT balance_nano_usd FROM users WHERE id = '{user_id}'{lock_suffix}"),
        )
    };
    // Locks are always taken from_user first, so concurrent distributions cannot deadlock.
    let from_row = tx
        .query_one(read_balance(from_user_id))
        .await
        .map_err(storage)?
        .ok_or_else(|| bad_request("source wallet not found"))?;
    let from_balance: i128 = from_row
        .try_get::<String>("", "balance_nano_usd")
        .map_err(storage)?
        .parse()
        .map_err(|_| bad_request("invalid persisted balance"))?;
    let to_row = tx
        .query_one(read_balance(to_user_id))
        .await
        .map_err(storage)?
        .ok_or_else(|| bad_request("target wallet not found"))?;
    let to_balance: i128 = to_row
        .try_get::<String>("", "balance_nano_usd")
        .map_err(storage)?
        .parse()
        .map_err(|_| bad_request("invalid persisted balance"))?;

    let from_after = from_balance
        .checked_sub(amount_nano)
        .ok_or_else(|| {
            AppError::new(StatusCode::BAD_REQUEST, "insufficient_balance", "wallet balance is insufficient")
        })?;
    if from_after < 0 {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "insufficient_balance",
            "wallet balance is insufficient",
        ));
    }
    let to_after = to_balance
        .checked_add(amount_nano)
        .ok_or_else(|| bad_request("balance overflow"))?;
    let now = Utc::now().to_rfc3339();

    for (user_id, balance) in [(from_user_id, from_after), (to_user_id, to_after)] {
        tx.execute(Statement::from_sql_and_values(
            backend,
            "UPDATE users SET balance_nano_usd = $2, updated_at = $3 WHERE id = $1",
            [user_id.into(), balance.to_string().into(), now.clone().into()],
        ))
        .await
        .map_err(storage)?;
    }
    write_ledger_row(
        &tx,
        backend,
        &uuid::Uuid::new_v4().to_string(),
        from_user_id,
        kind_out,
        -amount_nano,
        from_after,
        json!({"to_user_id": to_user_id, "extra": meta_extra}),
        &now,
    )
    .await?;
    write_ledger_row(
        &tx,
        backend,
        &uuid::Uuid::new_v4().to_string(),
        to_user_id,
        kind_in,
        amount_nano,
        to_after,
        json!({"from_user_id": from_user_id, "extra": meta_extra}),
        &now,
    )
    .await?;
    tx.commit().await.map_err(storage)?;
    Ok((from_after, to_after))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateOrgRequest {
    pub display_name: String,
    #[serde(default)]
    pub avatar_emoji: Option<String>,
    #[serde(default)]
    pub avatar_color: Option<String>,
    pub invite_expiry: String,
}

#[derive(Debug, Serialize)]
pub struct OrgSummary {
    pub id: String,
    pub display_name: String,
    pub avatar_emoji: String,
    pub avatar_color: String,
    pub role: String,
    pub member_count: i64,
    pub balance_nano_usd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invite_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invite_expires_at: Option<String>,
}

pub async fn create_org(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateOrgRequest>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;
    creation_eligible(&state, &user).await?;

    let name = body.display_name.trim();
    if name.is_empty() || name.len() > 64 {
        return Err(bad_request("display_name must be 1..64 characters"));
    }
    if !expiry_is_valid(&body.invite_expiry) {
        return Err(bad_request("invite_expiry must be one of 24h, 3d, 7d, 30d, never"));
    }
    let avatar_emoji = body.avatar_emoji.unwrap_or_else(|| "🏢".to_string());
    if avatar_emoji.is_empty() || avatar_emoji.len() > 8 {
        return Err(bad_request("avatar_emoji must be 1..8 bytes"));
    }
    let avatar_color = body.avatar_color.unwrap_or_else(|| "#6366f1".to_string());
    if !avatar_color.starts_with('#') || avatar_color.len() != 7 {
        return Err(bad_request("avatar_color must be #rrggbb"));
    }

    let backend = state.db_pool.read().get_database_backend();
    let read = state.db_pool.read();
    let owned = read
        .query_one(Statement::from_sql_and_values(
            backend,
            "SELECT COUNT(*) AS value FROM orgs WHERE owner_user_id = $1",
            [user.id.clone().into()],
        ))
        .await
        .map_err(storage)?
        .ok_or_else(|| storage("count returned no row"))?
        .try_get::<i64>("", "value")
        .map_err(storage)?;
    if owned >= MAX_ORGS_PER_USER {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "org_limit_reached",
            "org creation limit reached",
        ));
    }

    // ORG-6: deposit 1000 CNY converted at the current snapshot.
    let snapshot = state
        .exchange_rate_service
        .current()
        .await
        .map_err(|_| bad_request("exchange rate unavailable; try again later"))?;
    let rate = ExchangeRateRational::parse(&snapshot.cny_per_usd)
        .map_err(|_| bad_request("invalid exchange rate snapshot"))?;
    let deposit_nano = cny_fen_to_nano_usd(CREATION_DEPOSIT_MINOR, &rate)
        .map_err(|_| bad_request("deposit conversion overflow"))?;

    let org_id = uuid::Uuid::new_v4().to_string();
    let invite_token = format!(
        "{}{}",
        crate::store_billing::sales::generate_code(),
        crate::store_billing::sales::generate_code()
    )
    .to_lowercase();
    let invite_expires_at = parse_expiry(&body.invite_expiry, Utc::now());
    let now = Utc::now().to_rfc3339();

    // The wallet row is born with the deposit; the creator side is deducted in the same
    // transaction, so a failure leaves neither row.
    let tx = state.db_pool.write().await.begin().await.map_err(storage)?;
    let lock_suffix = if state.db_pool.is_postgres() { " FOR UPDATE" } else { "" };
    let creator_row = tx
        .query_one(Statement::from_string(
            backend,
            format!("SELECT balance_nano_usd FROM users WHERE id = '{}'{}", user.id, lock_suffix),
        ))
        .await
        .map_err(storage)?
        .ok_or_else(|| bad_request("creator wallet not found"))?;
    let creator_balance: i128 = creator_row
        .try_get::<String>("", "balance_nano_usd")
        .map_err(storage)?
        .parse()
        .map_err(|_| bad_request("invalid persisted balance"))?;
    let creator_after = creator_balance
        .checked_sub(deposit_nano)
        .filter(|value| *value >= 0)
        .ok_or_else(|| {
            AppError::new(
                StatusCode::BAD_REQUEST,
                "insufficient_balance",
                "personal balance is below the 1000 CNY creation deposit",
            )
        })?;

    // First public enterprise group, else the system default group.
    let group = tx
        .query_one(Statement::from_string(
            backend,
            "SELECT id, account_class FROM monoize_groups WHERE account_class = 'enterprise' AND is_public = 1
             ORDER BY sort_order ASC, created_at ASC LIMIT 1"
                .to_string(),
        ))
        .await
        .map_err(storage)?;
    let (group_id, org_class) = match group {
        Some(row) => (
            row.try_get::<String>("", "id").map_err(storage)?,
            crate::users::AccountClass::Enterprise,
        ),
        None => (
            state.user_store.default_group_id().await.map_err(storage)?,
            crate::users::AccountClass::Standard,
        ),
    };

    tx.execute(Statement::from_sql_and_values(
        backend,
        "INSERT INTO users (id, username, password_hash, role, created_at, updated_at, enabled,
                            balance_nano_usd, balance_unlimited, group_id, account_class, is_org)
         VALUES ($1, $1, '', 'user', $2, $2, 1, $3, 0, $4, $5, 1)",
        [
            org_id.clone().into(),
            now.clone().into(),
            deposit_nano.to_string().into(),
            group_id.into(),
            org_class.as_str().into(),
        ],
    ))
    .await
    .map_err(storage)?;
    tx.execute(Statement::from_sql_and_values(
        backend,
        "UPDATE users SET balance_nano_usd = $2, updated_at = $3 WHERE id = $1",
        [user.id.clone().into(), creator_after.to_string().into(), now.clone().into()],
    ))
    .await
    .map_err(storage)?;
    tx.execute(Statement::from_sql_and_values(
        backend,
        "INSERT INTO orgs (id, owner_user_id, display_name, avatar_emoji, avatar_color,
                           invite_token, invite_expires_at, invite_created_at, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8, $8)",
        [
            org_id.clone().into(),
            user.id.clone().into(),
            name.into(),
            avatar_emoji.clone().into(),
            avatar_color.clone().into(),
            invite_token.clone().into(),
            invite_expires_at.map(|t| t.to_rfc3339()).into(),
            now.clone().into(),
        ],
    ))
    .await
    .map_err(storage)?;
    tx.execute(Statement::from_sql_and_values(
        backend,
        "INSERT INTO org_members (org_id, user_id, role, joined_at) VALUES ($1, $2, 'owner', $3)",
        [org_id.clone().into(), user.id.clone().into(), now.clone().into()],
    ))
    .await
    .map_err(storage)?;
    write_ledger_row(
        &tx,
        backend,
        &uuid::Uuid::new_v4().to_string(),
        &user.id,
        "org_deposit",
        -deposit_nano,
        creator_after,
        json!({"org_id": org_id, "reason": "creation"}),
        &now,
    )
    .await?;
    write_ledger_row(
        &tx,
        backend,
        &uuid::Uuid::new_v4().to_string(),
        &org_id,
        "org_deposit_receive",
        deposit_nano,
        deposit_nano,
        json!({"org_id": org_id, "reason": "creation"}),
        &now,
    )
    .await?;
    tx.commit().await.map_err(storage)?;

    Ok((
        StatusCode::CREATED,
        Json(OrgSummary {
            id: org_id,
            display_name: name.to_string(),
            avatar_emoji,
            avatar_color,
            role: "owner".to_string(),
            member_count: 1,
            balance_nano_usd: deposit_nano.to_string(),
            invite_token: Some(invite_token.clone()),
            invite_expires_at: invite_expires_at.map(|t| t.to_rfc3339()),
        }),
    ))
}

pub async fn list_my_orgs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;
    let backend = state.db_pool.read().get_database_backend();
    let rows = state
        .db_pool
        .read()
        .query_all(Statement::from_sql_and_values(
            backend,
            "SELECT o.id, o.display_name, o.avatar_emoji, o.avatar_color, o.invite_token,
                    o.invite_expires_at, m.role AS my_role,
                    (SELECT COUNT(*) FROM org_members mm WHERE mm.org_id = o.id) AS member_count,
                    u.balance_nano_usd
             FROM orgs o
             JOIN org_members m ON m.org_id = o.id AND m.user_id = $1
             JOIN users u ON u.id = o.id
             ORDER BY o.created_at ASC",
            [user.id.clone().into()],
        ))
        .await
        .map_err(storage)?;
    let is_owner = |row: &sea_orm::QueryResult| {
        row.try_get::<String>("", "my_role").map(|role| role == "owner").unwrap_or(false)
    };
    let orgs = rows
        .iter()
        .map(|row| {
            let owner = is_owner(row);
            Ok(OrgSummary {
                id: row.try_get("", "id").map_err(storage)?,
                display_name: row.try_get("", "display_name").map_err(storage)?,
                avatar_emoji: row.try_get("", "avatar_emoji").map_err(storage)?,
                avatar_color: row.try_get("", "avatar_color").map_err(storage)?,
                role: row.try_get("", "my_role").map_err(storage)?,
                member_count: row.try_get("", "member_count").map_err(storage)?,
                balance_nano_usd: row.try_get("", "balance_nano_usd").map_err(storage)?,
                invite_token: owner.then(|| row.try_get("", "invite_token").map_err(storage)).transpose()?,
                invite_expires_at: owner
                    .then(|| row.try_get::<Option<String>>("", "invite_expires_at").map_err(storage))
                    .transpose()?
                    .flatten(),
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    Ok(Json(orgs))
}

#[derive(Debug, Serialize)]
pub struct OrgMemberView {
    pub user_id: String,
    pub username: String,
    pub role: String,
    pub joined_at: String,
}

pub async fn org_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(org_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;
    let backend = state.db_pool.read().get_database_backend();
    let read = state.db_pool.read();
    let role = member_role(&*read, backend, &org_id, &user.id)
        .await?
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "not_found", "org not found"))?;
    let org = read
        .query_one(Statement::from_sql_and_values(
            backend,
            "SELECT display_name, avatar_emoji, avatar_color, invite_token, invite_expires_at,
                    owner_user_id FROM orgs WHERE id = $1",
            [org_id.clone().into()],
        ))
        .await
        .map_err(storage)?
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "not_found", "org not found"))?;
    let members = read
        .query_all(Statement::from_sql_and_values(
            backend,
            "SELECT m.user_id, m.role, m.joined_at, u.username
             FROM org_members m JOIN users u ON u.id = m.user_id
             WHERE m.org_id = $1 ORDER BY m.joined_at ASC",
            [org_id.clone().into()],
        ))
        .await
        .map_err(storage)?;
    let balance = read
        .query_one(Statement::from_sql_and_values(
            backend,
            "SELECT balance_nano_usd FROM users WHERE id = $1",
            [org_id.clone().into()],
        ))
        .await
        .map_err(storage)?
        .ok_or_else(|| storage("org wallet row missing"))?
        .try_get::<String>("", "balance_nano_usd")
        .map_err(storage)?;

    let is_owner = role == "owner";
    Ok(Json(json!({
        "id": org_id,
        "display_name": org.try_get::<String>("", "display_name").map_err(storage)?,
        "avatar_emoji": org.try_get::<String>("", "avatar_emoji").map_err(storage)?,
        "avatar_color": org.try_get::<String>("", "avatar_color").map_err(storage)?,
        "my_role": role,
        "balance_nano_usd": balance,
        "members": members
            .iter()
            .map(|row| OrgMemberView {
                user_id: row.try_get("", "user_id").unwrap_or_default(),
                username: row.try_get("", "username").unwrap_or_default(),
                role: row.try_get("", "role").unwrap_or_default(),
                joined_at: row.try_get("", "joined_at").unwrap_or_default(),
            })
            .collect::<Vec<_>>(),
        "invite": if is_owner {
            json!({
                "token": org.try_get::<String>("", "invite_token").map_err(storage)?,
                "expires_at": org
                    .try_get::<Option<String>>("", "invite_expires_at")
                    .map_err(storage)?
                    .unwrap_or_default(),
            })
        } else {
            serde_json::Value::Null
        },
    })))
}

/// ORG-9: the landing page reads org info without marking anything.
pub async fn invite_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> AppResult<impl IntoResponse> {
    get_current_user(&headers, &state).await?;
    let backend = state.db_pool.read().get_database_backend();
    let read = state.db_pool.read();
    let row = read
        .query_one(Statement::from_sql_and_values(
            backend,
            "SELECT o.id, o.display_name, o.avatar_emoji, o.avatar_color, o.invite_expires_at,
                    u.username AS owner_username,
                    (SELECT COUNT(*) FROM org_members m WHERE m.org_id = o.id) AS member_count
             FROM orgs o JOIN users u ON u.id = o.owner_user_id WHERE o.invite_token = $1",
            [token.into()],
        ))
        .await
        .map_err(storage)?;
    let Some(row) = row else {
        return Err(invite_invalid());
    };
    let expires_at: Option<String> = row.try_get("", "invite_expires_at").map_err(storage)?;
    if let Some(expires) = expires_at.as_deref()
        && DateTime::parse_from_rfc3339(expires)
            .map(|parsed| parsed.with_timezone(&Utc) <= Utc::now())
            .unwrap_or(true)
    {
        return Err(invite_invalid());
    }
    let member_count: i64 = row.try_get("", "member_count").map_err(storage)?;
    if member_count >= MAX_ORG_MEMBERS {
        return Err(invite_invalid());
    }
    Ok(Json(json!({
        "org_id": row.try_get::<String>("", "id").map_err(storage)?,
        "display_name": row.try_get::<String>("", "display_name").map_err(storage)?,
        "avatar_emoji": row.try_get::<String>("", "avatar_emoji").map_err(storage)?,
        "avatar_color": row.try_get::<String>("", "avatar_color").map_err(storage)?,
        "owner_username": row.try_get::<String>("", "owner_username").map_err(storage)?,
        "member_count": member_count,
    })))
}

/// ORG-10: accept an invite.
pub async fn join_org(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<JoinOrgRequest>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;
    let backend = state.db_pool.read().get_database_backend();
    let tx = state.db_pool.write().await.begin().await.map_err(storage)?;
    let org = tx
        .query_one(Statement::from_sql_and_values(
            backend,
            "SELECT id, invite_expires_at FROM orgs WHERE invite_token = $1",
            [body.token.into()],
        ))
        .await
        .map_err(storage)?
        .ok_or_else(invite_invalid)?;
    let org_id: String = org.try_get("", "id").map_err(storage)?;
    let expires_at: Option<String> = org.try_get("", "invite_expires_at").map_err(storage)?;
    if let Some(expires) = expires_at.as_deref()
        && DateTime::parse_from_rfc3339(expires)
            .map(|parsed| parsed.with_timezone(&Utc) <= Utc::now())
            .unwrap_or(true)
    {
        return Err(invite_invalid());
    }
    if member_role(&tx, backend, &org_id, &user.id).await?.is_some() {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "org_already_member",
            "already a member of this organization",
        ));
    }
    if org_member_count(&tx, backend, &org_id).await? >= MAX_ORG_MEMBERS {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "org_member_limit_reached",
            "organization member limit reached",
        ));
    }
    tx.execute(Statement::from_sql_and_values(
        backend,
        "INSERT INTO org_members (org_id, user_id, role, joined_at) VALUES ($1, $2, 'member', $3)",
        [org_id.clone().into(), user.id.into(), Utc::now().to_rfc3339().into()],
    ))
    .await
    .map_err(storage)?;
    tx.commit().await.map_err(storage)?;
    Ok(Json(json!({ "org_id": org_id })))
}

#[derive(Debug, Deserialize)]
pub struct JoinOrgRequest {
    pub token: String,
}

/// ORG-11: regenerate the single invite link (owner only).
pub async fn regenerate_invite(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(org_id): Path<String>,
    Json(body): Json<RegenerateInviteRequest>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;
    if !expiry_is_valid(&body.invite_expiry) {
        return Err(bad_request("invite_expiry must be one of 24h, 3d, 7d, 30d, never"));
    }
    let backend = state.db_pool.read().get_database_backend();
    let tx = state.db_pool.write().await.begin().await.map_err(storage)?;
    let role = member_role(&tx, backend, &org_id, &user.id)
        .await?
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "not_found", "org not found"))?;
    if role != "owner" {
        return Err(forbidden("only the owner can regenerate the invite link"));
    }
    let token = format!(
        "{}{}",
        crate::store_billing::sales::generate_code(),
        crate::store_billing::sales::generate_code()
    )
    .to_lowercase();
    let expires_at = parse_expiry(&body.invite_expiry, Utc::now());
    tx.execute(Statement::from_sql_and_values(
        backend,
        "UPDATE orgs SET invite_token = $2, invite_expires_at = $3,
                invite_created_at = $4, updated_at = $4 WHERE id = $1",
        [
            org_id.into(),
            token.clone().into(),
            expires_at.map(|t| t.to_rfc3339()).into(),
            Utc::now().to_rfc3339().into(),
        ],
    ))
    .await
    .map_err(storage)?;
    tx.commit().await.map_err(storage)?;
    Ok(Json(json!({ "token": token, "expires_at": expires_at.map(|t| t.to_rfc3339()) })))
}

#[derive(Debug, Deserialize)]
pub struct RegenerateInviteRequest {
    pub invite_expiry: String,
}

#[derive(Debug, Deserialize)]
pub struct WalletMoveRequest {
    pub amount_nano_usd: String,
}

fn parse_amount(raw: &str) -> AppResult<i128> {
    let amount = raw
        .trim()
        .parse::<i128>()
        .map_err(|_| bad_request("amount_nano_usd must be an integer string"))?;
    if amount <= 0 {
        return Err(bad_request("amount must be positive"));
    }
    Ok(amount)
}

/// ORG-12: owner tops up the org wallet from their personal wallet.
pub async fn deposit_to_org(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(org_id): Path<String>,
    Json(body): Json<WalletMoveRequest>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;
    let amount = parse_amount(&body.amount_nano_usd)?;
    let backend = state.db_pool.read().get_database_backend();
    let read = state.db_pool.read();
    let role = member_role(&*read, backend, &org_id, &user.id)
        .await?
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "not_found", "org not found"))?;
    if role != "owner" {
        return Err(forbidden("only the owner can deposit into the organization wallet"));
    }
    let (personal_after, org_after) = move_wallet_balance(
        &state,
        &user.id,
        &org_id,
        amount,
        "org_deposit",
        "org_deposit_receive",
        json!({"org_id": org_id}),
    )
    .await?;
    Ok(Json(json!({
        "personal_balance_nano_usd": personal_after.to_string(),
        "org_balance_nano_usd": org_after.to_string(),
    })))
}

/// ORG-13: owner distributes org wallet funds to a member's personal wallet.
pub async fn distribute_from_org(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(org_id): Path<String>,
    Json(body): Json<DistributeRequest>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;
    let amount = parse_amount(&body.amount_nano_usd)?;
    let backend = state.db_pool.read().get_database_backend();
    let read = state.db_pool.read();
    let role = member_role(&*read, backend, &org_id, &user.id)
        .await?
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "not_found", "org not found"))?;
    if role != "owner" {
        return Err(forbidden("only the owner can distribute the organization wallet"));
    }
    let member_role_value = member_role(&*read, backend, &org_id, &body.member_user_id)
        .await?
        .ok_or_else(|| bad_request("member_user_id is not a member of this organization"))?;
    let _ = member_role_value;
    let (org_after, member_after) = move_wallet_balance(
        &state,
        &org_id,
        &body.member_user_id,
        amount,
        "org_grant",
        "org_receive",
        json!({"org_id": org_id}),
    )
    .await?;
    Ok(Json(json!({
        "org_balance_nano_usd": org_after.to_string(),
        "member_balance_nano_usd": member_after.to_string(),
    })))
}

#[derive(Debug, Deserialize)]
pub struct DistributeRequest {
    pub member_user_id: String,
    pub amount_nano_usd: String,
}

/// ORG-14: keys are owned and billed by the caller; sharing only exposes the material.
pub async fn create_org_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(org_id): Path<String>,
    Json(body): Json<CreateOrgKeyRequest>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;
    let backend = state.db_pool.read().get_database_backend();
    let read = state.db_pool.read();
    let role = member_role(&*read, backend, &org_id, &user.id)
        .await?
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "not_found", "org not found"))?;
    let name = body.name.trim();
    if name.is_empty() || name.len() > 64 {
        return Err(bad_request("name must be 1..64 characters"));
    }
    let share_mode = match body.share_mode.as_deref() {
        None | Some("default") => None,
        Some("all") => Some("all".to_string()),
        Some("private") => Some("__private".to_string()),
        other => return Err(bad_request(&format!("share_mode must be default, all, or private: {other:?}"))),
    };
    // The owner shares by default; members keep their keys private by default.
    let effective_mode = match share_mode {
        Some(mode) if mode == "__private" => None,
        Some(mode) => Some(mode),
        None => (role == "owner").then(|| "all".to_string()),
    };

    let (api_key, plaintext) = state
        .user_store
        .create_api_key(&user.id, name, None)
        .await
        .map_err(|e| bad_request(&e))?;
    state
        .db_pool
        .write()
        .await
        .execute(Statement::from_sql_and_values(
            backend,
            "UPDATE api_keys SET org_id = $2, org_share_mode = $3 WHERE id = $1",
            [
                api_key.id.clone().into(),
                org_id.into(),
                effective_mode.clone().into(),
            ],
        ))
        .await
        .map_err(storage)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": api_key.id, "name": api_key.name, "key": plaintext,
                     "share_mode": effective_mode, "owner_username": user.username })),
    ))
}

#[derive(Debug, Deserialize)]
pub struct CreateOrgKeyRequest {
    pub name: String,
    #[serde(default)]
    pub share_mode: Option<String>,
}

/// ORG-15: my keys plus keys shared to me (with material for copying).
pub async fn list_org_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(org_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;
    let backend = state.db_pool.read().get_database_backend();
    let read = state.db_pool.read();
    member_role(&*read, backend, &org_id, &user.id)
        .await?
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "not_found", "org not found"))?;

    let mine = read
        .query_all(Statement::from_sql_and_values(
            backend,
            "SELECT id, name, key_prefix, org_share_mode, created_at
             FROM api_keys WHERE org_id = $1 AND user_id = $2 ORDER BY created_at DESC",
            [org_id.clone().into(), user.id.clone().into()],
        ))
        .await
        .map_err(storage)?;
    let shared = read
        .query_all(Statement::from_sql_and_values(
            backend,
            "SELECT k.id, k.name, k.key, k.key_prefix, k.org_share_mode, u.username AS owner_username
             FROM api_keys k
             JOIN users u ON u.id = k.user_id
             WHERE k.org_id = $1 AND k.user_id != $2
               AND (k.org_share_mode = 'all'
                    OR EXISTS (SELECT 1 FROM org_key_shares s
                               WHERE s.api_key_id = k.id AND s.member_user_id = $2))
             ORDER BY k.created_at DESC",
            [org_id.clone().into(), user.id.clone().into()],
        ))
        .await
        .map_err(storage)?;

    Ok(Json(json!({
        "mine": mine
            .iter()
            .map(|row| json!({
                "id": row.try_get::<String>("", "id").unwrap_or_default(),
                "name": row.try_get::<String>("", "name").unwrap_or_default(),
                "key_prefix": row.try_get::<String>("", "key_prefix").unwrap_or_default(),
                "share_mode": row.try_get::<Option<String>>("", "org_share_mode").unwrap_or_default(),
                "created_at": row.try_get::<String>("", "created_at").unwrap_or_default(),
            }))
            .collect::<Vec<_>>(),
        "shared": shared
            .iter()
            .map(|row| json!({
                "id": row.try_get::<String>("", "id").unwrap_or_default(),
                "name": row.try_get::<String>("", "name").unwrap_or_default(),
                "key": row.try_get::<String>("", "key").unwrap_or_default(),
                "key_prefix": row.try_get::<String>("", "key_prefix").unwrap_or_default(),
                "share_mode": row.try_get::<Option<String>>("", "org_share_mode").unwrap_or_default(),
                "owner_username": row.try_get::<String>("", "owner_username").unwrap_or_default(),
            }))
            .collect::<Vec<_>>(),
    })))
}

/// ORG-16: the key owner chooses who may see the key inside the space.
pub async fn update_key_sharing(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((org_id, key_id)): Path<(String, String)>,
    Json(body): Json<UpdateSharingRequest>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;
    let backend = state.db_pool.read().get_database_backend();
    let tx = state.db_pool.write().await.begin().await.map_err(storage)?;
    member_role(&tx, backend, &org_id, &user.id)
        .await?
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "not_found", "org not found"))?;
    let key = tx
        .query_one(Statement::from_sql_and_values(
            backend,
            "SELECT id FROM api_keys WHERE id = $1 AND user_id = $2 AND org_id = $3",
            [key_id.clone().into(), user.id.clone().into(), org_id.clone().into()],
        ))
        .await
        .map_err(storage)?
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "not_found", "key not found"))?;
    let _ = key;

    let mode = match body.mode.as_str() {
        "private" => None,
        "all" => Some("all".to_string()),
        "selected" => Some("selected".to_string()),
        other => return Err(bad_request(&format!("mode must be private, all, or selected: {other}"))),
    };
    tx.execute(Statement::from_sql_and_values(
        backend,
        "DELETE FROM org_key_shares WHERE api_key_id = $1",
        [key_id.clone().into()],
    ))
    .await
    .map_err(storage)?;
    if mode.as_deref() == Some("selected") {
        for member_id in &body.member_ids {
            if member_role(&tx, backend, &org_id, member_id).await?.is_none() {
                return Err(bad_request("member_ids contains a non-member"));
            }
            tx.execute(Statement::from_sql_and_values(
                backend,
                "INSERT INTO org_key_shares (api_key_id, member_user_id, created_at)
                 VALUES ($1, $2, $3)",
                [key_id.clone().into(), member_id.clone().into(), Utc::now().to_rfc3339().into()],
            ))
            .await
            .map_err(storage)?;
        }
    }
    tx.execute(Statement::from_sql_and_values(
        backend,
        "UPDATE api_keys SET org_share_mode = $2 WHERE id = $1",
        [key_id.clone().into(), mode.into()],
    ))
    .await
    .map_err(storage)?;
    tx.commit().await.map_err(storage)?;
    Ok(Json(json!({ "success": true, "mode": body.mode })))
}

#[derive(Debug, Deserialize)]
pub struct UpdateSharingRequest {
    pub mode: String,
    #[serde(default)]
    pub member_ids: Vec<String>,
}

/// ORG-17: removal also un-shares the member's keys in this space.
pub async fn remove_org_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((org_id, member_id)): Path<(String, String)>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;
    let backend = state.db_pool.read().get_database_backend();
    let tx = state.db_pool.write().await.begin().await.map_err(storage)?;
    let role = member_role(&tx, backend, &org_id, &user.id)
        .await?
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "not_found", "org not found"))?;
    if role != "owner" {
        return Err(forbidden("only the owner can remove members"));
    }
    if member_id == user.id {
        return Err(bad_request("the owner cannot be removed"));
    }
    let target_role = member_role(&tx, backend, &org_id, &member_id)
        .await?
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "not_found", "member not found"))?;
    if target_role == "owner" {
        return Err(bad_request("the owner cannot be removed"));
    }
    tx.execute(Statement::from_sql_and_values(
        backend,
        "DELETE FROM org_members WHERE org_id = $1 AND user_id = $2",
        [org_id.clone().into(), member_id.clone().into()],
    ))
    .await
    .map_err(storage)?;
    tx.execute(Statement::from_sql_and_values(
        backend,
        "DELETE FROM org_key_shares WHERE member_user_id = $1 AND api_key_id IN
            (SELECT id FROM api_keys WHERE org_id = $2)",
        [member_id.clone().into(), org_id.clone().into()],
    ))
    .await
    .map_err(storage)?;
    tx.execute(Statement::from_sql_and_values(
        backend,
        "UPDATE api_keys SET org_id = NULL, org_share_mode = NULL
         WHERE user_id = $1 AND org_id = $2",
        [member_id.clone().into(), org_id.clone().into()],
    ))
    .await
    .map_err(storage)?;
    tx.commit().await.map_err(storage)?;
    Ok(Json(json!({ "success": true })))
}
