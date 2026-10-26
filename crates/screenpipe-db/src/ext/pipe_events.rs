// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Pipe event-run claims (`pipe_event_runs`) and execution trigger records.
//!
//! The tables come from the schema v2.7.104 migrations
//! (`20260728000000_create_pipe_event_runs`, which also adds
//! `pipe_executions.trigger_event` / `trigger_key`, and
//! `20260805000000_add_pipe_execution_conversation_id`); nothing here changes
//! the schema.
//!
//! Every write runs in its own short `BEGIN IMMEDIATE` transaction on the
//! write pool (the read pool is `query_only`), so callers learn whether a
//! row actually changed: the claim is decided by `INSERT OR IGNORE` against
//! the `(pipe_name, event_name, event_key)` primary key, and the running
//! transition only applies to a row that is still queued or running.

use sqlx::Error as SqlxError;

use crate::DatabaseManager;

/// A `pipe_executions` row to insert (status `queued`).
#[derive(Debug, Clone, Copy)]
pub struct NewPipeExecution<'a> {
    pub pipe_name: &'a str,
    pub trigger_type: &'a str,
    pub model: &'a str,
    pub provider: Option<&'a str>,
    /// When the run was queued (RFC 3339); stored as `started_at`, which the
    /// running transition keeps.
    pub queued_at: &'a str,
    pub trigger_event: Option<&'a str>,
    pub trigger_key: Option<&'a str>,
}

impl DatabaseManager {
    /// Claim `(pipe, event, key)`. `true` when this call inserted the claim,
    /// `false` when it existed already.
    pub async fn claim_pipe_event_run(
        &self,
        pipe_name: &str,
        event_name: &str,
        event_key: &str,
        claimed_at: &str,
    ) -> Result<bool, SqlxError> {
        let mut tx = self.begin_immediate_with_retry().await?;
        let inserted = sqlx::query(
            "INSERT OR IGNORE INTO pipe_event_runs (pipe_name, event_name, event_key, claimed_at) \
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(pipe_name)
        .bind(event_name)
        .bind(event_key)
        .bind(claimed_at)
        .execute(&mut **tx.conn())
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(inserted == 1)
    }

    /// Delete one claim. `true` when it existed.
    pub async fn release_pipe_event_run(
        &self,
        pipe_name: &str,
        event_name: &str,
        event_key: &str,
    ) -> Result<bool, SqlxError> {
        let mut tx = self.begin_immediate_with_retry().await?;
        let deleted = sqlx::query(
            "DELETE FROM pipe_event_runs WHERE pipe_name = ?1 AND event_name = ?2 AND event_key = ?3",
        )
        .bind(pipe_name)
        .bind(event_name)
        .bind(event_key)
        .execute(&mut **tx.conn())
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(deleted > 0)
    }

    /// Delete claims made before `claimed_before` (RFC 3339). Timestamps are
    /// compared as instants, so claims written with another offset or
    /// precision are pruned on time too; a value SQLite cannot read as a
    /// time falls back to text order.
    pub async fn prune_pipe_event_runs(&self, claimed_before: &str) -> Result<u64, SqlxError> {
        let mut tx = self.begin_immediate_with_retry().await?;
        let deleted = sqlx::query(
            "DELETE FROM pipe_event_runs \
             WHERE julianday(claimed_at) < julianday(?1) \
                OR (julianday(claimed_at) IS NULL AND claimed_at < ?1)",
        )
        .bind(claimed_before)
        .execute(&mut **tx.conn())
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(deleted)
    }

    /// Whether a claim exists.
    pub async fn pipe_event_run_claimed(
        &self,
        pipe_name: &str,
        event_name: &str,
        event_key: &str,
    ) -> Result<bool, SqlxError> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM pipe_event_runs WHERE pipe_name = ?1 AND event_name = ?2 AND event_key = ?3",
        )
        .bind(pipe_name)
        .bind(event_name)
        .bind(event_key)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Insert a queued execution with the event that caused it. Returns the
    /// row id.
    pub async fn insert_pipe_execution(&self, new: NewPipeExecution<'_>) -> Result<i64, SqlxError> {
        let mut tx = self.begin_immediate_with_retry().await?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO pipe_executions \
             (pipe_name, status, trigger_type, model, provider, started_at, trigger_event, trigger_key) \
             VALUES (?1, 'queued', ?2, ?3, ?4, ?5, ?6, ?7) \
             RETURNING id",
        )
        .bind(new.pipe_name)
        .bind(new.trigger_type)
        .bind(new.model)
        .bind(new.provider)
        .bind(new.queued_at)
        .bind(new.trigger_event)
        .bind(new.trigger_key)
        .fetch_one(&mut **tx.conn())
        .await?;
        tx.commit().await?;
        Ok(id)
    }

    /// Move an execution to `running`. Only a row still `queued` or
    /// `running` with no `finished_at` moves, so a late call from a stale
    /// worker never reopens a cancelled or finished run. The PID is kept
    /// when `pid` is `None`, and an existing `started_at` is kept. Returns
    /// whether the row moved.
    pub async fn mark_pipe_execution_running(
        &self,
        id: i64,
        pid: Option<i64>,
        now: &str,
    ) -> Result<bool, SqlxError> {
        let mut tx = self.begin_immediate_with_retry().await?;
        let moved = sqlx::query(
            "UPDATE pipe_executions \
             SET status = 'running', pid = COALESCE(?1, pid), started_at = COALESCE(started_at, ?2) \
             WHERE id = ?3 AND status IN ('queued', 'running') AND finished_at IS NULL",
        )
        .bind(pid)
        .bind(now)
        .bind(id)
        .execute(&mut **tx.conn())
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(moved == 1)
    }

    /// Finish a run that never started: a `queued` row with no
    /// `finished_at` becomes `cancelled`. Returns whether it changed.
    pub async fn cancel_queued_pipe_execution(&self, id: i64, now: &str) -> Result<bool, SqlxError> {
        let mut tx = self.begin_immediate_with_retry().await?;
        let changed = sqlx::query(
            "UPDATE pipe_executions \
             SET status = 'cancelled', finished_at = ?1, error_type = 'cancelled', \
                 error_message = 'cancelled before it started', duration_ms = 0 \
             WHERE id = ?2 AND status = 'queued' AND finished_at IS NULL",
        )
        .bind(now)
        .bind(id)
        .execute(&mut **tx.conn())
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(changed == 1)
    }

    /// Record the chat an execution continued.
    pub async fn set_pipe_execution_conversation_id(
        &self,
        id: i64,
        conversation_id: &str,
    ) -> Result<bool, SqlxError> {
        let mut tx = self.begin_immediate_with_retry().await?;
        let changed = sqlx::query("UPDATE pipe_executions SET conversation_id = ?1 WHERE id = ?2")
            .bind(conversation_id)
            .bind(id)
            .execute(&mut **tx.conn())
            .await?
            .rows_affected();
        tx.commit().await?;
        Ok(changed == 1)
    }

    /// Turn an execution that `completed` without an error into a failure
    /// with `error_type` / `error_message`. Any other row is left alone.
    /// Returns whether it changed.
    pub async fn fail_completed_pipe_execution(
        &self,
        id: i64,
        error_type: &str,
        error_message: &str,
    ) -> Result<bool, SqlxError> {
        let mut tx = self.begin_immediate_with_retry().await?;
        let changed = sqlx::query(
            "UPDATE pipe_executions \
             SET status = 'failed', error_type = ?1, error_message = ?2 \
             WHERE id = ?3 AND status = 'completed' AND error_type IS NULL",
        )
        .bind(error_type)
        .bind(error_message)
        .bind(id)
        .execute(&mut **tx.conn())
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(changed == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    async fn db() -> DatabaseManager {
        DatabaseManager::new("sqlite::memory:", Default::default())
            .await
            .unwrap()
    }

    fn new_exec<'a>(key: Option<&'a str>, now: &'a str) -> NewPipeExecution<'a> {
        NewPipeExecution {
            pipe_name: "meeting-summary",
            trigger_type: "event",
            model: "m",
            provider: None,
            queued_at: now,
            trigger_event: key.map(|_| "meeting_ended"),
            trigger_key: key,
        }
    }

    #[tokio::test]
    async fn a_claim_is_granted_once_until_released() {
        let db = db().await;
        let now = Utc::now().to_rfc3339();
        assert!(db
            .claim_pipe_event_run("p", "meeting_ended", "7", &now)
            .await
            .unwrap());
        assert!(!db
            .claim_pipe_event_run("p", "meeting_ended", "7", &now)
            .await
            .unwrap());
        // Other pipes, events and keys are separate claims.
        assert!(db
            .claim_pipe_event_run("q", "meeting_ended", "7", &now)
            .await
            .unwrap());
        assert!(db
            .claim_pipe_event_run("p", "meeting_ended", "8", &now)
            .await
            .unwrap());
        assert!(db
            .pipe_event_run_claimed("p", "meeting_ended", "7")
            .await
            .unwrap());

        assert!(db
            .release_pipe_event_run("p", "meeting_ended", "7")
            .await
            .unwrap());
        assert!(!db
            .release_pipe_event_run("p", "meeting_ended", "7")
            .await
            .unwrap());
        assert!(!db
            .pipe_event_run_claimed("p", "meeting_ended", "7")
            .await
            .unwrap());
        assert!(db
            .claim_pipe_event_run("p", "meeting_ended", "7", &now)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn racing_claims_have_one_winner() {
        let db = std::sync::Arc::new(db().await);
        let now = Utc::now().to_rfc3339();
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let db = db.clone();
            let now = now.clone();
            tasks.push(tokio::spawn(async move {
                db.claim_pipe_event_run("p", "meeting_ended", "99", &now)
                    .await
                    .unwrap()
            }));
        }
        let mut winners = 0;
        for t in tasks {
            if t.await.unwrap() {
                winners += 1;
            }
        }
        assert_eq!(winners, 1);
    }

    #[tokio::test]
    async fn pruning_removes_only_old_claims() {
        let db = db().await;
        let old = (Utc::now() - Duration::days(40)).to_rfc3339();
        let old_z = (Utc::now() - Duration::days(35))
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string();
        let fresh = Utc::now().to_rfc3339();
        db.claim_pipe_event_run("p", "e", "old", &old).await.unwrap();
        db.claim_pipe_event_run("p", "e", "old-z", &old_z)
            .await
            .unwrap();
        db.claim_pipe_event_run("p", "e", "fresh", &fresh)
            .await
            .unwrap();
        let cutoff = (Utc::now() - Duration::days(30)).to_rfc3339();
        assert_eq!(db.prune_pipe_event_runs(&cutoff).await.unwrap(), 2);
        assert!(db.pipe_event_run_claimed("p", "e", "fresh").await.unwrap());
        assert!(!db.pipe_event_run_claimed("p", "e", "old").await.unwrap());
    }

    #[tokio::test]
    async fn executions_carry_their_trigger_and_the_running_transition_is_guarded() {
        let db = db().await;
        let now = Utc::now().to_rfc3339();
        let id = db
            .insert_pipe_execution(new_exec(Some("12"), &now))
            .await
            .unwrap();
        let row: (String, Option<String>, Option<String>, String) = sqlx::query_as(
            "SELECT status, trigger_event, trigger_key, trigger_type FROM pipe_executions WHERE id = ?1",
        )
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(
            row,
            (
                "queued".to_string(),
                Some("meeting_ended".to_string()),
                Some("12".to_string()),
                "event".to_string()
            )
        );

        assert!(db
            .mark_pipe_execution_running(id, Some(41), &now)
            .await
            .unwrap());
        // Running again (the PID arriving) is still allowed and keeps the PID
        // when none is given.
        assert!(db
            .mark_pipe_execution_running(id, None, &now)
            .await
            .unwrap());
        let pid: (Option<i64>,) = sqlx::query_as("SELECT pid FROM pipe_executions WHERE id = ?1")
            .bind(id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(pid.0, Some(41));

        // A cancelled queued row is never reopened by a stale worker.
        let queued = db.insert_pipe_execution(new_exec(None, &now)).await.unwrap();
        assert!(db.cancel_queued_pipe_execution(queued, &now).await.unwrap());
        assert!(!db.cancel_queued_pipe_execution(queued, &now).await.unwrap());
        assert!(!db
            .mark_pipe_execution_running(queued, Some(5), &now)
            .await
            .unwrap());
        let status: (String, Option<String>) =
            sqlx::query_as("SELECT status, error_type FROM pipe_executions WHERE id = ?1")
                .bind(queued)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(
            status,
            ("cancelled".to_string(), Some("cancelled".to_string()))
        );
        // A running row is not "queued", so the queued cancel leaves it alone.
        assert!(!db.cancel_queued_pipe_execution(id, &now).await.unwrap());
    }

    #[tokio::test]
    async fn only_clean_completions_are_turned_into_failures() {
        let db = db().await;
        let now = Utc::now().to_rfc3339();
        let id = db
            .insert_pipe_execution(new_exec(Some("3"), &now))
            .await
            .unwrap();
        // Still queued: not changed.
        assert!(!db
            .fail_completed_pipe_execution(id, "summary_not_saved", "x")
            .await
            .unwrap());
        sqlx::query(
            "UPDATE pipe_executions SET status = 'completed', finished_at = ?1 WHERE id = ?2",
        )
        .bind(&now)
        .bind(id)
        .execute(db.write_pool())
        .await
        .unwrap();
        assert!(db
            .fail_completed_pipe_execution(id, "summary_not_saved", "no summary")
            .await
            .unwrap());
        let row: (String, Option<String>) =
            sqlx::query_as("SELECT status, error_type FROM pipe_executions WHERE id = ?1")
                .bind(id)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(
            row,
            ("failed".to_string(), Some("summary_not_saved".to_string()))
        );
        // Already failed: a second pass changes nothing.
        assert!(!db
            .fail_completed_pipe_execution(id, "nothing_to_summarize", "y")
            .await
            .unwrap());

        assert!(db
            .set_pipe_execution_conversation_id(id, "chat-1")
            .await
            .unwrap());
        let conv: (Option<String>,) =
            sqlx::query_as("SELECT conversation_id FROM pipe_executions WHERE id = ?1")
                .bind(id)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(conv.0.as_deref(), Some("chat-1"));
    }
}
