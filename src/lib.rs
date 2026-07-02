//! avahi-tui is built from three deliberately decoupled parts:
//!
//! - [`discovery`] produces *entries* (mDNS today, swappable behind the
//!   [`discovery::Discovery`] trait),
//! - [`plumber`] is the rules engine that matches entries and runs commands
//!   (behind the [`plumber::RuleEngine`] trait),
//! - [`ui`] ties them together for a person at the terminal.
//!
//! [`run`] is the composition root that wires the parts together; the `avahi-tui`
//! binary is a thin wrapper around it. Exposing these modules as a library also
//! lets the `fuzz/` targets exercise the discovery and parser code directly.

pub mod discovery;
pub mod plumber;
pub mod ui;

#[cfg(test)]
mod test_support;

use color_eyre::eyre::{Result, WrapErr};

use plumber::Matcher;
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
    let mut discovery = discovery::start(&cli.discovery_config());
    let discovery_rx = discovery.events();

    let mut app = App::new(cli, matcher, keybindings, discovery_rx);
    if !config_warnings.is_empty() {
        app.status = format!(
            "skipped {} command config file(s); details printed on exit",
            config_warnings.len()
        );
    }
    let exec_action = ratatui::run(|terminal| app.run(terminal))?;

    drop(discovery);

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
