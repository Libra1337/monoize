use super::{Spool, SpoolError, UpstreamCallEvent};
use sqlx::{Connection, Row, SqliteConnection};

#[derive(Clone, Copy, Debug, Default)]
pub struct StatusSink;

impl StatusSink {
    pub async fn create_sqlite(db: &mut SqliteConnection) -> Result<Self, sqlx::Error> {
        sqlx::query(
            r#"CREATE TABLE upstream_call_events (
                id TEXT PRIMARY KEY,
                group_id TEXT NOT NULL,
                provider_id TEXT NOT NULL,
                channel_id TEXT NOT NULL,
                outcome TEXT NOT NULL CHECK(outcome IN ('success', 'failure')),
                failure_class TEXT NULL CHECK(failure_class IS NULL OR failure_class IN ('rate_limited', 'transient', 'persistent')),
                upstream_status INTEGER NULL,
                occurred_at_unix_ms INTEGER NOT NULL,
                source_node_id TEXT NOT NULL,
                provider_generation INTEGER NOT NULL
            )"#,
        )
        .execute(db)
        .await?;
        Ok(Self)
    }

    pub async fn insert_sqlite(
        &self,
        db: &mut SqliteConnection,
        events: &[UpstreamCallEvent],
    ) -> Result<u64, sqlx::Error> {
        let mut transaction = db.begin().await?;
        let mut inserted = 0;
        for event in events.iter().take(100) {
            let failure_class = event.failure_class.map(|value| match value {
                super::FailureClass::RateLimited => "rate_limited",
                super::FailureClass::Transient => "transient",
                super::FailureClass::Persistent => "persistent",
            });
            inserted += sqlx::query(
                "INSERT OR IGNORE INTO upstream_call_events (id, group_id, provider_id, channel_id, outcome, failure_class, upstream_status, occurred_at_unix_ms, source_node_id, provider_generation) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&event.id)
            .bind(&event.group_id)
            .bind(&event.provider_id)
            .bind(&event.channel_id)
            .bind(&event.outcome)
            .bind(failure_class)
            .bind(event.upstream_status.map(i64::from))
            .bind(i64::try_from(event.occurred_at_unix_ms).map_err(|error| sqlx::Error::Protocol(error.to_string()))?)
            .bind(&event.source_node_id)
            .bind(i64::try_from(event.provider_generation).map_err(|error| sqlx::Error::Protocol(error.to_string()))?)
            .execute(&mut *transaction)
            .await?
            .rows_affected();
        }
        transaction.commit().await?;
        Ok(inserted)
    }

    pub async fn count_sqlite(&self, db: &mut SqliteConnection) -> Result<u64, sqlx::Error> {
        let row = sqlx::query("SELECT COUNT(*) AS count FROM upstream_call_events")
            .fetch_one(db)
            .await?;
        u64::try_from(row.try_get::<i64, _>("count")?)
            .map_err(|error| sqlx::Error::Protocol(error.to_string()))
    }

    pub async fn drain_sqlite(
        &self,
        db: &mut SqliteConnection,
        spool: &Spool,
        limit: usize,
    ) -> Result<u64, DrainError> {
        let events = spool.pending_events().map_err(DrainError::Spool)?;
        let selected = events.into_iter().take(limit.min(100)).collect::<Vec<_>>();
        self.insert_sqlite(db, &selected)
            .await
            .map_err(DrainError::Database)?;
        for event in &selected {
            spool
                .delete_committed(&event.id)
                .map_err(DrainError::Spool)?;
        }
        Ok(selected.len() as u64)
    }
}

#[derive(Debug)]
pub enum DrainError {
    Database(sqlx::Error),
    Spool(SpoolError),
}
