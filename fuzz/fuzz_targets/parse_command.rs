#![no_main]

//! Fuzz the command/action file parser. Arbitrary bytes interpreted as a TOML
//! command definition must only ever load successfully or return an `Err` —
//! never panic.
//!
//! Loading is also the validation step, so this target additionally pins what
//! "success" is allowed to mean: anything the loader accepts is a rule that
//! could actually be offered to a user. A rule with no name, no predicates, or
//! an empty program name must have been rejected, never merely tolerated and
//! left to fail once someone selects it.

use kinjo::discovery::Entry;
use kinjo::plumber::MatcherBuilder;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    let mut builder = MatcherBuilder::new();
    if builder.add_str("fuzz", source).is_err() {
        return;
    }

    let matcher = builder.build();
    for command in matcher.commands() {
        assert!(!command.name.is_empty(), "an accepted rule has a name");
        assert!(
            !command.predicates.is_empty(),
            "an accepted rule matches something"
        );
        for requirement in &command.requirements {
            assert!(
                !requirement.command.is_empty(),
                "an accepted requirement names a program"
            );
        }
        // A compiled template renders or reports a missing field; either way it
        // never panics, and it always yields a non-empty program name when it
        // does render.
        let entry = Entry::new("alpha", "_ssh._tcp", "local");
        if let Ok(prepared) = command.action.prepare(&entry) {
            assert!(
                !prepared.argv[0].is_empty(),
                "a prepared command names a program"
            );
        }
    }
});
