// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! With the maintenance intervals shortened through their environment
//! variables, an owning `DatabaseManager` runs its WAL checkpoint loop and
//! writes `sqlite-planner-maintenance.json` next to the database by itself.
//!
//! The variables are process-wide, so this file holds a single test.

use std::time::{Duration, Instant};

use screenpipe_config::DbConfig;
use screenpipe_db::maintenance::{CheckpointMode, WAL_MAINTENANCE_INTERVAL_ENV};
use screenpipe_db::planner_stats::{
    read_report, status, PLANNER_MAINTENANCE_INTERVAL_ENV, PLANNER_REPORT_FILE,
};
use screenpipe_db::DatabaseManager;

#[tokio::test]
async fn shortened_intervals_run_wal_and_planner_maintenance() {
    std::env::set_var(WAL_MAINTENANCE_INTERVAL_ENV, "1");
    std::env::set_var(PLANNER_MAINTENANCE_INTERVAL_ENV, "1");

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let db = DatabaseManager::new(path.to_str().unwrap(), DbConfig::default())
        .await
        .unwrap();
    for i in 0..50 {
        sqlx::query("INSERT INTO tags (name) VALUES (?1)")
            .bind(format!("t{i}"))
            .execute(db.write_pool())
            .await
            .unwrap();
    }

    let report_path = dir.path().join(PLANNER_REPORT_FILE);
    let deadline = Instant::now() + Duration::from_secs(20);
    let report = loop {
        let report = read_report(&report_path);
        if report.last_success.is_some() {
            break report;
        }
        assert!(
            Instant::now() < deadline,
            "no successful planner run recorded: {report:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let latest = report.latest.expect("latest attempt");
    assert!(
        latest.status == status::SUCCEEDED || latest.status == status::RUNNING,
        "{latest:?}"
    );
    let success = report.last_success.unwrap();
    assert!(!success.analyzed_tables.is_empty(), "{success:?}");
    let stats: i64 = sqlx::query_scalar("SELECT count(*) FROM sqlite_stat1")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert!(stats > 0);

    let deadline = Instant::now() + Duration::from_secs(10);
    let wal = loop {
        let wal = db.wal_maintenance_status();
        if wal.checkpoints > 0 {
            break wal;
        }
        assert!(Instant::now() < deadline, "no WAL checkpoint ran: {wal:?}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert!(wal.running);
    assert_eq!(wal.last_mode, Some(CheckpointMode::Passive));
}
