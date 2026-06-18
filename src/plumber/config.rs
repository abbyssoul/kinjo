//! Serialization layer for the rules engine: loads command rules from TOML
//! files and builds a [`Matcher`] with the system → user → CLI overlay order.

use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

use color_eyre::eyre::{Result, eyre};
use regex::Regex;

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
        let mut files = fs::read_dir(dir)?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
            .collect::<Vec<_>>();
        files.sort();
        for path in files {
            builder.add_file(&path)?;
        }
    }
    Ok(())
}

#[derive(Debug, Default)]
struct RawConfig {
    metadata: BTreeMap<String, Value>,
    action: BTreeMap<String, Value>,
    predicates: BTreeMap<String, BTreeMap<String, Value>>,
}

#[derive(Debug, Clone)]
enum Section {
    Metadata,
    Action,
    Match(String),
}

#[derive(Debug, Clone)]
enum Value {
    String(String),
    Array(Vec<String>),
}

fn parse_command_config(source_name: &str, source: &str) -> Result<CommandConfig> {
    let raw = parse_minimal_toml(source_name, source)?;
    let name = required_string(&raw.metadata, "name", source_name)?;
    let description = optional_string(&raw.metadata, "description")?;
    let requirements = optional_array(&raw.metadata, "requirements")?;
    let action_description = optional_string(&raw.action, "description")?;
    let command = required_string(&raw.action, "command", source_name)?;
    let mode = match required_string(&raw.action, "mode", source_name)?.as_str() {
        "fork" => ActionMode::Fork,
        "execute" | "exec" => ActionMode::Execute,
        value => return Err(eyre!("{source_name}: invalid action mode `{value}`")),
    };

    let mut predicates = Vec::new();
    for (field, values) in raw.predicates {
        for (kind, value) in values {
            let value = match value {
                Value::String(value) => value,
                Value::Array(_) => {
                    return Err(eyre!(
                        "{source_name}: match `{field}.{kind}` must be a string"
                    ));
                }
            };
            let predicate = match kind.as_str() {
                "equals" => Predicate::Equals(value),
                "contains" => Predicate::Contains(value),
                "regex" => Predicate::Regex(Regex::new(&value)?),
                _ => {
                    return Err(eyre!(
                        "{source_name}: unsupported predicate `{field}.{kind}`"
                    ));
                }
            };
            predicates.push(FieldPredicate {
                field: field.clone(),
                predicate,
            });
        }
    }

    if predicates.is_empty() {
        return Err(eyre!(
            "{source_name}: command `{name}` has no match predicates"
        ));
    }

    Ok(CommandConfig {
        name,
        description,
        requirements,
        predicates,
        action: CommandAction {
            description: action_description,
            command,
            mode,
        },
    })
}

fn parse_minimal_toml(source_name: &str, source: &str) -> Result<RawConfig> {
    let mut raw = RawConfig::default();
    let mut section: Option<Section> = None;

    for (index, line) in source.lines().enumerate() {
        let line_no = index + 1;
        let line = strip_comment(line).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = &line[1..line.len() - 1];
            section = Some(match name {
                "metadata" => Section::Metadata,
                "action" => Section::Action,
                value if value.starts_with("match.") => {
                    Section::Match(value.trim_start_matches("match.").to_string())
                }
                _ => {
                    return Err(eyre!(
                        "{source_name}:{line_no}: unsupported section `{name}`"
                    ));
                }
            });
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            return Err(eyre!("{source_name}:{line_no}: expected key = value"));
        };
        let key = key.trim().to_string();
        let value = parse_value(source_name, line_no, value.trim())?;
        match &section {
            Some(Section::Metadata) => {
                raw.metadata.insert(key, value);
            }
            Some(Section::Action) => {
                raw.action.insert(key, value);
            }
            Some(Section::Match(field)) => {
                raw.predicates
                    .entry(field.clone())
                    .or_default()
                    .insert(key, value);
            }
            None => return Err(eyre!("{source_name}:{line_no}: key outside a section")),
        }
    }

    Ok(raw)
}

fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        match ch {
            '\\' if in_string => escaped = !escaped,
            '"' if !escaped => in_string = !in_string,
            '#' if !in_string => return &line[..index],
            _ => escaped = false,
        }
    }
    line
}

fn parse_value(source_name: &str, line_no: usize, value: &str) -> Result<Value> {
    if value.starts_with('"') {
        return Ok(Value::String(parse_string(source_name, line_no, value)?));
    }
    if value.starts_with('[') && value.ends_with(']') {
        let inner = value[1..value.len() - 1].trim();
        if inner.is_empty() {
            return Ok(Value::Array(Vec::new()));
        }
        let mut values = Vec::new();
        for item in split_array(inner) {
            values.push(parse_string(source_name, line_no, item.trim())?);
        }
        return Ok(Value::Array(values));
    }
    Err(eyre!(
        "{source_name}:{line_no}: only quoted strings and string arrays are supported"
    ))
}

fn parse_string(source_name: &str, line_no: usize, value: &str) -> Result<String> {
    if !value.starts_with('"') || !value.ends_with('"') || value.len() < 2 {
        return Err(eyre!("{source_name}:{line_no}: expected quoted string"));
    }
    let raw = &value[1..value.len() - 1];
    let mut parsed = String::new();
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let Some(next) = chars.next() else {
                return Err(eyre!("{source_name}:{line_no}: trailing string escape"));
            };
            match next {
                'n' => parsed.push('\n'),
                't' => parsed.push('\t'),
                'r' => parsed.push('\r'),
                '"' => parsed.push('"'),
                '\\' => parsed.push('\\'),
                other => {
                    parsed.push('\\');
                    parsed.push(other);
                }
            }
        } else {
            parsed.push(ch);
        }
    }
    Ok(parsed)
}

fn split_array(value: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in value.char_indices() {
        match ch {
            '\\' if in_string => escaped = !escaped,
            '"' if !escaped => in_string = !in_string,
            ',' if !in_string => {
                result.push(&value[start..index]);
                start = index + 1;
            }
            _ => escaped = false,
        }
    }
    result.push(&value[start..]);
    result
}

fn required_string(
    values: &BTreeMap<String, Value>,
    key: &str,
    source_name: &str,
) -> Result<String> {
    optional_string(values, key)?.ok_or_else(|| eyre!("{source_name}: missing `{key}`"))
}

fn optional_string(values: &BTreeMap<String, Value>, key: &str) -> Result<Option<String>> {
    match values.get(key) {
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(Value::Array(_)) => Err(eyre!("`{key}` must be a string")),
        None => Ok(None),
    }
}

fn optional_array(values: &BTreeMap<String, Value>, key: &str) -> Result<Vec<String>> {
    match values.get(key) {
        Some(Value::Array(value)) => Ok(value.clone()),
        Some(Value::String(_)) => Err(eyre!("`{key}` must be an array")),
        None => Ok(Vec::new()),
    }
}
