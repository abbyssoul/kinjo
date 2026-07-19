//! kinjo is built from three deliberately decoupled parts:
//!
//! - [`discovery`] produces *entries* (mDNS today), handing the caller a
//!   [`discovery::DiscoverySession`] that owns the running adapter,
//! - [`plumber`] is the rules engine that matches entries and runs commands,
//! - [`ui`] ties them together for a person at the terminal.
//!
//! [`run`] is the composition root that wires the parts together; the `kinjo`
//! binary is a thin wrapper around it. Exposing these modules as a library also
//! lets the `fuzz/` targets exercise the discovery and parser code directly.
//!
//! # Extending kinjo
//!
//! There is exactly one seam a dependent crate can substitute:
//! [`plumber::RuleEngine`], which decides how entries are matched to commands.
//! [`plumber::Matcher`] is the engine kinjo ships; [`ui::App::new`] accepts any
//! implementor. See
//! `docs/adr/0001-rule-engine-is-a-supported-extension-point.md`.
//!
//! Reaching it means writing your own composition root. [`run`] is the concrete
//! default path — it loads a [`plumber::Matcher`] from command files and uses it
//! directly, so it is not generic over the engine. To substitute one, do what
//! `run` does: construct a [`ui::App`] with your engine, attach a config loader
//! and discovery factory, and run it against a terminal.
//!
//! Discovery is deliberately *not* such a seam. Its adapters differ in how they
//! browse but not in how a caller runs and stops them, so the seam sits inside
//! [`discovery`] at the browse loop, behind one concrete
//! [`discovery::DiscoverySession`] and a closed [`discovery::DiscoveryBackend`]
//! enum. Adding a backend is a change to this crate, not something a dependent
//! can do from outside.

mod config_home;
mod crash;
pub mod discovery;
pub mod plumber;
mod terminal;
pub mod ui;

#[cfg(test)]
mod test_support;

use std::{ffi::OsString, process::ExitCode, sync::Once};

use color_eyre::eyre::{Report, Result, WrapErr, eyre};

use plumber::{Matcher, RuleEngine};
use ui::App;
use ui::app::ReloadOutcome;
use ui::cli::CliCommand;

/// Source-compatible library entrypoint. Sequential calls in one process are
/// supported; each call becomes the current SIGHUP reload target.
///
/// Detailed diagnostics are written through the same safe process boundary as
/// the binary. A caller receives only a static summary when the process runner
/// fails, so printing the returned report cannot reveal raw dynamic text.
pub fn run() -> Result<()> {
    let exit_code = process_exit_code();
    if exit_code == 0 {
        Ok(())
    } else {
        Err(eyre!("kinjo exited with status {exit_code}"))
    }
}

/// Binary entrypoint that owns Kinjo's stdout/stderr formatting and exit code.
pub fn process_main() -> ExitCode {
    ExitCode::from(process_exit_code())
}

fn process_exit_code() -> u8 {
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    install_error_hook();

    run_with_args(std::env::args_os(), &mut stdout, &mut stderr)
}

/// Install enhanced diagnostics once when possible. Another library may have
/// installed a report/panic hook first; that already satisfies the process
/// invariant and is not an invocation failure.
fn install_error_hook() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let _ = color_eyre::install();
        // Chain crash-report persistence onto whatever panic hook is now in
        // place, so a panic also leaves a file a bug report can attach.
        crash::install();
    });
}

fn run_with_args<I, T>(
    args: I,
    stdout: &mut impl std::io::Write,
    stderr: &mut impl std::io::Write,
) -> u8
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<OsString>>();
    let untrusted_values = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let cli = match ui::cli::parse_from(args) {
        Ok(cli) => cli,
        Err(error) => return report_cli_error(stdout, stderr, error, &untrusted_values),
    };

    match run_invocation(cli, stdout, stderr) {
        Ok(()) => 0,
        Err(RunFailure::DiscoveryUsage(error)) => write_discovery_option_error(stderr, &error)
            .ok()
            .and_then(|code| u8::try_from(code).ok())
            .unwrap_or(1),
        Err(RunFailure::Report(report)) => {
            let _ = write_error_report(stderr, &report);
            1
        }
    }
}

enum RunFailure {
    DiscoveryUsage(ui::cli::DiscoveryUsageError),
    Report(Report),
}

impl From<Report> for RunFailure {
    fn from(report: Report) -> Self {
        Self::Report(report)
    }
}

impl From<std::io::Error> for RunFailure {
    fn from(error: std::io::Error) -> Self {
        Self::Report(error.into())
    }
}

fn run_invocation(
    cli: ui::cli::Cli,
    stdout: &mut impl std::io::Write,
    stderr: &mut impl std::io::Write,
) -> std::result::Result<(), RunFailure> {
    let (matcher, config_warnings) = ui::config::load_matcher(&cli)?;

    if cli.command == CliCommand::ListCommands {
        write_commands(stdout, &matcher)?;
        return Ok(());
    }

    let keybindings = ui::config::load_keybindings()?;
    // Validate the discovery options once, here, before any adapter starts: a
    // malformed service type or a domain the chosen backend cannot honor is a
    // usage error, not something to quietly reinterpret into a broader browse.
    // `list-commands` has already returned above, so it never has to answer for
    // discovery options it was never going to use.
    let options = cli
        .discovery_options()
        .map_err(RunFailure::DiscoveryUsage)?;
    // One value carries the running adapter and its events, so composition has
    // nothing to take apart and reattach.
    let session = discovery::start(&options);

    let mut app = App::new(cli, matcher, keybindings, session)
        // The factory lets the app's refresh command start a replacement
        // session. It captures the validated options, so a refresh re-runs
        // exactly the browse that startup did and cannot re-derive a different
        // (or unchecked) one.
        .with_discovery_factory(Box::new(move || discovery::start(&options)))
        // Startup above loaded leniently; a reload loads transactionally. The
        // app is handed a rule set only when the whole configured overlay
        // compiled, so it never has to choose between a partial rule set and
        // the working one it already has.
        .with_config_loader(Box::new(|cli| match ui::config::reload_matcher(cli) {
            Ok(matcher) => ReloadOutcome::Loaded(Box::new(matcher) as Box<dyn RuleEngine>),
            Err(diagnostics) => ReloadOutcome::Rejected(diagnostics),
        }));
    #[cfg(unix)]
    sighup::install(app.reload_trigger());

    app.note_skipped_configs(config_warnings.len());
    let exec_action = ratatui::run(|terminal| app.run(terminal))?;

    // Take the diagnostics of the last rejected reload out before the app goes:
    // they are the only record of it, and the status line that announced it is
    // already gone with the terminal.
    let reload_diagnostics = app.take_reload_diagnostics();

    // The app owns the discovery session; dropping it cancels and joins the
    // browse worker before a potential exec hand-off replaces the process.
    drop(app);

    // The status line is transient; repeat config details somewhere they
    // survive — the terminal scrollback after the TUI has been torn down.
    write_config_diagnostics(stderr, "warning", &config_warnings)?;
    // A rejected reload is not a warning about what was skipped: it is why the
    // rules the user edited never took effect. Distinguish the two.
    write_config_diagnostics(stderr, "reload rejected", &reload_diagnostics)?;

    if let Some(action) = exec_action {
        let command_line = action.argv.join(" ");
        plumber::exec::exec(action).wrap_err_with(|| format!("failed to run `{command_line}`"))?;
    }

    Ok(())
}

fn report_cli_error(
    stdout: &mut impl std::io::Write,
    stderr: &mut impl std::io::Write,
    error: clap::Error,
    untrusted_values: &[String],
) -> u8 {
    let use_stderr = error.use_stderr();
    let result = if use_stderr {
        write_cli_error(stderr, error, untrusted_values)
    } else {
        write_cli_error(stdout, error, untrusted_values)
    };
    result
        .ok()
        .and_then(|code| u8::try_from(code).ok())
        .unwrap_or(1)
}

/// SIGHUP → reload command configs: the conventional "re-read your
/// configuration" signal for long-running programs. Unix-only; other
/// platforms simply never set the app's reload flag.
///
/// SIGHUP means two unrelated things to a program that owns a terminal, and
/// Kinjo is on the receiving end of both: an administrator asking a
/// long-running process to re-read its configuration, and the kernel reporting
/// that the terminal has hung up. Telling them apart is this module's whole
/// job, because getting it wrong is not survivable — see [`on_sighup`].
#[cfg(unix)]
mod sighup {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicPtr, Ordering},
    };

    /// The flag the installed handler sets. A C signal handler cannot carry
    /// state, so the app's flag is stashed in a process-global.
    static RELOAD_REQUESTED: AtomicPtr<AtomicBool> = AtomicPtr::new(std::ptr::null_mut());

    /// Whether Kinjo had a terminal when the handler was installed.
    ///
    /// A process that never had one cannot be hung up on, so its SIGHUP is
    /// unambiguously a reload request. This is what keeps the reload path
    /// working when Kinjo's input is not a terminal at all — a test binary,
    /// most obviously, where treating a plain `raise(SIGHUP)` as a hangup would
    /// take the whole run down.
    static HAD_TERMINAL: AtomicBool = AtomicBool::new(false);

    /// Whether the terminal Kinjo reads from is still there.
    ///
    /// Asked of `STDIN_FILENO` specifically: that is the descriptor the input
    /// source polls, so it is the one whose death matters. A live terminal
    /// answers; one whose other end has gone fails with `EIO`. POSIX lists
    /// `tcgetattr` as async-signal-safe, so the handler is allowed to ask.
    fn terminal_is_alive() -> bool {
        // SAFETY: `tcgetattr` only writes through the pointer we give it, and
        // is async-signal-safe.
        unsafe {
            let mut termios = std::mem::zeroed::<libc::termios>();
            libc::tcgetattr(libc::STDIN_FILENO, &mut termios) == 0
        }
    }

    /// Reload, or die with the terminal.
    ///
    /// Installing a handler at all replaced SIGHUP's default action, which was
    /// to terminate the process — and for a TUI that default was load-bearing.
    /// A hangup leaves the event loop with no terminal, and it cannot recover:
    /// crossterm's input source busy-reads the EOF a dead tty reports and never
    /// returns, so nothing downstream — not the next draw, not the next poll —
    /// ever gets the chance to notice and exit. What is left is an orphan
    /// spinning at 100% CPU, outliving the terminal that started it.
    ///
    /// So a hangup ends the process here, which is exactly what would have
    /// happened had this handler never been installed. Only a SIGHUP that
    /// arrives while the terminal is still there is a reload request.
    extern "C" fn on_sighup(_signal: libc::c_int) {
        if HAD_TERMINAL.load(Ordering::Relaxed) && !terminal_is_alive() {
            // `_exit` is async-signal-safe, and there is no terminal left to
            // restore or report to. 128 + the signal number is the conventional
            // encoding for "died of this signal".
            unsafe { libc::_exit(128 + libc::SIGHUP) };
        }
        // Only async-signal-safe work is allowed in a handler; setting an
        // atomic flag that the event loop polls is the safe idiom.
        let flag = RELOAD_REQUESTED.load(Ordering::Acquire);
        if !flag.is_null() {
            // SAFETY: `install` turns each Arc into a process-lifetime raw
            // reference before publishing it. Published targets are never
            // freed, so a signal can safely race a later pointer replacement.
            unsafe { (*flag).store(true, Ordering::Relaxed) };
        }
    }

    /// Route SIGHUP to `flag`, and record whether there is a terminal to lose.
    pub(crate) fn install(flag: Arc<AtomicBool>) {
        // Signal handlers cannot lock or participate in Arc reference counts.
        // Keep each rare, per-run flag alive for the process lifetime and swap
        // one raw pointer atomically; this makes sequential composition-root
        // invocations re-pointable without a use-after-free race.
        let flag = Arc::into_raw(flag).cast_mut();
        RELOAD_REQUESTED.store(flag, Ordering::Release);
        HAD_TERMINAL.store(terminal_is_alive(), Ordering::Relaxed);
        unsafe {
            libc::signal(
                libc::SIGHUP,
                on_sighup as extern "C" fn(libc::c_int) as libc::sighandler_t,
            );
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::Mutex;

        static SIGNAL_TEST: Mutex<()> = Mutex::new(());

        /// This test is also the hangup path's canary. It runs in-process, so
        /// if the handler ever misread a plain `raise` as a hangup it would
        /// `_exit` and take the whole run with it — a failure far louder than
        /// an assertion.
        ///
        /// It passes either way the runner is started, and that is the point of
        /// the two-part condition in [`on_sighup`]: under a pipe there is no
        /// terminal to lose, and under a tty the terminal is still there.
        /// Neither is a hangup.
        #[test]
        fn sighup_sets_the_reload_flag_instead_of_terminating() {
            let _guard = SIGNAL_TEST.lock().unwrap();
            let flag = Arc::new(AtomicBool::new(false));
            install(flag.clone());

            // With the handler installed, raising SIGHUP must not kill the
            // process; the handler runs on this thread before `raise` returns.
            unsafe { libc::raise(libc::SIGHUP) };

            assert!(flag.load(Ordering::Relaxed));
        }

        #[test]
        fn reinstall_routes_sighup_to_the_latest_flag() {
            let _guard = SIGNAL_TEST.lock().unwrap();
            let first = Arc::new(AtomicBool::new(false));
            let second = Arc::new(AtomicBool::new(false));
            install(first.clone());
            install(second.clone());

            unsafe { libc::raise(libc::SIGHUP) };

            assert!(!first.load(Ordering::Relaxed));
            assert!(second.load(Ordering::Relaxed));
        }

        /// The handler's entire decision rests on this, so it must be total —
        /// answering for a pipe or a file rather than erroring — and it must
        /// agree with the independent question "is stdin a terminal?".
        #[test]
        fn terminal_liveness_agrees_with_whether_stdin_is_a_terminal() {
            let is_a_terminal = unsafe { libc::isatty(libc::STDIN_FILENO) == 1 };

            assert_eq!(terminal_is_alive(), is_a_terminal);
        }
    }
}

fn write_commands(writer: &mut impl std::io::Write, matcher: &Matcher) -> std::io::Result<()> {
    let rows = matcher
        .commands()
        .iter()
        .map(|command| {
            (
                terminal::text(&command.name),
                terminal::text(&command.action.mode.to_string()),
                terminal::text(command.description.as_deref().unwrap_or("")),
                terminal::text(&command.action.command),
            )
        })
        .collect::<Vec<_>>();
    let name_width = column_width("NAME", 22, rows.iter().map(|row| row.0.as_str()));
    let mode_width = column_width("MODE", 8, rows.iter().map(|row| row.1.as_str()));
    let description_width = column_width("DESCRIPTION", 36, rows.iter().map(|row| row.2.as_str()));

    write_command_row(
        writer,
        ("NAME", "MODE", "DESCRIPTION", "COMMAND"),
        (name_width, mode_width, description_width),
    )?;
    for (name, mode, description, command) in &rows {
        write_command_row(
            writer,
            (name, mode, description, command),
            (name_width, mode_width, description_width),
        )?;
    }
    Ok(())
}

fn column_width<'a>(heading: &str, minimum: usize, values: impl Iterator<Item = &'a str>) -> usize {
    values
        .map(terminal::width)
        .chain(std::iter::once(terminal::width(heading)))
        .fold(minimum, usize::max)
}

fn write_command_row(
    writer: &mut impl std::io::Write,
    columns: (&str, &str, &str, &str),
    widths: (usize, usize, usize),
) -> std::io::Result<()> {
    let (name, mode, description, command) = columns;
    let (name_width, mode_width, description_width) = widths;
    write!(writer, "{name}")?;
    write_padding(writer, name_width.saturating_sub(terminal::width(name)) + 1)?;
    write!(writer, "{mode}")?;
    write_padding(writer, mode_width.saturating_sub(terminal::width(mode)) + 1)?;
    write!(writer, "{description}")?;
    write_padding(
        writer,
        description_width.saturating_sub(terminal::width(description)) + 1,
    )?;
    writeln!(writer, "{command}")
}

fn write_padding(writer: &mut impl std::io::Write, columns: usize) -> std::io::Result<()> {
    write!(writer, "{:columns$}", "")
}

/// Print config diagnostics under `label`, one per line. Every message quotes a
/// path and an error from a file the process did not write, so each goes through
/// the same escaping as any other untrusted text.
fn write_config_diagnostics(
    writer: &mut impl std::io::Write,
    label: &str,
    diagnostics: &[String],
) -> std::io::Result<()> {
    for diagnostic in diagnostics {
        writeln!(writer, "{label}: {}", terminal::text(diagnostic))?;
    }
    Ok(())
}

fn write_cli_error(
    writer: &mut impl std::io::Write,
    mut error: clap::Error,
    untrusted_values: &[String],
) -> std::io::Result<i32> {
    use clap::error::ContextValue;

    let exit_code = error.exit_code();
    let safe_context = error
        .context()
        .filter_map(|(kind, value)| match value {
            ContextValue::String(value) => {
                Some((kind, ContextValue::String(terminal::text(value))))
            }
            ContextValue::Strings(values) => Some((
                kind,
                ContextValue::Strings(values.iter().map(|value| terminal::text(value)).collect()),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    for (kind, value) in safe_context {
        error.insert(kind, value);
    }

    let mut rendered = error.render().to_string();
    let mut unsafe_values = untrusted_values
        .iter()
        .filter(|value| value.chars().any(char::is_control))
        .collect::<Vec<_>>();
    // Replace larger values first in case one user argument contains another.
    unsafe_values
        .sort_unstable_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
    unsafe_values.dedup();
    for value in unsafe_values {
        rendered = rendered.replace(value, &terminal::text(value));
    }

    writer.write_all(rendered.as_bytes())?;
    Ok(exit_code)
}

fn write_error_report(
    writer: &mut impl std::io::Write,
    report: &color_eyre::eyre::Report,
) -> std::io::Result<()> {
    for (index, cause) in report.chain().enumerate() {
        let message = terminal::text(&cause.to_string());
        if index == 0 {
            writeln!(writer, "error: {message}")?;
        } else {
            writeln!(writer, "  caused by: {message}")?;
        }
    }
    Ok(())
}

fn write_discovery_option_error(
    writer: &mut impl std::io::Write,
    error: &ui::cli::DiscoveryUsageError,
) -> std::io::Result<i32> {
    writeln!(writer, "error: {}", terminal::text(&error.to_string()))?;
    writeln!(writer)?;
    writeln!(writer, "{}", ui::cli::usage())?;
    writeln!(writer)?;
    writeln!(writer, "For more information, try '--help'.")?;
    Ok(2)
}

#[cfg(test)]
mod terminal_output_tests {
    use super::*;
    use color_eyre::eyre::eyre;
    use ratatui::text::Line;

    #[test]
    fn config_warnings_render_controls_as_inert_text() {
        let warnings = vec!["bad\x1b[31m/path\nconfig\x07error".to_string()];
        let mut output = Vec::new();

        write_config_diagnostics(&mut output, "warning", &warnings).unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            "warning: bad\\x1B[31m/path\\x0Aconfig\\x07error\n"
        );
    }

    /// The exit report is the only place a rejected reload's detail survives to,
    /// and it must say which reload it is talking about: a file skipped at
    /// startup and a reload that never took effect call for different actions.
    #[test]
    fn rejected_reload_details_are_labelled_apart_from_startup_warnings() {
        let mut output = Vec::new();

        write_config_diagnostics(
            &mut output,
            "warning",
            &["/etc/kinjo/commands/old.toml: bad".to_string()],
        )
        .unwrap();
        write_config_diagnostics(
            &mut output,
            "reload rejected",
            &["/home/u/.config/kinjo/commands/ssh.toml: unterminated quote".to_string()],
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert_eq!(
            output.lines().collect::<Vec<_>>(),
            [
                "warning: /etc/kinjo/commands/old.toml: bad",
                "reload rejected: /home/u/.config/kinjo/commands/ssh.toml: unterminated quote",
            ],
            "each diagnostic keeps its full source path and message"
        );
    }

    #[test]
    fn command_listing_escapes_dynamic_columns_and_aligns_by_display_width() {
        let mut builder = plumber::MatcherBuilder::new();
        builder
            .add_str(
                "unsafe-command.toml",
                r#"
[metadata]
name = "名\u001B🙂"
description = "界e\u0301\u0007"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "echo \u0085終"
mode = "execute"
"#,
            )
            .unwrap();
        builder
            .add_str(
                "wide-command.toml",
                r#"
[metadata]
name = "a very long printable command name"
description = "plain"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "printf ok"
mode = "fork"
"#,
            )
            .unwrap();
        let matcher = builder.build();
        let mut output = Vec::new();

        write_commands(&mut output, &matcher).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("名\\x1B🙂"), "{output:?}");
        assert!(output.contains("界e\u{301}\\x07"), "{output:?}");
        assert!(output.contains("echo \\x85終"), "{output:?}");
        assert!(
            output
                .chars()
                .all(|character| character == '\n' || !character.is_control()),
            "{output:?}"
        );

        let command_columns = output
            .lines()
            .map(|line| {
                let command = if line.starts_with("NAME") {
                    "COMMAND"
                } else if line.contains("printf ok") {
                    "printf ok"
                } else {
                    "echo"
                };
                let byte_index = line.rfind(command).unwrap();
                Line::from(&line[..byte_index]).width()
            })
            .collect::<Vec<_>>();
        assert!(
            command_columns.windows(2).all(|pair| pair[0] == pair[1]),
            "COMMAND columns were {command_columns:?}: {output:?}"
        );
    }

    #[test]
    fn cli_parse_errors_escape_rejected_values_and_keep_usage_context() {
        let error = ui::cli::parse_from(["kinjo", "--backend", "bad\x1b[31m\nvalue"]).unwrap_err();
        let mut output = Vec::new();

        let exit_code =
            write_cli_error(&mut output, error, &["bad\x1b[31m\nvalue".to_string()]).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert_eq!(exit_code, 2);
        assert!(output.contains("bad\\x1B[31m\\x0Avalue"), "{output:?}");
        assert!(output.contains("--backend <BACKEND>"), "{output:?}");
        assert!(output.contains("--help"), "{output:?}");
        assert!(
            output
                .chars()
                .all(|character| character == '\n' || !character.is_control()),
            "{output:?}"
        );
    }

    #[test]
    fn discovery_option_errors_escape_rejected_values_and_keep_usage_context() {
        let cli = ui::cli::parse_from(["kinjo", "--service-type", "bad\x1b[2J\nservice"]).unwrap();
        let error = cli.discovery_options().unwrap_err();
        let mut output = Vec::new();

        let exit_code = write_discovery_option_error(&mut output, &error).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert_eq!(exit_code, 2);
        assert!(output.contains("bad\\x1B[2J\\x0Aservice"), "{output:?}");
        assert!(output.contains("--service-type"), "{output:?}");
        assert!(output.contains("Usage: kinjo"), "{output:?}");
        assert!(output.contains("--help"), "{output:?}");
        assert!(
            output
                .chars()
                .all(|character| character == '\n' || !character.is_control()),
            "{output:?}"
        );
    }

    #[test]
    fn final_error_report_escapes_every_cause_without_flattening_the_chain() {
        let report = Err::<(), _>(eyre!("inner\x07\nreason"))
            .wrap_err("middle\x1b[31m cause")
            .wrap_err("outer\u{85} context")
            .unwrap_err();
        let mut output = Vec::new();

        write_error_report(&mut output, &report).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("error: outer\\x85 context"), "{output:?}");
        assert!(
            output.contains("caused by: middle\\x1B[31m cause"),
            "{output:?}"
        );
        assert!(
            output.contains("caused by: inner\\x07\\x0Areason"),
            "{output:?}"
        );
        assert!(
            output
                .chars()
                .all(|character| character == '\n' || !character.is_control()),
            "{output:?}"
        );
    }

    #[test]
    fn process_entrypoint_routes_safe_usage_errors_to_stderr_with_code_two() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let exit_code = run_with_args(
            ["kinjo", "--backend", "bad\x1b[31m\nvalue"],
            &mut stdout,
            &mut stderr,
        );

        assert_eq!(exit_code, 2);
        assert!(stdout.is_empty());
        let stderr = String::from_utf8(stderr).unwrap();
        assert!(stderr.contains("bad\\x1B[31m\\x0Avalue"), "{stderr:?}");
        assert!(stderr.contains("--help"), "{stderr:?}");
    }

    /// Optional backend selection must stay inside the non-exiting parser and
    /// explicit process-output boundary introduced by task 020.
    #[cfg(not(feature = "fake"))]
    #[test]
    fn unavailable_fake_backend_reports_its_feature_through_the_process_boundary() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let exit_code = run_with_args(["kinjo", "--backend", "fake"], &mut stdout, &mut stderr);

        assert_eq!(exit_code, 2);
        assert!(stdout.is_empty());
        let stderr = String::from_utf8(stderr).unwrap();
        assert!(stderr.contains("--features fake"), "{stderr:?}");
        assert!(stderr.contains("Usage: kinjo"), "{stderr:?}");
    }

    #[test]
    fn public_entrypoint_signatures_keep_library_and_binary_contracts_separate() {
        let _: fn() -> color_eyre::eyre::Result<()> = run;
        let _: fn() -> ExitCode = process_main;
    }

    #[test]
    fn process_entrypoint_keeps_clap_help_on_stdout_with_success_status() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let exit_code = run_with_args(["kinjo", "--help"], &mut stdout, &mut stderr);

        assert_eq!(exit_code, 0);
        assert!(stderr.is_empty());
        let stdout = String::from_utf8(stdout).unwrap();
        assert!(stdout.contains("Usage: kinjo"), "{stdout:?}");
        assert!(stdout.contains("--service-type"), "{stdout:?}");
    }

    #[test]
    fn a_placeholder_program_is_rejected_at_the_config_seam() {
        let mut builder = plumber::MatcherBuilder::new();
        let err = builder
            .add_str(
                "placeholder-program.toml",
                r#"
[metadata]
name = "remote-program"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "{hostname} --flag"
mode = "execute"
"#,
            )
            .unwrap_err()
            .to_string();

        assert!(err.contains("program must be literal"), "{err}");
        assert!(err.contains("placeholder-program.toml"), "{err}");
    }
}
