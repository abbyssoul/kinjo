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

use super::{ActionMode, CommandAction, CommandConfig, FieldPredicate, Matcher, Predicate};

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

    pub fn add_str(&mut self, source_name: &str, source: &str) -> Result<()> {
        let command = parse_command_config(source_name, source)?;
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
        let source = fs::read_to_string(path)?;
        self.add_str(&path.display().to_string(), &source)
    }

    pub fn build(self) -> Matcher {
        Matcher {
            commands: self.commands,
        }
    }
}

/// System-wide command directory, loaded as the base layer for every run.
pub const SYSTEM_CONFIG_DIR: &str = "/etc/avahi-tui/commands";

/// Ordered list of command directories, lowest precedence first. Commands in a
/// later directory override same-named commands from an earlier one:
///
///   1. system-wide  (`/etc/avahi-tui/commands`)
///   2. user-local   (`$XDG_CONFIG_HOME/avahi-tui/commands` or `~/.config/...`)
///   3. command-line `--config-dir` entries, in the order given
pub fn config_dirs(extra: &[PathBuf]) -> Vec<PathBuf> {
    config_dirs_from(env::var_os("XDG_CONFIG_HOME"), env::var_os("HOME"), extra)
}

/// Build the ordered command-directory list from the relevant environment
/// variables. Split out from [`config_dirs`] so the precedence rules can be
/// unit tested without mutating process-global environment state.
pub(crate) fn config_dirs_from(
    xdg_config_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
    extra: &[PathBuf],
) -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from(SYSTEM_CONFIG_DIR)];
    if let Some(home) = xdg_config_home {
        dirs.push(PathBuf::from(home).join("avahi-tui").join("commands"));
    } else if let Some(home) = home {
        dirs.push(
            PathBuf::from(home)
                .join(".config")
                .join("avahi-tui")
                .join("commands"),
        );
    }
    dirs.extend(extra.iter().cloned());
    dirs
}

pub fn load_from_dirs(builder: &mut MatcherBuilder, dirs: &[PathBuf]) -> Result<()> {
    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        builder.start_layer();
        for path in toml_files_in(dir)? {
            builder.add_file(&path)?;
        }
    }
    Ok(())
}

/// Like [`load_from_dirs`], but never fails: unreadable directories and
/// malformed command files are skipped and reported as warnings, so one bad
/// file (e.g. in the shared system directory) cannot prevent the app from
/// starting. `list-commands` keeps using the strict variant, since validating
/// configs is its whole point.
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
fn toml_files_in(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .collect::<Vec<_>>();
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
}

fn parse_command_config(source_name: &str, source: &str) -> Result<CommandConfig> {
    let raw: RawConfig = toml::from_str(source).map_err(|err| eyre!("{source_name}: {err}"))?;

    let mode = match raw.action.mode.as_str() {
        "fork" => ActionMode::Fork,
        "execute" | "exec" => ActionMode::Execute,
        value => return Err(eyre!("{source_name}: invalid action mode `{value}`")),
    };

    let mut predicates = Vec::new();
    collect_predicates(source_name, "", &raw.matchers, &mut predicates)?;
    if predicates.is_empty() {
        return Err(eyre!(
            "{source_name}: command `{}` has no match predicates",
            raw.metadata.name
        ));
    }

    Ok(CommandConfig {
        name: raw.metadata.name,
        description: raw.metadata.description,
        requirements: raw.metadata.requirements,
        predicates,
        action: CommandAction {
            description: raw.action.description,
            command: raw.action.command,
            mode,
        },
    })
}

/// Walks the `[match.*]` table tree, turning each `<kind> = "value"` leaf into
/// a [`FieldPredicate`] on the field named by the table path. Nested tables
/// extend the field name with a dot (`[match.txt.path]` matches the `txt.path`
/// service field).
fn collect_predicates(
    source_name: &str,
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
                collect_predicates(source_name, &path, nested, predicates)?;
            }
            _ if field.is_empty() => {
                return Err(eyre!(
                    "{source_name}: match `{path}` must be a table like `[match.<field>]`"
                ));
            }
            toml::Value::String(value) => {
                let predicate = match key.as_str() {
                    "equals" => Predicate::Equals(value.clone()),
                    "contains" => Predicate::Contains(value.clone()),
                    "regex" => Predicate::Regex(Regex::new(value).map_err(|err| {
                        eyre!("{source_name}: invalid regex for `{path}`: {err}")
                    })?),
                    _ => return Err(eyre!("{source_name}: unsupported predicate `{path}`")),
                };
                predicates.push(FieldPredicate {
                    field: field.to_string(),
                    predicate,
                });
            }
            _ => return Err(eyre!("{source_name}: match `{path}` must be a string")),
        }
    }
    Ok(())
}
