//! The daemon's log file: an [`Observer`] that appends each event as one JSON
//! line (with a `ts_ms` stamp) to a file and rotates it by size.
//!
//! Rotation shifts `log` -> `log.1` -> `log.2` ... and drops whatever would
//! land past `keep`, so the file count is bounded at `keep + 1`. The sink
//! never panics and never aborts the caller: an I/O failure is reported once
//! on the reporter observer as [`names::LOG_WRITE_FAILED`].

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;

use crate::{Event, Level, NullObserver, Observer, names};

struct State {
    file: Option<File>,
    bytes: u64,
    failure_reported: bool,
}

/// Appends events to a size-rotated log file.
// reason: pii-observe is the dependency floor (serde only); it cannot take
// parking_lot, and a poisoned lock just drops a line.
#[allow(clippy::disallowed_types)]
pub struct FileLog {
    path: PathBuf,
    rotate_bytes: u64,
    keep: usize,
    reporter: Arc<dyn Observer>,
    state: std::sync::Mutex<State>,
}

impl std::fmt::Debug for FileLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileLog")
            .field("path", &self.path)
            .field("rotate_bytes", &self.rotate_bytes)
            .field("keep", &self.keep)
            .finish_non_exhaustive()
    }
}

impl FileLog {
    /// A log at `path` rotating once it would exceed `rotate_bytes` (0 = never
    /// rotate), keeping at most `keep` rotated files besides the live one.
    #[must_use]
    #[allow(clippy::disallowed_types)] // reason: see the struct
    pub fn new(path: impl Into<PathBuf>, rotate_bytes: u64, keep: usize) -> Self {
        Self {
            path: path.into(),
            rotate_bytes,
            keep,
            reporter: Arc::new(NullObserver),
            state: std::sync::Mutex::new(State {
                file: None,
                bytes: 0,
                failure_reported: false,
            }),
        }
    }

    /// Where write failures are reported.
    #[must_use]
    pub fn with_reporter(mut self, reporter: Arc<dyn Observer>) -> Self {
        self.reporter = reporter;
        self
    }

    /// The live log file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn rotated(&self, n: usize) -> PathBuf {
        let mut s = self.path.clone().into_os_string();
        s.push(format!(".{n}"));
        PathBuf::from(s)
    }

    fn open(&self) -> std::io::Result<File> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
    }

    /// Shifts `log.(n)` to `log.(n+1)`, oldest first, then moves the live file
    /// to `log.1`. With `keep == 0` the live file is removed instead.
    fn rotate(&self) -> std::io::Result<()> {
        if self.keep == 0 {
            return std::fs::remove_file(&self.path);
        }
        for n in (1..self.keep).rev() {
            let from = self.rotated(n);
            if from.exists() {
                std::fs::rename(&from, self.rotated(n + 1))?;
            }
        }
        std::fs::rename(&self.path, self.rotated(1))
    }

    // reason: a log line's timestamp is wall-clock by definition; it is
    // operator-facing, never replayed, and pii-observe (the dependency floor)
    // cannot take pii-core's Clock.
    #[allow(clippy::disallowed_methods)]
    fn line(e: &Event) -> Option<String> {
        let mut v = serde_json::to_value(e).ok()?;
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        if let Value::Object(m) = &mut v {
            m.insert("ts_ms".to_owned(), Value::from(ts));
        }
        Some(format!("{v}\n"))
    }

    fn write(&self, line: &str) -> std::io::Result<()> {
        let Ok(mut st) = self.state.lock() else {
            return Ok(());
        };
        if st.file.is_none() {
            let f = self.open()?;
            st.bytes = f.metadata().map_or(0, |m| m.len());
            st.file = Some(f);
        }
        let len = line.len() as u64;
        if self.rotate_bytes > 0 && st.bytes > 0 && st.bytes + len > self.rotate_bytes {
            st.file = None;
            self.rotate()?;
            let f = self.open()?;
            st.bytes = 0;
            st.file = Some(f);
        }
        if let Some(f) = st.file.as_mut() {
            f.write_all(line.as_bytes())?;
            st.bytes += len;
        }
        Ok(())
    }

    fn report_once(&self, why: &str) {
        let first = self.state.lock().is_ok_and(|mut st| {
            let first = !st.failure_reported;
            st.failure_reported = true;
            first
        });
        if first {
            self.reporter.event(
                &Event::new("observe", Level::Error, names::LOG_WRITE_FAILED)
                    .field("path", self.path.display().to_string())
                    .field("error", why),
            );
        }
    }
}

impl Observer for FileLog {
    fn event(&self, e: &Event) {
        let Some(line) = Self::line(e) else { return };
        if let Err(err) = self.write(&line) {
            self.report_once(&err.to_string());
        }
    }

    fn flush(&self) {
        if let Ok(mut st) = self.state.lock()
            && let Some(f) = st.file.as_mut()
        {
            // reason: nowhere to report a failed flush; the next write reports.
            let _ = f.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::Recorder;

    fn ev(i: u64) -> Event {
        Event::new("daemon", Level::Info, names::DAEMON_STARTED).field("i", i)
    }

    #[test]
    fn writes_one_json_line_per_event_with_a_timestamp() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().to_path_buf();
        let log = FileLog::new(d.join("pii.log"), 0, 3);
        log.event(&ev(1));
        log.event(&ev(2));
        let text = std::fs::read_to_string(d.join("pii.log")).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let [first, second] = lines.as_slice() else {
            panic!("expected 2 lines, got {lines:?}");
        };
        assert_eq!(first.pointer("/kind"), Some(&Value::from("daemon.started")));
        assert_eq!(second.pointer("/fields/i"), Some(&Value::from(2)));
        assert!(first.pointer("/ts_ms").and_then(Value::as_u64).unwrap() > 0);
    }

    #[test]
    fn rotation_triggers_and_the_kept_count_is_bounded() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().to_path_buf();
        let log = FileLog::new(d.join("pii.log"), 200, 2);
        for i in 0..60 {
            log.event(&ev(i));
        }
        let mut names: Vec<String> = std::fs::read_dir(&d)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["pii.log", "pii.log.1", "pii.log.2"], "{names:?}");
        for n in &names {
            let len = std::fs::metadata(d.join(n)).unwrap().len();
            assert!(len <= 200, "{n} is {len} bytes");
        }
        // Newest event is in the live file, never in a rotated one.
        let live = std::fs::read_to_string(d.join("pii.log")).unwrap();
        assert!(live.contains(r#""i":59"#), "{live}");
    }

    #[test]
    fn keep_zero_discards_instead_of_rotating() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().to_path_buf();
        let log = FileLog::new(d.join("pii.log"), 150, 0);
        for i in 0..20 {
            log.event(&ev(i));
        }
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
    }

    #[test]
    fn an_unwritable_path_is_reported_once_and_never_panics() {
        let rec = Arc::new(Recorder::new());
        let log = FileLog::new("/proc/definitely/not/writable/pii.log", 0, 1)
            .with_reporter(rec.clone() as Arc<dyn Observer>);
        log.event(&ev(1));
        log.event(&ev(2));
        assert_eq!(rec.events_named(names::LOG_WRITE_FAILED).len(), 1);
    }
}
