use super::CapacityResult;
use std::path::{Path, PathBuf};

const FREE_BYTE_RESERVE: u64 = 67_108_864;
const FILE_SLOT_RESERVE: u64 = 1_024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileSlotCapacity {
    Finite(u64),
    ExplicitlyUnbounded,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FilesystemCapacity {
    pub available_bytes: u64,
    pub file_slots: FileSlotCapacity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FilesystemProbe {
    pub allocation_unit: u64,
    pub entry_reservation_bytes: u64,
    pub probe_allocated_bytes: u64,
    pub available_bytes: u64,
    pub file_slots: FileSlotCapacity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpoolScan {
    pub final_events: Vec<PathBuf>,
    pub temporary_events: Vec<PathBuf>,
    pub accounted_spool_bytes: u64,
    pub state_allocated_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    Ready,
    RecoveryOnly(&'static str),
    Fatal(&'static str),
}

pub fn assess_startup_admission(
    computed: CapacityResult,
    quota_bytes: u64,
    accounted_spool_bytes: u64,
    filesystem: FilesystemCapacity,
) -> Admission {
    let Some(remaining_spool_bytes) = quota_bytes.checked_sub(accounted_spool_bytes) else {
        return Admission::Fatal("accounted_spool_exceeds_quota");
    };
    if remaining_spool_bytes < computed.minimum_spool_bytes {
        return Admission::RecoveryOnly("minimum_spool_reserve_unavailable");
    }
    let Some(required_free_bytes) = remaining_spool_bytes.checked_add(FREE_BYTE_RESERVE) else {
        return Admission::Fatal("filesystem_free_byte_requirement_overflow");
    };
    if filesystem.available_bytes < required_free_bytes {
        return Admission::RecoveryOnly("filesystem_free_bytes_insufficient");
    }
    let required_slots = remaining_spool_bytes / computed.entry_reservation_bytes;
    let Some(required_slots) = required_slots.checked_add(FILE_SLOT_RESERVE) else {
        return Admission::Fatal("file_slot_requirement_overflow");
    };
    match filesystem.file_slots {
        FileSlotCapacity::Finite(available) if available < required_slots => {
            Admission::RecoveryOnly("filesystem_file_slots_insufficient")
        }
        FileSlotCapacity::Unknown => Admission::Fatal("file_slot_capacity_unknown"),
        FileSlotCapacity::Finite(_) | FileSlotCapacity::ExplicitlyUnbounded => Admission::Ready,
    }
}

pub fn scan_spool(path: &Path, entry_reservation_bytes: u64) -> std::io::Result<SpoolScan> {
    let mut final_events = Vec::new();
    let mut temporary_events = Vec::new();
    let mut accounted_spool_bytes = 0_u64;
    let mut state_allocated_bytes = 0_u64;

    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = std::fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(invalid_data(
                "spool contains a symlink or non-regular entry",
            ));
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| invalid_data("spool entry name is not valid Unicode"))?;
        let allocated = allocated_bytes(&entry.path(), &metadata)?;
        if name.ends_with(".event.json") {
            accounted_spool_bytes = checked_add(accounted_spool_bytes, allocated)?;
            final_events.push(entry.path());
        } else if name.ends_with(".event.tmp") {
            accounted_spool_bytes = checked_add(
                accounted_spool_bytes,
                allocated.max(entry_reservation_bytes),
            )?;
            temporary_events.push(entry.path());
        } else if matches!(name.as_str(), "node-state.json" | "node-state.tmp") {
            state_allocated_bytes = checked_add(state_allocated_bytes, allocated)?;
        } else {
            return Err(invalid_data("spool contains an unrecognized regular entry"));
        }
    }
    final_events.sort();
    temporary_events.sort();
    Ok(SpoolScan {
        final_events,
        temporary_events,
        accounted_spool_bytes,
        state_allocated_bytes,
    })
}

pub fn probe_filesystem(path: &Path, entry_max_bytes: u64) -> std::io::Result<FilesystemProbe> {
    std::fs::create_dir_all(path)?;
    let allocation_unit = platform_allocation_unit(path)?;
    let entry_reservation_bytes = round_up(entry_max_bytes, allocation_unit)?;
    let logical_size = usize::try_from(entry_max_bytes)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "entry too large"))?;
    let stem = format!(".allocation-probe-{}", uuid::Uuid::new_v4());
    let temporary = path.join(format!("{stem}.tmp"));
    let final_path = path.join(format!("{stem}.final"));

    let result = (|| {
        use std::fs::OpenOptions;
        use std::io::Write;

        let mut probe = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        probe.write_all(&vec![0xA5; logical_size])?;
        probe.sync_all()?;
        drop(probe);
        let metadata = std::fs::symlink_metadata(&temporary)?;
        let probe_allocated_bytes = allocated_bytes(&temporary, &metadata)?;
        if probe_allocated_bytes > entry_reservation_bytes {
            return Err(invalid_data("probe allocation exceeds entry reservation"));
        }
        std::fs::rename(&temporary, &final_path)?;
        sync_directory(path)?;
        std::fs::remove_file(&final_path)?;
        sync_directory(path)?;
        let capacity = platform_capacity(path)?;
        Ok(FilesystemProbe {
            allocation_unit,
            entry_reservation_bytes,
            probe_allocated_bytes,
            available_bytes: capacity.available_bytes,
            file_slots: capacity.file_slots,
        })
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
        let _ = std::fs::remove_file(&final_path);
    }
    result
}

fn checked_add(left: u64, right: u64) -> std::io::Result<u64> {
    left.checked_add(right)
        .ok_or_else(|| invalid_data("spool allocation overflow"))
}

fn invalid_data(message: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message)
}

fn round_up(value: u64, unit: u64) -> std::io::Result<u64> {
    if unit == 0 {
        return Err(invalid_data("allocation unit is zero"));
    }
    let units = value
        .checked_add(unit - 1)
        .ok_or_else(|| invalid_data("round-up overflow"))?
        / unit;
    units
        .checked_mul(unit)
        .ok_or_else(|| invalid_data("round-up overflow"))
}

#[cfg(windows)]
fn allocated_bytes(path: &Path, _metadata: &std::fs::Metadata) -> std::io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, GetLastError, SetLastError};
    use windows_sys::Win32::Storage::FileSystem::GetCompressedFileSizeW;

    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut high = 0_u32;
    unsafe { SetLastError(ERROR_SUCCESS) };
    let low = unsafe { GetCompressedFileSizeW(wide.as_ptr(), &mut high) };
    if low == u32::MAX {
        let error = unsafe { GetLastError() };
        if error != ERROR_SUCCESS {
            return Err(std::io::Error::from_raw_os_error(error as i32));
        }
    }
    Ok((u64::from(high) << 32) | u64::from(low))
}

#[cfg(unix)]
fn allocated_bytes(_path: &Path, metadata: &std::fs::Metadata) -> std::io::Result<u64> {
    use std::os::unix::fs::MetadataExt;

    metadata
        .blocks()
        .checked_mul(512)
        .ok_or_else(|| invalid_data("allocated byte count overflow"))
}

#[cfg(windows)]
fn platform_allocation_unit(path: &Path) -> std::io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceW;

    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut sectors_per_cluster = 0;
    let mut bytes_per_sector = 0;
    let mut free_clusters = 0;
    let mut total_clusters = 0;
    let ok = unsafe {
        GetDiskFreeSpaceW(
            wide.as_ptr(),
            &mut sectors_per_cluster,
            &mut bytes_per_sector,
            &mut free_clusters,
            &mut total_clusters,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    u64::from(sectors_per_cluster)
        .checked_mul(u64::from(bytes_per_sector))
        .ok_or_else(|| invalid_data("allocation unit overflow"))
}

#[cfg(unix)]
fn platform_allocation_unit(path: &Path) -> std::io::Result<u64> {
    let stats = statvfs(path)?;
    u64::try_from(stats.f_frsize).map_err(|_| invalid_data("allocation unit overflow"))
}

#[cfg(windows)]
fn platform_capacity(path: &Path) -> std::io::Result<FilesystemCapacity> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut available_bytes = 0_u64;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available_bytes,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(FilesystemCapacity {
        available_bytes,
        file_slots: FileSlotCapacity::Unknown,
    })
}

#[cfg(unix)]
fn platform_capacity(path: &Path) -> std::io::Result<FilesystemCapacity> {
    let stats = statvfs(path)?;
    let fragment_size =
        u64::try_from(stats.f_frsize).map_err(|_| invalid_data("fragment size overflow"))?;
    let available_bytes = u64::try_from(stats.f_bavail)
        .ok()
        .and_then(|blocks| blocks.checked_mul(fragment_size))
        .ok_or_else(|| invalid_data("available byte count overflow"))?;
    let file_slots = if stats.f_files == 0 {
        FileSlotCapacity::ExplicitlyUnbounded
    } else {
        FileSlotCapacity::Finite(
            u64::try_from(stats.f_favail).map_err(|_| invalid_data("file slot count overflow"))?,
        )
    };
    Ok(FilesystemCapacity {
        available_bytes,
        file_slots,
    })
}

#[cfg(unix)]
fn statvfs(path: &Path) -> std::io::Result<libc::statvfs> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| invalid_data("filesystem path contains NUL"))?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { stats.assume_init() })
}

#[cfg(windows)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, FlushFileBuffers, OPEN_EXISTING,
    };

    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    let flushed = unsafe { FlushFileBuffers(handle) };
    let flush_error = (flushed == 0).then(std::io::Error::last_os_error);
    let closed = unsafe { CloseHandle(handle) };
    if let Some(error) = flush_error {
        return Err(error);
    }
    if closed == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}
