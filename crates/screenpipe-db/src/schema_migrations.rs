// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! The embedded sqlx migration set, and how a database moves between Freenpipe
//! and screenpipe v2.7.104 without either side re-running or rejecting
//! migrations.
//!
//! The 89 migrations inherited from the base are byte-identical to the ones
//! screenpipe embeds, so their stored checksums agree already. The 28 versions
//! that make up the v2.7.104 schema are shipped under screenpipe's versions,
//! file names and DDL, but with comments of our own, so the SHA-384 of our files
//! is not what screenpipe records for those versions. sqlx refuses to start when
//! a stored checksum differs from the embedded one. For these 28 versions the
//! migrator therefore carries screenpipe's checksum instead of the hash of our
//! file:
//!
//! * a database screenpipe migrated opens without any repair, and none of the
//!   versions it already applied run again;
//! * a database Freenpipe migrated records screenpipe's checksums, so screenpipe
//!   opens it without a repair either.
//!
//! `DatabaseManager::run_migrations` keeps the base's repair fallback (rewrite
//! the stored checksums, then retry) for databases from builds whose text and
//! checksums differ again, and `set_ignore_missing(true)` so a database that a
//! newer build migrated further still opens.

use std::borrow::Cow;

use sqlx::migrate::Migrator;
use sqlx::SqlitePool;
use tracing::debug;

/// Newest migration of the base (upstream 892199f) schema.
pub const BASE_SCHEMA_VERSION: i64 = 20260603120000;

/// Newest migration of the screenpipe v2.7.104 schema.
pub const TARGET_SCHEMA_VERSION: i64 = 20261002190000;

/// Rebuilds `frames_fts` as an external-content index (slow on big databases).
pub const FRAMES_FTS_EXTERNAL_CONTENT_VERSION: i64 = 20260415000000;

/// Folds `ocr_text` into `frames` and drops it (slow on big databases).
pub const OCR_TEXT_RETIREMENT_VERSION: i64 = 20260613130000;

/// Migrations that rewrite every frame row, each with the notice logged
/// before it runs on a database that already holds frames.
pub const SLOW_MIGRATION_NOTICES: &[(i64, &str)] = &[
    (
        FRAMES_FTS_EXTERNAL_CONTENT_VERSION,
        "migrating frames_fts search index, this may take a few minutes on large databases...",
    ),
    (
        OCR_TEXT_RETIREMENT_VERSION,
        "folding the retired ocr_text table (OCR text and word boxes) into frames; \
         with a long capture history this step can run for several minutes...",
    ),
];

/// A migration of the v2.7.104 schema as screenpipe records it in
/// `_sqlx_migrations`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpstreamMigration {
    pub version: i64,
    /// sqlx's description, derived from the file name.
    pub description: &'static str,
    /// SHA-384 of screenpipe's migration text, lowercase hex.
    pub sha384: &'static str,
}

/// The 28 migrations the v2.7.104 schema adds to the base, in version order.
pub const UPSTREAM_MIGRATIONS: &[UpstreamMigration] = &[
    UpstreamMigration {
        version: 20260601000000,
        description: "create outputs",
        sha384: "30d19eaa97f4b767c909049454ce37c11e9b9a5696230dfbb244c0cc3a603b36c48d2c3ee1c8e18bd4d21826b2906240",
    },
    UpstreamMigration {
        version: 20260612000000,
        description: "add saf columns to outputs",
        sha384: "62e96663f0416322aaedf798be8e2b0ce9a4488380394c186c85599daf90fff8211648d278116003e22e6d944a619433",
    },
    UpstreamMigration {
        version: 20260613000000,
        description: "add elements redacted at",
        sha384: "5edabb504f7815c62a4efe7d9b596396dc322eb67947643cb851fb72137595d99fe3eaad57e97f07ba506e9ea33e2eba",
    },
    UpstreamMigration {
        version: 20260613000001,
        description: "add frames full text redacted at",
        sha384: "81612f52543f2ecfb2fbc41b1ab830dd23e079d60dd5ec1174cd0e623f63f9a49bdf815dccee73ef366995cef0f0a555",
    },
    UpstreamMigration {
        version: 20260613130000,
        description: "unify ocr text into frames",
        sha384: "302fcffa07df07ebb6b33e9f3719d949dbf90c3d503723e74040180912b145c9c48545fedac5b48555248ac4e1e6c5dd",
    },
    UpstreamMigration {
        version: 20260616120000,
        description: "speaker filter autocomplete indexes",
        sha384: "1767ab1e3e5abe08adcd147b525c42d036a82db92c29e55f35d7c63e33c9a8c1f25ccc16eee7d0a42be2de16f9dd0ce3",
    },
    UpstreamMigration {
        version: 20260618120000,
        description: "frames app name timestamp index",
        sha384: "364a179becd5fda84dc485d96573ccc653a856f58708a8322a5d7cf965de8a1d3a3ed1d58c79400a77f91fca4b33d35a",
    },
    UpstreamMigration {
        version: 20260619000000,
        description: "add frames derived text redacted at",
        sha384: "b4e49096aa88e2a5a6ca8b6152ae069999569c28a736b0bdcc760b7ec345764cc10f159c5e284a9e6d9ceaeafae9d4b0",
    },
    UpstreamMigration {
        version: 20260619120000,
        description: "add frames browser url redacted at",
        sha384: "43dd4916e058cdf9a23d3237274b30ce48dc0d577e608a98e1f9f1cbd0bca54e34210403b5923577132be0da81669846",
    },
    UpstreamMigration {
        version: 20260622000000,
        description: "create output search index",
        sha384: "fb70062178eb58c2b0581319e5f8c4dd83bf5cd424e80950ae17b2c7920333ff69be69529e8ce4abad5a282ea31cc8f7",
    },
    UpstreamMigration {
        version: 20260627000000,
        description: "add frames text json redacted at",
        sha384: "aedd5fe505370dbcf129aede33110b76da05d12dee82a9522d341054598fb2eb67c54fe103a846a439532acfdb736fb9",
    },
    UpstreamMigration {
        version: 20260702000000,
        description: "add frames snapshot pending index",
        sha384: "36e81f295d9c9e63cd8389bf29df6a7d251ae54c975296eaf873485a8f8149247a332b27a456a1e6f07324dd9a981251",
    },
    UpstreamMigration {
        version: 20260702130000,
        description: "capture action fidelity",
        sha384: "ea637eb9895076823c2f37b580efcae4c3258211b95212975b9d11fe5dc24c26d315401f4749b522b1a69e0c7e07681d",
    },
    UpstreamMigration {
        version: 20260709000000,
        description: "make redacted at indexes partial",
        sha384: "467d6aa5192816c666fe47185957023083b27405e15920169f9dc3704bfc243826aaef713fe4fe44cacd5222e8ab2b9e",
    },
    UpstreamMigration {
        version: 20260719000000,
        description: "index meeting transcript segments captured at",
        sha384: "28fd070c5d46cab85d19e4772ae805c1df00a53e9385b22da444acad6c631d1db869885fe97f19e0e3ccdbe0b4c71e5d",
    },
    UpstreamMigration {
        version: 20260724000000,
        description: "create semantic context tables",
        sha384: "d7bc7b69094021dd0d5a4cb6b5b4283397b47ba2dfcb467d62ca35a771e2407504936e660014d43dd99834f7cd2cd482",
    },
    UpstreamMigration {
        version: 20260727000000,
        description: "create semantic actors",
        sha384: "af1bd0dcc6968d575f42c5e86f87da756378e46a5c4df60e9c6934cef3da4371c55a877c15accb11f632956f83b06ab8",
    },
    UpstreamMigration {
        version: 20260728000000,
        description: "create pipe event runs",
        sha384: "c71ca8f1f0522c20326f9380ef77cd56c6473d775f516b6475e31b4cb26ea801ab013b28a78fec2c4533c6ca95e3c3d2",
    },
    UpstreamMigration {
        version: 20260729130000,
        description: "index outputs original path",
        sha384: "0f419ca2729c99b515535dfc2531632c80c5df4cb8e448598b60a4848ccc45d16c68a436a40e61bd9121cf42ba74779f",
    },
    UpstreamMigration {
        version: 20260730180000,
        description: "create feedback",
        sha384: "056701af08e132d9f79656b32de4204bc28d79644482297e4bf837443dd778d1a5e54e8a47a0ca10eedbb527bed74b93",
    },
    UpstreamMigration {
        version: 20260805000000,
        description: "add pipe execution conversation id",
        sha384: "33cbe2af6c5bdf6f96e47ea23f2c3132c9fe6b7915cf196cc09bfceb75ac8b27a7d68590492be60ef44df1e237dadfca",
    },
    UpstreamMigration {
        version: 20260812120000,
        description: "restore orphaned speaker references",
        sha384: "d3a963d755335c8dd3e44090e4b9c356b17ac582739a78a92e9dea4c755dd8064264da55b2514f996c55af89456272be",
    },
    UpstreamMigration {
        version: 20260813000000,
        description: "add calendar event id to meetings",
        sha384: "0dd9db9c16d1f956d66acef81f502d1410091b3b231a3053ca206dcd6bd3208aa773561d5090c3381d4fd740e39ff5fd",
    },
    UpstreamMigration {
        version: 20260817000000,
        description: "create activity ledger",
        sha384: "820278e6c67ab85c38315d6378e53bf2aa853700a524299501768f332886ff6213a36aefe05122ba5bf577604d0a66fd",
    },
    UpstreamMigration {
        version: 20260817160000,
        description: "rebuild activity ledger for week backfill",
        sha384: "897c85164aa712d73d83ea1ec0888bbdbe969e525a3da862db8af4e1ba08cdf2fca2414fb1a63d9daa714d7d476a5b43",
    },
    UpstreamMigration {
        version: 20260818133000,
        description: "activity ledger query indexes",
        sha384: "87b24280229975913f1ac27476f7b5bf097586e7f2ba4e2f79bda3db794ccc762a5311d73af76e61a7574263bbc8eefb",
    },
    UpstreamMigration {
        version: 20260828143000,
        description: "scope live meeting speakers",
        sha384: "61087cd7c19be5e1ba990d6f0cc1f2a13d7b1e2ff3cd0fa659bea28e3098f58f89759aab5233b8d8411371c3a2968b7a",
    },
    UpstreamMigration {
        version: 20261002190000,
        description: "starred sessions",
        sha384: "788d49f20afbf8325938e183d8e81bea449bd55775f512512702c4b44a7059dac3ceab3512eed68e9c01920a27eca2d0",
    },
];

/// Screenpipe's stored checksum for `version`, when it is one of the
/// v2.7.104 migrations.
pub fn upstream_checksum(version: i64) -> Option<Vec<u8>> {
    UPSTREAM_MIGRATIONS
        .iter()
        .find(|migration| migration.version == version)
        .map(|migration| decode_hex(migration.sha384))
}

/// The migrator `DatabaseManager` runs: every embedded migration, with
/// screenpipe's checksum in place of ours for the v2.7.104 versions, and
/// applied versions this build does not know about tolerated.
pub fn migrator() -> Migrator {
    let embedded = sqlx::migrate!("./src/migrations");
    let migrations = embedded
        .iter()
        .cloned()
        .map(|mut migration| {
            if let Some(checksum) = upstream_checksum(migration.version) {
                migration.checksum = Cow::Owned(checksum);
            }
            migration
        })
        .collect();
    let mut migrator = Migrator::with_migrations(migrations);
    migrator.set_ignore_missing(true);
    migrator
}

/// The notices of `SLOW_MIGRATION_NOTICES` that apply before `migrator` runs
/// on `pool`: the migration is embedded, not applied yet, and there are frames
/// for it to work through.
pub async fn pending_slow_migration_notices(
    pool: &SqlitePool,
    migrator: &Migrator,
) -> Vec<&'static str> {
    let mut notices = Vec::new();
    for (version, notice) in SLOW_MIGRATION_NOTICES {
        if migrator.version_exists(*version) && pending_with_frames(pool, *version).await {
            notices.push(*notice);
        }
    }
    notices
}

/// True when `version` has not been applied yet and the database already
/// holds frames. A database with frames but no migrations table (a very old
/// schema) counts as pending. A query error, which includes a fresh database
/// without a `frames` table, means no notice.
async fn pending_with_frames(pool: &SqlitePool, version: i64) -> bool {
    match sqlx::query_scalar::<_, i64>("SELECT 1 FROM frames LIMIT 1")
        .fetch_optional(pool)
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => return false,
        Err(e) => {
            debug!("could not inspect existing frames before migrate: {}", e);
            return false;
        }
    }

    let has_migrations_table = match sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_one(pool)
    .await
    {
        Ok(count) => count > 0,
        Err(e) => {
            debug!("could not inspect _sqlx_migrations before migrate: {}", e);
            return false;
        }
    };
    if !has_migrations_table {
        return true;
    }

    match sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM _sqlx_migrations WHERE version = ?")
        .bind(version)
        .fetch_one(pool)
        .await
    {
        Ok(count) => count == 0,
        Err(e) => {
            debug!("could not inspect applied migrations before migrate: {}", e);
            false
        }
    }
}

/// Decodes the checksum table's hex. The table is a compile-time constant
/// checked by the tests below, so malformed input is a programming error.
fn decode_hex(hex: &str) -> Vec<u8> {
    fn nibble(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => panic!("checksum table holds a non-hex character: {}", c as char),
        }
    }
    assert!(hex.len() % 2 == 0, "checksum hex has an odd length");
    hex.as_bytes()
        .chunks_exact(2)
        .map(|pair| (nibble(pair[0]) << 4) | nibble(pair[1]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_table_is_sorted_unique_and_well_formed() {
        assert_eq!(UPSTREAM_MIGRATIONS.len(), 28);
        for pair in UPSTREAM_MIGRATIONS.windows(2) {
            assert!(pair[0].version < pair[1].version, "{:?}", pair);
        }
        for migration in UPSTREAM_MIGRATIONS {
            assert_eq!(migration.sha384.len(), 96, "{}", migration.version);
            assert_eq!(decode_hex(migration.sha384).len(), 48);
        }
        assert_eq!(
            UPSTREAM_MIGRATIONS.last().map(|m| m.version),
            Some(TARGET_SCHEMA_VERSION)
        );
    }

    #[test]
    fn decode_hex_reads_bytes() {
        assert_eq!(decode_hex("00ff7a"), vec![0x00, 0xff, 0x7a]);
        assert!(upstream_checksum(BASE_SCHEMA_VERSION).is_none());
    }

    #[test]
    fn every_upstream_version_is_embedded_under_its_upstream_name() {
        let migrator = migrator();
        for upstream in UPSTREAM_MIGRATIONS {
            let embedded = migrator
                .iter()
                .find(|m| m.version == upstream.version)
                .unwrap_or_else(|| panic!("{} is not embedded", upstream.version));
            assert_eq!(embedded.description, upstream.description);
            assert_eq!(
                embedded.checksum.as_ref(),
                decode_hex(upstream.sha384).as_slice(),
                "{} does not carry the upstream checksum",
                upstream.version
            );
        }
    }

    async fn memory_pool() -> SqlitePool {
        sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap()
    }

    async fn exec(pool: &SqlitePool, sql: &'static str) {
        sqlx::raw_sql(sql).execute(pool).await.unwrap();
    }

    #[tokio::test]
    async fn no_slow_migration_notice_for_a_fresh_database() {
        let pool = memory_pool().await;
        assert!(pending_slow_migration_notices(&pool, &migrator())
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn no_slow_migration_notice_without_frames() {
        let pool = memory_pool().await;
        exec(&pool, "CREATE TABLE frames (id INTEGER PRIMARY KEY)").await;
        assert!(pending_slow_migration_notices(&pool, &migrator())
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn slow_migration_notices_follow_the_applied_versions() {
        let pool = memory_pool().await;
        exec(
            &pool,
            "CREATE TABLE frames (id INTEGER PRIMARY KEY); INSERT INTO frames (id) VALUES (1);",
        )
        .await;
        let migrator = migrator();

        // Frames but no bookkeeping table: a very old schema, both pending.
        assert_eq!(
            pending_slow_migration_notices(&pool, &migrator).await,
            vec![SLOW_MIGRATION_NOTICES[0].1, SLOW_MIGRATION_NOTICES[1].1]
        );

        // A base-schema database: the FTS rebuild is done, the ocr_text fold is not.
        exec(
            &pool,
            "CREATE TABLE _sqlx_migrations (version BIGINT PRIMARY KEY); \
             INSERT INTO _sqlx_migrations (version) VALUES (20260415000000);",
        )
        .await;
        let notices = pending_slow_migration_notices(&pool, &migrator).await;
        assert_eq!(notices, vec![SLOW_MIGRATION_NOTICES[1].1]);
        assert!(notices[0].contains("ocr_text"));

        // Fully migrated: nothing to announce.
        exec(
            &pool,
            "INSERT INTO _sqlx_migrations (version) VALUES (20260613130000);",
        )
        .await;
        assert!(pending_slow_migration_notices(&pool, &migrator)
            .await
            .is_empty());
    }

    #[test]
    fn migrator_holds_the_full_target_set() {
        let migrator = migrator();
        let versions: Vec<i64> = migrator.iter().map(|m| m.version).collect();
        assert_eq!(versions.len(), 117);
        assert!(versions.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(versions.last(), Some(&TARGET_SCHEMA_VERSION));
        let base = versions
            .iter()
            .filter(|v| upstream_checksum(**v).is_none())
            .count();
        assert_eq!(base, 89);
        assert!(versions
            .iter()
            .filter(|v| upstream_checksum(**v).is_none())
            .all(|v| *v <= BASE_SCHEMA_VERSION));
        assert!(versions.contains(&FRAMES_FTS_EXTERNAL_CONTENT_VERSION));
        assert!(versions.contains(&OCR_TEXT_RETIREMENT_VERSION));
    }
}
