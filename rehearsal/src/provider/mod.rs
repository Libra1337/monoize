mod model;
mod postgres;
mod preflight;
mod sqlite;
mod transform;

pub use model::{CanonicalDecimal, ModelKeys, PublicNameKey};
pub use postgres::{
    create_postgres_target_schema, migrate_postgres_provider_schema, postgres_table_exists,
};
pub use preflight::{
    PreflightReport, PreflightSource, ProviderPreflight, PublicName, build_preflight_report,
    canonical_json,
};
pub use sqlite::{
    MigrationFailurePoint, MigrationOutcome, create_sqlite_target_schema,
    migrate_sqlite_provider_schema, sqlite_table_exists,
};
pub use transform::{
    Classification, LegacyChannel, LegacyModel, LegacyProvider, PricingMode, TargetChannel,
    TargetModel, TargetProvider, TransformError, TransformResult, deterministic_id,
    transform_provider,
};
