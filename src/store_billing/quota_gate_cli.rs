use super::quota_gate::{GateSlot, QuotaGateError, QuotaGateStore, QuotaManifest};
use crate::db::DbPool;
use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;
use std::ffi::OsString;
use std::path::PathBuf;
use thiserror::Error;

const MAX_MANIFEST_BYTES: u64 = 65_536;

#[derive(Debug, Parser)]
#[command(name = "monoize-store-ops")]
struct StoreOpsCli {
    #[command(subcommand)]
    command: StoreOpsCommand,
}

#[derive(Debug, Subcommand)]
enum StoreOpsCommand {
    #[command(name = "quota-gate")]
    QuotaGate {
        #[command(subcommand)]
        command: QuotaGateCommand,
    },
}

#[derive(Debug, Subcommand)]
enum QuotaGateCommand {
    Import {
        #[arg(long, value_enum)]
        slot: GateSlotArg,
        #[arg(long)]
        manifest: PathBuf,
    },
    Promote {
        #[arg(long)]
        expected_fingerprint: String,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum GateSlotArg {
    Current,
    Next,
}

impl GateSlotArg {
    const fn into_gate_slot(self) -> GateSlot {
        match self {
            Self::Current => GateSlot::Current,
            Self::Next => GateSlot::Next,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Next => "next",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuotaGateCliOutput {
    pub operation: String,
    pub slot: Option<String>,
    pub compatibility_fingerprint: String,
    pub state: String,
}

#[derive(Debug, Error)]
pub enum QuotaGateCliError {
    #[error("MONOIZE_DATABASE_DSN is required")]
    DatabaseDsnMissing,
    #[error("MONOIZE_DATABASE_DSN must use SQLite")]
    DatabaseBackendInvalid,
    #[error("quota manifest exceeds 65536 bytes")]
    ManifestTooLarge,
    #[error("quota manifest read failed: {0}")]
    ManifestRead(String),
    #[error("quota manifest JSON is invalid: {0}")]
    ManifestJson(serde_json::Error),
    #[error("database connection failed: {0}")]
    DatabaseConnect(String),
    #[error(transparent)]
    Gate(#[from] QuotaGateError),
    #[error(transparent)]
    Arguments(#[from] clap::Error),
}

impl QuotaGateCliError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::DatabaseDsnMissing => "database_dsn_missing",
            Self::DatabaseBackendInvalid => "database_backend_invalid",
            Self::ManifestTooLarge => "quota_manifest_too_large",
            Self::ManifestRead(_) => "quota_manifest_read_failed",
            Self::ManifestJson(_) => "quota_manifest_invalid",
            Self::DatabaseConnect(_) => "database_connect_failed",
            Self::Gate(QuotaGateError::FingerprintConflict) => "quota_gate_fingerprint_conflict",
            Self::Gate(QuotaGateError::InvalidManifest) => "quota_manifest_invalid",
            Self::Gate(QuotaGateError::Storage(_)) => "quota_gate_storage_failed",
            Self::Gate(QuotaGateError::Serialize(_)) => "quota_manifest_invalid",
            Self::Arguments(_) => "arguments_invalid",
        }
    }
}

pub async fn execute_from<I, T>(
    arguments: I,
    database_dsn: Option<&str>,
) -> Result<QuotaGateCliOutput, QuotaGateCliError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = StoreOpsCli::try_parse_from(arguments)?;
    let dsn = database_dsn
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(QuotaGateCliError::DatabaseDsnMissing)?;
    if !dsn.starts_with("sqlite://") && !dsn.starts_with("sqlite::memory:") {
        return Err(QuotaGateCliError::DatabaseBackendInvalid);
    }

    match cli.command {
        StoreOpsCommand::QuotaGate { command } => execute_quota_gate(command, dsn).await,
    }
}

async fn execute_quota_gate(
    command: QuotaGateCommand,
    database_dsn: &str,
) -> Result<QuotaGateCliOutput, QuotaGateCliError> {
    match command {
        QuotaGateCommand::Import { slot, manifest } => {
            let metadata = std::fs::metadata(&manifest)
                .map_err(|error| QuotaGateCliError::ManifestRead(error.to_string()))?;
            if metadata.len() > MAX_MANIFEST_BYTES {
                return Err(QuotaGateCliError::ManifestTooLarge);
            }
            let bytes = std::fs::read(&manifest)
                .map_err(|error| QuotaGateCliError::ManifestRead(error.to_string()))?;
            if bytes.len() as u64 > MAX_MANIFEST_BYTES {
                return Err(QuotaGateCliError::ManifestTooLarge);
            }
            let manifest: QuotaManifest =
                serde_json::from_slice(&bytes).map_err(QuotaGateCliError::ManifestJson)?;
            let fingerprint = manifest.compatibility_fingerprint.clone();
            let db = connect_sqlite(database_dsn).await?;
            QuotaGateStore::new(db)
                .import_matching_manifest(slot.into_gate_slot(), manifest)
                .await?;
            Ok(QuotaGateCliOutput {
                operation: "import".to_string(),
                slot: Some(slot.as_str().to_string()),
                compatibility_fingerprint: fingerprint,
                state: "passed".to_string(),
            })
        }
        QuotaGateCommand::Promote {
            expected_fingerprint,
        } => {
            let expected_fingerprint = expected_fingerprint.trim();
            if expected_fingerprint.is_empty() {
                return Err(QuotaGateError::FingerprintConflict.into());
            }
            let db = connect_sqlite(database_dsn).await?;
            QuotaGateStore::new(db)
                .promote_next(expected_fingerprint)
                .await?;
            Ok(QuotaGateCliOutput {
                operation: "promote".to_string(),
                slot: None,
                compatibility_fingerprint: expected_fingerprint.to_string(),
                state: "passed".to_string(),
            })
        }
    }
}

async fn connect_sqlite(database_dsn: &str) -> Result<DbPool, QuotaGateCliError> {
    DbPool::connect(database_dsn)
        .await
        .map_err(|error| QuotaGateCliError::DatabaseConnect(error.to_string()))
}
