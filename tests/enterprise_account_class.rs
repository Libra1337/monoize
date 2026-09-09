use monoize::db::DbPool;
use monoize::migration::Migrator;
use monoize::users::{
    AccountClass, CreateApiKeyInput, CreateGroupInput, ReorderGroupsInput, RequestCaptureMode,
    UserRole, UserStore,
};
use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
use sea_orm_migration::MigratorTrait;
use serde_json::Value;

async fn migrated_store() -> (DbPool, UserStore) {
    let db = DbPool::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    {
        let write = db.write().await;
        Migrator::up(&*write, None).await.expect("run migrations");
    }
    let (log_tx, _) = tokio::sync::broadcast::channel(4);
    let store = UserStore::new(db.clone(), log_tx)
        .await
        .expect("create user store");
    (db, store)
}

async fn create_enterprise_group(store: &UserStore, name: &str) -> String {
    store
        .create_group(CreateGroupInput {
            confirm_public_exposure: true,
            name: name.to_string(),
            description: String::new(),
            user_selectable: true,
            sort_order: 0,
            account_class: AccountClass::Enterprise,
        })
        .await
        .expect("create Enterprise Group")
        .id
}

#[tokio::test]
async fn group_reorder_is_complete_within_one_account_class() {
    let (_db, store) = migrated_store().await;
    let first = create_enterprise_group(&store, "Enterprise A").await;
    let second = create_enterprise_group(&store, "Enterprise B").await;

    store
        .reorder_groups(ReorderGroupsInput {
            group_ids: vec![second.clone(), first.clone()],
        })
        .await
        .expect("Enterprise-only reorder succeeds");

    let groups = store.list_groups().await.expect("list Groups");
    let enterprise: Vec<_> = groups
        .iter()
        .filter(|group| group.account_class == AccountClass::Enterprise)
        .collect();
    assert_eq!(enterprise[0].id, second);
    assert_eq!(enterprise[0].sort_order, 0);
    assert_eq!(enterprise[1].id, first);
    assert_eq!(enterprise[1].sort_order, 1);
    assert_eq!(
        groups
            .iter()
            .find(|group| group.account_class == AccountClass::Standard)
            .expect("standard Group remains")
            .sort_order,
        0,
    );
}

#[tokio::test]
async fn account_class_filtered_admin_lists_never_mix_catalogs() {
    let (_db, store) = migrated_store().await;
    let enterprise_group_id = create_enterprise_group(&store, "Enterprise Filter").await;
    let enterprise_groups = store
        .list_groups_by_account_class(AccountClass::Enterprise)
        .await
        .expect("filter Enterprise Groups");
    assert_eq!(enterprise_groups.len(), 1);
    assert_eq!(enterprise_groups[0].id, enterprise_group_id);
    assert!(
        enterprise_groups
            .iter()
            .all(|group| group.account_class == AccountClass::Enterprise)
    );
}

fn sub_account_key(name: &str, balance: &str) -> CreateApiKeyInput {
    CreateApiKeyInput {
        name: name.to_string(),
        expires_in_days: None,
        sub_account_enabled: true,
        sub_account_balance_nano_usd: Some(balance.to_string()),
        model_limits_enabled: false,
        model_limits: Vec::new(),
        ip_whitelist: Vec::new(),
        group_ids: Vec::new(),
        channel_bindings: Vec::new(),
        max_multiplier: None,
        transforms: Vec::new(),
        model_redirects: Vec::new(),
        reasoning_envelope_enabled: true,
        request_capture_mode: RequestCaptureMode::Off,
    }
}

async fn sqlite_columns(
    db: &sea_orm::DatabaseConnection,
    table: &str,
) -> Vec<(String, String, bool, Option<String>)> {
    db.query_all(Statement::from_string(
        DbBackend::Sqlite,
        format!("PRAGMA table_info({table})"),
    ))
    .await
    .expect("read SQLite columns")
    .into_iter()
    .map(|row| {
        (
            row.try_get::<String>("", "name").expect("column name"),
            row.try_get::<String>("", "type").expect("column type"),
            row.try_get::<i64>("", "notnull").expect("not-null flag") == 1,
            row.try_get::<Option<String>>("", "dflt_value")
                .expect("default value"),
        )
    })
    .collect()
}

#[tokio::test]
async fn migration_063_adds_standard_account_class_and_audit_schema() {
    let db = Database::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    Migrator::up(&db, None).await.expect("run migrations");

    for table in ["users", "monoize_groups"] {
        let columns = sqlite_columns(&db, table).await;
        assert!(
            columns.iter().any(|(name, kind, not_null, default)| {
                name == "account_class"
                    && kind.eq_ignore_ascii_case("TEXT")
                    && *not_null
                    && default.as_deref() == Some("'standard'")
            }),
            "{table}.account_class must be required TEXT with standard default: {columns:?}"
        );
    }

    let audit_columns = sqlite_columns(&db, "user_account_class_audits").await;
    for required in [
        "id",
        "user_id",
        "actor_user_id",
        "from_account_class",
        "to_account_class",
        "deleted_api_key_count",
        "deleted_api_keys_json",
        "created_at",
    ] {
        assert!(
            audit_columns.iter().any(|(name, _, _, _)| name == required),
            "missing audit column {required}: {audit_columns:?}"
        );
    }
}

#[tokio::test]
async fn migration_063_indexes_api_key_analytics_and_rejects_invalid_classes() {
    let db = Database::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    Migrator::up(&db, None).await.expect("run migrations");

    let indexes = db
        .query_all(Statement::from_string(
            DbBackend::Sqlite,
            "PRAGMA index_list(request_logs)".to_string(),
        ))
        .await
        .expect("read request log indexes");
    assert!(
        indexes.iter().any(|row| {
            row.try_get::<String>("", "name").as_deref() == Ok("idx_request_logs_api_key_created")
        }),
        "missing API Key analytics index"
    );

    let user_error = db
        .execute(Statement::from_string(
            DbBackend::Sqlite,
            "INSERT INTO users (id, username, password_hash, role, created_at, updated_at, account_class) VALUES ('invalid-class-user', 'invalid-class-user', 'hash', 'user', '2026-09-08T00:00:00Z', '2026-09-08T00:00:00Z', 'partner')".to_string(),
        ))
        .await
        .expect_err("invalid user account class must fail");
    assert!(!user_error.to_string().is_empty());

    let group_error = db
        .execute(Statement::from_string(
            DbBackend::Sqlite,
            "UPDATE monoize_groups SET account_class = 'partner'".to_string(),
        ))
        .await
        .expect_err("invalid Group account class must fail");
    assert!(!group_error.to_string().is_empty());
}

#[tokio::test]
async fn account_class_switch_settles_and_deletes_keys_then_writes_audit() {
    let (db, store) = migrated_store().await;
    let enterprise_group_id = create_enterprise_group(&store, "Enterprise").await;
    let actor = store
        .create_user("class-admin", "password123", UserRole::Admin, None)
        .await
        .expect("create actor");
    let user = store
        .create_user("class-user", "password123", UserRole::User, None)
        .await
        .expect("create user");
    let (key, secret) = store
        .create_api_key_extended(&user.id, sub_account_key("settled", "25"), true)
        .await
        .expect("create sub-account Key");
    store
        .create_api_key(&user.id, "ordinary", None)
        .await
        .expect("create ordinary Key");
    db.write()
        .await
        .execute(db.stmt(
            "UPDATE users SET balance_nano_usd = '100' WHERE id = $1",
            vec![user.id.clone().into()],
        ))
        .await
        .expect("seed wallet balance");

    assert!(
        store
            .validate_api_key(&secret)
            .await
            .expect("prime Key cache")
            .is_some()
    );
    store
        .change_user_account_class(&user.id, AccountClass::Enterprise, &actor.id)
        .await
        .expect("switch account class");

    let changed = store
        .get_user_by_id(&user.id)
        .await
        .expect("read user")
        .expect("user remains");
    assert_eq!(changed.account_class, AccountClass::Enterprise);
    assert_eq!(changed.group_id, enterprise_group_id);
    assert_eq!(changed.balance_nano_usd, "125");
    assert_eq!(
        store
            .count_user_api_keys(&user.id)
            .await
            .expect("count Keys"),
        0
    );
    assert!(
        store
            .validate_api_key(&secret)
            .await
            .expect("read invalidated cache")
            .is_none()
    );

    let ledger = store
        .list_billing_ledger(&user.id, 50)
        .await
        .expect("read ledger");
    assert!(ledger.iter().any(|entry| {
        entry.kind == "sub_account_delete_settlement"
            && entry.delta_nano_usd == "25"
            && entry.meta["api_key_id"] == key.id
            && entry.meta["reason"] == "account_class_change"
    }));

    let audit = db
        .read()
        .query_one(db.stmt(
            "SELECT actor_user_id, from_account_class, to_account_class, deleted_api_key_count, deleted_api_keys_json FROM user_account_class_audits WHERE user_id = $1",
            vec![user.id.into()],
        ))
        .await
        .expect("read audit")
        .expect("audit exists");
    assert_eq!(
        audit.try_get::<String>("", "actor_user_id").unwrap(),
        actor.id
    );
    assert_eq!(
        audit.try_get::<String>("", "from_account_class").unwrap(),
        "standard"
    );
    assert_eq!(
        audit.try_get::<String>("", "to_account_class").unwrap(),
        "enterprise"
    );
    assert_eq!(
        audit.try_get::<i64>("", "deleted_api_key_count").unwrap(),
        2
    );
    let deleted: Value = serde_json::from_str(
        &audit
            .try_get::<String>("", "deleted_api_keys_json")
            .expect("decode audit Keys"),
    )
    .expect("parse audit Keys");
    assert_eq!(deleted.as_array().map(Vec::len), Some(2));
    assert!(!deleted.to_string().contains(&secret));
}

#[tokio::test]
async fn account_class_switch_without_target_group_changes_nothing() {
    let (db, store) = migrated_store().await;
    let user = store
        .create_user("no-enterprise-group", "password123", UserRole::User, None)
        .await
        .expect("create user");
    let (key, _) = store
        .create_api_key(&user.id, "preserved", None)
        .await
        .expect("create Key");

    let error = store
        .change_user_account_class(&user.id, AccountClass::Enterprise, "actor")
        .await
        .expect_err("missing target Group must reject");
    assert_eq!(error, "target account class has no Group");

    let unchanged = store.get_user_by_id(&user.id).await.unwrap().unwrap();
    assert_eq!(unchanged.account_class, AccountClass::Standard);
    assert_eq!(unchanged.group_id, user.group_id);
    assert!(store.get_api_key_by_id(&key.id).await.unwrap().is_some());
    let audit_count = db
        .read()
        .query_one(db.stmt(
            "SELECT COUNT(*) AS count FROM user_account_class_audits WHERE user_id = $1",
            vec![user.id.into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "count")
        .unwrap();
    assert_eq!(audit_count, 0);
}

#[tokio::test]
async fn account_class_switch_rolls_back_when_audit_write_fails() {
    let (db, store) = migrated_store().await;
    create_enterprise_group(&store, "Enterprise rollback").await;
    let user = store
        .create_user("class-rollback", "password123", UserRole::User, None)
        .await
        .expect("create user");
    let (key, _) = store
        .create_api_key_extended(&user.id, sub_account_key("rollback-key", "17"), true)
        .await
        .expect("create sub-account Key");
    db.write()
        .await
        .execute(db.stmt(
            "CREATE TRIGGER reject_class_audit BEFORE INSERT ON user_account_class_audits BEGIN SELECT RAISE(FAIL, 'audit blocked'); END",
            vec![],
        ))
        .await
        .expect("create failure trigger");

    assert!(
        store
            .change_user_account_class(&user.id, AccountClass::Enterprise, "actor")
            .await
            .is_err()
    );

    let unchanged = store.get_user_by_id(&user.id).await.unwrap().unwrap();
    assert_eq!(unchanged.account_class, AccountClass::Standard);
    assert_eq!(unchanged.balance_nano_usd, "0");
    assert!(store.get_api_key_by_id(&key.id).await.unwrap().is_some());
    let settlements = store
        .list_billing_ledger(&user.id, 50)
        .await
        .unwrap()
        .into_iter()
        .filter(|entry| entry.kind == "sub_account_delete_settlement")
        .count();
    assert_eq!(settlements, 0);
}

#[tokio::test]
async fn group_visibility_never_crosses_account_class() {
    let (_db, store) = migrated_store().await;
    let enterprise_group_id = create_enterprise_group(&store, "Enterprise private catalog").await;
    let standard = store
        .create_user("standard-catalog", "password123", UserRole::User, None)
        .await
        .unwrap();
    let enterprise = store
        .create_user("enterprise-catalog", "password123", UserRole::User, None)
        .await
        .unwrap();
    store
        .change_user_account_class(&enterprise.id, AccountClass::Enterprise, "actor")
        .await
        .unwrap();

    let standard_groups = store
        .list_groups_for_user(&standard.id, UserRole::User)
        .await
        .unwrap();
    assert!(
        standard_groups
            .iter()
            .all(|group| group.account_class == AccountClass::Standard)
    );
    assert!(
        standard_groups
            .iter()
            .all(|group| group.id != enterprise_group_id)
    );

    let enterprise_groups = store
        .list_groups_for_user(&enterprise.id, UserRole::User)
        .await
        .unwrap();
    assert!(!enterprise_groups.is_empty());
    assert!(
        enterprise_groups
            .iter()
            .all(|group| group.account_class == AccountClass::Enterprise)
    );
    assert!(
        enterprise_groups
            .iter()
            .any(|group| group.id == enterprise_group_id)
    );

    assert!(
        store
            .grant_group_access(&standard.id, &enterprise_group_id)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn create_user_rejects_a_group_from_the_other_account_class() {
    let (_db, store) = migrated_store().await;
    let enterprise_group_id = create_enterprise_group(&store, "Enterprise Pricing").await;

    // A new user is always Standard, so binding an Enterprise Group here would
    // store a cross-class reference that every later access check rejects.
    let rejected = store
        .create_user(
            "cross-class",
            "password-cross-class",
            UserRole::User,
            Some(&enterprise_group_id),
        )
        .await;
    assert!(
        rejected.is_err(),
        "an Enterprise Group must not bind to a Standard user at creation"
    );

    let accepted = store
        .create_user("in-class", "password-in-class", UserRole::User, None)
        .await
        .expect("the default Standard Group is accepted");
    assert_eq!(accepted.account_class, AccountClass::Standard);
}
