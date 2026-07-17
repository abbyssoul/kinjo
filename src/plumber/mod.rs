use std::{collections::HashSet, net::IpAddr};

use color_eyre::eyre::{Result, eyre};
use regex::Regex;

use crate::discovery::{Entry, EntryGroup};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

/// What a rule leaves for the caller to do once it has run.
///
/// Which of the two happens is the rule's decision, made from its validated
/// `mode`. Callers react to the outcome; they do not re-derive it.
#[derive(Debug)]
pub enum ActionOutcome {
    /// The command was spawned and reaped in the background. The caller keeps
    /// running as it was.
    Forked,
    /// The command must replace this process, which can only happen once the
    /// caller has restored the terminal. Ownership of the prepared command
    /// passes to the caller.
    Handoff(PreparedCommand),
}

/// The validated action half of a command rule: what to run, how to run it, and
/// how to describe it.
#[derive(Debug, Clone)]
pub struct CommandAction {
    pub description: Option<String>,
    /// The template exactly as written in the command file. Retained only so the
    /// UI can show a user the rule they wrote; [`Self::prepare`] never reads it.
    pub command: String,
    pub mode: ActionMode,
    /// The executable form of `command`, compiled once at load time.
    template: CommandTemplate,
}

impl CommandAction {
    /// Compile `command` into a validated action, or explain why it is not a
    /// runnable one. This is the only way to construct a [`CommandAction`], so
    /// an action that exists is an action that can be prepared.
    pub fn compile(description: Option<String>, command: String, mode: ActionMode) -> Result<Self> {
        let template = CommandTemplate::compile(&command)?;
        Ok(Self {
            description,
            command,
            mode,
            template,
        })
    }

    /// Compile an action whose trusted author explicitly accepts discovered
    /// values in option-sensitive argument positions.
    ///
    /// Prefer [`Self::compile`]. This escape hatch exists for programs that do
    /// not support a literal `--`, and for placeholders intentionally used as
    /// option values. Discovered data still cannot select `argv[0]`.
    pub fn compile_allowing_option_like_values(
        description: Option<String>,
        command: String,
        mode: ActionMode,
    ) -> Result<Self> {
        let template = CommandTemplate::compile_allowing_option_like_values(&command)?;
        Ok(Self {
            description,
            command,
            mode,
            template,
        })
    }

    /// Turn a chosen candidate into the exact argument vector to run.
    ///
    /// Interpolation happens strictly inside token boundaries decided at compile
    /// time, so a discovered value — which arrives from an untrusted device on
    /// the network — can fill an argument but never add, remove, or split one.
    ///
    /// Fails only when `candidate` does not carry a field the template names.
    /// The matcher's own candidates always do; a library caller's hand-built
    /// [`Entry`] need not.
    pub fn prepare(&self, candidate: &Entry) -> Result<PreparedCommand> {
        let argv = self.template.render(candidate)?;
        Ok(PreparedCommand {
            argv,
            mode: self.mode,
        })
    }
}

/// A validated command rule: metadata to render, predicates to match, parsed
/// requirements, and a compiled action.
///
/// Every value here has already been checked against the command-file grammar,
/// so nothing downstream re-parses raw strings. Build one by loading a command
/// file through [`MatcherBuilder`].
#[derive(Debug, Clone)]
pub struct CommandConfig {
    pub name: String,
    pub description: Option<String>,
    pub requirements: Vec<Requirement>,
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

/// A rule that matches a row, together with the distinct ways running it can
/// turn out.
#[derive(Debug, Clone)]
pub struct MatchResult {
    pub command: CommandConfig,
    /// The rule's distinct execution targets within the row, in discovery
    /// order. Candidates that would run the identical command have already
    /// collapsed, so this is precisely what a user still has to choose between.
    pub targets: Vec<Entry>,
}

impl MatchResult {
    /// Whether running this rule requires the user to choose a target.
    ///
    /// True exactly when the row's candidates prepare more than one distinct
    /// command. Below that there is nothing to decide; above it, deciding for
    /// the user would mean running one service's command against another.
    pub fn needs_selection(&self) -> bool {
        self.targets.len() > 1
    }
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
                let candidates: Vec<Entry> = group
                    .instances()
                    .iter()
                    .flat_map(|record| command.candidates(record))
                    .collect();
                let targets = command.distinct_targets(candidates);
                if targets.is_empty() {
                    return None;
                }
                Some(MatchResult {
                    command: command.clone(),
                    targets,
                })
            })
            .collect()
    }

    pub fn command_count(&self) -> usize {
        self.commands.len()
    }

    /// The rules applicable to `group` when the caller only needs their
    /// identities. This avoids cloning candidates and preparing commands for
    /// the command projection, where targets are not shown until selection.
    pub fn matching_command_names(&self, group: &EntryGroup) -> Vec<String> {
        self.commands
            .iter()
            .filter(|command| {
                group
                    .instances()
                    .iter()
                    .any(|record| command.has_candidate(record))
            })
            .map(|command| command.name.clone())
            .collect()
    }

    pub fn commands(&self) -> &[CommandConfig] {
        &self.commands
    }
}

impl CommandConfig {
    /// Run this rule against a chosen candidate.
    ///
    /// This is the whole execution decision in one place: check the rule's
    /// dependencies, prepare the argument vector, then honour the validated mode
    /// by either forking or handing the command back for exec. Callers do not
    /// sequence those steps, and so cannot get the order wrong or skip one — an
    /// earlier arrangement where the UI drove each step in turn let a caller
    /// launch a command whose requirements were never checked.
    ///
    /// Errors are user-facing and describe the rule, not the mechanism.
    pub fn run(&self, candidate: &Entry) -> Result<ActionOutcome> {
        if let Some(missing) = exec::missing_requirement(&self.requirements) {
            return Err(eyre!("needs `{missing}`, which is not installed"));
        }
        let prepared = self.action.prepare(candidate)?;
        match prepared.mode {
            ActionMode::Fork => {
                exec::fork(&prepared)?;
                Ok(ActionOutcome::Forked)
            }
            ActionMode::Execute => Ok(ActionOutcome::Handoff(prepared)),
        }
    }

    /// The distinct execution targets among `candidates`, in the order the
    /// candidates were discovered.
    ///
    /// Two candidates are the same target when they prepare the same command:
    /// running either does the identical thing, so asking which one would be a
    /// question with one answer. Any field that changes the argument vector —
    /// hostname, name, service type, domain, TXT value, port, address — leaves
    /// them distinct, and then the caller must ask.
    ///
    /// Keying on the prepared command rather than on a list of fields deemed
    /// "instance-specific" is the point. A field list cannot know what a rule
    /// *does*: it let `ssh {hostname}` run the lexically first host of a
    /// service-type row without asking, because `hostname` was not on the list.
    /// Preparing the command answers the question directly, for every field at
    /// once, including ones nobody thought to enumerate.
    ///
    /// A candidate that fails to prepare is kept as a target rather than
    /// dropped, so a rule that cannot run for the chosen service can never
    /// quietly run against a different one instead. Selecting it reports the
    /// failure, which is the honest outcome.
    ///
    /// No candidate reaching here can actually fail that way today:
    /// [`Self::candidates`] admits a record only once
    /// [`Self::template_fields_resolve`] has proved every non-address field
    /// resolves, and gives the rule one concrete address when it names one. The
    /// branch stays because that invariant is enforced in another method — if
    /// candidate gating is ever loosened, this is what keeps a rule from
    /// quietly running against a service the user did not choose.
    fn distinct_targets(&self, candidates: Vec<Entry>) -> Vec<Entry> {
        let mut seen: HashSet<Option<PreparedCommand>> = HashSet::new();
        let mut targets = Vec::new();
        for candidate in candidates {
            let prepared = self.action.prepare(&candidate).ok();
            if !seen.insert(prepared) {
                continue;
            }
            targets.push(candidate);
        }
        targets
    }

    /// Whether this command distinguishes between a service's individual
    /// addresses (it matches on `address` or templates `{address}`).
    fn uses_address(&self) -> bool {
        self.action.template.references("address")
            || self.predicates.iter().any(|p| is_address_field(&p.field))
    }

    /// The concrete candidates of `record` that satisfy this command's *whole*
    /// rule, in the record's existing address order. Empty when the record does
    /// not match, so "does this rule match?" and "what can it act on?" are one
    /// question that cannot be answered two different ways.
    ///
    /// Address predicates are a conjunction over a single concrete address: an
    /// address is a candidate only if it satisfies *every* address predicate.
    /// Two predicates can therefore never be satisfied by two different
    /// addresses. Non-address predicates are evaluated against the record.
    fn candidates(&self, record: &Entry) -> Vec<Entry> {
        if !self.non_address_predicates_match(record) || !self.template_fields_resolve(record) {
            return Vec::new();
        }
        if !self.uses_address() {
            // The command cannot tell the addresses apart, so the service stays
            // one candidate carrying all of them.
            return vec![record.clone()];
        }
        // The rule needs one concrete address: either a predicate constrains it
        // or the template interpolates it. Without a satisfying address (an
        // entry that has none included) there is no executable candidate, so the
        // action is not offered at all rather than deferred to a preparation
        // error. With no address predicate every address qualifies, and the
        // instance picker disambiguates.
        record
            .addresses
            .iter()
            .filter(|addr| self.address_predicates_match(addr))
            .map(|addr| {
                let mut candidate = record.clone();
                candidate.addresses = vec![*addr];
                candidate
            })
            .collect()
    }

    /// Whether `record` yields at least one runnable candidate, without
    /// constructing that candidate. The command view uses this existence check
    /// while assigning services to rules; concrete targets are built only when
    /// the user opens an action.
    fn has_candidate(&self, record: &Entry) -> bool {
        if !self.non_address_predicates_match(record) || !self.template_fields_resolve(record) {
            return false;
        }
        !self.uses_address()
            || record
                .addresses
                .iter()
                .any(|address| self.address_predicates_match(address))
    }

    /// Whether every field this rule's template interpolates is one `record`
    /// actually carries.
    ///
    /// A rule that cannot be rendered is not an action a user can take, so the
    /// record yields no candidate and the action is never offered for it. The
    /// alternative — offering it and failing once chosen — turns a knowable fact
    /// into a dead end. `address` is excluded here because it is decided per
    /// concrete address by [`Self::candidates`].
    fn template_fields_resolve(&self, record: &Entry) -> bool {
        self.action
            .template
            .fields()
            .filter(|field| !is_address_field(field))
            .all(|field| record.has_field(field))
    }

    /// Whether `record` satisfies every predicate on a field other than
    /// `address`. Address predicates are excluded because they are only
    /// meaningful against one concrete address; [`Self::candidates`] applies
    /// them per address.
    fn non_address_predicates_match(&self, record: &Entry) -> bool {
        self.predicates
            .iter()
            .filter(|predicate| !is_address_field(&predicate.field))
            .all(|predicate| predicate.matches(record))
    }

    /// Whether `addr` satisfies every `address` predicate on this command (so it
    /// is an address worth offering as a candidate). Vacuously true when the
    /// command has no `address` predicate.
    fn address_predicates_match(&self, addr: &IpAddr) -> bool {
        let value = addr.to_string();
        self.predicates
            .iter()
            .filter(|p| is_address_field(&p.field))
            .all(|p| p.predicate.matches_value(&value))
    }
}

impl FieldPredicate {
    /// Whether `record`'s value for this predicate's field satisfies it.
    ///
    /// Only valid for non-address fields. An `address` predicate must be
    /// evaluated against one concrete address via
    /// [`CommandConfig::address_predicates_match`]: `Entry::field("address")`
    /// reports only the primary address, and testing each predicate against a
    /// record independently would let two predicates be satisfied by two
    /// different addresses.
    fn matches(&self, record: &Entry) -> bool {
        debug_assert!(
            !is_address_field(&self.field),
            "address predicates are evaluated per concrete address, not per record"
        );
        let Some(value) = record.field_value(&self.field) else {
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

/// The service fields a rule may match on or interpolate.
///
/// This is the rule language's vocabulary, so it lives with the rules rather
/// than with the discovery layer that answers the lookups. `type` is an accepted
/// alias of `service_type`, and `txt.<key>` admits any non-empty TXT key, since
/// devices invent their own. Anything else is a typo or a wish, and both are
/// rejected at load time instead of silently never matching.
///
/// `supported_fields_resolve_against_a_populated_record` keeps this list honest
/// against [`Entry::field`].
pub(crate) fn is_supported_field(field: &str) -> bool {
    const FIXED: [&str; 7] = [
        "name",
        "type",
        "service_type",
        "domain",
        "hostname",
        "address",
        "port",
    ];
    FIXED.contains(&field)
        || field
            .strip_prefix("txt.")
            .is_some_and(|key| !key.is_empty())
}

/// The match field naming a service's IP address. Address predicates are the one
/// field evaluated per concrete address rather than per record, so the spelling
/// is defined once here.
fn is_address_field(field: &str) -> bool {
    field == "address"
}

/// A strategy for deciding which command rules apply to a discovered row.
///
/// This is a supported extension point, not an internal convenience: kinjo
/// publishes `plumber`, and [`crate::ui::App::new`] accepts any engine, so a
/// crate depending on kinjo can substitute its own matching strategy from its
/// own composition root. See
/// `docs/adr/0001-rule-engine-is-a-supported-extension-point.md`.
///
/// [`Matcher`] is the engine kinjo ships, not the only one this interface
/// admits. Nothing here may assume an implementor stores its rules the way
/// `Matcher` does — in particular an engine is free to keep them in a map, to
/// derive them, or to hold no `CommandConfig` values at all until asked.
///
/// # Implementor's contract
///
/// - **Agreement.** Every [`MatchResult::command`] returned by
///   [`Self::matches_group`] must be one [`Self::commands`] also reports, and
///   the two must agree on `name`. The UI joins the two by name to build its
///   command view; a match naming a rule the engine will not list is dropped
///   from that view without complaint.
/// - **Names are identity.** Rule names must be unique within an engine. The UI
///   uses the name to track a selected rule across recomputes, so duplicates
///   make selection follow whichever rule is found first.
/// - **Purity.** Both methods are called during a redraw and must not block, run
///   a command, or depend on wall-clock time. Matching is a decision about the
///   row it is given; running is [`CommandConfig::run`].
/// - **Order.** [`Self::commands`] must return a stable order across calls that
///   report the same rules, since it is the order the user sees. Rules that
///   change between calls are fine — that is what a config reload is.
/// - **Targets.** Each returned `MatchResult` must carry at least one target;
///   an empty match means "no match" and should be omitted. Targets that
///   would prepare identical commands should already have collapsed, because
///   [`MatchResult::needs_selection`] decides from their count whether to ask
///   the user to choose.
pub trait RuleEngine {
    /// The rules applying to `group`, each with the distinct targets that
    /// running it would choose between.
    ///
    /// Returns empty when no rule applies. Called once per recompute for every
    /// visible row, so an engine that matches expensively should say so.
    fn matches_group(&self, group: &EntryGroup) -> Vec<MatchResult>;

    /// The names of rules applying to `group`, in engine order.
    ///
    /// The default preserves compatibility for engines that only implement
    /// full matching. Engines with a cheaper existence check should override
    /// it; this is the Interface used to assemble the command projection.
    fn matching_command_names(&self, group: &EntryGroup) -> Vec<String> {
        self.matches_group(group)
            .into_iter()
            .map(|result| result.command.name)
            .collect()
    }

    /// Every rule this engine can offer, in the order the user should see them.
    ///
    /// Returns owned rules rather than a borrowed slice deliberately: a slice
    /// would oblige every engine to keep its rules contiguously in a
    /// `Vec<CommandConfig>`, which is `Matcher`'s storage decision and no one
    /// else's. The caller consumes what it gets, so an engine that already owns
    /// a `Vec` pays one clone and an engine that derives its rules pays nothing
    /// it would not have paid anyway.
    fn commands(&self) -> Vec<CommandConfig>;

    /// How many rules [`Self::commands`] would report.
    ///
    /// Provided, so a minimal engine implements two methods rather than three.
    /// The default materialises every rule to count it; override when the count
    /// is cheaper than that, as it is for anything holding a collection.
    fn command_count(&self) -> usize {
        self.commands().len()
    }
}

impl RuleEngine for Matcher {
    fn matches_group(&self, group: &EntryGroup) -> Vec<MatchResult> {
        Matcher::matches_group(self, group)
    }

    fn commands(&self) -> Vec<CommandConfig> {
        self.commands.clone()
    }

    fn matching_command_names(&self, group: &EntryGroup) -> Vec<String> {
        Matcher::matching_command_names(self, group)
    }

    /// Overridden: the rules are already a `Vec`, so counting them need not
    /// clone them.
    fn command_count(&self) -> usize {
        Matcher::command_count(self)
    }
}

mod config;
pub use config::*;

mod template;
use template::CommandTemplate;

pub mod exec;
pub use exec::{PreparedCommand, Requirement};

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
        let extra = PathBuf::from("/tmp/kinjo-extra");
        let dirs = config_dirs(std::slice::from_ref(&extra));
        assert!(dirs.contains(&PathBuf::from(SYSTEM_CONFIG_DIR)));
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
command = "ssh -- {hostname}"
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
        assert_eq!(command.action.command, "ssh -- {hostname}");
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
        // Requirements are parsed at load time, so the rule holds the marker as
        // a decided fact rather than as text to re-read on every invocation.
        assert_eq!(
            command.requirements,
            vec![
                Requirement {
                    command: "xdg-open".to_string(),
                    optional: false,
                },
                Requirement {
                    command: "browser".to_string(),
                    optional: true,
                },
            ]
        );
        assert_eq!(
            command.action.description.as_deref(),
            Some("Open the printer web UI")
        );
        assert_eq!(command.action.mode, ActionMode::Fork);
        assert_eq!(command.action.mode.to_string(), "fork");
    }

    #[test]
    fn command_loading_enforces_option_safety_and_the_explicit_opt_out() {
        let unsafe_rule = r#"
[metadata]
name = "unsafe"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "ssh {hostname}"
mode = "execute"
"#;
        let mut builder = MatcherBuilder::new();
        let err = builder
            .add_str("unsafe.toml", unsafe_rule)
            .unwrap_err()
            .to_string();
        assert!(err.contains("can begin an option-like argument"), "{err}");

        let guarded = unsafe_rule
            .replace("name = \"unsafe\"", "name = \"guarded\"")
            .replace("ssh {hostname}", "ssh -- {hostname}");
        builder.add_str("guarded.toml", &guarded).unwrap();

        let opted_out = unsafe_rule
            .replace("name = \"unsafe\"", "name = \"opted-out\"")
            .replace(
                "mode = \"execute\"",
                "mode = \"execute\"\nallow_option_like_values = true",
            );
        builder.add_str("opted-out.toml", &opted_out).unwrap();

        assert_eq!(builder.build().command_count(), 2);
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
command = "ssh -- {hostname}"
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
command = "ssh -- {hostname}"
mode = "execute"
"#,
            )
            .unwrap();
        let matcher = builder.build();
        let mut record = Entry::new("alpha", "_ssh._tcp", "local");
        record.hostname = Some("alpha.local".to_string());
        let group = crate::discovery::browse_groups(
            &[record],
            crate::discovery::BrowseMode::LogicalService,
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
        let group = crate::discovery::browse_groups(
            &[matching, wrong_txt],
            crate::discovery::BrowseMode::ServiceType,
        )
        .remove(0);

        let matches = matcher.matches_group(&group);

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].targets.len(), 1);
        assert_eq!(
            matches[0].targets[0].hostname.as_deref(),
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
command = "ssh -- {hostname}"
mode = "execute"
"#,
            )
            .unwrap();
        let matcher = builder.build();
        let record = Entry::new("alpha", "_ssh._tcp", "local");
        let group = crate::discovery::browse_groups(
            &[record],
            crate::discovery::BrowseMode::LogicalService,
        )
        .remove(0);

        assert!(matcher.matches_group(&group).is_empty());
    }

    /// A rule may constrain the address without rendering it. Every address
    /// that satisfies the predicate then prepares the very same command, so
    /// there is nothing to choose: asking would be a question with one answer.
    #[test]
    fn addresses_preparing_the_same_command_do_not_ask_which_address() {
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
allow_option_like_values = true
"#,
            )
            .unwrap();
        let matcher = builder.build();
        let mut record = Entry::new("site", "_http._tcp", "local");
        record.hostname = Some("site.local".to_string());
        // Both satisfy the predicate, so both are candidates.
        record.addresses = vec![
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)),
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 11)),
        ];
        let group = crate::discovery::browse_groups(
            &[record],
            crate::discovery::BrowseMode::LogicalService,
        )
        .remove(0);

        let matches = matcher.matches_group(&group);

        assert_eq!(matches.len(), 1);
        assert!(
            !matches[0].needs_selection(),
            "two addresses rendering one identical command must not ask"
        );
        assert_eq!(matches[0].targets.len(), 1);
    }

    /// The same rule that renders the address, over the same two addresses,
    /// does have something to ask about.
    #[test]
    fn addresses_preparing_different_commands_ask_which_address() {
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
command = "ping {address}"
mode = "execute"
"#,
            )
            .unwrap();
        let matcher = builder.build();
        let mut record = Entry::new("site", "_http._tcp", "local");
        record.hostname = Some("site.local".to_string());
        record.addresses = vec![
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)),
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 11)),
        ];
        let group = crate::discovery::browse_groups(
            &[record],
            crate::discovery::BrowseMode::LogicalService,
        )
        .remove(0);

        let matches = matcher.matches_group(&group);

        assert_eq!(matches.len(), 1);
        assert!(matches[0].needs_selection());
        assert_eq!(candidate_addresses(&matches), ["192.0.2.10", "192.0.2.11"]);
    }

    #[test]
    fn merged_per_interface_instances_offer_address_selection() {
        let mut builder = MatcherBuilder::new();
        builder
            .add_str(
                "by-address",
                r#"
[metadata]
name = "by-address"

[match.service_type]
equals = "_workstation._tcp"

[action]
command = "ping {address}"
mode = "execute"
"#,
            )
            .unwrap();
        let matcher = builder.build();

        // A multi-homed host: one service instance per interface, names
        // differing only in Avahi's ` [MAC]` decoration. Grouping merges them
        // into one logical service with two instances.
        let mut wired = Entry::new("rpi5-0 [d8:3a:dd:f4:b1:dc]", "_workstation._tcp", "local");
        wired.hostname = Some("rpi5-0.local".to_string());
        wired.addresses = vec![IpAddr::V4(Ipv4Addr::new(192, 168, 50, 244))];
        wired.port = Some(9);
        let mut wireless = Entry::new("rpi5-0 [d8:3a:dd:f4:b1:dd]", "_workstation._tcp", "local");
        wireless.hostname = Some("rpi5-0.local".to_string());
        wireless.addresses = vec![IpAddr::V4(Ipv4Addr::new(192, 168, 50, 245))];
        wireless.port = Some(9);
        let group = crate::discovery::browse_groups(
            &[wired, wireless],
            crate::discovery::BrowseMode::LogicalService,
        )
        .remove(0);
        assert_eq!(group.instances().len(), 2);

        let matches = matcher.matches_group(&group);

        // The `{address}` command needs a concrete IP, so both interfaces'
        // addresses are offered for selection.
        assert_eq!(matches.len(), 1);
        assert!(matches[0].needs_selection());
        let addresses: Vec<String> = matches[0]
            .targets
            .iter()
            .map(|record| record.primary_address().unwrap().to_string())
            .collect();
        assert!(addresses.contains(&"192.168.50.244".to_string()));
        assert!(addresses.contains(&"192.168.50.245".to_string()));
    }

    /// Builds a matcher holding one rule, through the real loading interface.
    fn matcher_with(source: &str) -> Matcher {
        let mut builder = MatcherBuilder::new();
        builder.add_str("address-rule", source).unwrap();
        builder.build()
    }

    /// One `_workstation._tcp` service advertising `addresses`, as the single
    /// logical-service group the matcher is asked about.
    fn workstation_group(addresses: &[&str]) -> EntryGroup {
        let mut record = Entry::new("host", "_workstation._tcp", "local");
        record.hostname = Some("host.local".to_string());
        record.addresses = addresses
            .iter()
            .map(|address| address.parse().unwrap())
            .collect();
        record.port = Some(9);
        crate::discovery::browse_groups(&[record], crate::discovery::BrowseMode::LogicalService)
            .remove(0)
    }

    /// The concrete address each offered candidate would act on, in order.
    fn candidate_addresses(matches: &[MatchResult]) -> Vec<String> {
        matches
            .iter()
            .flat_map(|result| &result.targets)
            .map(|record| {
                let [address] = record.addresses.as_slice() else {
                    panic!("candidate must carry exactly one concrete address");
                };
                address.to_string()
            })
            .collect()
    }

    #[test]
    fn address_predicates_satisfied_by_different_addresses_do_not_match() {
        // `contains "10."` is satisfied only by the IPv4 address and `regex ":"`
        // only by the IPv6 one. No single address satisfies the whole rule, so
        // the command must not be offered against either.
        let matcher = matcher_with(
            r#"
[metadata]
name = "dual-stack"

[match.service_type]
equals = "_workstation._tcp"

[match.address]
contains = "10."
regex = ":"

[action]
command = "ping {address}"
mode = "execute"
"#,
        );
        let group = workstation_group(&["10.0.0.1", "2001:db8::1"]);

        assert!(matcher.matches_group(&group).is_empty());
    }

    #[test]
    fn address_predicates_are_conjunctive_over_one_address() {
        // Only 10.0.0.99 satisfies both predicates: 10.0.0.1 fails the regex and
        // 192.168.1.5 fails the `contains`.
        let matcher = matcher_with(
            r#"
[metadata]
name = "conjunctive"

[match.service_type]
equals = "_workstation._tcp"

[match.address]
contains = "10."
regex = "\\.99$"

[action]
command = "ping {address}"
mode = "execute"
"#,
        );
        let group = workstation_group(&["10.0.0.1", "192.168.1.5", "10.0.0.99"]);

        let matches = matcher.matches_group(&group);

        assert_eq!(candidate_addresses(&matches), vec!["10.0.0.99".to_string()]);
    }

    #[test]
    fn address_template_without_predicates_expands_every_address_in_order() {
        let matcher = matcher_with(
            r#"
[metadata]
name = "any-address"

[match.service_type]
equals = "_workstation._tcp"

[action]
command = "ping {address}"
mode = "execute"
"#,
        );
        let group = workstation_group(&["10.0.0.1", "192.168.1.5", "2001:db8::1"]);

        let matches = matcher.matches_group(&group);

        // Every address is a candidate for the user to disambiguate, in the
        // entry's existing order.
        assert_eq!(
            candidate_addresses(&matches),
            vec![
                "10.0.0.1".to_string(),
                "192.168.1.5".to_string(),
                "2001:db8::1".to_string(),
            ]
        );
    }

    #[test]
    fn entry_without_address_does_not_satisfy_an_address_predicate() {
        let matcher = matcher_with(
            r#"
[metadata]
name = "by-address"

[match.service_type]
equals = "_workstation._tcp"

[match.address]
contains = "10."

[action]
command = "ping {address}"
mode = "execute"
"#,
        );

        assert!(matcher.matches_group(&workstation_group(&[])).is_empty());
    }

    #[test]
    fn entry_without_address_offers_no_candidate_for_an_address_template() {
        // No address predicate constrains the rule, but the template needs a
        // concrete address. An unresolved entry has none, so the action is not
        // offered rather than failing later during preparation.
        let matcher = matcher_with(
            r#"
[metadata]
name = "any-address"

[match.service_type]
equals = "_workstation._tcp"

[action]
command = "ping {address}"
mode = "execute"
"#,
        );

        assert!(matcher.matches_group(&workstation_group(&[])).is_empty());
    }

    #[test]
    fn address_predicate_still_matches_a_single_satisfying_address() {
        let matcher = matcher_with(
            r#"
[metadata]
name = "by-address"

[match.service_type]
equals = "_workstation._tcp"

[match.address]
contains = "10."

[action]
command = "ping {address}"
mode = "execute"
"#,
        );

        let matches = matcher.matches_group(&workstation_group(&["10.0.0.1"]));
        assert_eq!(candidate_addresses(&matches), vec!["10.0.0.1".to_string()]);

        // ...and rejects a lone address that violates it, rather than falling
        // back to offering it anyway.
        assert!(
            matcher
                .matches_group(&workstation_group(&["192.168.1.5"]))
                .is_empty()
        );
    }

    #[test]
    fn commands_without_address_use_keep_all_addresses_on_one_candidate() {
        // The rule cannot tell the addresses apart, so the service stays a
        // single candidate carrying every address.
        let matcher = matcher_with(
            r#"
[metadata]
name = "by-host"

[match.service_type]
equals = "_workstation._tcp"

[action]
command = "ssh -- {hostname}"
mode = "execute"
"#,
        );
        let group = workstation_group(&["10.0.0.1", "192.168.1.5"]);

        let matches = matcher.matches_group(&group);

        assert_eq!(matches.len(), 1);
        assert!(!matches[0].needs_selection());
        assert_eq!(matches[0].targets.len(), 1);
        assert_eq!(matches[0].targets[0].addresses.len(), 2);
    }

    #[test]
    fn supported_match_fields_and_aliases_load() {
        // The `type` alias and an arbitrary TXT key are part of the rule
        // vocabulary, so they must survive the field check that rejects
        // `service_typ`.
        let matcher = matcher_with(
            r#"
[metadata]
name = "aliases"

[match.type]
equals = "_ssh._tcp"

[match.txt.anything-a-device-invented]
contains = "x"

[action]
command = "echo {type} {txt.anything-a-device-invented}"
mode = "execute"
allow_option_like_values = true
"#,
        );

        assert_eq!(matcher.command_count(), 1);
        assert_eq!(matcher.commands()[0].predicates.len(), 2);
    }

    /// Every field the rule vocabulary admits must be one the discovery layer
    /// can actually answer. The two lists are defined apart — `is_supported_field`
    /// here, `Entry::field` in discovery — so this pins them together; a field
    /// accepted at load but unresolvable at run would offer an action that can
    /// never fire.
    #[test]
    fn supported_fields_resolve_against_a_populated_record() {
        let mut record = Entry::new("alpha", "_ssh._tcp", "local");
        record.hostname = Some("alpha.local".to_string());
        record.addresses = vec![IpAddr::V4(Ipv4Addr::new(192, 0, 2, 5))];
        record.port = Some(22);
        record.txt.insert("path".to_string(), "/admin".to_string());

        for field in [
            "name",
            "type",
            "service_type",
            "domain",
            "hostname",
            "address",
            "port",
            "txt.path",
        ] {
            assert!(is_supported_field(field), "`{field}` must be supported");
            assert!(
                record.field(field).is_some(),
                "supported field `{field}` must resolve"
            );
        }

        for field in ["", "txt", "txt.", "service_typ", "unknown"] {
            assert!(!is_supported_field(field), "`{field}` must be rejected");
        }
    }

    #[test]
    fn a_record_missing_a_templated_field_offers_no_candidate() {
        // The rule matches the service type, but the record never resolved a
        // hostname. Rendering would fail, so the action is not offered at all
        // rather than presented and then refused.
        let matcher = matcher_with(
            r#"
[metadata]
name = "ssh"

[match.service_type]
equals = "_workstation._tcp"

[action]
command = "ssh -- {hostname}"
mode = "execute"
"#,
        );
        let mut record = Entry::new("host", "_workstation._tcp", "local");
        record.addresses = vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))];
        let group = crate::discovery::browse_groups(
            &[record],
            crate::discovery::BrowseMode::LogicalService,
        )
        .remove(0);

        assert!(matcher.matches_group(&group).is_empty());
    }

    #[test]
    fn a_record_missing_a_templated_txt_key_offers_no_candidate() {
        let matcher = matcher_with(
            r#"
[metadata]
name = "open-admin"

[match.service_type]
equals = "_http._tcp"

[action]
command = "xdg-open {txt.adminurl}"
mode = "fork"
allow_option_like_values = true
"#,
        );
        let with_key = {
            let mut record = Entry::new("nas", "_http._tcp", "local");
            record
                .txt
                .insert("adminurl".to_string(), "http://nas/admin".to_string());
            record
        };
        let without_key = Entry::new("other", "_http._tcp", "local");
        let group = crate::discovery::browse_groups(
            &[with_key, without_key],
            crate::discovery::BrowseMode::ServiceType,
        )
        .remove(0);

        let matches = matcher.matches_group(&group);

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].targets.len(), 1);
        assert_eq!(matches[0].targets[0].name, "nas");
    }

    /// A group of `records` under `mode`, as the matcher is asked about it.
    fn group_of(records: &[Entry], mode: crate::discovery::BrowseMode) -> EntryGroup {
        crate::discovery::browse_groups(records, mode).remove(0)
    }

    /// An `_ssh._tcp` service on `hostname`.
    fn ssh_on(name: &str, hostname: &str) -> Entry {
        let mut record = Entry::new(name, "_ssh._tcp", "local");
        record.hostname = Some(hostname.to_string());
        record.port = Some(22);
        record
    }

    /// The argv each target would run, in order.
    fn target_argv(result: &MatchResult) -> Vec<Vec<String>> {
        result
            .targets
            .iter()
            .map(|target| result.command.action.prepare(target).unwrap().argv)
            .collect()
    }

    /// The defect this task exists for: a service-type row legitimately holds
    /// several hosts, and `hostname` is not an "instance-specific" field, so the
    /// old field-list heuristic ran the lexically first host with no picker.
    #[test]
    fn a_hostname_template_over_several_hosts_asks_which_host() {
        let matcher = matcher_with(
            r#"
[metadata]
name = "ssh"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "ssh -- {hostname}"
mode = "execute"
"#,
        );
        let group = group_of(
            &[ssh_on("alpha", "alpha.local"), ssh_on("beta", "beta.local")],
            crate::discovery::BrowseMode::ServiceType,
        );
        assert_eq!(group.instances().len(), 2);

        let matches = matcher.matches_group(&group);

        assert_eq!(matches.len(), 1);
        assert!(
            matches[0].needs_selection(),
            "two hosts run two different commands, so the user must choose"
        );
        assert_eq!(
            target_argv(&matches[0]),
            [
                vec![
                    "ssh".to_string(),
                    "--".to_string(),
                    "alpha.local".to_string(),
                ],
                vec![
                    "ssh".to_string(),
                    "--".to_string(),
                    "beta.local".to_string(),
                ],
            ]
        );
    }

    /// The same generalization, for a field nobody would have put on a list of
    /// instance-specific ones.
    #[test]
    fn a_txt_template_over_differing_values_asks_which_service() {
        let matcher = matcher_with(
            r#"
[metadata]
name = "open-path"

[match.txt.path]
regex = "."

[action]
command = "open http://host/{txt.path}"
mode = "execute"
"#,
        );
        let mut first = Entry::new("one", "_http._tcp", "local");
        first.hostname = Some("host.local".to_string());
        first.txt.insert("path".to_string(), "admin".to_string());
        let mut second = Entry::new("two", "_http._tcp", "local");
        second.hostname = Some("host.local".to_string());
        second.txt.insert("path".to_string(), "status".to_string());
        let group = group_of(&[first, second], crate::discovery::BrowseMode::Host);

        let matches = matcher.matches_group(&group);

        assert_eq!(matches.len(), 1);
        assert!(matches[0].needs_selection());
        assert_eq!(
            target_argv(&matches[0]),
            [
                vec!["open".to_string(), "http://host/admin".to_string()],
                vec!["open".to_string(), "http://host/status".to_string()],
            ]
        );
    }

    /// A rule that renders nothing about the service runs one command however
    /// many services the row holds, so it must not ask.
    #[test]
    fn several_services_preparing_one_identical_command_do_not_ask() {
        let matcher = matcher_with(
            r#"
[metadata]
name = "constant"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "echo hello"
mode = "execute"
"#,
        );
        let group = group_of(
            &[ssh_on("alpha", "alpha.local"), ssh_on("beta", "beta.local")],
            crate::discovery::BrowseMode::ServiceType,
        );

        let matches = matcher.matches_group(&group);

        assert_eq!(matches.len(), 1);
        assert!(
            !matches[0].needs_selection(),
            "one effective command must not raise a picker"
        );
        assert_eq!(matches[0].targets.len(), 1);
    }

    /// Distinct services that happen to render the same command collapse too:
    /// the question is what runs, not how many services are behind it.
    #[test]
    fn different_services_rendering_the_same_command_collapse() {
        let matcher = matcher_with(
            r#"
[metadata]
name = "ssh"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "ssh -- {hostname}"
mode = "execute"
"#,
        );
        // Two registrations of one host — different service names, one host.
        let group = group_of(
            &[
                ssh_on("alpha", "shared.local"),
                ssh_on("beta", "shared.local"),
            ],
            crate::discovery::BrowseMode::ServiceType,
        );
        assert_eq!(group.instances().len(), 2);

        let matches = matcher.matches_group(&group);

        assert_eq!(matches.len(), 1);
        assert!(!matches[0].needs_selection());
        assert_eq!(
            target_argv(&matches[0]),
            [vec![
                "ssh".to_string(),
                "--".to_string(),
                "shared.local".to_string(),
            ]]
        );
    }

    /// Network data cannot select the executable, even through the explicit
    /// option-like-value escape hatch.
    #[test]
    fn a_discovered_value_cannot_select_the_executable() {
        let mut builder = MatcherBuilder::new();
        let err = builder
            .add_str(
                "dynamic-program",
                r#"
[metadata]
name = "run-named"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "{name} --version"
mode = "execute"
allow_option_like_values = true
"#,
            )
            .unwrap_err()
            .to_string();

        assert!(err.contains("program must be literal"), "{err}");
    }

    /// The rule, not the caller, decides what running it means.
    #[test]
    fn run_hands_back_an_execute_command_without_spawning_it() {
        let matcher = matcher_with(
            r#"
[metadata]
name = "ssh"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "ssh -p {port} -- '{hostname}'"
mode = "execute"
"#,
        );
        let mut record = Entry::new("alpha", "_ssh._tcp", "local");
        record.hostname = Some("alpha.local".to_string());
        record.port = Some(2222);

        let outcome = matcher.commands()[0].run(&record).unwrap();

        match outcome {
            ActionOutcome::Handoff(prepared) => {
                assert_eq!(prepared.argv, ["ssh", "-p", "2222", "--", "alpha.local"]);
                assert_eq!(prepared.mode, ActionMode::Execute);
            }
            other => panic!("expected a hand-off, got {other:?}"),
        }
    }

    #[test]
    fn run_forks_a_fork_mode_command() {
        let matcher = matcher_with(
            r#"
[metadata]
name = "noop"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "true"
mode = "fork"
"#,
        );
        let record = Entry::new("alpha", "_ssh._tcp", "local");

        // `true` exits 0 immediately; the rule forks it and reports that the
        // caller has nothing left to do.
        assert!(matches!(
            matcher.commands()[0].run(&record).unwrap(),
            ActionOutcome::Forked
        ));
    }

    #[test]
    fn run_refuses_before_launching_when_a_mandatory_requirement_is_absent() {
        // The requirement gate belongs to the rule: a caller cannot forget it,
        // and `false` must never be spawned.
        let matcher = matcher_with(
            r#"
[metadata]
name = "needs-tool"
requirements = ["kinjo-absent-tool-xyz", "kinjo-also-absent-xyz, optional"]

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "false"
mode = "fork"
"#,
        );
        let record = Entry::new("alpha", "_ssh._tcp", "local");

        let err = matcher.commands()[0].run(&record).unwrap_err().to_string();

        // The mandatory one is reported; the optional one is not.
        assert!(err.contains("needs `kinjo-absent-tool-xyz`"), "{err}");
        assert!(!err.contains("kinjo-also-absent-xyz"), "{err}");
    }

    #[test]
    fn a_discovered_value_cannot_reshape_a_prepared_argv() {
        // The injection barrier, through the loading interface a user's config
        // actually travels: a hostile service name carrying separators, quotes,
        // and braces fills exactly one argument.
        let matcher = matcher_with(
            r#"
[metadata]
name = "notify"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "notify-send -- {name} tail"
mode = "execute"
"#,
        );
        let record = Entry::new(
            r#"evil" 'x' {hostname} && rm -rf / #"#,
            "_ssh._tcp",
            "local",
        );

        let prepared = matcher.commands()[0].action.prepare(&record).unwrap();

        assert_eq!(
            prepared.argv,
            [
                "notify-send",
                "--",
                r#"evil" 'x' {hostname} && rm -rf / #"#,
                "tail",
            ]
        );
    }

    #[test]
    fn a_quoted_empty_argument_survives_into_the_prepared_argv() {
        let matcher = matcher_with(
            r#"
[metadata]
name = "empty-arg"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "cmd '' {hostname} \"\""
mode = "execute"
allow_option_like_values = true
"#,
        );
        let mut record = Entry::new("alpha", "_ssh._tcp", "local");
        record.hostname = Some("alpha.local".to_string());

        let prepared = matcher.commands()[0].action.prepare(&record).unwrap();

        assert_eq!(prepared.argv, ["cmd", "", "alpha.local", ""]);
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
command = "ssh -- {hostname}"
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
                "unknown field `commands`",
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
                "expected a sequence",
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
            (
                "misspelled-match-field",
                r#"
[metadata]
name = "misspelled-match-field"

[match.service_typ]
equals = "_ssh._tcp"

[action]
command = "ssh"
mode = "execute"
"#,
                "unsupported match field `service_typ`",
            ),
            (
                "bare-txt-match-field",
                r#"
[metadata]
name = "bare-txt"

[match.txt]
equals = "anything"

[action]
command = "ssh"
mode = "execute"
"#,
                "unsupported match field `txt`",
            ),
            (
                "empty-name",
                r#"
[metadata]
name = ""

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "ssh"
mode = "execute"
"#,
                "`metadata.name` is empty",
            ),
            (
                "empty-command",
                r#"
[metadata]
name = "empty-command"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = ""
mode = "execute"
"#,
                "action command is empty",
            ),
            (
                "whitespace-command",
                r#"
[metadata]
name = "whitespace-command"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "   "
mode = "execute"
"#,
                "action command is empty",
            ),
            (
                "unknown-placeholder",
                r#"
[metadata]
name = "unknown-placeholder"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "echo {nonexistent_field}"
mode = "execute"
"#,
                "unknown service field `nonexistent_field`",
            ),
            (
                "unterminated-quote",
                r#"
[metadata]
name = "unterminated-quote"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "echo 'alpha"
mode = "execute"
"#,
                "unterminated `'` quote",
            ),
            (
                "malformed-requirement",
                r#"
[metadata]
name = "malformed-requirement"
requirements = ["browser, mandatory"]

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "ssh"
mode = "execute"
"#,
                "unsupported suffix `mandatory`",
            ),
        ];

        for (source_name, source, expected) in cases {
            let mut builder = MatcherBuilder::new();
            let err = builder
                .add_str(source_name, source)
                .unwrap_err()
                .to_string();
            assert!(
                err.contains(expected),
                "expected `{err}` to contain `{expected}`"
            );
            // Every rejection names the file it came from: a warning a user
            // cannot trace back to a file is not actionable.
            assert!(
                err.contains(source_name),
                "expected `{err}` to name its source `{source_name}`"
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
    fn unknown_escape_sequences_are_rejected() {
        // `\z` is not a valid TOML escape; a real TOML parser reports it
        // instead of silently keeping it verbatim like the old hand-rolled
        // parser did.
        let mut builder = MatcherBuilder::new();
        let err = builder
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
            .unwrap_err();

        assert!(err.to_string().starts_with("verbatim:"));
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
        assert!(array_description.to_string().contains("expected a string"));
    }

    #[test]
    fn key_outside_a_section_is_an_error() {
        let mut builder = MatcherBuilder::new();
        let err = builder.add_str("stray", "name = \"orphan\"\n").unwrap_err();

        assert!(err.to_string().contains("unknown field `name`"));
    }

    #[test]
    fn add_file_reads_a_command_from_disk() {
        let dir = temp_dir("add-file");
        let path = dir.join("ssh.toml");
        fs::write(&path, command_toml("ssh", "ssh -- {hostname}")).unwrap();

        let mut builder = MatcherBuilder::new();
        builder.start_layer();
        builder.add_file(&path).unwrap();

        assert_eq!(builder.build().commands()[0].name, "ssh");

        remove(&dir);
    }

    #[test]
    fn lenient_load_skips_malformed_files_with_a_warning() {
        let dir = temp_dir("lenient");
        fs::write(dir.join("good.toml"), command_toml("good", "true")).unwrap();
        fs::write(dir.join("bad.toml"), "not toml at all [").unwrap();

        let mut builder = MatcherBuilder::new();
        let warnings = load_from_dirs_lenient(&mut builder, std::slice::from_ref(&dir));

        let matcher = builder.build();
        assert_eq!(matcher.command_count(), 1);
        assert_eq!(matcher.commands()[0].name, "good");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("bad.toml"));

        remove(&dir);
    }

    /// A file that is valid TOML but not a valid rule is exactly the case the
    /// old loader let through to fail at invocation. Strict loading (what
    /// `list-commands` uses) must fail on it; lenient startup must skip it,
    /// keep the good rules, and say which file it dropped.
    #[test]
    fn a_semantically_invalid_file_fails_strictly_and_warns_leniently() {
        let dir = temp_dir("semantic-invalid");
        fs::write(dir.join("good.toml"), command_toml("good", "true")).unwrap();
        fs::write(
            dir.join("unknown-field.toml"),
            command_toml("unknown-field", "echo {nonexistent_field}"),
        )
        .unwrap();

        let mut strict = MatcherBuilder::new();
        let err = load_from_dirs(&mut strict, std::slice::from_ref(&dir))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("unknown service field `nonexistent_field`"),
            "{err}"
        );
        assert!(err.contains("unknown-field.toml"), "{err}");

        let mut lenient = MatcherBuilder::new();
        let warnings = load_from_dirs_lenient(&mut lenient, std::slice::from_ref(&dir));

        let matcher = lenient.build();
        assert_eq!(matcher.command_count(), 1);
        assert_eq!(matcher.commands()[0].name, "good");
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains("unknown-field.toml"),
            "{}",
            warnings[0]
        );
        assert!(
            warnings[0].contains("unknown service field"),
            "{}",
            warnings[0]
        );

        remove(&dir);
    }

    /// Every invalid file is reported, not just the first: a user fixing their
    /// configuration should see the whole list in one run.
    #[test]
    fn lenient_load_warns_about_every_invalid_file() {
        let dir = temp_dir("many-invalid");
        fs::write(dir.join("a-good.toml"), command_toml("good", "true")).unwrap();
        fs::write(dir.join("b-bad.toml"), command_toml("b", "echo {bogus}")).unwrap();
        fs::write(dir.join("c-bad.toml"), command_toml("c", "echo 'open")).unwrap();

        let mut builder = MatcherBuilder::new();
        let warnings = load_from_dirs_lenient(&mut builder, std::slice::from_ref(&dir));

        assert_eq!(builder.build().command_count(), 1);
        assert_eq!(warnings.len(), 2);
        assert!(warnings.iter().any(|w| w.contains("b-bad.toml")));
        assert!(warnings.iter().any(|w| w.contains("c-bad.toml")));

        remove(&dir);
    }

    /// A directory that cannot be enumerated is not an empty directory. Reading
    /// it must fail or warn under the caller's policy, never quietly contribute
    /// nothing: an overlay silently missing its rules looks identical to one
    /// that was never configured, and a reload would then be free to install it
    /// over a working rule set.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_directory_fails_strictly_and_warns_leniently() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("unreadable");
        fs::write(dir.join("good.toml"), command_toml("good", "true")).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read_dir(&dir).is_ok() {
            // Running as root, where file permissions do not apply: this test
            // has nothing to observe.
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
            remove(&dir);
            return;
        }

        let mut strict = MatcherBuilder::new();
        let err = load_from_dirs(&mut strict, std::slice::from_ref(&dir))
            .unwrap_err()
            .to_string();
        assert!(err.contains("cannot read"), "{err}");
        assert!(err.contains(&dir.display().to_string()), "{err}");

        let mut lenient = MatcherBuilder::new();
        let warnings = load_from_dirs_lenient(&mut lenient, std::slice::from_ref(&dir));

        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains(&dir.display().to_string()),
            "{warnings:?}"
        );
        assert_eq!(lenient.build().command_count(), 0);

        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        remove(&dir);
    }

    #[test]
    fn load_from_dirs_skips_missing_directories() {
        let mut builder = MatcherBuilder::new();
        load_from_dirs(
            &mut builder,
            &[PathBuf::from("/tmp/kinjo-definitely-missing-xyz")],
        )
        .unwrap();

        assert_eq!(builder.build().command_count(), 0);
    }

    #[test]
    fn config_dirs_from_orders_system_user_then_extras() {
        let extra = PathBuf::from("/tmp/extra");
        let dirs = config_dirs_from(
            None,
            Some(OsString::from("/xdg")),
            Some(OsString::from("/home/user")),
            std::slice::from_ref(&extra),
        );

        assert_eq!(
            dirs,
            vec![
                PathBuf::from(SYSTEM_CONFIG_DIR),
                PathBuf::from("/xdg/kinjo/commands"),
                extra,
            ]
        );
    }

    #[test]
    fn config_dirs_from_uses_home_when_xdg_is_absent() {
        let dirs = config_dirs_from(None, None, Some(OsString::from("/home/user")), &[]);

        assert_eq!(
            dirs,
            vec![
                PathBuf::from(SYSTEM_CONFIG_DIR),
                PathBuf::from("/home/user/.config/kinjo/commands"),
            ]
        );
    }

    #[test]
    fn config_dirs_from_omits_user_dir_without_env() {
        let dirs = config_dirs_from(None, None, None, &[]);

        assert_eq!(dirs, vec![PathBuf::from(SYSTEM_CONFIG_DIR)]);
    }

    #[test]
    fn config_dirs_from_ignores_empty_and_relative_config_homes() {
        let dirs = config_dirs_from(
            None,
            Some(OsString::from("relative")),
            Some(OsString::from("/home/user")),
            &[],
        );
        assert_eq!(
            dirs,
            vec![
                PathBuf::from(SYSTEM_CONFIG_DIR),
                PathBuf::from("/home/user/.config/kinjo/commands"),
            ]
        );

        let dirs = config_dirs_from(
            None,
            Some(OsString::new()),
            Some(OsString::from("relative-home")),
            &[],
        );
        assert_eq!(dirs, vec![PathBuf::from(SYSTEM_CONFIG_DIR)]);
    }

    #[test]
    fn config_dirs_from_puts_install_dirs_below_system_and_user() {
        let dirs = config_dirs_from(
            Some(PathBuf::from("/opt/homebrew/bin/kinjo")),
            Some(OsString::from("/xdg")),
            None,
            &[],
        );

        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/opt/homebrew/bin/commands"),
                PathBuf::from("/opt/homebrew/share/kinjo/commands"),
                PathBuf::from(SYSTEM_CONFIG_DIR),
                PathBuf::from("/xdg/kinjo/commands"),
            ]
        );
    }
}
