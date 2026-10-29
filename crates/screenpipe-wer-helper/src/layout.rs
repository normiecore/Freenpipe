// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! What the app and the helper DLL agree on: the context block the app
//! registers with WER, the directory names, and how dump files are named.
//!
//! The app keeps its own copy of these constants (`src-tauri/src/wer.rs`,
//! which is built in a separate Cargo workspace); a test there reads this
//! file and fails when the two drift apart.

/// `FPWR` as a little-endian `u32`: marks a context block written by the app.
pub const CONTEXT_MAGIC: u32 = u32::from_le_bytes(*b"FPWR");
/// Layout version of the context block.
pub const CONTEXT_VERSION: u32 = 1;
/// Capacity of the directory field, in UTF-16 code units.
pub const CONTEXT_DIR_UNITS: usize = 1024;
/// Bytes before the directory field: magic, version, length, reserved.
pub const CONTEXT_HEADER_BYTES: usize = 16;
/// Total size of the block in the app's memory.
pub const CONTEXT_BLOCK_BYTES: usize = CONTEXT_HEADER_BYTES + CONTEXT_DIR_UNITS * 2;

/// Directory of finished dumps inside the data directory. Also the name of
/// the fallback directory next to the DLL.
pub const CRASH_DUMPS_DIR: &str = "crash-dumps";
/// Sub-directory of [`CRASH_DUMPS_DIR`] the helper writes into. The app moves
/// what it finds there one level up at its next start.
pub const PENDING_DIR: &str = "pending";
/// Extension of a finished dump.
pub const DUMP_EXTENSION: &str = "dmp";
/// Suffix of a dump that is still being written.
pub const PARTIAL_SUFFIX: &str = ".partial";
/// The helper writes nothing while this many finished dumps are waiting to be
/// collected, so a crash loop cannot fill the disk.
pub const MAX_PENDING_DUMPS: usize = 5;
/// Longest executable stem kept in a dump file name.
const MAX_STEM_CHARS: usize = 48;

/// The block the app keeps alive for the life of its process and passes to
/// `WerRegisterRuntimeExceptionModule` as the context pointer. The helper
/// runs inside WerFault.exe, so it reads the block out of the crashed
/// process with `ReadProcessMemory` and checks it with [`parse_context`].
#[repr(C)]
#[derive(Clone)]
pub struct ContextBlock {
    pub magic: u32,
    pub version: u32,
    /// Used length of `dir`, in UTF-16 units, without a terminator.
    pub dir_len: u32,
    pub reserved: u32,
    /// Directory the helper writes dumps into (UTF-16, NUL-padded).
    pub dir: [u16; CONTEXT_DIR_UNITS],
}

impl ContextBlock {
    /// Block naming `dir`. `None` when the path does not fit (or is empty).
    pub fn new(dir: &[u16]) -> Option<Self> {
        if dir.is_empty() || dir.len() >= CONTEXT_DIR_UNITS || dir.contains(&0) {
            return None;
        }
        let mut block = ContextBlock {
            magic: CONTEXT_MAGIC,
            version: CONTEXT_VERSION,
            dir_len: dir.len() as u32,
            reserved: 0,
            dir: [0; CONTEXT_DIR_UNITS],
        };
        block.dir[..dir.len()].copy_from_slice(dir);
        Some(block)
    }

    /// The block as the bytes another process reads.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CONTEXT_BLOCK_BYTES);
        out.extend_from_slice(&self.magic.to_le_bytes());
        out.extend_from_slice(&self.version.to_le_bytes());
        out.extend_from_slice(&self.dir_len.to_le_bytes());
        out.extend_from_slice(&self.reserved.to_le_bytes());
        for unit in self.dir {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        out
    }
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let raw: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(raw))
}

/// The dump directory named by a context block read from another process,
/// as UTF-16 units. `None` for anything that is not a well-formed block of
/// this version: the helper then falls back to the directory next to it.
pub fn parse_context(bytes: &[u8]) -> Option<Vec<u16>> {
    if bytes.len() < CONTEXT_BLOCK_BYTES {
        return None;
    }
    if read_u32(bytes, 0)? != CONTEXT_MAGIC || read_u32(bytes, 4)? != CONTEXT_VERSION {
        return None;
    }
    let len = read_u32(bytes, 8)? as usize;
    if len == 0 || len >= CONTEXT_DIR_UNITS {
        return None;
    }
    let units: Vec<u16> = bytes[CONTEXT_HEADER_BYTES..CONTEXT_HEADER_BYTES + len * 2]
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    if units.contains(&0) {
        return None;
    }
    Some(units)
}

/// Executable name reduced to characters that are safe in a file name.
fn file_stem_of(exe_path: &str) -> String {
    let name = exe_path.rsplit(['\\', '/']).next().unwrap_or_default();
    let stem = match name.rsplit_once('.') {
        Some((stem, ext)) if ext.eq_ignore_ascii_case("exe") => stem,
        _ => name,
    };
    let cleaned: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(MAX_STEM_CHARS)
        .collect();
    if cleaned.is_empty() {
        "process".to_string()
    } else {
        cleaned
    }
}

/// Days since 1970-01-01 to a (year, month, day) civil date, proleptic
/// Gregorian calendar, using 400-year eras.
fn civil_date(days_since_epoch: i64) -> (i64, u32, u32) {
    let shifted = days_since_epoch + 719_468; // epoch moved to 0000-03-01
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153; // 0 = March
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// UTC timestamp `YYYYMMDDTHHMMSSZ` for seconds since the Unix epoch.
pub fn utc_stamp(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let secs_of_day = unix_secs % 86_400;
    let (year, month, day) = civil_date(days);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        secs_of_day / 3_600,
        (secs_of_day / 60) % 60,
        secs_of_day % 60
    )
}

/// File name of the dump for one crash: executable stem, process id and the
/// UTC time, e.g. `Handvel-4242-20261020T101500Z.dmp`.
pub fn dump_file_name(exe_path: &str, pid: u32, unix_secs: u64) -> String {
    format!(
        "{}-{pid}-{}.{DUMP_EXTENSION}",
        file_stem_of(exe_path),
        utc_stamp(unix_secs)
    )
}

/// True for the name of a finished dump (not one still being written).
pub fn is_finished_dump(file_name: &str) -> bool {
    file_name
        .rsplit_once('.')
        .is_some_and(|(stem, ext)| !stem.is_empty() && ext.eq_ignore_ascii_case(DUMP_EXTENSION))
}

/// Whether another dump may be written while `pending` finished dumps wait.
pub fn may_write_dump(pending: usize) -> bool {
    pending < MAX_PENDING_DUMPS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn block_round_trips_through_bytes() {
        let dir = utf16(r"C:\Users\Ada\.screenpipe\crash-dumps\pending");
        let block = ContextBlock::new(&dir).expect("fits");
        let bytes = block.to_bytes();
        assert_eq!(bytes.len(), CONTEXT_BLOCK_BYTES);
        assert_eq!(std::mem::size_of::<ContextBlock>(), CONTEXT_BLOCK_BYTES);
        assert_eq!(parse_context(&bytes), Some(dir));
    }

    #[test]
    fn rejects_malformed_blocks() {
        let dir = utf16(r"D:\data\crash-dumps\pending");
        let good = ContextBlock::new(&dir).unwrap().to_bytes();

        let mut wrong_magic = good.clone();
        wrong_magic[0] ^= 0xff;
        assert_eq!(parse_context(&wrong_magic), None);

        let mut wrong_version = good.clone();
        wrong_version[4] = 9;
        assert_eq!(parse_context(&wrong_version), None);

        let mut too_long = good.clone();
        too_long[8..12].copy_from_slice(&(CONTEXT_DIR_UNITS as u32).to_le_bytes());
        assert_eq!(parse_context(&too_long), None);

        let mut empty = good.clone();
        empty[8..12].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(parse_context(&empty), None);

        // A length that runs into the NUL padding.
        let mut padded = good.clone();
        padded[8..12].copy_from_slice(&((dir.len() + 3) as u32).to_le_bytes());
        assert_eq!(parse_context(&padded), None);

        assert_eq!(parse_context(&good[..CONTEXT_BLOCK_BYTES - 1]), None);
    }

    #[test]
    fn block_refuses_paths_that_do_not_fit() {
        assert!(ContextBlock::new(&[]).is_none());
        assert!(ContextBlock::new(&vec![b'a' as u16; CONTEXT_DIR_UNITS]).is_none());
        assert!(ContextBlock::new(&[b'a' as u16, 0, b'b' as u16]).is_none());
        assert!(ContextBlock::new(&vec![b'a' as u16; CONTEXT_DIR_UNITS - 1]).is_some());
    }

    #[test]
    fn utc_stamps_match_known_dates() {
        assert_eq!(utc_stamp(0), "19700101T000000Z");
        assert_eq!(utc_stamp(951_782_400), "20000229T000000Z");
        assert_eq!(utc_stamp(1_792_491_300), "20261020T101500Z");
        assert_eq!(utc_stamp(4_102_444_799), "20991231T235959Z");
    }

    #[test]
    fn dump_names_are_safe_and_ordered() {
        assert_eq!(
            dump_file_name(
                r"C:\Program Files\Handvel\Handvel.exe",
                4242,
                1_792_491_300
            ),
            "Handvel-4242-20261020T101500Z.dmp"
        );
        assert_eq!(
            dump_file_name(r"C:\x\Handvel Beta.EXE", 7, 0),
            "Handvel_Beta-7-19700101T000000Z.dmp"
        );
        assert_eq!(dump_file_name("", 1, 0), "process-1-19700101T000000Z.dmp");
        let long = format!(r"C:\{}.exe", "a".repeat(200));
        let name = dump_file_name(&long, 1, 0);
        assert!(name.starts_with(&"a".repeat(48)));
        assert!(!name.starts_with(&"a".repeat(49)));
    }

    #[test]
    fn finished_dumps_are_told_apart_from_partial_ones() {
        assert!(is_finished_dump("Handvel-1-19700101T000000Z.dmp"));
        assert!(is_finished_dump("x.DMP"));
        assert!(!is_finished_dump(
            "Handvel-1-19700101T000000Z.dmp.partial"
        ));
        assert!(!is_finished_dump(".dmp"));
        assert!(!is_finished_dump("notes.txt"));
    }

    #[test]
    fn pending_limit_stops_a_crash_loop() {
        assert!(may_write_dump(0));
        assert!(may_write_dump(MAX_PENDING_DUMPS - 1));
        assert!(!may_write_dump(MAX_PENDING_DUMPS));
    }
}
