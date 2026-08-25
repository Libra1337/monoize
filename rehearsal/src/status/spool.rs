use super::UpstreamCallEvent;
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const STATE_FILE: &str = "node-state.json";
const STATE_TEMP_FILE: &str = "node-state.tmp";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpoolConfig {
    pub entry_max_bytes: usize,
    pub state_max_bytes: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeState {
    pub clean_shutdown: bool,
    pub incomplete_since_unix_ms: Option<u64>,
    pub incomplete_until_unix_ms: Option<u64>,
    pub most_recent_loss_unix_ms: Option<u64>,
}

#[derive(Debug)]
pub struct Spool {
    directory: PathBuf,
    config: SpoolConfig,
    state: Mutex<NodeState>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpoolError {
    Io,
    Encode,
    EntryTooLarge,
    StateTooLarge,
    InvalidEntry,
}

impl Spool {
    pub fn open(directory: &Path, config: SpoolConfig) -> Result<Self, SpoolError> {
        std::fs::create_dir_all(directory).map_err(|_| SpoolError::Io)?;
        let state_path = directory.join(STATE_FILE);
        let state = if state_path.exists() {
            let bytes = std::fs::read(&state_path).map_err(|_| SpoolError::Io)?;
            serde_json::from_slice(&bytes).map_err(|_| SpoolError::InvalidEntry)?
        } else {
            NodeState {
                clean_shutdown: true,
                ..NodeState::default()
            }
        };

        let mut found_temporary_event = false;
        for entry in std::fs::read_dir(directory).map_err(|_| SpoolError::Io)? {
            let entry = entry.map_err(|_| SpoolError::Io)?;
            let file_type = entry.file_type().map_err(|_| SpoolError::Io)?;
            if !file_type.is_file() {
                return Err(SpoolError::InvalidEntry);
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(".event.tmp") {
                found_temporary_event = true;
                std::fs::remove_file(entry.path()).map_err(|_| SpoolError::Io)?;
            } else if name == STATE_TEMP_FILE {
                if !state_path.exists() {
                    found_temporary_event = true;
                }
                std::fs::remove_file(entry.path()).map_err(|_| SpoolError::Io)?;
            }
        }
        let spool = Self {
            directory: directory.to_owned(),
            config,
            state: Mutex::new(state),
        };
        if found_temporary_event {
            spool.mark_loss(now_unix_ms())?;
        }
        Ok(spool)
    }

    pub fn publish(&self, event: &UpstreamCallEvent) -> Result<(), SpoolError> {
        let bytes = serde_json::to_vec(event).map_err(|_| SpoolError::Encode)?;
        if bytes.len() > self.config.entry_max_bytes {
            self.mark_loss(event.occurred_at_unix_ms)?;
            return Err(SpoolError::EntryTooLarge);
        }
        let temporary = self.directory.join(format!("{}.event.tmp", event.id));
        let final_path = self.directory.join(format!("{}.event.json", event.id));
        if final_path.exists() {
            return Ok(());
        }
        if write_synced_exclusive(&temporary, &bytes).is_err() {
            self.mark_loss(event.occurred_at_unix_ms)?;
            return Err(SpoolError::Io);
        }
        if std::fs::rename(&temporary, &final_path).is_err() {
            self.mark_loss(event.occurred_at_unix_ms)?;
            return Err(SpoolError::Io);
        }
        sync_directory(&self.directory).map_err(|_| SpoolError::Io)
    }

    pub fn pending_events(&self) -> Result<Vec<UpstreamCallEvent>, SpoolError> {
        let mut paths = std::fs::read_dir(&self.directory)
            .map_err(|_| SpoolError::Io)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().ends_with(".event.json"))
            })
            .collect::<Vec<_>>();
        paths.sort();
        paths
            .into_iter()
            .map(|path| {
                let bytes = std::fs::read(path).map_err(|_| SpoolError::Io)?;
                serde_json::from_slice(&bytes).map_err(|_| SpoolError::InvalidEntry)
            })
            .collect()
    }

    pub fn state(&self) -> NodeState {
        *self.state.lock().expect("state mutex is not poisoned")
    }

    pub fn write_state(&self, state: NodeState) -> Result<(), SpoolError> {
        let bytes = serde_json::to_vec(&state).map_err(|_| SpoolError::Encode)?;
        if bytes.len() > self.config.state_max_bytes {
            return Err(SpoolError::StateTooLarge);
        }
        let temporary = self.directory.join(STATE_TEMP_FILE);
        let final_path = self.directory.join(STATE_FILE);
        write_synced_replace(&temporary, &bytes)?;
        if final_path.exists() {
            std::fs::remove_file(&final_path).map_err(|_| SpoolError::Io)?;
        }
        std::fs::rename(&temporary, &final_path).map_err(|_| SpoolError::Io)?;
        sync_directory(&self.directory).map_err(|_| SpoolError::Io)?;
        *self.state.lock().expect("state mutex is not poisoned") = state;
        Ok(())
    }

    fn mark_loss(&self, occurred_at_unix_ms: u64) -> Result<(), SpoolError> {
        let mut state = self.state();
        state.incomplete_since_unix_ms = Some(
            state
                .incomplete_since_unix_ms
                .map_or(occurred_at_unix_ms, |value| value.min(occurred_at_unix_ms)),
        );
        state.most_recent_loss_unix_ms = Some(
            state
                .most_recent_loss_unix_ms
                .map_or(occurred_at_unix_ms, |value| value.max(occurred_at_unix_ms)),
        );
        state.incomplete_until_unix_ms = state
            .most_recent_loss_unix_ms
            .and_then(|value| value.checked_add(86_400_000));
        self.write_state(state)
    }
}

fn write_synced_exclusive(path: &Path, bytes: &[u8]) -> Result<(), SpoolError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| SpoolError::Io)?;
    file.write_all(bytes).map_err(|_| SpoolError::Io)?;
    file.sync_all().map_err(|_| SpoolError::Io)
}

fn write_synced_replace(path: &Path, bytes: &[u8]) -> Result<(), SpoolError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .map_err(|_| SpoolError::Io)?;
    file.write_all(bytes).map_err(|_| SpoolError::Io)?;
    file.sync_all().map_err(|_| SpoolError::Io)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(windows)]
fn sync_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
