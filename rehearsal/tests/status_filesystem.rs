use monoize_lynshen_rehearsal::status::{
    Admission, CapacityInput, FileSlotCapacity, FilesystemCapacity, assess_startup_admission,
    calculate_capacity, probe_filesystem, scan_spool,
};
use std::fs::OpenOptions;
use std::io::Write;
use tempfile::TempDir;

fn capacity() -> monoize_lynshen_rehearsal::status::CapacityResult {
    calculate_capacity(CapacityInput {
        peak_events_per_second: 1,
        max_outage_seconds: 900,
        safety_factor_milli: 1_200,
        max_in_flight_dispatches: 4,
        entry_max_bytes: 4_096,
        allocation_unit: 4_096,
        configured_quota_bytes: 16 * 1_024 * 1_024,
    })
    .unwrap()
}

#[test]
fn real_probe_reports_allocation_unit_allocated_bytes_and_free_bytes() {
    let directory = TempDir::new().unwrap();
    let result = probe_filesystem(directory.path(), 4_096).unwrap();
    assert!(result.allocation_unit > 0);
    assert!(result.probe_allocated_bytes > 0);
    assert!(result.probe_allocated_bytes <= result.entry_reservation_bytes);
    assert!(result.available_bytes > 0);
}

#[test]
fn scan_accounts_final_and_temporary_event_allocations() {
    let directory = TempDir::new().unwrap();
    for name in ["node.id.0.event.json", "node.id.1.event.tmp"] {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(directory.path().join(name))
            .unwrap();
        file.write_all(&vec![0xA5; 4_096]).unwrap();
        file.sync_all().unwrap();
    }
    std::fs::write(directory.path().join("node-state.json"), b"{}").unwrap();

    let scan = scan_spool(directory.path(), 4_096).unwrap();
    assert_eq!(scan.final_events.len(), 1);
    assert_eq!(scan.temporary_events.len(), 1);
    assert!(scan.accounted_spool_bytes >= 8_192);
    assert!(scan.state_allocated_bytes > 0);
}

#[test]
fn scan_rejects_unknown_regular_entries_and_subdirectories() {
    let directory = TempDir::new().unwrap();
    std::fs::write(directory.path().join("unowned.txt"), b"x").unwrap();
    assert_eq!(
        scan_spool(directory.path(), 4_096).unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );

    std::fs::remove_file(directory.path().join("unowned.txt")).unwrap();
    std::fs::create_dir(directory.path().join("nested")).unwrap();
    assert_eq!(
        scan_spool(directory.path(), 4_096).unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );
}

#[cfg(windows)]
#[test]
fn scan_rejects_symlinks() {
    use std::os::windows::fs::symlink_file;

    let directory = TempDir::new().unwrap();
    let target = directory.path().join("target.event.json");
    std::fs::write(&target, b"event").unwrap();
    let link = directory.path().join("linked.event.json");
    if let Err(error) = symlink_file(&target, &link) {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            return;
        }
        panic!("failed to create test symlink: {error}");
    }
    assert_eq!(
        scan_spool(directory.path(), 4_096).unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );
}

#[test]
fn unknown_file_slot_capacity_fails_closed() {
    let result = assess_startup_admission(
        capacity(),
        16 * 1_024 * 1_024,
        0,
        FilesystemCapacity {
            available_bytes: u64::MAX,
            file_slots: FileSlotCapacity::Unknown,
        },
    );
    assert_eq!(result, Admission::Fatal("file_slot_capacity_unknown"));
}

#[test]
fn backlog_and_free_space_shortfalls_enter_recovery_only() {
    let computed = capacity();
    assert_eq!(
        assess_startup_admission(
            computed,
            16 * 1_024 * 1_024,
            16 * 1_024 * 1_024 - computed.minimum_spool_bytes + 1,
            FilesystemCapacity {
                available_bytes: u64::MAX,
                file_slots: FileSlotCapacity::Finite(u64::MAX),
            },
        ),
        Admission::RecoveryOnly("minimum_spool_reserve_unavailable")
    );
    assert_eq!(
        assess_startup_admission(
            computed,
            16 * 1_024 * 1_024,
            0,
            FilesystemCapacity {
                available_bytes: 1,
                file_slots: FileSlotCapacity::Finite(u64::MAX),
            },
        ),
        Admission::RecoveryOnly("filesystem_free_bytes_insufficient")
    );
    assert_eq!(
        assess_startup_admission(
            computed,
            16 * 1_024 * 1_024,
            0,
            FilesystemCapacity {
                available_bytes: u64::MAX,
                file_slots: FileSlotCapacity::Finite(1),
            },
        ),
        Admission::RecoveryOnly("filesystem_file_slots_insufficient")
    );
}

#[test]
fn valid_capacity_is_ready() {
    assert_eq!(
        assess_startup_admission(
            capacity(),
            16 * 1_024 * 1_024,
            0,
            FilesystemCapacity {
                available_bytes: u64::MAX,
                file_slots: FileSlotCapacity::Finite(u64::MAX),
            },
        ),
        Admission::Ready
    );
}
