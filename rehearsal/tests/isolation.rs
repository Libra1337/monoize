#[test]
fn root_application_does_not_register_rehearsal() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("rehearsal crate is inside repository root");
    let lib = std::fs::read_to_string(root.join("src/lib.rs")).expect("read root library");
    let migrations = std::fs::read_to_string(root.join("src/migration/mod.rs"))
        .expect("read root migration registry");
    let rehearsal_binary = std::fs::read_to_string(
        root.join("rehearsal/src/bin/lynshen-rehearsal.rs"),
    )
    .expect("isolated rehearsal binary exists");

    assert!(!lib.contains("lynshen_rehearsal"));
    assert!(!migrations.contains("lynshen_rehearsal"));
    assert!(rehearsal_binary.contains("monoize_lynshen_rehearsal"));
}
