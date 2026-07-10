#![no_main]

//! Fuzz the discovery-side entry model. Arbitrary service records — the values a
//! discovery backend would emit — must build, derive stable ids, expose their
//! fields, render display names, and group without panicking. On top of that,
//! two semantic properties guard against the string-joining class of bugs
//! (separators, spaces, or escape sequences inside one field bleeding into
//! another):
//!
//!  1. Two entries' ids are equal exactly when their identity fields are equal.
//!  2. Grouping neither loses nor duplicates entries, group ids stay unique,
//!     and every instance agrees with its group on the fields the mode
//!     groups by.

use std::collections::HashSet;

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

/// The fields that are supposed to identify an entry, as a structured tuple.
/// Mirrors the documented `EntryId` semantics: the registration triple plus,
/// once instance data exists, the (hostname, port) pair.
fn identity(entry: &Entry) -> (&str, &str, &str, Option<(&str, Option<u16>)>) {
    let instance = entry
        .has_instance_data()
        .then(|| (entry.hostname.as_deref().unwrap_or(""), entry.port));
    (&entry.name, &entry.service_type, &entry.domain, instance)
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

    // Identity property: id equality must coincide with field-tuple equality,
    // so a separator character inside one field can never make two different
    // identities compare equal (or split one identity in two).
    for a in &records {
        for b in &records {
            assert_eq!(
                a.id == b.id,
                identity(a) == identity(b),
                "id/field equality diverged for {a:?} vs {b:?}"
            );
        }
    }

    for mode in [
        GroupingMode::LogicalService,
        GroupingMode::Host,
        GroupingMode::ServiceType,
        GroupingMode::Command,
    ] {
        let groups = group_entries(&records, mode);

        let total: usize = groups.iter().map(|g| g.instances.len()).sum();
        assert_eq!(total, records.len(), "grouping lost or duplicated entries");

        let ids: HashSet<_> = groups.iter().map(|g| g.id.0.as_str()).collect();
        assert_eq!(ids.len(), groups.len(), "group ids collided");

        for group in &groups {
            for instance in &group.instances {
                match mode {
                    GroupingMode::LogicalService | GroupingMode::Command => {
                        assert_eq!(instance.base_display_name(), group.label);
                        assert_eq!(instance.service_type, group.service_type);
                        assert_eq!(instance.domain, group.domain);
                        assert_eq!(instance.hostname, group.hostname);
                        assert_eq!(instance.port, group.port);
                    }
                    GroupingMode::Host => assert_eq!(instance.hostname, group.hostname),
                    GroupingMode::ServiceType => {
                        assert_eq!(instance.service_type, group.service_type);
                    }
                }
            }
        }
    }
});
