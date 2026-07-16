use std::{ffi::OsString, path::PathBuf};

use clap::{Arg, ArgAction, ArgMatches, Command, error::ErrorKind};

use crate::discovery::{DiscoveryBackend, DiscoveryConfig, DiscoveryOptionError, DiscoveryOptions};

#[derive(Debug, Clone)]
pub struct Cli {
    pub domain: String,
    pub config_dirs: Vec<PathBuf>,
    pub service_type: Option<String>,
    pub backend: DiscoveryBackend,
    pub command: CliCommand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliCommand {
    Run,
    ListCommands,
}

/// A discovery option rejected after Clap has parsed its raw value.
///
/// The flag and domain error stay structured and unescaped until the process
/// output boundary renders them alongside Clap's usage text.
#[derive(Debug)]
pub struct DiscoveryUsageError {
    flag: &'static str,
    source: DiscoveryOptionError,
}

impl std::fmt::Display for DiscoveryUsageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "invalid value for `{}`: {}",
            self.flag, self.source
        )
    }
}

impl std::error::Error for DiscoveryUsageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

impl Cli {
    /// Project the CLI options onto the inputs the discovery layer needs, so
    /// discovery stays decoupled from CLI parsing. The result is a *request*;
    /// [`discovery_options`](Self::discovery_options) is what can be started.
    pub fn discovery_config(&self) -> DiscoveryConfig {
        DiscoveryConfig {
            backend: self.backend,
            domain: self.domain.clone(),
            service_type: self.service_type.clone(),
        }
    }

    /// The validated discovery inputs, or the usage error explaining why these
    /// options cannot be honored.
    ///
    /// Discovery owns the semantics — what a service type may look like and
    /// which backend can browse which domain. The composition root retains the
    /// structured error so it can safely dress the verdict as a usage error at
    /// the final terminal boundary.
    ///
    /// Deliberately not called during [`parse_from`]: `list-commands` never
    /// browses anything, and must not be refused over an option it will not use.
    pub fn discovery_options(&self) -> Result<DiscoveryOptions, DiscoveryUsageError> {
        self.discovery_config().validate().map_err(|source| {
            let flag = match &source {
                DiscoveryOptionError::ServiceType { .. } => "--service-type",
                // The domain is what cannot be honored; the message goes on to
                // explain that `--backend` is the other way out.
                DiscoveryOptionError::UnsupportedDomain { .. } => "--domain",
            };
            DiscoveryUsageError { flag, source }
        })
    }
}

pub(crate) fn usage() -> String {
    command().render_usage().to_string()
}

pub fn parse_from<I, T>(args: I) -> Result<Cli, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let matches = command().try_get_matches_from(args)?;

    // `--config-dir` is repeatable and accepted on either side of
    // `list-commands`. Both sides are merged in occurrence order, so placement
    // never changes which directories are loaded, nor the order they overlay in.
    let (command, config_dirs) = match matches.subcommand() {
        None => (CliCommand::Run, merge_config_dirs(&matches, None)),
        Some(("list-commands", sub)) => (
            CliCommand::ListCommands,
            merge_config_dirs(&matches, Some(sub)),
        ),
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
        backend: match matches.get_one::<String>("backend").map(String::as_str) {
            #[cfg(feature = "fake")]
            Some("fake") => DiscoveryBackend::Fake,
            // Optional backend names remain recognized in every build so the
            // error can explain the feature needed to obtain them.
            #[cfg(not(feature = "fake"))]
            Some("fake") => {
                return Err(self::command().error(
                    ErrorKind::InvalidValue,
                    "the `fake` backend is not compiled into this build; \
                     reinstall with `cargo install kinjo --features fake`",
                ));
            }
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
        .arg(config_dir_arg())
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
            Arg::new("backend")
                .long("backend")
                .help(
                    "Discovery backend to use (`zeroconf` can browse only `local`; `fake` \
                     and `zeroconf` require same-named Cargo features at build time)",
                )
                .value_parser(["mdns-sd", "fake", "zeroconf"])
                .default_value("mdns-sd")
                .value_name("BACKEND"),
        )
        .subcommand(
            Command::new("list-commands")
                .about("Validate and list registered command configs")
                .long_about(
                    "Validate and list registered command configs.\n\n\
                     With no `--config-dir`, the default directories are validated. \
                     With one or more, only those directories are validated, so a \
                     layer can be checked on its own.",
                )
                .arg(config_dir_arg()),
        )
}

/// The one `--config-dir` definition, registered in every context that accepts
/// the flag.
///
/// It is deliberately not `global(true)`. A Clap global option does not append
/// across levels: an occurrence after the subcommand *replaces* the root's
/// values outright — in both the subcommand and the root matches — so the
/// root-position value would be silently discarded. That is precisely the
/// defect this option must never reintroduce, so the flag is registered per
/// context and merged once, in [`merge_config_dirs`].
fn config_dir_arg() -> Arg {
    Arg::new("config-dir")
        .long("config-dir")
        .help("Command directory; repeatable, each overlays the previous")
        .action(ArgAction::Append)
        .value_name("PATH")
}

/// The `--config-dir` occurrences from both sides of the subcommand, in
/// command-line order.
///
/// Clap records each occurrence in the context it was written in. Because the
/// option is not global, the root matches hold exactly the occurrences written
/// *before* the subcommand and `sub` exactly those written *after* it. Chaining
/// root-then-sub therefore reproduces left-to-right occurrence order, which is
/// what the overlay precedence documented in `docs/actions.md` is defined
/// against. No accepted occurrence is dropped, whichever side it was written on.
fn merge_config_dirs(root: &ArgMatches, sub: Option<&ArgMatches>) -> Vec<PathBuf> {
    let occurrences = |matches: &ArgMatches| -> Vec<PathBuf> {
        matches
            .get_many::<String>("config-dir")
            .into_iter()
            .flatten()
            .map(PathBuf::from)
            .collect()
    };

    let mut dirs = occurrences(root);
    dirs.extend(sub.map(occurrences).unwrap_or_default());
    dirs
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

    /// Optional backends stay in the parser's vocabulary even when absent, so
    /// selecting one explains how to obtain it instead of producing a bare
    /// possible-values error.
    #[cfg(not(feature = "fake"))]
    #[test]
    fn fake_backend_without_the_feature_explains_the_fix() {
        let err = parse_from(["kinjo", "--backend", "fake"]).unwrap_err();

        assert_eq!(err.kind(), ErrorKind::InvalidValue);
        assert!(err.to_string().contains("--features fake"));
    }

    #[cfg(feature = "fake")]
    #[test]
    fn parses_fake_backend_selection() {
        let cli = parse_from(["kinjo", "--backend", "fake"]).unwrap();

        assert_eq!(cli.backend.name(), "fake");
    }

    /// The legacy flag is removed outright: keeping an alias would leave two
    /// independent spellings for the same backend and preserve the misleading
    /// model this migration removes.
    #[test]
    fn rejects_the_removed_fake_discovery_flag() {
        let err = parse_from(["kinjo", "--fake-discovery"]).unwrap_err();

        assert_eq!(err.kind(), ErrorKind::UnknownArgument);
    }

    /// Backend help must keep the capability limit established by task 003
    /// alongside the feature-gate guidance added for optional backends.
    #[test]
    fn backend_help_names_the_zeroconf_domain_limit_and_optional_features() {
        let err = parse_from(["kinjo", "--help"]).unwrap_err();
        let help = err
            .to_string()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        assert!(
            help.contains("`zeroconf` can browse only `local`"),
            "{help}"
        );
        assert!(help.contains("same-named Cargo features"), "{help}");
    }

    #[test]
    fn parses_run_options_and_repeatable_config_dirs() {
        let cli = parse_from([
            "kinjo",
            "--domain",
            "corp",
            "--service-type",
            "_ssh._tcp",
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
        assert_eq!(cli.command, CliCommand::Run);
    }

    fn config_dirs_of(args: &[&str]) -> Vec<PathBuf> {
        let cli = parse_from(args.to_vec()).unwrap_or_else(|err| panic!("{args:?}: {err}"));
        assert_eq!(cli.command, CliCommand::ListCommands, "{args:?}");
        cli.config_dirs
    }

    /// A single directory means the same thing on either side of the
    /// subcommand. Reading only the subcommand context used to discard the
    /// root-position value while still reporting success, so `list-commands`
    /// appeared to validate a directory it had never loaded.
    #[test]
    fn one_config_dir_is_equivalent_before_and_after_the_subcommand() {
        let before = config_dirs_of(&["kinjo", "--config-dir", "commands", "list-commands"]);
        let after = config_dirs_of(&["kinjo", "list-commands", "--config-dir", "commands"]);

        assert_eq!(before, vec![PathBuf::from("commands")]);
        assert_eq!(before, after);
    }

    /// Several directories on one side keep their order, and the side they are
    /// written on is not part of the meaning.
    #[test]
    fn repeated_config_dirs_are_equivalent_before_and_after_the_subcommand() {
        let before = config_dirs_of(&[
            "kinjo",
            "--config-dir",
            "base",
            "--config-dir",
            "overrides",
            "list-commands",
        ]);
        let after = config_dirs_of(&[
            "kinjo",
            "list-commands",
            "--config-dir",
            "base",
            "--config-dir",
            "overrides",
        ]);

        assert_eq!(
            before,
            vec![PathBuf::from("base"), PathBuf::from("overrides")]
        );
        assert_eq!(before, after);
    }

    /// Directories written on both sides all survive, in command-line
    /// occurrence order — the order overlay precedence is defined against.
    /// The root-position values must come first because that is where they
    /// were written, not merely be present somewhere in the vector.
    #[test]
    fn mixed_config_dirs_preserve_command_line_occurrence_order() {
        let dirs = config_dirs_of(&[
            "kinjo",
            "--config-dir",
            "first",
            "--config-dir",
            "second",
            "list-commands",
            "--config-dir",
            "third",
            "--config-dir",
            "fourth",
        ]);

        assert_eq!(
            dirs,
            vec![
                PathBuf::from("first"),
                PathBuf::from("second"),
                PathBuf::from("third"),
                PathBuf::from("fourth"),
            ]
        );
    }

    /// Interleaving one on each side is the tightest statement of the rule:
    /// the value written first overlays first, whichever side of the
    /// subcommand name each was written on.
    #[test]
    fn a_root_config_dir_precedes_a_subcommand_one_written_after_it() {
        assert_eq!(
            config_dirs_of(&[
                "kinjo",
                "--config-dir",
                "base",
                "list-commands",
                "--config-dir",
                "overlay",
            ]),
            vec![PathBuf::from("base"), PathBuf::from("overlay")]
        );
    }

    /// `list-commands` with no explicit directory keeps the default-directory
    /// policy: the CLI reports nothing, and `ui::config` expands the defaults.
    #[test]
    fn list_commands_without_config_dirs_reports_none() {
        let cli = parse_from(["kinjo", "list-commands"]).unwrap();

        assert_eq!(cli.command, CliCommand::ListCommands);
        assert!(cli.config_dirs.is_empty());
    }

    /// The flag is documented in both contexts, so neither placement looks
    /// unsupported in `--help`.
    #[test]
    fn config_dir_help_appears_in_both_contexts() {
        for args in [
            vec!["kinjo", "--help"],
            vec!["kinjo", "list-commands", "--help"],
        ] {
            let err = parse_from(args.clone()).unwrap_err();
            let help = err.to_string();

            assert_eq!(err.kind(), ErrorKind::DisplayHelp, "{args:?}");
            assert!(help.contains("--config-dir <PATH>"), "{args:?}: {help}");
        }
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
    #[cfg(feature = "fake")]
    #[test]
    fn fake_backend_accepts_a_custom_domain() {
        let cli = parse_from(["kinjo", "--backend", "fake", "--domain", "corp"]).unwrap();

        let options = cli
            .discovery_options()
            .expect("the sample backend supports custom domains");

        assert_eq!(options.backend(), DiscoveryBackend::Fake);
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
