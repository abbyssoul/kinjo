use std::{ffi::OsString, path::PathBuf};

use clap::{Arg, ArgAction, Command, error::ErrorKind};

use crate::discovery::{DiscoveryBackend, DiscoveryConfig};

#[derive(Debug, Clone)]
pub struct Cli {
    pub domain: String,
    pub config_dirs: Vec<PathBuf>,
    pub service_type: Option<String>,
    pub fake_discovery: bool,
    pub backend: DiscoveryBackend,
    pub command: CliCommand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliCommand {
    Run,
    ListCommands,
}

impl Cli {
    /// Project the CLI options onto the inputs the discovery layer needs, so
    /// discovery stays decoupled from CLI parsing.
    pub fn discovery_config(&self) -> DiscoveryConfig {
        DiscoveryConfig {
            fake: self.fake_discovery,
            backend: self.backend,
            domain: self.domain.clone(),
            service_type: self.service_type.clone(),
        }
    }
}

/// Parse the process arguments. On a usage error (unknown flag, bad subcommand,
/// invalid value) clap renders a friendly message — with usage hints — to stderr
/// and exits, rather than surfacing a `color_eyre` report with code locations.
/// `--help`/`--version` likewise print and exit through the same path.
pub fn parse() -> Cli {
    parse_from(std::env::args_os()).unwrap_or_else(|err| err.exit())
}

pub fn parse_from<I, T>(args: I) -> Result<Cli, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let matches = command().try_get_matches_from(args)?;

    // `--config-dir` is repeatable and is read from whichever argument context
    // applies: the `list-commands` subcommand carries its own copy of the flag,
    // while a plain run reads the top-level one.
    let (command, config_dirs) = match matches.subcommand() {
        None => (CliCommand::Run, collect_config_dirs(&matches)),
        Some(("list-commands", sub)) => (CliCommand::ListCommands, collect_config_dirs(sub)),
        Some((name, _)) => {
            return Err(command().error(
                ErrorKind::InvalidSubcommand,
                format!(
                    "unknown subcommand `{name}`; use `list-commands` to see available commands"
                ),
            ));
        }
    };

    Ok(Cli {
        domain: matches
            .get_one::<String>("domain")
            .cloned()
            .unwrap_or_else(|| "local".to_string()),
        config_dirs,
        service_type: matches.get_one::<String>("service-type").cloned(),
        fake_discovery: matches.get_flag("fake-discovery"),
        backend: match matches.get_one::<String>("backend").map(String::as_str) {
            #[cfg(feature = "zeroconf")]
            Some("zeroconf") => DiscoveryBackend::Zeroconf,
            // `zeroconf` stays a recognized value in every build so the CLI
            // surface is stable; without the feature it explains the fix
            // instead of a bare "invalid value".
            #[cfg(not(feature = "zeroconf"))]
            Some("zeroconf") => {
                // `self::` skips the local `command` binding above.
                return Err(self::command().error(
                    ErrorKind::InvalidValue,
                    "the `zeroconf` backend is not compiled into this build; \
                     reinstall with `cargo install kinjo --features zeroconf`",
                ));
            }
            // `mdns-sd` and the default both map to the mdns-sd backend.
            _ => DiscoveryBackend::MdnsSd,
        },
        command,
    })
}

fn command() -> Command {
    Command::new("kinjo")
        .about("TUI browser and launcher for DNS-SD services")
        .arg(
            // A flag rather than a positional so it never competes with the
            // subcommand slot — `kinjo <unknown>` now errors instead of
            // being silently treated as a domain name.
            Arg::new("domain")
                .long("domain")
                .short('d')
                .help("DNS-SD domain to browse")
                .default_value("local")
                .value_name("DOMAIN"),
        )
        .arg(
            Arg::new("config-dir")
                .long("config-dir")
                .help("Extra command directory; repeatable, each overlays the previous")
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
        .arg(
            Arg::new("backend")
                .long("backend")
                .help(if cfg!(feature = "zeroconf") {
                    "mDNS/DNS-SD discovery backend to use"
                } else {
                    "mDNS/DNS-SD discovery backend to use \
                     (zeroconf requires a build with the `zeroconf` feature)"
                })
                .value_parser(["mdns-sd", "zeroconf"])
                .default_value("mdns-sd")
                .value_name("BACKEND"),
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
}

fn collect_config_dirs(matches: &clap::ArgMatches) -> Vec<PathBuf> {
    matches
        .get_many::<String>("config-dir")
        .into_iter()
        .flatten()
        .map(PathBuf::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_run_command() {
        let cli = parse_from(["kinjo"]).unwrap();

        assert_eq!(cli.domain, "local");
        assert!(cli.config_dirs.is_empty());
        assert_eq!(cli.service_type, None);
        assert!(!cli.fake_discovery);
        assert_eq!(cli.backend, DiscoveryBackend::MdnsSd);
        assert_eq!(cli.command, CliCommand::Run);
    }

    #[cfg(feature = "zeroconf")]
    #[test]
    fn parses_zeroconf_backend_selection() {
        let cli = parse_from(["kinjo", "--backend", "zeroconf"]).unwrap();
        assert_eq!(cli.backend, DiscoveryBackend::Zeroconf);
    }

    /// Without the `zeroconf` feature, asking for that backend is a usage
    /// error that names the cargo feature — not a silent fallback, and not a
    /// bare "invalid value".
    #[cfg(not(feature = "zeroconf"))]
    #[test]
    fn zeroconf_backend_without_the_feature_explains_the_fix() {
        let err = parse_from(["kinjo", "--backend", "zeroconf"]).unwrap_err();

        assert_eq!(err.kind(), ErrorKind::InvalidValue);
        assert!(err.to_string().contains("--features zeroconf"));
    }

    #[test]
    fn rejects_unknown_backend() {
        assert!(parse_from(["kinjo", "--backend", "bonjour"]).is_err());
    }

    #[test]
    fn parses_run_options_and_repeatable_config_dirs() {
        let cli = parse_from([
            "kinjo",
            "--domain",
            "corp",
            "--service-type",
            "_ssh._tcp",
            "--fake-discovery",
            "--config-dir",
            "team",
            "--config-dir",
            "local",
        ])
        .unwrap();

        assert_eq!(cli.domain, "corp");
        assert_eq!(
            cli.config_dirs,
            vec![PathBuf::from("team"), PathBuf::from("local")]
        );
        assert_eq!(cli.service_type, Some("_ssh._tcp".to_string()));
        assert!(cli.fake_discovery);
        assert_eq!(cli.command, CliCommand::Run);
    }

    #[test]
    fn parses_list_commands_config_dirs_from_subcommand_context() {
        let cli = parse_from([
            "kinjo",
            "--config-dir",
            "run-only",
            "list-commands",
            "--config-dir",
            "commands",
            "--config-dir",
            "overrides",
        ])
        .unwrap();

        assert_eq!(cli.command, CliCommand::ListCommands);
        assert_eq!(
            cli.config_dirs,
            vec![PathBuf::from("commands"), PathBuf::from("overrides")]
        );
    }

    #[test]
    fn rejects_unknown_subcommand_instead_of_treating_it_as_domain() {
        let err = parse_from(["kinjo", "browse"]).unwrap_err();

        assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
    }
}
