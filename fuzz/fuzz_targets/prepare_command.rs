#![no_main]

//! Fuzz command templates: compiling one at load time, and rendering it into an
//! argument vector for a discovered service. Three properties:
//!
//!  1. Load-time validation is total: an arbitrary template compiles or returns
//!     an error, never panics. Whatever survives `compile` is executable.
//!  2. The argv *shape* is fixed at compile time. Rendering one compiled
//!     template against two services with wildly different field values must
//!     yield the same number of arguments — a discovered value can fill an
//!     argument but never add, remove, or split one.
//!  3. The injection barrier, stated exactly: with a known template, every argv
//!     slot is precisely the corresponding field value, whatever it contains —
//!     spaces, quotes, backslashes, or braces.
//!
//! Together these cover the reason templates are never handed to a shell:
//! service names, hostnames, and TXT values arrive from untrusted devices.

use arbitrary::Arbitrary;
use kinjo::discovery::Entry;
use kinjo::plumber::{ActionMode, CommandAction};
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

/// A service carrying a value for every supported field, so any template that
/// compiles can be rendered against it.
fn populated(name: &str, hostname: &str, txt_value: &str) -> Entry {
    let mut entry = Entry::new(name, "_ssh._tcp", "local");
    entry.hostname = Some(hostname.to_string());
    entry.addresses = vec!["192.0.2.1".parse().unwrap()];
    entry.port = Some(22);
    entry.txt.insert("v".to_string(), txt_value.to_string());
    entry
}

fuzz_target!(|input: Input| {
    let mut entry = Entry::new(
        input.name.clone(),
        input.service_type.clone(),
        input.domain.clone(),
    );
    entry.hostname = input.hostname;
    entry.port = input.port;
    entry.txt.insert("v".to_string(), input.txt_value.clone());

    // 1. An arbitrary template compiles or reports an error, never panics.
    if let Ok(action) = CommandAction::compile(None, input.template, ActionMode::Execute) {
        // 2. The compiled shape does not depend on the values poured into it.
        //    Both services carry every supported field, so a template naming
        //    only supported fields renders against both or neither.
        let tame = populated("tame", "host.local", "value");
        let hostile = populated(
            r#"a b" 'c' \ {d} {{e}"#,
            r#"h.local" -oProxyCommand=x '"#,
            "  x  y  ",
        );
        if let (Ok(first), Ok(second)) = (action.prepare(&tame), action.prepare(&hostile)) {
            assert_eq!(
                first.argv.len(),
                second.argv.len(),
                "argv shape must not depend on discovered values"
            );
        }
    }

    // 3. With a fixed template, arbitrary field values land in their own
    //    argument. `name`, `service_type`, `domain`, and the inserted TXT key
    //    are always present, so this must prepare successfully.
    let fixed = CommandAction::compile(
        None,
        "run {name} {service_type} {domain} {txt.v}".to_string(),
        ActionMode::Execute,
    )
    .expect("a template naming supported fields must compile");
    let prepared = fixed
        .prepare(&entry)
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
