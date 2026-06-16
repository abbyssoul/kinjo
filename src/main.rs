mod app;
mod cli;
mod config;
mod discovery;
mod filter;
mod keymap;
mod plumber;
mod process;
mod service;
#[cfg(test)]
mod test_support;
mod ui;

use app::App;
use cli::CliCommand;
use color_eyre::eyre::Result;

fn main() -> Result<()> {
    color_eyre::install()?;

    let cli = cli::parse()?;
    let matcher = config::load_matcher(&cli)?;

    if cli.command == CliCommand::ListCommands {
        print_commands(&matcher);
        return Ok(());
    } else if cli.command != CliCommand::Run {
        print!("Unknown command. Use `list-commands` to see available commands.");
        return Ok(());
    }

    let keybindings = config::load_keybindings()?;
    let mut discovery = discovery::start(&cli);
    let discovery_rx = discovery.take_receiver();

    let mut app = App::new(cli, matcher, keybindings, discovery_rx);
    let exec_action = ratatui::run(|terminal| app.run(terminal))?;

    drop(discovery);

    if let Some(action) = exec_action {
        process::exec(action)?;
    }

    Ok(())
}

fn print_commands(matcher: &plumber::Matcher) {
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
