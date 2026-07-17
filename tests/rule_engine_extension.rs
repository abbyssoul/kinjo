//! Proof that [`kinjo::plumber::RuleEngine`] is an extension point someone
//! other than `Matcher` can actually implement.
//!
//! ADR 0001 keeps the `RuleEngine` seam on the grounds that kinjo publishes
//! `plumber`, so a crate depending on kinjo can substitute its own matching
//! strategy even though this repository ships only one engine. That claim is
//! about *external* callers, so asserting it from inside `src/` would beg the
//! question: a unit test there can reach private items a real extender cannot.
//!
//! This file is an integration test and therefore compiles against kinjo the
//! way any dependent crate would — public API only. If the extension path
//! regresses to something that needs crate internals, this stops compiling,
//! which is the point.

use std::collections::BTreeMap;

use kinjo::{
    discovery::{BrowseMode, Entry, EntryGroup, browse_groups},
    plumber::{CommandConfig, MatchResult, MatcherBuilder, RuleEngine},
};

/// An engine that indexes rules by the service type they serve.
///
/// Deliberately unlike `Matcher` in the two ways that matter to the interface:
/// it stores rules in a `BTreeMap` rather than a `Vec<CommandConfig>`, so it
/// could not satisfy a `&[CommandConfig]` return without keeping a second copy
/// purely to be sliced; and it matches by keyed lookup rather than by scanning
/// every rule, so it does not merely re-describe what `Matcher` does.
///
/// It leaves [`RuleEngine::command_count`] to the trait's default, which is the
/// other half of what a minimal implementor is promised.
struct KeyedEngine {
    by_service_type: BTreeMap<String, CommandConfig>,
}

impl KeyedEngine {
    /// Build rules with the real loader, then re-index them. An engine is free
    /// to source its rules however it likes; what is being tested is the shape
    /// of the interface, not a new command-file parser.
    fn new(rules: &[(&str, &str)]) -> Self {
        let mut builder = MatcherBuilder::new();
        builder.start_layer();
        for (name, source) in rules {
            builder.add_str(name, source).unwrap();
        }
        let by_service_type = builder
            .build()
            .commands()
            .iter()
            .map(|command| (service_type_of(command), command.clone()))
            .collect();
        Self { by_service_type }
    }
}

/// The service type a rule is keyed under, read back off its own predicates.
fn service_type_of(command: &CommandConfig) -> String {
    use kinjo::plumber::Predicate;
    command
        .predicates
        .iter()
        .find(|p| p.field == "service_type")
        .map(|p| match &p.predicate {
            Predicate::Equals(value) => value.clone(),
            Predicate::Contains(value) => value.clone(),
            Predicate::Regex(regex) => regex.as_str().to_string(),
        })
        .expect("fixture rules all match on service_type")
}

impl RuleEngine for KeyedEngine {
    fn matches_group(&self, group: &EntryGroup) -> Vec<MatchResult> {
        let instances = group.instances();
        let Some(first) = instances.first() else {
            return Vec::new();
        };
        let Some(command) = self.by_service_type.get(&first.service_type) else {
            return Vec::new();
        };
        vec![MatchResult {
            command: command.clone(),
            targets: instances.to_vec(),
        }]
    }

    fn commands(&self) -> Vec<CommandConfig> {
        self.by_service_type.values().cloned().collect()
    }
}

fn rule_toml(name: &str, service_type: &str, command: &str) -> String {
    format!(
        r#"
[metadata]
name = "{name}"

[match.service_type]
equals = "{service_type}"

[action]
command = "{command}"
mode = "execute"
"#
    )
}

fn fixture() -> KeyedEngine {
    let ssh = rule_toml("ssh", "_ssh._tcp", "ssh {hostname}");
    let http = rule_toml("http", "_http._tcp", "curl {address}");
    KeyedEngine::new(&[("ssh", ssh.as_str()), ("http", http.as_str())])
}

fn entry(name: &str, service_type: &str, address: &str) -> Entry {
    let mut record = Entry::new(name, service_type, "local");
    record.hostname = Some(format!("{name}.local"));
    record.addresses = vec![address.parse().unwrap()];
    record.port = Some(22);
    record
}

fn group_of(records: &[Entry]) -> EntryGroup {
    browse_groups(records, BrowseMode::LogicalService)
        .into_iter()
        .next()
        .expect("fixture always produces one group")
}

/// The trait is satisfiable by a type that holds no `Vec<CommandConfig>`, and
/// is usable behind the same trait object the app stores.
#[test]
fn an_engine_without_vec_storage_satisfies_the_trait() {
    let engine: Box<dyn RuleEngine> = Box::new(fixture());

    let names: Vec<String> = engine
        .commands()
        .into_iter()
        .map(|command| command.name)
        .collect();
    assert_eq!(names, vec!["http".to_string(), "ssh".to_string()]);
}

/// The provided `command_count` counts what `commands` reports, so an engine
/// that does not override it still agrees with itself.
#[test]
fn the_default_command_count_agrees_with_commands() {
    let engine = fixture();

    assert_eq!(engine.command_count(), 2);
    assert_eq!(engine.command_count(), engine.commands().len());
}

/// A foreign matching strategy decides matches, and the rule it names is one it
/// also lists — the agreement the trait's contract requires.
#[test]
fn a_foreign_strategy_decides_its_own_matches() {
    let engine = fixture();
    let records = vec![entry("alpha", "_ssh._tcp", "10.0.0.1")];

    let matches = engine.matches_group(&group_of(&records));

    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].command.name, "ssh");
    assert!(
        !matches[0].needs_selection(),
        "one target, nothing to choose"
    );
    assert!(
        engine
            .commands()
            .iter()
            .any(|command| command.name == matches[0].command.name),
        "a match must name a rule the engine also lists"
    );
}

#[test]
fn a_foreign_strategy_reports_no_match_for_an_unknown_service_type() {
    let engine = fixture();
    let records = vec![entry("printer", "_ipp._tcp", "10.0.0.9")];

    assert!(engine.matches_group(&group_of(&records)).is_empty());
}

/// A rule a foreign engine returns is a real, runnable rule: the caller can
/// prepare a command from it without the engine that produced it.
#[test]
fn a_rule_from_a_foreign_engine_prepares_a_command() {
    let engine = fixture();
    let records = vec![entry("alpha", "_ssh._tcp", "10.0.0.1")];

    let matches = engine.matches_group(&group_of(&records));
    let prepared = matches[0]
        .command
        .action
        .prepare(&matches[0].targets[0])
        .expect("the rule templates only fields the entry has");

    assert_eq!(prepared.argv, vec!["ssh", "alpha.local"]);
}

/// The transactional reload path accepts a foreign engine, so an extender's
/// composition root can install one on SIGHUP exactly as `kinjo::run` installs
/// a `Matcher`.
#[test]
fn the_reload_path_accepts_a_foreign_engine() {
    use kinjo::ui::app::ReloadOutcome;

    let outcome = ReloadOutcome::Loaded(Box::new(fixture()));

    match outcome {
        ReloadOutcome::Loaded(engine) => assert_eq!(engine.command_count(), 2),
        ReloadOutcome::Rejected(diagnostics) => panic!("unexpected rejection: {diagnostics:?}"),
    }
}

/// The whole composition path from ADR 0001: an extender builds an `App` around
/// their own engine, attaches both capabilities, wires a reload trigger, and
/// collects what outlives the terminal — all through public API.
///
/// This is `kinjo::run` rewritten from outside the crate, and it is the test
/// that keeps `App`'s public surface honest. Every operation an external
/// composition root needs is exercised here, so if encapsulation ever privatises
/// one away, this stops compiling rather than silently making the extension
/// point unreachable.
///
/// Needs a `DiscoverySession`, and the only session an external caller can
/// start without touching the network is the sample backend, hence the feature
/// gate. `--all-features` in the completion gate covers it.
#[cfg(feature = "fake")]
#[test]
fn a_foreign_engine_composes_into_a_runnable_app() {
    use kinjo::{
        discovery::{self, DiscoveryBackend},
        ui::{
            App,
            app::ReloadOutcome,
            cli::{Cli, CliCommand},
            keymap::KeyBindings,
        },
    };

    let cli = Cli {
        domain: "local".to_string(),
        config_dirs: Vec::new(),
        service_type: None,
        backend: DiscoveryBackend::Fake,
        command: CliCommand::Run,
    };
    let options = cli
        .discovery_options()
        .expect("the sample backend honours the default domain");
    let session = discovery::start(&options);

    // `App::new` takes `impl RuleEngine`, so this composes only while a foreign
    // engine is substitutable.
    let mut app = App::new(cli, fixture(), KeyBindings::default(), session)
        .with_discovery_factory(Box::new(move || discovery::start(&options)))
        .with_config_loader(Box::new(|_cli| ReloadOutcome::Loaded(Box::new(fixture()))));

    // An extender owns their own signal handling; the app only hands out the
    // flag it polls.
    let trigger = app.reload_trigger();
    assert!(
        !trigger.load(std::sync::atomic::Ordering::Relaxed),
        "nothing has asked for a reload yet"
    );

    // Startup warnings are reported as a count; the app words the message.
    app.note_skipped_configs(0);

    // Nothing has been rejected, so there is nothing to print after the
    // terminal goes.
    assert!(app.take_reload_diagnostics().is_empty());

    // Running would need a real terminal, which is what `scripts/drive-tui.sh`
    // is for. Everything up to `app.run(terminal)` is covered here.
}
