#![no_main]

//! Round-trip fuzz for command/action files: arbitrary field values — spaces,
//! quotes, newlines, unicode — serialized into a well-formed TOML command file
//! must load back with exactly the same values. Guards the loader against
//! silently mangling command strings the way a hand-rolled parser once did;
//! `parse_command` separately covers robustness against malformed bytes.
//!
//! Loading now *validates* as well as parses, so an arbitrary template or
//! requirement may legitimately be rejected. That splits the oracle in two:
//!
//!  - accepted: every value the rule retains round-trips exactly, and the
//!    compiled rule agrees with the text it was built from;
//!  - rejected: the report names the source file, so a lenient startup warning
//!    always tells a user which file to go and fix.

use arbitrary::Arbitrary;
use kinjo::plumber::{MatcherBuilder, Predicate, Requirement};
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct Input {
    name: String,
    description: Option<String>,
    requirement: Option<String>,
    match_value: String,
    command: String,
}

fn string(value: &str) -> toml::Value {
    toml::Value::String(value.to_string())
}

/// `toml_writer` 1.1.1 tallies consecutive quotes in a `u8`
/// (`ValueMetrics::calculate` in its `string.rs`), so serializing a string
/// with a run of 256+ `"` or `'` overflows and panics under debug assertions.
/// Only the serializer is affected — kinjo itself just parses TOML — so skip
/// such inputs until the fix for <https://github.com/toml-rs/toml/issues/1188>
/// lands.
fn overflows_toml_writer(value: &str) -> bool {
    [b'"', b'\''].iter().any(|quote| {
        let mut run = 0usize;
        value.bytes().any(|byte| {
            run = if byte == *quote { run + 1 } else { 0 };
            run > u8::MAX as usize
        })
    })
}

fuzz_target!(|input: Input| {
    let fields = [
        Some(&input.name),
        input.description.as_ref(),
        input.requirement.as_ref(),
        Some(&input.match_value),
        Some(&input.command),
    ];
    if fields
        .into_iter()
        .flatten()
        .any(|value| overflows_toml_writer(value))
    {
        return;
    }

    let mut metadata = toml::Table::new();
    metadata.insert("name".to_string(), string(&input.name));
    if let Some(description) = &input.description {
        metadata.insert("description".to_string(), string(description));
    }
    if let Some(requirement) = &input.requirement {
        metadata.insert(
            "requirements".to_string(),
            toml::Value::Array(vec![string(requirement)]),
        );
    }

    let mut predicate = toml::Table::new();
    predicate.insert("equals".to_string(), string(&input.match_value));
    let mut matchers = toml::Table::new();
    matchers.insert("service_type".to_string(), toml::Value::Table(predicate));

    let mut action = toml::Table::new();
    action.insert("command".to_string(), string(&input.command));
    action.insert("mode".to_string(), string("execute"));

    let mut root = toml::Table::new();
    root.insert("metadata".to_string(), toml::Value::Table(metadata));
    root.insert("match".to_string(), toml::Value::Table(matchers));
    root.insert("action".to_string(), toml::Value::Table(action));

    let Ok(source) = toml::to_string(&root) else {
        return;
    };

    let mut builder = MatcherBuilder::new();
    if let Err(err) = builder.add_str("fuzz-source", &source) {
        // Validation may reject this rule; it must still say where it came from.
        assert!(
            err.to_string().contains("fuzz-source"),
            "a rejection must name its source file: {err}"
        );
        return;
    }

    let matcher = builder.build();
    let command = &matcher.commands()[0];

    assert_eq!(command.name, input.name);
    assert_eq!(command.description, input.description);
    // The raw template is retained verbatim for display, byte for byte.
    assert_eq!(command.action.command, input.command);
    // Requirements are stored parsed; the rule must hold exactly what the
    // grammar says the entry means, with no text left to re-read.
    let expected: Vec<Requirement> = input
        .requirement
        .iter()
        .map(|raw| Requirement::parse(raw).expect("an accepted rule has valid requirements"))
        .collect();
    assert_eq!(command.requirements, expected);
    match &command.predicates[0].predicate {
        Predicate::Equals(value) => assert_eq!(value, &input.match_value),
        other => panic!("expected an equals predicate, got {other:?}"),
    }
});
