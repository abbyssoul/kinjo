#![no_main]

//! Fuzz the command/action file parser. Arbitrary bytes interpreted as a TOML
//! command definition must only ever parse successfully or return an `Err` —
//! never panic.

use kinjo::plumber::MatcherBuilder;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(source) = std::str::from_utf8(data) {
        let mut builder = MatcherBuilder::new();
        let _ = builder.add_str("fuzz", source);
    }
});
