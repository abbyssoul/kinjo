use std::net::IpAddr;

use regex::Regex;

use crate::discovery::{Entry, EntryGroup};

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
    pub matching_records: Vec<Entry>,
    pub needs_instance: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Matcher {
    commands: Vec<CommandConfig>,
}

impl Matcher {
    pub fn matches_group(&self, group: &EntryGroup) -> Vec<MatchResult> {
        self.commands
            .iter()
            .filter_map(|command| {
                let matching_records: Vec<Entry> = group
                    .instances
                    .iter()
                    .filter(|record| command.matches_record(record))
                    .flat_map(|record| command.candidate_instances(record))
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
    fn matches_record(&self, record: &Entry) -> bool {
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

    /// Whether this command distinguishes between a service's individual
    /// addresses (it matches on `address` or templates `{address}`).
    fn uses_address(&self) -> bool {
        self.action.command.contains("{address}")
            || self.predicates.iter().any(|p| p.field == "address")
    }

    /// Expands a matched service into the per-instance candidates the command
    /// can act on. A service carries all of its addresses on one entry; when the
    /// command distinguishes addresses, each (matching) address becomes its own
    /// single-address candidate so the instance picker can offer them. Otherwise
    /// the service stays a single candidate.
    fn candidate_instances(&self, record: &Entry) -> Vec<Entry> {
        if !self.uses_address() || record.addresses.len() <= 1 {
            return vec![record.clone()];
        }
        let matching: Vec<IpAddr> = record
            .addresses
            .iter()
            .copied()
            .filter(|addr| self.address_predicates_match(addr))
            .collect();
        let addresses = if matching.is_empty() {
            record.addresses.clone()
        } else {
            matching
        };
        addresses
            .into_iter()
            .map(|addr| {
                let mut candidate = record.clone();
                candidate.addresses = vec![addr];
                candidate
            })
            .collect()
    }

    /// Whether `addr` satisfies every `address` predicate on this command (so it
    /// is an address worth offering as a candidate). Vacuously true when the
    /// command has no `address` predicate.
    fn address_predicates_match(&self, addr: &IpAddr) -> bool {
        let value = addr.to_string();
        self.predicates
            .iter()
            .filter(|p| p.field == "address")
            .all(|p| p.predicate.matches_value(&value))
    }
}

impl FieldPredicate {
    fn matches(&self, record: &Entry) -> bool {
        // A service may advertise several addresses; match if ANY satisfies the
        // predicate, then `candidate_instances` narrows to the matching ones.
        if self.field == "address" {
            return record
                .addresses
                .iter()
                .any(|addr| self.predicate.matches_value(&addr.to_string()));
        }
        let Some(value) = record.field(&self.field) else {
            return false;
        };
        self.predicate.matches_value(&value)
    }
}

impl Predicate {
    fn matches_value(&self, value: &str) -> bool {
        match self {
            Predicate::Equals(expected) => value == expected,
            Predicate::Contains(expected) => value.contains(expected),
            Predicate::Regex(regex) => regex.is_match(value),
        }
    }
}

fn is_instance_field(field: &str) -> bool {
    matches!(field, "address" | "port")
}

/// Engine that matches discovered entry groups against the loaded rules. The
/// default implementation is [`Matcher`]; an alternative engine can be swapped
/// in behind this trait without touching the discovery or UI layers.
pub trait RuleEngine {
    /// Rules that match at least one entry in `group`.
    fn matches_group(&self, group: &EntryGroup) -> Vec<MatchResult>;
    /// All loaded rules, in load order.
    fn commands(&self) -> &[CommandConfig];
    /// Number of loaded rules.
    fn command_count(&self) -> usize;
}

impl RuleEngine for Matcher {
    fn matches_group(&self, group: &EntryGroup) -> Vec<MatchResult> {
        Matcher::matches_group(self, group)
    }

    fn commands(&self) -> &[CommandConfig] {
        Matcher::commands(self)
    }

    fn command_count(&self) -> usize {
        Matcher::command_count(self)
    }
}

mod config;
pub use config::*;

pub mod exec;
pub use exec::PreparedCommand;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{remove, temp_dir};
    use std::{
        ffi::OsString,
        fs,
        net::{IpAddr, Ipv4Addr},
        path::PathBuf,
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
        let mut record = Entry::new("alpha", "_ssh._tcp", "local");
        record.hostname = Some("alpha.local".to_string());
        let group = crate::discovery::group_entries(
            &[record],
            crate::discovery::GroupingMode::LogicalService,
        )
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
        let mut matching = Entry::new("Printer", "_ipp._tcp", "local");
        matching.hostname = Some("print-01.local".to_string());
        matching
            .txt
            .insert("path".to_string(), "admin/status".to_string());
        let mut wrong_txt = Entry::new("Printer", "_ipp._tcp", "local");
        wrong_txt.hostname = Some("print-02.local".to_string());
        wrong_txt
            .txt
            .insert("path".to_string(), "ipp/print".to_string());
        let group = crate::discovery::group_entries(
            &[matching, wrong_txt],
            crate::discovery::GroupingMode::ServiceType,
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
        let record = Entry::new("alpha", "_ssh._tcp", "local");
        let group = crate::discovery::group_entries(
            &[record],
            crate::discovery::GroupingMode::LogicalService,
        )
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
        let mut record = Entry::new("site", "_http._tcp", "local");
        record.hostname = Some("site.local".to_string());
        record.addresses = vec![IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10))];
        record.port = Some(8080);
        let group = crate::discovery::group_entries(
            &[record],
            crate::discovery::GroupingMode::LogicalService,
        )
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
