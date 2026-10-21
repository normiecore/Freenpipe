// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Checks that run before [`crate::DatabaseManager`] opens its pools.
//!
//! - **SQLite runtime guard.** The linked SQLite must be 3.51.3 or newer:
//!   older releases carry the WAL-reset bug that can corrupt a database when
//!   a checkpoint races a writer. The bundled build (libsqlite3-sys 0.37)
//!   ships 3.51.3, so this only fires for a build linked against an older
//!   system SQLite.
//! - **sqlite-vec registration.** The vector extension is registered once per
//!   process through `sqlite3_auto_extension`, and a non-OK result code is a
//!   start-up error instead of a silent "no such function" later.
//! - **Header preflight.** An existing database file gets its 100-byte header
//!   read and validated before any pool touches it, so a file that is not a
//!   database, or whose header is damaged, fails fast with the same text
//!   SQLite itself uses (`(code: 26) file is not a database`,
//!   `(code: 11) database disk image is malformed`). The check reads only the
//!   header and gives up quietly after [`HEADER_READ_TIMEOUT`], leaving the
//!   verdict to SQLite, so a slow disk never blocks start-up.
//!
//! Every check is lenient towards valid files: a database that SQLite can
//! open (including one written by screenpipe, or one whose first migration
//! never ran) passes.

use std::borrow::Cow;
use std::error::Error as StdError;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use tracing::{info, warn};

/// SQLite's result code for "file is not a database".
pub const SQLITE_NOTADB: i32 = 26;
/// SQLite's result code for "database disk image is malformed".
pub const SQLITE_CORRUPT: i32 = 11;
/// SQLite's own message for [`SQLITE_NOTADB`].
pub const NOTADB_MESSAGE: &str = "file is not a database";
/// SQLite's own message for [`SQLITE_CORRUPT`].
pub const CORRUPT_MESSAGE: &str = "database disk image is malformed";

/// Oldest SQLite release Freenpipe runs on (3.51.3, the WAL-reset fix).
pub const MIN_SQLITE_VERSION_NUMBER: i32 = 3_051_003;

/// How long the header preflight may take before it steps aside.
pub const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Size of the SQLite database header.
const HEADER_LEN: usize = 100;
/// The 16 bytes every SQLite database file starts with.
const MAGIC: &[u8; 16] = b"SQLite format 3\0";

/// A start-up failure that reads like a SQLite error: `(code: N) message`.
///
/// It travels as `sqlx::Error::Database`, so callers that classify SQLite
/// errors by code or message (the write queue, the app's boot error) treat
/// it exactly like the error SQLite would have raised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupSqliteError {
    code: i32,
    message: String,
}

impl StartupSqliteError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// The SQLite result code this error stands for.
    pub fn sqlite_code(&self) -> i32 {
        self.code
    }

    pub fn into_sqlx(self) -> sqlx::Error {
        sqlx::Error::Database(Box::new(self))
    }
}

impl fmt::Display for StartupSqliteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(code: {}) {}", self.code, self.message)
    }
}

impl StdError for StartupSqliteError {}

impl sqlx::error::DatabaseError for StartupSqliteError {
    fn message(&self) -> &str {
        &self.message
    }

    fn code(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Owned(self.code.to_string()))
    }

    fn as_error(&self) -> &(dyn StdError + Send + Sync + 'static) {
        self
    }

    fn as_error_mut(&mut self) -> &mut (dyn StdError + Send + Sync + 'static) {
        self
    }

    fn into_error(self: Box<Self>) -> Box<dyn StdError + Send + Sync + 'static> {
        self
    }

    fn kind(&self) -> sqlx::error::ErrorKind {
        sqlx::error::ErrorKind::Other
    }
}

// ---------------------------------------------------------------------------
// SQLite runtime guard
// ---------------------------------------------------------------------------

/// The linked SQLite library, as reported by SQLite itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteRuntime {
    pub version: String,
    pub version_number: i32,
    pub source_id: String,
}

/// Version of the SQLite library this process is linked against.
pub fn sqlite_runtime() -> SqliteRuntime {
    // SAFETY: these three functions only return pointers to static strings
    // compiled into the library (or a plain integer); they take no arguments
    // and need no initialisation.
    unsafe {
        let text = |ptr: *const std::os::raw::c_char| {
            if ptr.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned()
            }
        };
        SqliteRuntime {
            version: text(libsqlite3_sys::sqlite3_libversion()),
            version_number: libsqlite3_sys::sqlite3_libversion_number(),
            source_id: text(libsqlite3_sys::sqlite3_sourceid()),
        }
    }
}

/// Whether a `sqlite3_libversion_number()` value is new enough.
pub fn sqlite_version_supported(version_number: i32) -> bool {
    version_number >= MIN_SQLITE_VERSION_NUMBER
}

/// Refuse a SQLite that is too old; `Ok` with the runtime otherwise.
pub fn check_sqlite_version(runtime: &SqliteRuntime) -> Result<(), sqlx::Error> {
    if sqlite_version_supported(runtime.version_number) {
        return Ok(());
    }
    Err(sqlx::Error::Configuration(
        format!(
            "the linked SQLite library is {} ({}), older than the required 3.51.3; \
             older releases can corrupt a WAL database, so the database was not opened. \
             Rebuild with the bundled SQLite (libsqlite3-sys `bundled` feature)",
            runtime.version, runtime.version_number
        )
        .into(),
    ))
}

/// Log the SQLite runtime once per process and apply the version guard.
pub fn check_sqlite_runtime() -> Result<(), sqlx::Error> {
    static LOGGED: OnceLock<()> = OnceLock::new();
    let runtime = sqlite_runtime();
    LOGGED.get_or_init(|| {
        info!(
            sqlite_version = %runtime.version,
            sqlite_version_number = runtime.version_number,
            sqlite_source_id = %runtime.source_id,
            "sqlite runtime"
        );
    });
    check_sqlite_version(&runtime)
}

// ---------------------------------------------------------------------------
// sqlite-vec registration
// ---------------------------------------------------------------------------

/// Turn an `sqlite3_auto_extension` result code into a start-up result.
pub fn vec_registration_result(rc: i32) -> Result<(), sqlx::Error> {
    if rc == libsqlite3_sys::SQLITE_OK {
        Ok(())
    } else {
        Err(StartupSqliteError::new(
            rc,
            format!("registering the sqlite-vec extension failed (sqlite3_auto_extension returned {rc})"),
        )
        .into_sqlx())
    }
}

/// Register sqlite-vec for every connection this process opens. Runs the
/// registration once; later calls return the first outcome.
pub fn register_sqlite_vec() -> Result<(), sqlx::Error> {
    static RESULT: OnceLock<i32> = OnceLock::new();
    let rc = *RESULT.get_or_init(|| {
        // sqlite-vec declares its entry point without arguments; SQLite calls
        // it with the standard extension-init signature, which
        // libsqlite3-sys 0.37 spells out in the sqlite3_auto_extension binding.
        type ExtensionInit = unsafe extern "C" fn(
            *mut libsqlite3_sys::sqlite3,
            *mut *mut std::os::raw::c_char,
            *const libsqlite3_sys::sqlite3_api_routines,
        ) -> std::os::raw::c_int;
        // SAFETY: sqlite3_vec_init has the extension-init ABI SQLite expects;
        // only the Rust-side declaration lacks the parameters.
        unsafe {
            libsqlite3_sys::sqlite3_auto_extension(Some(std::mem::transmute::<
                *const (),
                ExtensionInit,
            >(
                sqlite_vec::sqlite3_vec_init as *const (),
            )))
        }
    });
    let outcome = vec_registration_result(rc);
    if let Err(e) = &outcome {
        warn!("{}", e);
    }
    outcome
}

// ---------------------------------------------------------------------------
// Header preflight
// ---------------------------------------------------------------------------

/// What is wrong with a database header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderProblem {
    /// The file does not start with the SQLite magic string.
    NotADatabase,
    /// The file is shorter than the 100-byte header.
    TruncatedHeader,
    /// The page size field is not a power of two between 512 and 65536.
    InvalidPageSize(u32),
    /// The file length is not a whole number of pages.
    PartialPage { len: u64, page_size: u32 },
    /// The write (byte 18) or read (byte 19) format version is not 1 or 2.
    InvalidFormatVersion { write: u8, read: u8 },
    /// The payload fractions (bytes 21..=23) are not 64/32/32.
    InvalidPayloadFractions([u8; 3]),
    /// The schema format number is outside 0..=4.
    InvalidSchemaFormat(u32),
    /// The text encoding is outside 0..=3.
    InvalidTextEncoding(u32),
}

impl HeaderProblem {
    /// SQLite result code the problem is reported under.
    pub fn sqlite_code(self) -> i32 {
        match self {
            HeaderProblem::NotADatabase => SQLITE_NOTADB,
            _ => SQLITE_CORRUPT,
        }
    }

    /// Short description for the log line.
    pub fn describe(self) -> String {
        match self {
            HeaderProblem::NotADatabase => "the file does not start with the SQLite header".into(),
            HeaderProblem::TruncatedHeader => "the file is shorter than the 100-byte header".into(),
            HeaderProblem::InvalidPageSize(size) => format!("invalid page size {size}"),
            HeaderProblem::PartialPage { len, page_size } => {
                format!("file length {len} is not a whole number of {page_size}-byte pages")
            }
            HeaderProblem::InvalidFormatVersion { write, read } => {
                format!("invalid file format versions (write {write}, read {read})")
            }
            HeaderProblem::InvalidPayloadFractions(f) => {
                format!("invalid payload fractions {}/{}/{}", f[0], f[1], f[2])
            }
            HeaderProblem::InvalidSchemaFormat(n) => format!("invalid schema format {n}"),
            HeaderProblem::InvalidTextEncoding(n) => format!("invalid text encoding {n}"),
        }
    }

    pub fn to_error(self) -> StartupSqliteError {
        let message = match self.sqlite_code() {
            SQLITE_NOTADB => NOTADB_MESSAGE,
            _ => CORRUPT_MESSAGE,
        };
        StartupSqliteError::new(self.sqlite_code(), message)
    }
}

fn be_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Validate the start of a database file. `header` holds the first bytes
/// that could be read (at most 100) and `file_len` the file's length.
///
/// An empty file is a new database and passes. Zero in the schema format and
/// text encoding fields also passes: SQLite leaves both at zero until the
/// first table is created, so a database whose first migration never ran (or
/// whose first schema write still sits in the WAL) has them unset.
pub fn check_header(header: &[u8], file_len: u64) -> Result<(), HeaderProblem> {
    if file_len == 0 {
        return Ok(());
    }
    let magic_len = header.len().min(MAGIC.len());
    if header[..magic_len] != MAGIC[..magic_len] || header.len() < MAGIC.len() {
        // Too short to even hold the magic string, or the wrong magic: SQLite
        // reports both as "not a database".
        return Err(HeaderProblem::NotADatabase);
    }
    if header.len() < HEADER_LEN || file_len < HEADER_LEN as u64 {
        return Err(HeaderProblem::TruncatedHeader);
    }

    let raw_page_size = u32::from(u16::from_be_bytes([header[16], header[17]]));
    let page_size = if raw_page_size == 1 {
        65_536
    } else {
        raw_page_size
    };
    if !(512..=65_536).contains(&page_size) || !page_size.is_power_of_two() {
        return Err(HeaderProblem::InvalidPageSize(raw_page_size));
    }
    if file_len % u64::from(page_size) != 0 {
        return Err(HeaderProblem::PartialPage {
            len: file_len,
            page_size,
        });
    }

    let (write, read) = (header[18], header[19]);
    if !matches!(write, 1 | 2) || !matches!(read, 1 | 2) {
        return Err(HeaderProblem::InvalidFormatVersion { write, read });
    }

    let fractions = [header[21], header[22], header[23]];
    if fractions != [64, 32, 32] {
        return Err(HeaderProblem::InvalidPayloadFractions(fractions));
    }

    let schema_format = be_u32(&header[44..48]);
    if schema_format > 4 {
        return Err(HeaderProblem::InvalidSchemaFormat(schema_format));
    }

    let encoding = be_u32(&header[56..60]);
    if encoding > 3 {
        return Err(HeaderProblem::InvalidTextEncoding(encoding));
    }
    Ok(())
}

/// Read the header of `path`: `None` when the file is missing or cannot be
/// read (SQLite reports those itself), else the bytes read and the length.
fn read_header(path: &Path) -> Option<(Vec<u8>, u64)> {
    let file = std::fs::File::open(path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() {
        return None;
    }
    let mut header = Vec::with_capacity(HEADER_LEN);
    file.take(HEADER_LEN as u64)
        .read_to_end(&mut header)
        .ok()?;
    Some((header, meta.len()))
}

/// Run the header preflight on an existing database file.
///
/// A missing or unreadable file, or a read that takes longer than `timeout`,
/// passes: SQLite then opens (or fails to open) the file with its own error.
pub async fn preflight_database_file(path: &Path, timeout: Duration) -> Result<(), sqlx::Error> {
    let owned: PathBuf = path.to_path_buf();
    let read = tokio::task::spawn_blocking(move || read_header(&owned));
    let (header, len) = match tokio::time::timeout(timeout, read).await {
        Ok(Ok(Some(found))) => found,
        Ok(Ok(None)) | Ok(Err(_)) => return Ok(()),
        Err(_) => {
            warn!(
                db = %path.display(),
                "database header preflight did not finish within {}s; leaving the check to SQLite",
                timeout.as_secs()
            );
            return Ok(());
        }
    };
    match check_header(&header, len) {
        Ok(()) => Ok(()),
        Err(problem) => {
            let err = problem.to_error();
            warn!(
                db = %path.display(),
                reason = %problem.describe(),
                "database header preflight failed: {}",
                err
            );
            Err(err.into_sqlx())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A well-formed header for a database of `pages` 4096-byte pages.
    fn header(page_size_field: u16) -> Vec<u8> {
        let mut h = vec![0u8; HEADER_LEN];
        h[..16].copy_from_slice(MAGIC);
        h[16..18].copy_from_slice(&page_size_field.to_be_bytes());
        h[18] = 2;
        h[19] = 2;
        h[21] = 64;
        h[22] = 32;
        h[23] = 32;
        h[44..48].copy_from_slice(&4u32.to_be_bytes());
        h[56..60].copy_from_slice(&1u32.to_be_bytes());
        h
    }

    #[test]
    fn valid_headers_pass() {
        assert_eq!(check_header(&header(4096), 4096 * 3), Ok(()));
        assert_eq!(check_header(&header(512), 512), Ok(()));
        // Page size field 1 means 65536.
        assert_eq!(check_header(&header(1), 65_536 * 2), Ok(()));
        // Empty file: a new database.
        assert_eq!(check_header(&[], 0), Ok(()));
    }

    #[test]
    fn schema_less_database_passes() {
        let mut h = header(4096);
        h[44..48].copy_from_slice(&0u32.to_be_bytes());
        h[56..60].copy_from_slice(&0u32.to_be_bytes());
        assert_eq!(check_header(&h, 4096), Ok(()));
    }

    #[test]
    fn bad_magic_is_not_a_database() {
        let junk = b"this is a text file, not sqlite at all, but it is long enough to hold a header......".to_vec();
        assert_eq!(
            check_header(&junk, junk.len() as u64),
            Err(HeaderProblem::NotADatabase)
        );
        assert_eq!(check_header(b"SQLite", 6), Err(HeaderProblem::NotADatabase));
        let err = HeaderProblem::NotADatabase.to_error();
        assert_eq!(err.to_string(), "(code: 26) file is not a database");
    }

    #[test]
    fn damaged_fields_are_malformed() {
        let short = header(4096)[..60].to_vec();
        assert_eq!(check_header(&short, 60), Err(HeaderProblem::TruncatedHeader));

        assert_eq!(
            check_header(&header(1000), 4000),
            Err(HeaderProblem::InvalidPageSize(1000))
        );
        assert_eq!(
            check_header(&header(256), 512),
            Err(HeaderProblem::InvalidPageSize(256))
        );
        assert_eq!(
            check_header(&header(4096), 4096 * 2 + 17),
            Err(HeaderProblem::PartialPage {
                len: 4096 * 2 + 17,
                page_size: 4096
            })
        );

        let mut h = header(4096);
        h[19] = 3;
        assert_eq!(
            check_header(&h, 4096),
            Err(HeaderProblem::InvalidFormatVersion { write: 2, read: 3 })
        );

        let mut h = header(4096);
        h[22] = 33;
        assert_eq!(
            check_header(&h, 4096),
            Err(HeaderProblem::InvalidPayloadFractions([64, 33, 32]))
        );

        let mut h = header(4096);
        h[44..48].copy_from_slice(&5u32.to_be_bytes());
        assert_eq!(
            check_header(&h, 4096),
            Err(HeaderProblem::InvalidSchemaFormat(5))
        );

        let mut h = header(4096);
        h[56..60].copy_from_slice(&4u32.to_be_bytes());
        assert_eq!(
            check_header(&h, 4096),
            Err(HeaderProblem::InvalidTextEncoding(4))
        );

        let err = HeaderProblem::InvalidPageSize(1000).to_error();
        assert_eq!(err.to_string(), "(code: 11) database disk image is malformed");
    }

    #[test]
    fn version_guard_accepts_the_bundled_sqlite() {
        let runtime = sqlite_runtime();
        assert!(
            runtime.version_number >= MIN_SQLITE_VERSION_NUMBER,
            "bundled SQLite {} is older than 3.51.3",
            runtime.version
        );
        assert!(check_sqlite_version(&runtime).is_ok());
        assert!(check_sqlite_runtime().is_ok());
        assert!(!runtime.source_id.is_empty());
    }

    #[test]
    fn version_guard_refuses_older_releases() {
        assert!(sqlite_version_supported(3_051_003));
        assert!(sqlite_version_supported(3_052_000));
        assert!(!sqlite_version_supported(3_051_002));
        assert!(!sqlite_version_supported(3_045_000));
        let old = SqliteRuntime {
            version: "3.51.2".into(),
            version_number: 3_051_002,
            source_id: String::new(),
        };
        let err = check_sqlite_version(&old).unwrap_err().to_string();
        assert!(err.contains("3.51.2"), "{err}");
        assert!(err.contains("3.51.3"), "{err}");
    }

    #[test]
    fn vec_registration_reports_the_result_code() {
        assert!(vec_registration_result(libsqlite3_sys::SQLITE_OK).is_ok());
        let err = vec_registration_result(libsqlite3_sys::SQLITE_NOMEM).unwrap_err();
        match &err {
            sqlx::Error::Database(db) => {
                assert_eq!(db.code().as_deref(), Some("7"));
            }
            other => panic!("unexpected error {other:?}"),
        }
        assert!(err.to_string().contains("returned 7"), "{err}");
        assert!(register_sqlite_vec().is_ok());
    }

    #[tokio::test]
    async fn preflight_rejects_a_text_file_and_passes_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let text = dir.path().join("notes.sqlite");
        std::fs::write(&text, "hello, this is not a database\n".repeat(20)).unwrap();
        let err = preflight_database_file(&text, HEADER_READ_TIMEOUT)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("(code: 26) file is not a database"),
            "{err}"
        );

        let missing = dir.path().join("missing.sqlite");
        assert!(preflight_database_file(&missing, HEADER_READ_TIMEOUT)
            .await
            .is_ok());

        let empty = dir.path().join("empty.sqlite");
        std::fs::write(&empty, b"").unwrap();
        assert!(preflight_database_file(&empty, HEADER_READ_TIMEOUT)
            .await
            .is_ok());
    }
}
