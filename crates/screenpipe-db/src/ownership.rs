// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! One live owning [`crate::DatabaseManager`] per database file.
//!
//! Two guards back the rule:
//!
//! - **In this process**, a registry of database paths with a live owner. A
//!   second owning open of the same file waits briefly for the first to be
//!   dropped (an engine restart retires the old generation first) and is then
//!   refused.
//! - **Across processes**, the sidecar `<db>.owner` (for `db.sqlite` that is
//!   `db.sqlite.owner`). The owner holds an exclusive OS lock on it for its
//!   whole lifetime and writes its pid into it for diagnostics. The OS drops
//!   the lock when the process exits, however it exits, so a file left behind
//!   by a crash is taken over at once: nothing ever has to be deleted by hand.
//!   The file's content is never trusted for liveness, which also makes a
//!   sidecar in another format (written by screenpipe) harmless.
//!
//! A file system without lock support (some network shares) degrades to the
//! in-process guard with a warning instead of refusing the database.
//!
//! The sidecar is left in place when the owner closes; deleting it while
//! another process may be opening it would let two owners lock two different
//! files of the same name.

use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

/// Suffix appended to the database file name to name its ownership sidecar.
pub const OWNER_FILE_SUFFIX: &str = ".owner";

/// How long an owning open waits for an earlier owner in this process.
pub const IN_PROCESS_RELEASE_WAIT: Duration = Duration::from_secs(10);

/// Path of the ownership sidecar for `database`.
pub fn owner_file_path(database: &Path) -> PathBuf {
    let mut name = database.as_os_str().to_os_string();
    name.push(OWNER_FILE_SUFFIX);
    PathBuf::from(name)
}

/// What an owner writes into its sidecar. Informational only.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OwnerRecord {
    pub pid: u32,
    #[serde(default)]
    pub executable: Option<String>,
    #[serde(default)]
    pub acquired_at_unix_ms: u64,
    #[serde(default)]
    pub database_path: String,
}

/// Why an owning open was refused.
#[derive(Debug)]
pub enum OwnershipError {
    /// This process already holds a live owner for the database.
    HeldInThisProcess { database: PathBuf },
    /// Another running process owns the database.
    HeldByOtherProcess {
        database: PathBuf,
        pid: Option<u32>,
    },
    /// The sidecar could not be opened or written.
    OwnerFile { path: PathBuf, error: std::io::Error },
}

impl std::fmt::Display for OwnershipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OwnershipError::HeldInThisProcess { database } => write!(
                f,
                "database {} is already open in this process by an earlier DatabaseManager \
                 that is still alive; it must be dropped before the database is opened again",
                database.display()
            ),
            OwnershipError::HeldByOtherProcess { database, pid } => {
                write!(f, "database {} is owned by another running process", database.display())?;
                if let Some(pid) = pid {
                    write!(f, " (pid {pid})")?;
                }
                write!(
                    f,
                    "; quit that process before recording into this database or changing its storage"
                )
            }
            OwnershipError::OwnerFile { path, error } => write!(
                f,
                "cannot open the database ownership file {}: {}",
                path.display(),
                error
            ),
        }
    }
}

impl std::error::Error for OwnershipError {}

impl OwnershipError {
    pub fn into_sqlx(self) -> sqlx::Error {
        sqlx::Error::Configuration(Box::new(self))
    }
}

/// Whether `err` is an ownership refusal from [`acquire`].
pub fn is_ownership_error(err: &sqlx::Error) -> bool {
    match err {
        sqlx::Error::Configuration(source) => source.downcast_ref::<OwnershipError>().is_some(),
        _ => false,
    }
}

fn live_owners() -> &'static Mutex<HashSet<PathBuf>> {
    static OWNERS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    OWNERS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Normalised registry key for a database path: canonical when the file
/// exists, else the path made absolute.
fn registry_key(database: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(database) {
        return canonical;
    }
    if database.is_absolute() {
        database.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(database))
            .unwrap_or_else(|_| database.to_path_buf())
    }
}

/// Held by an owning `DatabaseManager`; releases both guards when dropped.
#[derive(Debug)]
pub struct OwnershipGuard {
    key: PathBuf,
    /// The locked sidecar. `None` when the file system cannot lock files.
    file: Option<File>,
}

impl OwnershipGuard {
    /// The database this guard owns.
    pub fn database(&self) -> &Path {
        &self.key
    }

    /// Whether the cross-process lock is held (false on file systems without
    /// lock support).
    pub fn holds_os_lock(&self) -> bool {
        self.file.is_some()
    }
}

impl Drop for OwnershipGuard {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            if let Err(e) = file.unlock() {
                debug!("releasing the database ownership lock: {}", e);
            }
        }
        if let Ok(mut owners) = live_owners().lock() {
            owners.remove(&self.key);
        }
        debug!(db = %self.key.display(), "database ownership released");
    }
}

/// Claim ownership of `database`, waiting up to `wait` for an earlier owner in
/// this process to go away.
pub async fn acquire(database: &Path, wait: Duration) -> Result<OwnershipGuard, sqlx::Error> {
    let key = registry_key(database);
    let deadline = Instant::now() + wait;
    let mut logged_wait = false;
    loop {
        let inserted = live_owners()
            .lock()
            .map(|mut owners| owners.insert(key.clone()))
            .unwrap_or(false);
        if inserted {
            break;
        }
        if Instant::now() >= deadline {
            let err = OwnershipError::HeldInThisProcess { database: key };
            warn!("{}", err);
            return Err(err.into_sqlx());
        }
        if !logged_wait {
            info!(
                db = %key.display(),
                "waiting for the previous database owner in this process to close"
            );
            logged_wait = true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // The registry slot is ours; any error below must give it back.
    let sidecar = owner_file_path(&key);
    let lock_path = sidecar.clone();
    let lock_key = key.clone();
    let locked = tokio::task::spawn_blocking(move || lock_owner_file(&lock_path, &lock_key))
        .await
        .unwrap_or_else(|e| {
            Err(OwnershipError::OwnerFile {
                path: sidecar.clone(),
                error: std::io::Error::other(e.to_string()),
            })
        });
    match locked {
        Ok(file) => Ok(OwnershipGuard { key, file }),
        Err(err) => {
            if let Ok(mut owners) = live_owners().lock() {
                owners.remove(&key);
            }
            warn!("{}", err);
            Err(err.into_sqlx())
        }
    }
}

/// Open and lock the sidecar. `Ok(None)` when locking is not supported.
fn lock_owner_file(sidecar: &Path, database: &Path) -> Result<Option<File>, OwnershipError> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(sidecar)
        .map_err(|error| OwnershipError::OwnerFile {
            path: sidecar.to_path_buf(),
            error,
        })?;
    match file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            return Err(OwnershipError::HeldByOtherProcess {
                database: database.to_path_buf(),
                pid: read_owner_record(&mut file).map(|r| r.pid),
            });
        }
        Err(std::fs::TryLockError::Error(error)) => {
            warn!(
                owner_file = %sidecar.display(),
                "this file system cannot lock the database ownership file ({}); \
                 only this process is guarded against opening the database twice",
                error
            );
            return Ok(None);
        }
    }

    let previous = read_owner_record(&mut file);
    if let Some(prev) = previous.as_ref().filter(|r| r.pid != std::process::id()) {
        info!(
            db = %database.display(),
            previous_pid = prev.pid,
            "taking over database ownership left by a process that is no longer running"
        );
    }
    let record = OwnerRecord {
        pid: std::process::id(),
        executable: std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())),
        acquired_at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        database_path: database.display().to_string(),
    };
    let write = (|| -> std::io::Result<()> {
        let body = serde_json::to_vec_pretty(&record).map_err(std::io::Error::other)?;
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&body)?;
        file.flush()
    })();
    if let Err(e) = write {
        // The lock is what guards the database; a record that could not be
        // written only loses the pid in another process's error message.
        debug!("writing the database ownership record: {}", e);
    }
    Ok(Some(file))
}

/// Best-effort read of an owner record. Fails (returns `None`) for a file in
/// another format, and on Windows for a file another process holds locked.
fn read_owner_record(file: &mut File) -> Option<OwnerRecord> {
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut body = Vec::new();
    file.take(64 * 1024).read_to_end(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

/// Fail when another running process owns `database`; used before an offline
/// storage change on a database this process does not own. Takes no lock.
pub fn ensure_not_owned_elsewhere(database: &Path) -> Result<(), sqlx::Error> {
    let key = registry_key(database);
    if live_owners()
        .lock()
        .map(|owners| owners.contains(&key))
        .unwrap_or(false)
    {
        return Err(OwnershipError::HeldInThisProcess { database: key }.into_sqlx());
    }
    let sidecar = owner_file_path(&key);
    let mut file = match OpenOptions::new().read(true).write(true).open(&sidecar) {
        Ok(file) => file,
        // No sidecar: nobody has ever owned this database here.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(OwnershipError::OwnerFile { path: sidecar, error }.into_sqlx()),
    };
    match file.try_lock() {
        Ok(()) => {
            let _ = file.unlock();
            Ok(())
        }
        Err(std::fs::TryLockError::WouldBlock) => Err(OwnershipError::HeldByOtherProcess {
            database: key,
            pid: read_owner_record(&mut file).map(|r| r.pid),
        }
        .into_sqlx()),
        Err(std::fs::TryLockError::Error(_)) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_name_appends_owner() {
        assert_eq!(
            owner_file_path(Path::new("/data/db.sqlite")),
            PathBuf::from("/data/db.sqlite.owner")
        );
    }

    #[tokio::test]
    async fn second_owner_in_process_is_refused_until_the_first_drops() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("db.sqlite");
        std::fs::write(&db, b"").unwrap();

        let first = acquire(&db, Duration::ZERO).await.expect("first owner");
        assert!(first.holds_os_lock());
        let err = acquire(&db, Duration::from_millis(200)).await.unwrap_err();
        assert!(is_ownership_error(&err));
        assert!(err.to_string().contains("already open in this process"), "{err}");
        assert!(ensure_not_owned_elsewhere(&db).is_err());

        drop(first);
        let again = acquire(&db, Duration::ZERO).await.expect("reacquire");
        drop(again);
        assert!(ensure_not_owned_elsewhere(&db).is_ok());
    }

    #[tokio::test]
    async fn waiting_owner_gets_the_database_once_the_first_drops() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("db.sqlite");
        std::fs::write(&db, b"").unwrap();
        let first = acquire(&db, Duration::ZERO).await.unwrap();
        let path = db.clone();
        let waiter = tokio::spawn(async move { acquire(&path, Duration::from_secs(5)).await });
        tokio::time::sleep(Duration::from_millis(300)).await;
        drop(first);
        let second = waiter.await.unwrap().expect("waiter takes over");
        assert!(second.holds_os_lock());
    }

    #[tokio::test]
    async fn stale_or_foreign_sidecar_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("db.sqlite");
        std::fs::write(&db, b"").unwrap();
        let sidecar = owner_file_path(&std::fs::canonicalize(&db).unwrap());
        // A record from a crashed process, then a file in an unknown format.
        for body in [
            br#"{"pid": 999999, "database_path": "x"}"#.to_vec(),
            b"owned-by: something else\n".to_vec(),
        ] {
            std::fs::write(&sidecar, &body).unwrap();
            let guard = acquire(&db, Duration::ZERO).await.expect("take over");
            let record: OwnerRecord =
                serde_json::from_slice(&std::fs::read(&sidecar).unwrap()).unwrap();
            assert_eq!(record.pid, std::process::id());
            drop(guard);
        }
    }

    #[tokio::test]
    async fn lock_held_through_another_handle_reads_as_another_process() {
        // flock and LockFileEx both treat a second handle like another
        // process, which stands in for one here.
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("db.sqlite");
        std::fs::write(&db, b"").unwrap();
        let sidecar = owner_file_path(&std::fs::canonicalize(&db).unwrap());
        std::fs::write(&sidecar, br#"{"pid": 4242}"#).unwrap();
        let foreign = OpenOptions::new().read(true).write(true).open(&sidecar).unwrap();
        foreign.try_lock().unwrap();

        let err = acquire(&db, Duration::ZERO).await.unwrap_err();
        assert!(is_ownership_error(&err));
        let text = err.to_string();
        assert!(text.contains("owned by another running process"), "{text}");
        assert!(ensure_not_owned_elsewhere(&db).is_err());

        // The refused attempt gave its in-process slot back.
        foreign.unlock().unwrap();
        drop(foreign);
        let guard = acquire(&db, Duration::ZERO).await.expect("free after unlock");
        drop(guard);
    }
}
