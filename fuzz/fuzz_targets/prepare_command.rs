#![no_main]

//! Fuzz command preparation: tokenizing an action's command template and
//! interpolating service fields into it. Two properties:
//!
//!  1. `prepare` must parse or error on any template — never panic.
//!  2. The injection barrier: service field values arrive from untrusted
//!     devices on the network, so whatever a value contains — spaces, quotes,
//!     backslashes, braces — it can only ever fill in its own argument. With a
//!     known template, every argv slot must be exactly the corresponding field
//!     value; the argument count and boundaries must never move.

use arbitrary::Arbitrary;
use kinjo::discovery::Entry;
use kinjo::plumber::{ActionMode, CommandAction, exec};
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct Input {
    name: String,
    service_type: String,
    domain: String,
    hostname: Option<String>,
    port: Option<u16>,
    txt_value: String,
    template: String,
}

fuzz_target!(|input: Input| {
    let mut entry = Entry::new(
        input.name.clone(),
        input.service_type.clone(),
        input.domain.clone(),
    );
    entry.hostname = input.hostname;
    entry.port = input.port;
    entry
        .txt
        .insert("v".to_string(), input.txt_value.clone());

    // 1. An arbitrary template parses or reports an error, never panics.
    let arbitrary_template = CommandAction {
        description: None,
        command: input.template,
        mode: ActionMode::Fork,
    };
    let _ = exec::prepare(&arbitrary_template, &entry);

    // 2. With a fixed template, arbitrary field values cannot add, remove, or
    //    reshape arguments. `name`, `service_type`, `domain`, and the inserted
    //    TXT key are always present, so this must prepare successfully.
    let fixed_template = CommandAction {
        description: None,
        command: "run {name} {service_type} {domain} {txt.v}".to_string(),
        mode: ActionMode::Execute,
    };
    let prepared = exec::prepare(&fixed_template, &entry)
        .expect("all interpolated fields are present");
    assert_eq!(
        prepared.argv,
        vec![
            "run".to_string(),
            input.name,
            input.service_type,
            input.domain,
            input.txt_value,
        ]
    );
});
