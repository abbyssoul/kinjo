//! Persist a crash report to a file when kinjo panics.
//!
//! kinjo already renders a panic to stderr (color_eyre) and restores the
//! terminal before doing so (ratatui). Both are transient: a TUI panic scrolls
//! past or is swallowed when the alternate screen is torn down, so a bug
//! reporter rarely still has the backtrace to attach. This module chains one
//! more step onto the existing panic hook — it writes the panic, a captured
//! backtrace, and a little environment to a file, then points the user at it.
//!
//! Everything except the hook glue is a plain function taking owned data, so the
//! report format is unit-tested without having to actually panic.

use std::{
    any::Any,
    backtrace::Backtrace,
    ffi::OsString,
    io::{self, Write},
    panic::PanicHookInfo,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Where bug reports go; printed after a crash report is saved.
const ISSUES_URL: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "/issues");

/// The pieces of a panic a report is built from, kept as owned data so the
/// report can be rendered and written away from the panic hook. That separation
/// is what makes the formatting testable without a real panic.
pub(crate) struct CrashReport {
    seconds: u64,
    pid: u32,
    location: Option<String>,
    message: String,
    backtrace: String,
}

impl CrashReport {
    fn from_panic(info: &PanicHookInfo<'_>) -> Self {
        let location = info
            .location()
            .map(|at| format!("{}:{}:{}", at.file(), at.line(), at.column()));
        Self::new(location, panic_message(info.payload()))
    }

    /// Stamp a report with the current time, pid, and a backtrace. Split out from
    /// [`Self::from_panic`] so the parts a hook cannot hand a test — the pid, the
    /// captured backtrace — are still exercised directly.
    fn new(location: Option<String>, message: String) -> Self {
        Self {
            seconds: unix_seconds(),
            pid: std::process::id(),
            location,
            message,
            // `force_capture` ignores `RUST_BACKTRACE`, so a report always
            // carries a backtrace even when the user never set the variable.
            backtrace: Backtrace::force_capture().to_string(),
        }
    }

    /// Collision-proof within a run: the wall-clock second plus the pid.
    fn file_name(&self) -> String {
        format!("kinjo-crash-{}-{}.log", self.seconds, self.pid)
    }

    fn render(&self) -> String {
        let location = self.location.as_deref().unwrap_or("unknown");
        format!(
            "kinjo crash report\n\
             \n\
             This file can contain local hostnames and service names seen on\n\
             your network. Review it before sharing.\n\
             \n\
             version:   {version}\n\
             target:    {os} {arch}\n\
             unix time: {seconds}\n\
             pid:       {pid}\n\
             location:  {location}\n\
             \n\
             message:\n{message}\n\
             \n\
             backtrace:\n{backtrace}\n",
            version = env!("CARGO_PKG_VERSION"),
            os = std::env::consts::OS,
            arch = std::env::consts::ARCH,
            seconds = self.seconds,
            pid = self.pid,
            message = self.message,
            backtrace = self.backtrace,
        )
    }
}

/// Write `report` into `dir`, creating `dir` if needed, and return the file path.
pub(crate) fn write_report(dir: &Path, report: &CrashReport) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(report.file_name());
    let mut file = std::fs::File::create(&path)?;
    file.write_all(report.render().as_bytes())?;
    Ok(path)
}

/// Write the report and return the line to show the user: where the file went,
/// or why it could not be saved. Kept separate from the hook so both outcomes
/// are exercised by tests.
fn save_and_describe(dir: &Path, report: &CrashReport) -> String {
    match write_report(dir, report) {
        Ok(path) => format!(
            "\nkinjo saved a crash report to:\n    {}\n\
             Please attach it to a bug report at {ISSUES_URL}\n\
             (review it first — it can contain local hostnames and service names).",
            path.display()
        ),
        Err(error) => format!("\nkinjo crashed and could not save a crash report: {error}"),
    }
}

/// The directory crash reports are written to: `KINJO_CRASH_DIR` when set (the
/// bug-report wrapper points it at its output folder), otherwise the system
/// temp directory, which is always writable.
fn report_dir() -> PathBuf {
    report_dir_from(std::env::var_os("KINJO_CRASH_DIR"))
}

fn report_dir_from(override_dir: Option<OsString>) -> PathBuf {
    match override_dir {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => std::env::temp_dir(),
    }
}

/// Chain crash-report persistence onto the current panic hook. The existing hook
/// (color_eyre, later wrapped by ratatui to restore the terminal) still runs and
/// renders to stderr; this adds a durable copy and tells the user where it is.
pub(crate) fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Let the terminal be restored and the panic rendered to stderr first,
        // so the pointer line below is the last thing the user sees.
        previous(info);
        let report = CrashReport::from_panic(info);
        eprintln!("{}", save_and_describe(&report_dir(), &report));
    }));
}

/// Recover a human-readable message from a panic payload, which is `&str` for a
/// string literal, `String` for a formatted `panic!`, and otherwise opaque.
fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "<non-string panic payload>".to_owned()
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    fn sample_report() -> CrashReport {
        CrashReport {
            seconds: 42,
            pid: 7,
            location: Some("src/ui/app.rs:12:5".to_owned()),
            message: "index out of bounds".to_owned(),
            backtrace: "0: kinjo::ui::app::draw".to_owned(),
        }
    }

    #[test]
    fn written_report_is_named_by_time_and_pid_and_keeps_the_detail() {
        let dir = test_support::temp_dir("crash");

        let path = write_report(&dir, &sample_report()).unwrap();

        assert_eq!(path.file_name().unwrap(), "kinjo-crash-42-7.log");
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains(env!("CARGO_PKG_VERSION")), "{body}");
        assert!(body.contains("src/ui/app.rs:12:5"), "{body}");
        assert!(body.contains("index out of bounds"), "{body}");
        assert!(body.contains("0: kinjo::ui::app::draw"), "{body}");
        // The privacy note travels with every report.
        assert!(body.contains("Review it before sharing"), "{body}");

        test_support::remove(&dir);
    }

    #[test]
    fn a_missing_location_renders_as_unknown_rather_than_dropping_the_field() {
        let dir = test_support::temp_dir("crash");
        let mut report = sample_report();
        report.location = None;

        let path = write_report(&dir, &report).unwrap();

        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("location:  unknown"), "{body}");

        test_support::remove(&dir);
    }

    #[test]
    fn saving_points_the_user_at_the_file_and_the_issue_tracker() {
        let dir = test_support::temp_dir("crash");

        let message = save_and_describe(&dir, &sample_report());

        assert!(message.contains("kinjo-crash-42-7.log"), "{message}");
        assert!(message.contains(ISSUES_URL), "{message}");

        test_support::remove(&dir);
    }

    #[test]
    fn saving_reports_the_error_when_the_directory_cannot_be_made() {
        // A regular file where a directory component is expected makes
        // `create_dir_all` fail, so the error branch is exercised for real.
        let file = test_support::temp_file("crash-not-a-dir", "");
        let dir = file.join("subdir");

        let message = save_and_describe(&dir, &sample_report());

        assert!(
            message.contains("could not save a crash report"),
            "{message}"
        );

        test_support::remove(&file);
    }

    #[test]
    fn a_stamped_report_captures_this_process_and_a_real_backtrace() {
        let report = CrashReport::new(Some("src/lib.rs:1:1".to_owned()), "boom".to_owned());

        assert_eq!(report.pid, std::process::id());
        assert!(report.seconds > 0);
        assert!(!report.backtrace.is_empty());
        assert!(report.render().contains("boom"), "{}", report.render());
    }

    #[test]
    fn report_dir_prefers_a_non_empty_override_and_falls_back_to_temp() {
        assert_eq!(
            report_dir_from(Some(OsString::from("/var/kinjo-crashes"))),
            PathBuf::from("/var/kinjo-crashes")
        );
        assert_eq!(report_dir_from(Some(OsString::new())), std::env::temp_dir());
        assert_eq!(report_dir_from(None), std::env::temp_dir());
    }

    #[test]
    fn panic_messages_are_recovered_from_both_string_payload_shapes() {
        assert_eq!(panic_message(&"boom"), "boom");
        assert_eq!(panic_message(&"boom".to_owned()), "boom");
        assert_eq!(panic_message(&42_u32), "<non-string panic payload>");
    }

    #[test]
    fn unix_seconds_is_past_the_epoch() {
        assert!(unix_seconds() > 0);
    }

    /// End-to-end proof that `install` wires the hook to the filesystem: a real
    /// panic must leave a report. The file name ends in this process's pid, so it
    /// is found without steering `KINJO_CRASH_DIR` (which would race other
    /// threads). No other test installs a panic hook, so restoring the default
    /// afterwards keeps a later assertion failure reporting normally.
    #[test]
    fn the_installed_hook_writes_a_report_on_a_real_panic() {
        let pid = std::process::id();

        install();
        let outcome = std::panic::catch_unwind(|| panic!("wired up"));
        let _ = std::panic::take_hook();
        assert!(outcome.is_err());

        let suffix = format!("-{pid}.log");
        let report = std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("kinjo-crash-") && name.ends_with(&suffix))
            })
            .expect("the hook wrote no crash report to the temp dir");

        let body = std::fs::read_to_string(&report).unwrap();
        assert!(body.contains("wired up"), "{body}");
        let _ = std::fs::remove_file(&report);
    }
}
