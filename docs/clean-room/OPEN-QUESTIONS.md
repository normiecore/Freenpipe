# Open questions

## W0-11

1. **Which sqlx 0.9 release, and which SQLite crates go with it?**
   Decision: sqlx 0.9.0 (stable, published 2026-05-21), libsqlite3-sys 0.37.0, rusqlite 0.39.0. The
   Tauri app crate uses the same three versions in its own lockfile.
   Reason: 0.9.0 is the only stable 0.9 release and is older than two weeks. sqlx-sqlite 0.9.0
   accepts libsqlite3-sys `>=0.30.1, <0.38.0`, so 0.37.0 is the newest it can share, and rusqlite
   0.39.0 is the last rusqlite on 0.37 (0.40 moves to libsqlite3-sys 0.38, which would put a second
   `links = "sqlite3"` crate in the graph). libsqlite3-sys 0.37.0 bundles SQLite 3.51.3, source id
   `2026-03-13 10:38:09 737ae4a3...`, the same date the spec records for the target. These are
   also the versions the spec lists for the target (db-029).

2. **sqlx 0.9.0 declares `rust-version = 1.94.0`; the repository pinned Rust 1.93.1.**
   Decision: move `rust-toolchain.toml` to 1.94.1 (the last 1.94 patch, 2026-03-26) and the app
   crate's `rust-version` to 1.94. bootstrap.sh's fallback channel, the generated cloud setup script
   and windows-build.md follow.
   Reason: cargo refuses to build a package whose `rust-version` is newer than the active compiler.
   `--ignore-rust-version` would have to be passed on every cargo invocation (it has no config key),
   which is not workable for CI, tauri builds or contributors.

3. **What does "recover extension compiled in" require from this WP?**
   Decision: the single bundled SQLite is built with `SQLITE_ENABLE_DBPAGE_VTAB` (through
   `LIBSQLITE3_FLAGS` in `.cargo/config.toml`), which is the core feature the recover extension
   reads pages through; FTS5 is already enabled by libsqlite3-sys's bundled build. The extension's
   own C sources (`sqlite3recover.c`, `dbdata.c`) are not part of the amalgamation and are left to
   W4-09's `screenpipe-sqlite-recovery` crate, which can compile them against the same headers via
   the `DEP_SQLITE3_INCLUDE` variable that libsqlite3-sys exports.
   Reason: the target ships page-level recovery as its own crate (db-020); compiling the extension
   here as well would give W4-09 duplicate C symbols to untangle. A test pins the prerequisite
   (`sqlite_dbpage` must be queryable and `PRAGMA compile_options` must list `ENABLE_DBPAGE_VTAB`).
   A build started from outside the repository tree does not read `.cargo/config.toml`; that test
   is what catches it.
   Note for W4-09: libsqlite3-sys 0.37.0 ships only `sqlite3.c`, `sqlite3.h` and `sqlite3ext.h`.
   Cargo hands `DEP_SQLITE3_INCLUDE` only to build scripts of crates that depend on libsqlite3-sys
   directly, so the recovery crate must list it itself (`{ workspace = true }`). The matching
   public-domain sources are `ext/recover/sqlite3recover.{c,h}` and `ext/misc/dbdata.c` at tag
   `version-3.51.3` of the official GitHub mirror `sqlite/sqlite`; www.sqlite.org is not reachable
   through the cloud VM's proxy, the GitHub mirror is.

4. **What replaces sqlx's removed `runtime-tokio-rustls` feature?**
   Decision: `runtime-tokio` with no TLS backend, declared once in `[workspace.dependencies]`.
   Reason: only the SQLite driver is used and it never opens a network connection. Dropping the TLS
   feature also removes rustls 0.21 and its webpki stack from the lockfile.

5. **parquet, zstd, sha2 and rusqlite are added to screenpipe-db before any code uses them.**
   Decision: parquet 59.3.0 with default features off and only `zstd` on (no arrow, no other
   codecs), zstd 0.13 (the major parquet 59 uses, so one zstd-sys build), sha2 0.10.9 (already in
   the lockfile), rusqlite with `vtab`. They are listed in `[package.metadata.cargo-machete]
   ignored` until the storage module (W5-07) and the virtual tables (db-022) use them.
   Reason: the plan asks for them in this WP so the SQLite toolchain is settled in one change. The
   lean parquet feature set keeps build time down; W5-07 can enable `arrow` if its writer needs it.
   parquet 59.3.1 and the newest twox-hash and zstd-sys releases were younger than two weeks, so the
   lockfiles pin parquet 59.3.0, twox-hash 2.1.4 and zstd-sys 2.1.0+zstd.1.5.7.

6. **The bundled SQLite makes `PRAGMA quick_check` run FTS5 integrity checks.**
   Decision: keep the startup `quick_check(1)` for file databases (db-016 keeps it in the target,
   which links the same SQLite), and skip it for in-memory databases.
   Reason: since SQLite 3.44, integrity_check and quick_check call each virtual table's xIntegrity,
   and FTS5 ignores the quick flag. On the shared-cache connections sqlx opens for `sqlite::memory:`
   pools, that check never returns with 3.51.3 (reproduced with a one-row FTS5 table); file
   databases are unaffected. In-memory databases have no file that could be damaged, and the
   hang froze screenpipe-db's timeline performance test ten seconds after startup. On large file
   databases the check now also walks the FTS5 indexes, so it takes longer; it still runs in the
   background.
