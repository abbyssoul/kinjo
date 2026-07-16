//! kinjo is built from three deliberately decoupled parts:
//!
//! - [`discovery`] produces *entries* (mDNS today), handing the caller a
//!   [`discovery::DiscoverySession`] that owns the running adapter,
//! - [`plumber`] is the rules engine that matches entries and runs commands
//!   (behind the [`plumber::RuleEngine`] trait),
//! - [`ui`] ties them together for a person at the terminal.
//!
//! [`run`] is the composition root that wires the parts together; the `kinjo`
//! binary is a thin wrapper around it. Exposing these modules as a library also
//! lets the `fuzz/` targets exercise the discovery and parser code directly.

pub mod discovery;
pub mod plumber;
mod terminal;
pub mod ui;

#[cfg(test)]
mod test_support;

use std::{ffi::OsString, process::ExitCode};

use color_eyre::eyre::{Report, Result, WrapErr, eyre};

use plumber::{Matcher, RuleEngine};
use ui::App;
use ui::cli::CliCommand;

/// Source-compatible library entrypoint.
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
    if let Err(report) = color_eyre::install() {
        let _ = write_error_report(&mut stderr, &report);
        return 1;
    }

    run_with_args(std::env::args_os(), &mut stdout, &mut stderr)
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
        .with_config_loader(Box::new(|cli| {
            ui::config::load_matcher(cli)
                .map(|(matcher, warnings)| (Box::new(matcher) as Box<dyn RuleEngine>, warnings))
        }));
    #[cfg(unix)]
    sighup::install(app.reload_requested.clone());

    if !config_warnings.is_empty() {
        app.status = format!(
            "skipped {} command config file(s); details printed on exit",
            config_warnings.len()
        );
    }
    let exec_action = ratatui::run(|terminal| app.run(terminal))?;

    // The app owns the discovery session; dropping it cancels and joins the
    // browse worker before a potential exec hand-off replaces the process.
    drop(app);

    // The status line is transient; repeat skipped-config details somewhere
    // they survive — the terminal scrollback after the TUI has been torn down.
    write_config_warnings(stderr, &config_warnings)?;

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
#[cfg(unix)]
mod sighup {
    use std::sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    };

    /// The flag the installed handler sets. A C signal handler cannot carry
    /// state, so the app's flag is stashed in a process-global.
    static RELOAD_REQUESTED: OnceLock<Arc<AtomicBool>> = OnceLock::new();

    /// Only async-signal-safe work is allowed in a handler; setting an atomic
    /// flag that the event loop polls is the safe idiom.
    extern "C" fn on_sighup(_signal: libc::c_int) {
        if let Some(flag) = RELOAD_REQUESTED.get() {
            flag.store(true, Ordering::Relaxed);
        }
    }

    /// Route SIGHUP to `flag`. Installing the handler also replaces SIGHUP's
    /// default action, which would terminate the process.
    pub(crate) fn install(flag: Arc<AtomicBool>) {
        let _ = RELOAD_REQUESTED.set(flag);
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

        #[test]
        fn sighup_sets_the_reload_flag_instead_of_terminating() {
            let flag = Arc::new(AtomicBool::new(false));
            install(flag.clone());

            // With the handler installed, raising SIGHUP must not kill the
            // process; the handler runs on this thread before `raise` returns.
            unsafe { libc::raise(libc::SIGHUP) };

            assert!(flag.load(Ordering::Relaxed));
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

fn write_config_warnings(
    writer: &mut impl std::io::Write,
    warnings: &[String],
) -> std::io::Result<()> {
    for warning in warnings {
        writeln!(writer, "warning: {}", terminal::text(warning))?;
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

        write_config_warnings(&mut output, &warnings).unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            "warning: bad\\x1B[31m/path\\x0Aconfig\\x07error\n"
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
    fn failed_placeholder_program_is_raw_for_exec_but_safe_in_final_stderr() {
        let mut builder = plumber::MatcherBuilder::new();
        builder
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
            .unwrap();
        let raw_program = "kinjo-no-such\x1b[2J\nprogram.local";
        let mut entry = discovery::Entry::new("remote", "_ssh._tcp", "local");
        entry.hostname = Some(raw_program.to_string());
        let command = builder.build().commands()[0]
            .action
            .prepare(&entry)
            .unwrap();
        assert_eq!(command.argv, [raw_program, "--flag"]);
        let command_line = command.argv.join(" ");

        let report = plumber::exec::exec(command)
            .wrap_err_with(|| format!("failed to run `{command_line}`"))
            .unwrap_err();

        assert!(
            report
                .chain()
                .any(|cause| cause.to_string().contains(raw_program)),
            "the execution layer must retain the exact attempted program"
        );
        let mut stderr = Vec::new();
        write_error_report(&mut stderr, &report).unwrap();
        let stderr = String::from_utf8(stderr).unwrap();
        assert!(
            stderr.contains("kinjo-no-such\\x1B[2J\\x0Aprogram.local"),
            "{stderr:?}"
        );
        assert!(
            stderr
                .chars()
                .all(|character| character == '\n' || !character.is_control()),
            "{stderr:?}"
        );
    }
}
