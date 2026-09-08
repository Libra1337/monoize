use tempfile::TempDir;

fn test_runtime(database_dsn: String) -> monoize::app::RuntimeConfig {
    monoize::app::RuntimeConfig {
        listen: "127.0.0.1:0".to_string(),
        metrics_path: "/metrics".to_string(),
        database_dsn,
        request_log_spool_dir: None,
        node: monoize::node_config::NodeSettings::primary_default(),
    }
}

#[tokio::test]
async fn sqlite_file_created_for_runtime_dsn() {
    let temp_dir = TempDir::new().expect("temp dir");
    let db_path = temp_dir.path().join("data").join("monoize.db");
    assert!(!db_path.exists());

    let runtime = test_runtime(format!("sqlite://{}", db_path.display()));
    let _state = monoize::app::load_state_with_runtime(runtime)
        .await
        .expect("load state");

    assert!(db_path.exists());
}

#[tokio::test]
async fn sqlite_memory_dsn_starts_without_files() {
    let runtime = test_runtime("sqlite::memory:".to_string());
    let _state = monoize::app::load_state_with_runtime(runtime)
        .await
        .expect("load state");
}

#[tokio::test]
async fn explicit_request_log_spool_directories_are_instance_local() {
    let first_temp = TempDir::new().expect("first temp dir");
    let second_temp = TempDir::new().expect("second temp dir");
    let first_spool = first_temp.path().join("request-log-spool");
    let second_spool = second_temp.path().join("request-log-spool");

    let mut first_runtime = test_runtime("sqlite::memory:".to_string());
    first_runtime.request_log_spool_dir = Some(first_spool.clone());
    let first_state = monoize::app::load_state_with_runtime(first_runtime)
        .await
        .expect("first state loads");
    let first_reservation = first_state
        .user_store
        .reserve_terminal_request_log()
        .expect("first spool reserves");

    let mut second_runtime = test_runtime("sqlite::memory:".to_string());
    second_runtime.request_log_spool_dir = Some(second_spool.clone());
    let second_state = monoize::app::load_state_with_runtime(second_runtime)
        .await
        .expect("second state loads");
    let second_reservation = second_state
        .user_store
        .reserve_terminal_request_log()
        .expect("second spool reserves");

    assert_eq!(std::fs::read_dir(&first_spool).unwrap().count(), 1);
    assert_eq!(std::fs::read_dir(&second_spool).unwrap().count(), 1);
    drop(first_reservation);
    assert_eq!(std::fs::read_dir(&first_spool).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(&second_spool).unwrap().count(), 1);
    drop(second_reservation);
}

#[test]
fn production_runtime_keeps_spool_override_unset() {
    let runtime = monoize::app::RuntimeConfig::from_env().expect("env config resolves");
    assert!(runtime.request_log_spool_dir.is_none());
    // PRP1 default role without explicit env configuration.
    assert_eq!(runtime.node.role, monoize::node_config::NodeRole::Primary);
}
