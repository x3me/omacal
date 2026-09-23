//! Where OmaCal's log goes.
//!
//! Until 2026-09-09 the answer was "standard output, and nowhere else". That
//! is fine when the app is started from a terminal and useless every other
//! time: from the desktop entry both streams are `/dev/null`, so
//! [`crate::errors::OPAQUE`] — "Sync failed. See the application log for
//! details." — sent people to a log that did not exist. A failed calendar
//! move went undiagnosed for an evening because of it.
//!
//! So: standard output as before, **and** a file beside the database, which
//! is the directory a user already knows how to find.

/// How large the log may grow before the next launch sets it aside.
///
/// Two megabytes is weeks of ordinary use at [`LEVEL`]. Small enough to paste
/// the interesting part out of, large enough that the interesting part is
/// still there. Checked only at launch, which is safe only because `LEVEL`
/// keeps a running session's output small.
const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// Whether a log of `size` should be set aside before this run appends to it.
///
/// Pure, and separate from the file handling, because the rule is the part
/// worth testing: the rest is `std::fs` doing what it says.
pub(crate) fn should_rotate(size: u64) -> bool {
    size >= MAX_BYTES
}

/// `<data dir>/omacal.log`, beside the database.
///
/// Shares [`crate::cli::app_data_dir`] rather than reproducing the identifier
/// a third time — it moves only if `tauri.conf.json` does, which would move
/// every user's data and so never happens.
pub(crate) fn log_path() -> Option<std::path::PathBuf> {
    Some(crate::cli::app_data_dir()?.join("omacal.log"))
}

/// Opens the log for appending, setting aside an oversized one first.
///
/// The previous run's log is kept as `omacal.log.old` and no further: two
/// files bound what this costs on disk, and the run before last has never
/// been the one anybody wanted.
fn open_log() -> Option<std::fs::File> {
    open_log_at(&log_path()?)
}

/// The half of [`open_log`] that does not need to know where the data
/// directory is, so a test can hand it a temporary one. Setting `HOME` or
/// `XDG_DATA_HOME` from a test would reach into every other test in the
/// binary, which run in the same process.
fn open_log_at(path: &std::path::Path) -> Option<std::fs::File> {
    std::fs::create_dir_all(path.parent()?).ok()?;
    if std::fs::metadata(path).is_ok_and(|m| should_rotate(m.len())) {
        // A failed rename is not a reason to run without a log: the append
        // below still works, the file merely keeps growing until a rename
        // succeeds.
        let _ = std::fs::rename(path, path.with_extension("log.old"));
    }
    std::fs::OpenOptions::new().create(true).append(true).open(path).ok()
}

/// The most detail either destination records: `info` and above.
///
/// **Set here, not inherited.** `fmt::init()` caps itself at `info`, but a
/// `fmt::layer()` on a bare `registry()` has no cap at all — and until this
/// existed the file recorded every `trace` event of every dependency: each SQL
/// statement sqlx ran, each inotify event, each connection hyper pooled. A
/// field log reached 186 MB in sixteen hours, 9 of its 858,999 lines at
/// `info`, and since the size cap above is only checked at launch, a long
/// session grew it without limit.
const LEVEL: tracing_subscriber::filter::LevelFilter =
    tracing_subscriber::filter::LevelFilter::INFO;

/// Both destinations, filtered alike. Separate from [`init`] so a test can
/// install it for one closure instead of for the whole process.
fn subscriber<W>(file: W) -> impl tracing::Subscriber + Send + Sync
where
    W: for<'w> tracing_subscriber::fmt::MakeWriter<'w> + Send + Sync + 'static,
{
    use tracing_subscriber::layer::SubscriberExt;

    tracing_subscriber::registry()
        .with(LEVEL)
        .with(tracing_subscriber::fmt::layer())
        // **No ANSI in the file.** The terminal layer above keeps its colour;
        // written to a file those escapes have to be stripped before the log
        // can be read or pasted, which is a chore handed to somebody who is
        // already having a bad day.
        .with(tracing_subscriber::fmt::layer().with_ansi(false).with_writer(file))
}

/// Starts tracing: standard output, plus the file when one can be opened.
///
/// **Never fails.** A log that cannot be written is worth less than the app
/// starting, so every filesystem error here falls back to the stdout-only
/// subscriber this replaced.
pub(crate) fn init() {
    use tracing_subscriber::util::SubscriberInitExt;

    // No `EnvFilter`: it needs a tracing-subscriber feature this build does
    // not carry, and would pull a regex engine in. A fixed [`LEVEL`] is the
    // whole policy.
    let Some(file) = open_log() else {
        tracing_subscriber::fmt::init();
        return;
    };
    subscriber(file).init();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rule, at its edges. `>=` rather than `>` so a log sitting exactly
    /// on the cap is set aside rather than growing one line past it forever.
    #[test]
    fn a_log_is_set_aside_only_once_it_reaches_the_cap() {
        assert!(!should_rotate(0), "a fresh log is not rotated");
        assert!(!should_rotate(MAX_BYTES - 1));
        assert!(should_rotate(MAX_BYTES), "exactly at the cap counts");
        assert!(should_rotate(MAX_BYTES * 10));
    }

    /// The file half, end to end: it is created, it **appends** rather than
    /// truncating, and an oversized log is moved aside with its contents
    /// intact rather than deleted. Rotation that lost the previous run would
    /// be worse than never rotating, since the previous run is usually the
    /// one somebody is being asked about.
    #[test]
    fn the_log_is_created_appended_and_rotated_without_losing_anything() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("omacal.log");

        writeln!(open_log_at(&path).unwrap(), "first run").unwrap();
        writeln!(open_log_at(&path).unwrap(), "second run").unwrap();
        let after_two = std::fs::read_to_string(&path).unwrap();
        assert!(after_two.contains("first run"), "the second run truncated the first");
        assert!(after_two.contains("second run"));

        // Push it over the cap, then open again: this run starts clean and
        // the old one is still readable beside it.
        std::fs::write(&path, vec![b'x'; MAX_BYTES as usize]).unwrap();
        writeln!(open_log_at(&path).unwrap(), "after rotation").unwrap();
        let fresh = std::fs::read_to_string(&path).unwrap();
        assert_eq!(fresh.trim(), "after rotation", "the new log kept the old bytes");
        let old = std::fs::read_to_string(path.with_extension("log.old")).unwrap();
        assert_eq!(old.len(), MAX_BYTES as usize, "the set-aside log lost content");
    }

    /// **`debug` and `trace` never reach the file**, from our code or a
    /// dependency's. Installed for this closure only, so no other test in the
    /// binary is affected.
    #[test]
    fn the_file_records_info_and_above_and_nothing_finer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("omacal.log");
        let file = open_log_at(&path).unwrap();
        tracing::subscriber::with_default(subscriber(file), || {
            tracing::trace!("a trace line");
            tracing::debug!(target: "sqlx::query", "a dependency's debug line");
            tracing::info!("an info line");
            tracing::warn!("a warn line");
        });
        let log = std::fs::read_to_string(&path).unwrap();
        assert!(log.contains("an info line") && log.contains("a warn line"), "{log}");
        assert!(!log.contains("a trace line"), "trace reached the file:\n{log}");
        assert!(!log.contains("debug line"), "debug reached the file:\n{log}");
    }

    /// The log sits **beside the database**, which is the directory a user is
    /// already told to look in. Asserted against `db_path` rather than a
    /// literal so the two cannot drift apart.
    #[test]
    fn the_log_sits_beside_the_database() {
        let (Some(log), Some(db)) = (log_path(), crate::cli::db_path()) else {
            return; // No HOME, as in some sandboxes: nothing to compare.
        };
        assert_eq!(log.parent(), db.parent(), "the log left the data directory");
        assert_eq!(log.file_name().unwrap(), "omacal.log");
    }
}
