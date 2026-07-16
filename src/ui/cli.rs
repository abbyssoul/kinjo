use std::{ffi::OsString, path::PathBuf};

use clap::{Arg, ArgAction, Command, error::ErrorKind};

use crate::discovery::{DiscoveryBackend, DiscoveryConfig, DiscoveryOptionError, DiscoveryOptions};

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
    /// discovery stays decoupled from CLI parsing. The result is a *request*;
    /// [`discovery_options`](Self::discovery_options) is what can be started.
    pub fn discovery_config(&self) -> DiscoveryConfig {
        DiscoveryConfig {
            fake: self.fake_discovery,
            backend: self.backend,
            domain: self.domain.clone(),
            service_type: self.service_type.clone(),
        }
    }

    /// The validated discovery inputs, or the usage error explaining why these
    /// options cannot be honored.
    ///
    /// Discovery owns the semantics — what a service type may look like, which
    /// backend can browse which domain — and this only dresses its verdict as a
    /// clap error, so the message names the flag the user typed and reads like
    /// every other usage error rather than a panic or an eyre report.
    ///
    /// Deliberately not called during [`parse_from`]: `list-commands` never
    /// browses anything, and must not be refused over an option it will not use.
    pub fn discovery_options(&self) -> Result<DiscoveryOptions, clap::Error> {
        self.discovery_config().validate().map_err(|err| {
            let flag = match &err {
                DiscoveryOptionError::ServiceType { .. } => "--service-type",
                // The domain is what cannot be honored; the message goes on to
                // explain that `--backend` is the other way out.
                DiscoveryOptionError::UnsupportedDomain { .. } => "--domain",
            };
            command().error(
                ErrorKind::InvalidValue,
                format!("invalid value for `{flag}`: {err}"),
            )
        })
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
        .version(env!("CARGO_PKG_VERSION"))
        // Replace clap's default `-V` short with `-v`: nothing else claims the
        // letter, and `-v` is what people type first.
        .disable_version_flag(true)
        .arg(
            Arg::new("version")
                .long("version")
                .short('v')
                .help("Print version")
                .action(ArgAction::Version),
        )
        .arg(
            // A flag rather than a positional so it never competes with the
            // subcommand slot — `kinjo <unknown>` now errors instead of
            // being silently treated as a domain name.
            Arg::new("domain")
                .long("domain")
                .short('d')
                .help("DNS-SD domain to browse (the zeroconf backend supports only `local`)")
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
                .help(
                    "Limit discovery to one DNS-SD service type, as in `_ssh._tcp` \
                     (default: browse every service type)",
                )
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
                    "mDNS/DNS-SD discovery backend to use \
                     (mdns-sd browses any domain; zeroconf only `local`)"
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

    /// `--version` and `-v` surface as a `DisplayVersion` "error" that the
    /// binary turns into print-and-exit; the text must carry the crate version.
    #[test]
    fn version_flag_reports_the_crate_version() {
        for args in [["kinjo", "--version"], ["kinjo", "-v"]] {
            let err = parse_from(args).unwrap_err();

            assert_eq!(err.kind(), ErrorKind::DisplayVersion);
            assert!(err.to_string().contains(env!("CARGO_PKG_VERSION")));
        }
    }

    #[test]
    fn rejects_unknown_subcommand_instead_of_treating_it_as_domain() {
        let err = parse_from(["kinjo", "browse"]).unwrap_err();

        assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
    }

    /// Valid TCP and UDP types reach discovery as validated values, canonical
    /// and browsing exactly the one type that was asked for.
    #[test]
    fn valid_service_types_reach_discovery_as_validated_values() {
        for (typed, canonical) in [
            ("_ssh._tcp", "_ssh._tcp"),
            ("_dns-sd._udp", "_dns-sd._udp"),
            ("_SSH._TCP", "_ssh._tcp"),
        ] {
            let cli = parse_from(["kinjo", "--service-type", typed]).unwrap();

            let options = cli.discovery_options().expect(typed);
            assert_eq!(options.service_type().unwrap().to_string(), canonical);
        }
    }

    /// A malformed `--service-type` is a usage error that names the flag and
    /// the shape of a good value — never a silent widening to "browse all".
    #[test]
    fn a_malformed_service_type_is_a_usage_error_naming_the_flag() {
        for bad in ["bogus", "_ssh", "_ssh._sctp", "_-ssh._tcp", "_s--h._tcp"] {
            let cli = parse_from(["kinjo", "--service-type", bad]).unwrap();

            let err = cli.discovery_options().unwrap_err();

            assert_eq!(
                err.kind(),
                ErrorKind::InvalidValue,
                "`{bad}` must be refused"
            );
            let message = err.to_string();
            assert!(message.contains("--service-type"), "{message}");
            assert!(message.contains(bad), "{message}");
            assert!(message.contains("_ssh._tcp"), "{message}");
        }
    }

    /// No `--service-type` is not a malformed one: it means browse everything.
    #[test]
    fn no_service_type_browses_every_type() {
        let cli = parse_from(["kinjo"]).unwrap();

        let options = cli.discovery_options().unwrap();

        assert_eq!(options.service_type(), None);
    }

    /// The default domain has several spellings and one meaning.
    #[test]
    fn default_domain_spellings_canonicalize() {
        for spelling in ["local", "local.", "LOCAL"] {
            let cli = parse_from(["kinjo", "--domain", spelling]).unwrap();

            let options = cli.discovery_options().expect(spelling);

            assert_eq!(options.domain(), "local");
        }
    }

    /// mdns-sd can set a browse domain, so a custom one is honored exactly.
    #[test]
    fn mdns_sd_accepts_a_custom_domain() {
        let cli = parse_from(["kinjo", "--backend", "mdns-sd", "--domain", "corp"]).unwrap();

        let options = cli.discovery_options().unwrap();

        assert_eq!(options.backend(), DiscoveryBackend::MdnsSd);
        assert_eq!(options.domain(), "corp");
    }

    /// zeroconf's browser has no domain setter, so a custom domain is refused
    /// with text naming the flag and the backend that can do it.
    #[cfg(feature = "zeroconf")]
    #[test]
    fn zeroconf_with_a_custom_domain_is_a_usage_error() {
        let cli = parse_from(["kinjo", "--backend", "zeroconf", "--domain", "corp"]).unwrap();

        let err = cli.discovery_options().unwrap_err();

        assert_eq!(err.kind(), ErrorKind::InvalidValue);
        let message = err.to_string();
        assert!(message.contains("--domain"), "{message}");
        assert!(message.contains("corp"), "{message}");
        assert!(
            message.contains("mdns-sd"),
            "the remedy must be named: {message}"
        );
    }

    /// The default domain is the one zeroconf can browse, in any spelling.
    #[cfg(feature = "zeroconf")]
    #[test]
    fn zeroconf_accepts_the_default_domain() {
        for args in [
            vec!["kinjo", "--backend", "zeroconf"],
            vec!["kinjo", "--backend", "zeroconf", "--domain", "local."],
        ] {
            let cli = parse_from(args.clone()).unwrap();

            let options = cli
                .discovery_options()
                .unwrap_or_else(|err| panic!("{args:?}: {err}"));

            assert_eq!(options.domain(), "local");
        }
    }

    /// Explicit fake discovery exercises no real adapter, so a domain no real
    /// backend could browse is still fine for sample records.
    #[cfg(feature = "zeroconf")]
    #[test]
    fn fake_discovery_accepts_a_domain_no_backend_supports() {
        let cli = parse_from([
            "kinjo",
            "--backend",
            "zeroconf",
            "--domain",
            "corp",
            "--fake-discovery",
        ])
        .unwrap();

        let options = cli
            .discovery_options()
            .expect("fake bypasses capability checks");

        assert!(options.fake());
        assert_eq!(options.domain(), "corp");
    }

    /// `list-commands` validates command files, not the network options it will
    /// never use. Parsing must not refuse it over a bad `--service-type`, and
    /// the run path is what would have rejected the same value.
    #[test]
    fn list_commands_does_not_validate_unused_discovery_options() {
        let cli = parse_from(["kinjo", "--service-type", "bogus", "list-commands"])
            .expect("list-commands must parse regardless of discovery options");

        assert_eq!(cli.command, CliCommand::ListCommands);
        // Nothing in the `list-commands` path asks for discovery options, so
        // the value is never checked and no discovery is started. It is only
        // the run path that would refuse it:
        assert!(cli.discovery_options().is_err());
    }

    /// The same for a backend/domain pair no real adapter could honor.
    #[cfg(feature = "zeroconf")]
    #[test]
    fn list_commands_ignores_an_unsupported_backend_and_domain_pair() {
        let cli = parse_from([
            "kinjo",
            "--backend",
            "zeroconf",
            "--domain",
            "corp",
            "list-commands",
        ])
        .expect("list-commands must parse regardless of discovery options");

        assert_eq!(cli.command, CliCommand::ListCommands);
    }
}
