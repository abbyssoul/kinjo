//! Serialization layer for the rules engine: loads command rules from TOML
//! files and builds a [`Matcher`] with the system → user → CLI overlay order.

use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

use color_eyre::eyre::{Result, eyre};
use regex::Regex;
use serde_derive::Deserialize;

use super::{
    ActionMode, CommandAction, CommandConfig, FieldPredicate, Matcher, Predicate, Requirement,
    is_supported_field,
};

#[derive(Debug, Default)]
pub struct MatcherBuilder {
    commands: Vec<CommandConfig>,
    /// command name -> (layer it was last defined in, index into `commands`)
    names: BTreeMap<String, (usize, usize)>,
    layer: usize,
}

impl MatcherBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin a new override layer. Commands added afterwards override same-named
    /// commands from earlier layers; duplicates within one layer remain errors.
    pub fn start_layer(&mut self) {
        self.layer += 1;
    }

    /// Compile one command file into a validated rule and add it to this layer.
    ///
    /// Every failure is attributed to `source_name` here, at the one boundary
    /// that knows it, so a warning from a lenient startup always tells the user
    /// which file to go and fix.
    pub fn add_str(&mut self, source_name: &str, source: &str) -> Result<()> {
        let command = parse_command_config(source).map_err(|err| eyre!("{source_name}: {err}"))?;
        match self.names.get(&command.name).copied() {
            Some((layer, _)) if layer == self.layer => {
                return Err(eyre!(
                    "duplicate command name `{}` in {source_name}",
                    command.name
                ));
            }
            Some((_, index)) => {
                // Same name from an earlier layer: override it in place so the
                // command keeps its original position in the list.
                let name = command.name.clone();
                self.commands[index] = command;
                self.names.insert(name, (self.layer, index));
            }
            None => {
                let index = self.commands.len();
                self.names.insert(command.name.clone(), (self.layer, index));
                self.commands.push(command);
            }
        }
        Ok(())
    }

    pub fn add_file(&mut self, path: &Path) -> Result<()> {
        // An unreadable file is reported with its path too: a lenient startup
        // turns this into a warning, and `cannot read` on its own names nothing.
        let source = fs::read_to_string(path).map_err(|err| eyre!("{}: {err}", path.display()))?;
        self.add_str(&path.display().to_string(), &source)
    }

    pub fn build(self) -> Matcher {
        Matcher {
            commands: self.commands,
        }
    }
}

/// System-wide command directory, loaded as the base layer for every run.
pub const SYSTEM_CONFIG_DIR: &str = "/etc/kinjo/commands";

/// Ordered list of command directories, lowest precedence first. Commands in a
/// later directory override same-named commands from an earlier one:
///
///   1. install-relative (`<exe_dir>/commands`, `<prefix>/share/kinjo/commands`)
///   2. system-wide      (`/etc/kinjo/commands`)
///   3. user-local       (`$XDG_CONFIG_HOME/kinjo/commands` or `~/.config/...`)
///   4. command-line `--config-dir` entries, in the order given
pub fn config_dirs(extra: &[PathBuf]) -> Vec<PathBuf> {
    config_dirs_from(
        env::current_exe().ok(),
        env::var_os("XDG_CONFIG_HOME"),
        env::var_os("HOME"),
        extra,
    )
}

/// Build the ordered command-directory list from the executable location and
/// the relevant environment variables. Split out from [`config_dirs`] so the
/// precedence rules can be unit tested without mutating process-global
/// environment state.
pub(crate) fn config_dirs_from(
    exe: Option<PathBuf>,
    xdg_config_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
    extra: &[PathBuf],
) -> Vec<PathBuf> {
    let mut dirs = install_dirs_from(exe.as_deref());
    dirs.push(PathBuf::from(SYSTEM_CONFIG_DIR));
    if let Some(config_dir) = crate::config_home::kinjo_config_dir(xdg_config_home, home) {
        dirs.push(config_dir.join("commands"));
    }
    dirs.extend(extra.iter().cloned());
    dirs
}

/// Command directories shipped alongside a relocatable install, resolved from
/// where the running binary lives. These carry the packaged default commands
/// on systems where nothing installs into `/etc/kinjo/commands`:
///
///   - `<exe_dir>/commands` — a release tarball extracted and run in place
///   - `<prefix>/share/kinjo/commands` — a prefix install such as Homebrew,
///     where the binary is at `<prefix>/bin/kinjo`
fn install_dirs_from(exe: Option<&Path>) -> Vec<PathBuf> {
    let Some(exe_dir) = exe.and_then(Path::parent) else {
        return Vec::new();
    };
    let mut dirs = vec![exe_dir.join("commands")];
    if let Some(prefix) = exe_dir.parent() {
        dirs.push(prefix.join("share").join("kinjo").join("commands"));
    }
    dirs
}

/// Load every command directory strictly: the first unreadable directory,
/// unreadable entry, or malformed command file fails the whole load.
///
/// This is the policy for `list-commands`, whose entire job is to answer
/// whether a configuration is valid, and for any caller that would rather have
/// no rule set than an incomplete one.
pub fn load_from_dirs(builder: &mut MatcherBuilder, dirs: &[PathBuf]) -> Result<()> {
    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        builder.start_layer();
        // Name the directory here, where it is known: `Permission denied` on
        // its own tells the user nothing about what to go and fix.
        let files =
            toml_files_in(dir).map_err(|err| eyre!("cannot read {}: {err}", dir.display()))?;
        for path in &files {
            builder.add_file(path)?;
        }
    }
    Ok(())
}

/// Like [`load_from_dirs`], but never fails: unreadable directories and
/// malformed command files are skipped and reported as warnings, so one bad
/// file (e.g. in the shared system directory) cannot prevent the app from
/// starting.
///
/// The warnings are the whole point of the return value, and each names its
/// source. A caller that cannot tolerate an incomplete rule set — a live
/// reload, which already has a working one — builds through this policy and
/// then refuses to install the result unless it came back with no warnings at
/// all. That way one loading and validating path serves both policies, and
/// "which files are wrong" is answered in full rather than one file at a time.
pub fn load_from_dirs_lenient(builder: &mut MatcherBuilder, dirs: &[PathBuf]) -> Vec<String> {
    let mut warnings = Vec::new();
    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        builder.start_layer();
        let files = match toml_files_in(dir) {
            Ok(files) => files,
            Err(err) => {
                warnings.push(format!("cannot read {}: {err}", dir.display()));
                continue;
            }
        };
        for path in files {
            if let Err(err) = builder.add_file(&path) {
                warnings.push(err.to_string());
            }
        }
    }
    warnings
}

/// The `.toml` files directly inside `dir`, sorted by name (the load order
/// within one overlay layer).
///
/// A failure *part way through* the directory is an error like any other, not
/// an empty spot in the listing. Skipping unreadable entries would silently
/// shorten the overlay, and a rule that vanishes looks exactly like a rule that
/// was never configured — so both loading policies get told, and each decides:
/// strict loading fails, lenient loading warns and moves on.
fn toml_files_in(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "toml") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// On-disk shape of a command file, deserialized by the `toml` crate. The
/// `match` table stays a generic [`toml::Table`] because its keys are service
/// field names (possibly dotted, e.g. `txt.path`), not a fixed schema.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    metadata: RawMetadata,
    action: RawAction,
    #[serde(rename = "match", default)]
    matchers: toml::Table,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMetadata {
    name: String,
    description: Option<String>,
    #[serde(default)]
    requirements: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAction {
    description: Option<String>,
    command: String,
    mode: String,
    #[serde(default)]
    allow_option_like_values: bool,
}

/// Compile the on-disk form of a command file into a validated rule.
///
/// Everything a rule can be wrong about is decided here: the mode, the match
/// fields and their predicates, the requirement grammar, and the command
/// template. A `CommandConfig` returned from this function is executable, so
/// `list-commands` validating a file and the TUI offering it cannot disagree.
fn parse_command_config(source: &str) -> Result<CommandConfig> {
    let raw: RawConfig = toml::from_str(source)?;

    if raw.metadata.name.is_empty() {
        return Err(eyre!("`metadata.name` is empty"));
    }

    let mode = match raw.action.mode.as_str() {
        "fork" => ActionMode::Fork,
        "execute" | "exec" => ActionMode::Execute,
        value => return Err(eyre!("invalid action mode `{value}`")),
    };

    let requirements = raw
        .metadata
        .requirements
        .iter()
        .map(|entry| Requirement::parse(entry))
        .collect::<Result<Vec<_>>>()?;

    let mut predicates = Vec::new();
    collect_predicates("", &raw.matchers, &mut predicates)?;
    if predicates.is_empty() {
        return Err(eyre!(
            "command `{}` has no match predicates",
            raw.metadata.name
        ));
    }

    let action = if raw.action.allow_option_like_values {
        CommandAction::compile_allowing_option_like_values(
            raw.action.description,
            raw.action.command,
            mode,
        )?
    } else {
        CommandAction::compile(raw.action.description, raw.action.command, mode)?
    };

    Ok(CommandConfig {
        name: raw.metadata.name,
        description: raw.metadata.description,
        requirements,
        predicates,
        action,
    })
}

/// Walks the `[match.*]` table tree, turning each `<kind> = "value"` leaf into
/// a [`FieldPredicate`] on the field named by the table path. Nested tables
/// extend the field name with a dot (`[match.txt.path]` matches the `txt.path`
/// service field).
///
/// The field a leaf names is checked against the rule vocabulary here. A rule
/// matching on `service_typ` was previously accepted and then simply never
/// matched anything, which looks exactly like a service that is not on the
/// network — the one thing a discovery tool must not be ambiguous about.
fn collect_predicates(
    field: &str,
    table: &toml::Table,
    predicates: &mut Vec<FieldPredicate>,
) -> Result<()> {
    for (key, value) in table {
        let path = if field.is_empty() {
            key.clone()
        } else {
            format!("{field}.{key}")
        };
        match value {
            toml::Value::Table(nested) => {
                collect_predicates(&path, nested, predicates)?;
            }
            _ if field.is_empty() => {
                return Err(eyre!(
                    "match `{path}` must be a table like `[match.<field>]`"
                ));
            }
            toml::Value::String(value) => {
                if !is_supported_field(field) {
                    return Err(eyre!(
                        "unsupported match field `{field}`; supported fields are \
                         name, service_type (or type), domain, hostname, address, \
                         port, and txt.<key>"
                    ));
                }
                let predicate = match key.as_str() {
                    "equals" => Predicate::Equals(value.clone()),
                    "contains" => Predicate::Contains(value.clone()),
                    "regex" => Predicate::Regex(
                        Regex::new(value)
                            .map_err(|err| eyre!("invalid regex for `{path}`: {err}"))?,
                    ),
                    _ => return Err(eyre!("unsupported predicate `{path}`")),
                };
                predicates.push(FieldPredicate {
                    field: field.to_string(),
                    predicate,
                });
            }
            _ => return Err(eyre!("match `{path}` must be a string")),
        }
    }
    Ok(())
}
