use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

use color_eyre::eyre::{Result, eyre};
use regex::Regex;

use crate::service::{ServiceGroup, ServiceRecord};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionMode {
    Fork,
    Execute,
}

impl std::fmt::Display for ActionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActionMode::Fork => f.write_str("fork"),
            ActionMode::Execute => f.write_str("execute"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CommandAction {
    pub description: Option<String>,
    pub command: String,
    pub mode: ActionMode,
}

#[derive(Debug, Clone)]
pub struct CommandConfig {
    pub name: String,
    pub description: Option<String>,
    pub requirements: Vec<String>,
    pub predicates: Vec<FieldPredicate>,
    pub action: CommandAction,
}

#[derive(Debug, Clone)]
pub struct FieldPredicate {
    pub field: String,
    pub predicate: Predicate,
}

#[derive(Debug, Clone)]
pub enum Predicate {
    Equals(String),
    Contains(String),
    Regex(Regex),
}

#[derive(Debug, Clone)]
pub struct MatchResult {
    pub command: CommandConfig,
    pub matching_records: Vec<ServiceRecord>,
    pub needs_instance: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Matcher {
    commands: Vec<CommandConfig>,
}

impl Matcher {
    pub fn matches_group(&self, group: &ServiceGroup) -> Vec<MatchResult> {
        self.commands
            .iter()
            .filter_map(|command| {
                let matching_records: Vec<ServiceRecord> = group
                    .instances
                    .iter()
                    .filter(|record| command.matches_record(record))
                    .cloned()
                    .collect();
                if matching_records.is_empty() {
                    return None;
                }
                let needs_instance = command.needs_instance()
                    || matching_records.len() > 1 && command.has_instance_specific_template();
                Some(MatchResult {
                    command: command.clone(),
                    matching_records,
                    needs_instance,
                })
            })
            .collect()
    }

    pub fn command_count(&self) -> usize {
        self.commands.len()
    }

    pub fn commands(&self) -> &[CommandConfig] {
        &self.commands
    }
}

impl CommandConfig {
    fn matches_record(&self, record: &ServiceRecord) -> bool {
        self.predicates
            .iter()
            .all(|predicate| predicate.matches(record))
    }

    pub fn needs_instance(&self) -> bool {
        self.predicates
            .iter()
            .any(|predicate| is_instance_field(&predicate.field))
            || self.has_instance_specific_template()
    }

    fn has_instance_specific_template(&self) -> bool {
        self.action.command.contains("{address}") || self.action.command.contains("{port}")
    }
}

impl FieldPredicate {
    fn matches(&self, record: &ServiceRecord) -> bool {
        let Some(value) = record.field(&self.field) else {
            return false;
        };
        match &self.predicate {
            Predicate::Equals(expected) => value == *expected,
            Predicate::Contains(expected) => value.contains(expected),
            Predicate::Regex(regex) => regex.is_match(&value),
        }
    }
}

fn is_instance_field(field: &str) -> bool {
    matches!(field, "address" | "port")
}

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
fn config_dirs_from(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{remove, temp_dir};
    use std::{
        ffi::OsString,
        fs,
        net::{IpAddr, Ipv4Addr},
    };

    fn command_toml(name: &str, command: &str) -> String {
        format!(
            r#"
[metadata]
name = "{name}"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "{command}"
mode = "execute"
"#
        )
    }

    #[test]
    fn later_layers_override_earlier_commands() {
        let mut builder = MatcherBuilder::new();
        builder.start_layer(); // system
        builder
            .add_str("system/ssh", &command_toml("ssh", "ssh system"))
            .unwrap();
        builder
            .add_str("system/mosh", &command_toml("mosh", "mosh system"))
            .unwrap();
        builder.start_layer(); // user overlay
        builder
            .add_str("user/ssh", &command_toml("ssh", "ssh user"))
            .unwrap();

        let matcher = builder.build();
        assert_eq!(matcher.command_count(), 2);
        // The override keeps the command in its original position.
        assert_eq!(matcher.commands()[0].name, "ssh");
        assert_eq!(matcher.commands()[0].action.command, "ssh user");
        assert_eq!(matcher.commands()[1].name, "mosh");
    }

    #[test]
    fn duplicate_within_one_layer_is_rejected() {
        let mut builder = MatcherBuilder::new();
        builder.start_layer();
        builder.add_str("a", &command_toml("ssh", "ssh a")).unwrap();
        let err = builder
            .add_str("b", &command_toml("ssh", "ssh b"))
            .unwrap_err();
        assert!(err.to_string().contains("duplicate command name"));
    }

    #[test]
    fn config_dirs_layer_system_then_user_then_extras() {
        let extra = PathBuf::from("/tmp/avahi-extra");
        let dirs = config_dirs(std::slice::from_ref(&extra));
        assert_eq!(dirs.first(), Some(&PathBuf::from(SYSTEM_CONFIG_DIR)));
        assert_eq!(dirs.last(), Some(&extra));
    }

    #[test]
    fn parses_structured_matcher() {
        let mut builder = MatcherBuilder::new();
        builder
            .add_str(
                "test",
                r#"
[metadata]
name = "ssh"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "ssh {hostname}"
mode = "execute"
"#,
            )
            .unwrap();

        let matcher = builder.build();
        assert_eq!(matcher.command_count(), 1);

        let command = &matcher.commands()[0];
        assert_eq!(command.name, "ssh");
        assert_eq!(command.description, None);
        assert!(command.requirements.is_empty());
        assert_eq!(command.predicates.len(), 1);
        let predicate = &command.predicates[0];
        assert_eq!(predicate.field, "service_type");
        match &predicate.predicate {
            Predicate::Equals(value) => assert_eq!(value, "_ssh._tcp"),
            _ => panic!("unexpected predicate type"),
        }
        assert_eq!(command.action.description, None);
        assert_eq!(command.action.command, "ssh {hostname}");
        assert_eq!(command.action.mode, ActionMode::Execute);
    }

    #[test]
    fn parses_optional_metadata_action_description_and_fork_mode() {
        let mut builder = MatcherBuilder::new();
        builder
            .add_str(
                "printer",
                r#"
[metadata]
name = "open-printer"
description = "Open printer admin"
requirements = ["xdg-open", "browser, optional"]

[match.service_type]
equals = "_ipp._tcp"

[action]
description = "Open the printer web UI"
command = "xdg-open http://{hostname}:{port}"
mode = "fork"
"#,
            )
            .unwrap();

        let matcher = builder.build();
        let command = &matcher.commands()[0];

        assert_eq!(command.name, "open-printer");
        assert_eq!(command.description.as_deref(), Some("Open printer admin"));
        assert_eq!(
            command.requirements,
            vec!["xdg-open".to_string(), "browser, optional".to_string()]
        );
        assert_eq!(
            command.action.description.as_deref(),
            Some("Open the printer web UI")
        );
        assert_eq!(command.action.mode, ActionMode::Fork);
        assert_eq!(command.action.mode.to_string(), "fork");
    }

    #[test]
    fn execute_mode_accepts_exec_alias() {
        let mut builder = MatcherBuilder::new();
        builder
            .add_str(
                "exec-alias",
                r#"
[metadata]
name = "ssh"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "ssh {hostname}"
mode = "exec"
"#,
            )
            .unwrap();

        assert_eq!(
            builder.build().commands()[0].action.mode,
            ActionMode::Execute
        );
    }

    #[test]
    fn matcher_filters_records() {
        let mut builder = MatcherBuilder::new();
        builder
            .add_str(
                "ssh",
                r#"
[metadata]
name = "ssh"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "ssh {hostname}"
mode = "execute"
"#,
            )
            .unwrap();
        let matcher = builder.build();
        let mut record = ServiceRecord::new("alpha", "_ssh._tcp", "local");
        record.hostname = Some("alpha.local".to_string());
        let group =
            crate::service::group_records(&[record], crate::service::GroupingMode::LogicalService)
                .remove(0);
        assert_eq!(matcher.matches_group(&group).len(), 1);
    }

    #[test]
    fn matcher_supports_contains_regex_and_txt_predicates() {
        let mut builder = MatcherBuilder::new();
        builder
            .add_str(
                "printer-admin",
                r#"
[metadata]
name = "printer-admin"

[match.service_type]
equals = "_ipp._tcp"

[match.hostname]
regex = "^print-[0-9]+[.]local$"

[match.txt.path]
contains = "admin"

[action]
command = "xdg-open http://{hostname}/{txt.path}"
mode = "fork"
"#,
            )
            .unwrap();
        let matcher = builder.build();
        let mut matching = ServiceRecord::new("Printer", "_ipp._tcp", "local");
        matching.hostname = Some("print-01.local".to_string());
        matching
            .txt
            .insert("path".to_string(), "admin/status".to_string());
        let mut wrong_txt = ServiceRecord::new("Printer", "_ipp._tcp", "local");
        wrong_txt.hostname = Some("print-02.local".to_string());
        wrong_txt
            .txt
            .insert("path".to_string(), "ipp/print".to_string());
        let group = crate::service::group_records(
            &[matching, wrong_txt],
            crate::service::GroupingMode::ServiceType,
        )
        .remove(0);

        let matches = matcher.matches_group(&group);

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].matching_records.len(), 1);
        assert_eq!(
            matches[0].matching_records[0].hostname.as_deref(),
            Some("print-01.local")
        );
    }

    #[test]
    fn missing_instance_field_prevents_a_match() {
        let mut builder = MatcherBuilder::new();
        builder
            .add_str(
                "ssh-port",
                r#"
[metadata]
name = "ssh-port"

[match.port]
equals = "22"

[action]
command = "ssh {hostname}"
mode = "execute"
"#,
            )
            .unwrap();
        let matcher = builder.build();
        let record = ServiceRecord::new("alpha", "_ssh._tcp", "local");
        let group =
            crate::service::group_records(&[record], crate::service::GroupingMode::LogicalService)
                .remove(0);

        assert!(matcher.matches_group(&group).is_empty());
    }

    #[test]
    fn instance_specific_predicates_and_templates_request_instance_selection() {
        let mut builder = MatcherBuilder::new();
        builder
            .add_str(
                "by-address",
                r#"
[metadata]
name = "by-address"

[match.address]
regex = "^192[.]0[.]2[.]"

[action]
command = "echo {hostname}"
mode = "execute"
"#,
            )
            .unwrap();
        builder
            .add_str(
                "by-port-template",
                r#"
[metadata]
name = "by-port-template"

[match.service_type]
equals = "_http._tcp"

[action]
command = "curl http://{hostname}:{port}"
mode = "execute"
"#,
            )
            .unwrap();
        builder
            .add_str(
                "by-host-template",
                r#"
[metadata]
name = "by-host-template"

[match.service_type]
equals = "_http._tcp"

[action]
command = "open http://{hostname}"
mode = "execute"
"#,
            )
            .unwrap();
        let matcher = builder.build();
        let mut record = ServiceRecord::new("site", "_http._tcp", "local");
        record.hostname = Some("site.local".to_string());
        record.address = Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)));
        record.port = Some(8080);
        let group =
            crate::service::group_records(&[record], crate::service::GroupingMode::LogicalService)
                .remove(0);

        let matches = matcher.matches_group(&group);
        let needs_instance = matches
            .iter()
            .map(|result| (result.command.name.as_str(), result.needs_instance))
            .collect::<Vec<_>>();

        assert_eq!(
            needs_instance,
            vec![
                ("by-address", true),
                ("by-port-template", true),
                ("by-host-template", false),
            ]
        );
    }

    #[test]
    fn lists_loaded_command_names() {
        let mut builder = MatcherBuilder::new();
        builder
            .add_str(
                "ssh",
                r#"
[metadata]
name = "ssh"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "ssh {hostname}"
mode = "execute"
"#,
            )
            .unwrap();
        builder
            .add_str(
                "open-http",
                r#"
[metadata]
name = "open-http"

[match.service_type]
equals = "_http._tcp"

[action]
command = "xdg-open http://{hostname}:{port}"
mode = "execute"
"#,
            )
            .unwrap();

        let matcher = builder.build();
        let names = matcher
            .commands()
            .iter()
            .map(|command| command.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["ssh", "open-http"]);
    }

    #[test]
    fn load_from_dirs_loads_sorted_toml_files_and_ignores_other_files() {
        let dir = temp_dir("load-sorted");
        fs::write(dir.join("02-second.toml"), command_toml("second", "second")).unwrap();
        fs::write(dir.join("01-first.toml"), command_toml("first", "first")).unwrap();
        fs::write(dir.join("ignored.txt"), command_toml("ignored", "ignored")).unwrap();

        let mut builder = MatcherBuilder::new();
        load_from_dirs(&mut builder, std::slice::from_ref(&dir)).unwrap();

        let matcher = builder.build();
        let names = matcher
            .commands()
            .iter()
            .map(|command| command.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["first", "second"]);

        remove(&dir);
    }

    #[test]
    fn load_from_dirs_allows_later_directories_to_override() {
        let base = temp_dir("base");
        let overlay = temp_dir("overlay");
        fs::write(base.join("ssh.toml"), command_toml("ssh", "ssh base")).unwrap();
        fs::write(overlay.join("ssh.toml"), command_toml("ssh", "ssh overlay")).unwrap();

        let mut builder = MatcherBuilder::new();
        load_from_dirs(&mut builder, &[base.clone(), overlay.clone()]).unwrap();

        let matcher = builder.build();
        assert_eq!(matcher.command_count(), 1);
        assert_eq!(matcher.commands()[0].action.command, "ssh overlay");

        remove(&base);
        remove(&overlay);
    }

    #[test]
    fn invalid_command_configs_return_actionable_errors() {
        let cases = [
            (
                "no-predicates",
                r#"
[metadata]
name = "empty"

[action]
command = "true"
mode = "execute"
"#,
                "has no match predicates",
            ),
            (
                "bad-mode",
                r#"
[metadata]
name = "bad-mode"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "ssh"
mode = "daemon"
"#,
                "invalid action mode",
            ),
            (
                "bad-section",
                r#"
[metadata]
name = "bad-section"

[commands]
name = "ignored"
"#,
                "unsupported section",
            ),
            (
                "bad-requirements",
                r#"
[metadata]
name = "bad-requirements"
requirements = "ssh"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "ssh"
mode = "execute"
"#,
                "`requirements` must be an array",
            ),
            (
                "bad-predicate",
                r#"
[metadata]
name = "bad-predicate"

[match.service_type]
starts_with = "_ssh"

[action]
command = "ssh"
mode = "execute"
"#,
                "unsupported predicate",
            ),
        ];

        for (source_name, source, expected) in cases {
            let mut builder = MatcherBuilder::new();
            let err = builder.add_str(source_name, source).unwrap_err();
            assert!(
                err.to_string().contains(expected),
                "expected `{}` to contain `{}`",
                err,
                expected
            );
        }
    }

    #[test]
    fn hash_inside_a_quoted_string_is_not_a_comment() {
        let mut builder = MatcherBuilder::new();
        builder
            .add_str(
                "fragment",
                r#"
[metadata]
name = "open-anchor"

[match.service_type]
equals = "_http._tcp"

[action]
command = "xdg-open http://host/page#section"
mode = "fork"
"#,
            )
            .unwrap();

        assert_eq!(
            builder.build().commands()[0].action.command,
            "xdg-open http://host/page#section"
        );
    }

    #[test]
    fn string_escapes_are_decoded() {
        let mut builder = MatcherBuilder::new();
        builder
            .add_str(
                "escapes",
                r#"
[metadata]
name = "escapes"
description = "tab\tand\nnewline and quote \" and backslash \\"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "ssh"
mode = "execute"
"#,
            )
            .unwrap();

        assert_eq!(
            builder.build().commands()[0].description.as_deref(),
            Some("tab\tand\nnewline and quote \" and backslash \\")
        );
    }

    #[test]
    fn unknown_escape_sequences_are_left_verbatim() {
        let mut builder = MatcherBuilder::new();
        builder
            .add_str(
                "verbatim",
                r#"
[metadata]
name = "verbatim"
description = "keep \z as-is"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "ssh"
mode = "execute"
"#,
            )
            .unwrap();

        assert_eq!(
            builder.build().commands()[0].description.as_deref(),
            Some(r"keep \z as-is")
        );
    }

    #[test]
    fn invalid_regex_predicate_is_rejected() {
        let mut builder = MatcherBuilder::new();
        let err = builder
            .add_str(
                "bad-regex",
                r#"
[metadata]
name = "bad-regex"

[match.hostname]
regex = "("

[action]
command = "ssh"
mode = "execute"
"#,
            )
            .unwrap_err();

        assert!(err.to_string().contains("regex"));
    }

    #[test]
    fn array_match_value_and_array_description_are_rejected() {
        let mut builder = MatcherBuilder::new();
        let array_predicate = builder
            .add_str(
                "array-predicate",
                r#"
[metadata]
name = "array-predicate"

[match.service_type]
equals = ["_ssh._tcp"]

[action]
command = "ssh"
mode = "execute"
"#,
            )
            .unwrap_err();
        let array_description = builder
            .add_str(
                "array-description",
                r#"
[metadata]
name = "array-description"

[match.service_type]
equals = "_ssh._tcp"

[action]
description = ["nope"]
command = "ssh"
mode = "execute"
"#,
            )
            .unwrap_err();

        assert!(
            array_predicate
                .to_string()
                .contains("`service_type.equals` must be a string")
        );
        assert!(
            array_description
                .to_string()
                .contains("`description` must be a string")
        );
    }

    #[test]
    fn key_outside_a_section_is_an_error() {
        let mut builder = MatcherBuilder::new();
        let err = builder.add_str("stray", "name = \"orphan\"\n").unwrap_err();

        assert!(err.to_string().contains("key outside a section"));
    }

    #[test]
    fn add_file_reads_a_command_from_disk() {
        let dir = temp_dir("add-file");
        let path = dir.join("ssh.toml");
        fs::write(&path, command_toml("ssh", "ssh {hostname}")).unwrap();

        let mut builder = MatcherBuilder::new();
        builder.start_layer();
        builder.add_file(&path).unwrap();

        assert_eq!(builder.build().commands()[0].name, "ssh");

        remove(&dir);
    }

    #[test]
    fn load_from_dirs_skips_missing_directories() {
        let mut builder = MatcherBuilder::new();
        load_from_dirs(
            &mut builder,
            &[PathBuf::from("/tmp/avahi-tui-definitely-missing-xyz")],
        )
        .unwrap();

        assert_eq!(builder.build().command_count(), 0);
    }

    #[test]
    fn config_dirs_from_orders_system_user_then_extras() {
        let extra = PathBuf::from("/tmp/extra");
        let dirs = config_dirs_from(
            Some(OsString::from("/xdg")),
            Some(OsString::from("/home/user")),
            std::slice::from_ref(&extra),
        );

        assert_eq!(
            dirs,
            vec![
                PathBuf::from(SYSTEM_CONFIG_DIR),
                PathBuf::from("/xdg/avahi-tui/commands"),
                extra,
            ]
        );
    }

    #[test]
    fn config_dirs_from_uses_home_when_xdg_is_absent() {
        let dirs = config_dirs_from(None, Some(OsString::from("/home/user")), &[]);

        assert_eq!(
            dirs,
            vec![
                PathBuf::from(SYSTEM_CONFIG_DIR),
                PathBuf::from("/home/user/.config/avahi-tui/commands"),
            ]
        );
    }

    #[test]
    fn config_dirs_from_omits_user_dir_without_env() {
        let dirs = config_dirs_from(None, None, &[]);

        assert_eq!(dirs, vec![PathBuf::from(SYSTEM_CONFIG_DIR)]);
    }
}
