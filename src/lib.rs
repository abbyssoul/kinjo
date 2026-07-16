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
pub mod ui;

#[cfg(test)]
mod test_support;

use color_eyre::eyre::{Result, WrapErr};

use plumber::{Matcher, RuleEngine};
use ui::App;
use ui::cli::CliCommand;

/// Parse CLI arguments, load configuration, and run the TUI — or execute the
/// `list-commands` subcommand. Connects the discovery, plumber, and ui layers.
pub fn run() -> Result<()> {
    color_eyre::install()?;

    let cli = ui::cli::parse();
    let (matcher, config_warnings) = ui::config::load_matcher(&cli)?;

    if cli.command == CliCommand::ListCommands {
        print_commands(&matcher);
        return Ok(());
    }

    let keybindings = ui::config::load_keybindings()?;
    // Validate the discovery options once, here, before any adapter starts: a
    // malformed service type or a domain the chosen backend cannot honor is a
    // usage error, not something to quietly reinterpret into a broader browse.
    // `list-commands` has already returned above, so it never has to answer for
    // discovery options it was never going to use.
    let options = cli.discovery_options().unwrap_or_else(|err| err.exit());
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
    for warning in &config_warnings {
        eprintln!("warning: {warning}");
    }

    if let Some(action) = exec_action {
        let command_line = action.argv.join(" ");
        plumber::exec::exec(action).wrap_err_with(|| format!("failed to run `{command_line}`"))?;
    }

    Ok(())
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

fn print_commands(matcher: &Matcher) {
    println!("{:<22} {:<8} {:<36} COMMAND", "NAME", "MODE", "DESCRIPTION");
    for command in matcher.commands() {
        println!(
            "{:<22} {:<8} {:<36} {}",
            command.name,
            command.action.mode,
            command.description.as_deref().unwrap_or(""),
            command.action.command
        );
    }
}
