use std::path::PathBuf;

use clap::{Arg, ArgAction, Command};

#[derive(Debug, Clone)]
pub struct Cli {
    pub domain: String,
    pub config_dirs: Vec<PathBuf>,
    pub service_type: Option<String>,
    pub fake_discovery: bool,
    pub command: CliCommand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliCommand {
    Run,
    ListCommands,
}

pub fn parse() -> Cli {
    let matches = Command::new("avahi-tui")
        .about("TUI browser and launcher for DNS-SD services")
        .arg(
            Arg::new("domain")
                .help("DNS-SD domain to browse")
                .default_value("local")
                .value_name("DOMAIN"),
        )
        .arg(
            Arg::new("config-dir")
                .long("config-dir")
                .help("Additional command config directory")
                .action(ArgAction::Append)
                .value_name("PATH"),
        )
        .arg(
            Arg::new("service-type")
                .long("service-type")
                .help("Limit discovery to one DNS-SD service type")
                .value_name("TYPE"),
        )
        .arg(
            Arg::new("fake-discovery")
                .long("fake-discovery")
                .help("Use built-in sample records instead of mDNS discovery")
                .action(ArgAction::SetTrue),
        )
        .subcommand(
            Command::new("list-commands")
                .about("Validate and list registered command configs")
                .arg(
                    Arg::new("config-dir")
                        .long("config-dir")
                        .help("Command config directory to load")
                        .action(ArgAction::Append)
                        .value_name("PATH"),
                ),
        )
        .get_matches();

    let (command, config_dirs) = if let Some(("list-commands", subcommand)) = matches.subcommand() {
        let config_dirs = collect_config_dirs(subcommand);
        (CliCommand::ListCommands, config_dirs)
    } else {
        (CliCommand::Run, collect_config_dirs(&matches))
    };

    Cli {
        domain: matches
            .get_one::<String>("domain")
            .cloned()
            .unwrap_or_else(|| "local".to_string()),
        config_dirs,
        service_type: matches.get_one::<String>("service-type").cloned(),
        fake_discovery: matches.get_flag("fake-discovery"),
        command,
    }
}

fn collect_config_dirs(matches: &clap::ArgMatches) -> Vec<PathBuf> {
    matches
        .get_many::<String>("config-dir")
        .into_iter()
        .flatten()
        .map(PathBuf::from)
        .collect()
}
