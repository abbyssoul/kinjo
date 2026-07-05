#![no_main]

//! Fuzz the discovery-side entry model. Arbitrary service records — the values a
//! discovery backend would emit — must build, derive stable ids, expose their
//! fields, render display names, and group without panicking.

use arbitrary::Arbitrary;
use kinjo::discovery::{Entry, GroupingMode, group_entries};
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct RawEntry {
    name: String,
    service_type: String,
    domain: String,
    hostname: Option<String>,
    port: Option<u16>,
    txt: Vec<(String, String)>,
}

fuzz_target!(|raw: Vec<RawEntry>| {
    let records: Vec<Entry> = raw
        .into_iter()
        .map(|r| {
            let mut entry = Entry::new(r.name, r.service_type, r.domain);
            entry.hostname = r.hostname;
            entry.port = r.port;
            for (key, value) in r.txt {
                entry.txt.insert(key, value);
            }
            entry.with_instance_id()
        })
        .collect();

    for entry in &records {
        let _ = entry.display_name();
        let _ = entry.searchable_text();
        let _ = entry.field("txt.path");
        let _ = entry.has_instance_data();
    }

    for mode in [
        GroupingMode::LogicalService,
        GroupingMode::Host,
        GroupingMode::ServiceType,
        GroupingMode::Command,
    ] {
        let _ = group_entries(&records, mode);
    }
});
