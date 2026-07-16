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
//!  3. A row's facts are true of every occurrence it aggregates. A row never
//!     reports a host or service type that only its first child has: an
//!     aggregate says `Varies` instead.

use std::{collections::HashSet, num::NonZeroU32};

use arbitrary::Arbitrary;
use kinjo::discovery::{
    BrowseMode, Entry, GroupFacts, OccurrenceId, RowHost, RowServiceType, browse_groups,
};
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct RawEntry {
    name: String,
    service_type: String,
    domain: String,
    hostname: Option<String>,
    port: Option<u16>,
    txt: Vec<(String, String)>,
    /// The adapter's occurrence name, as an interface-aware backend supplies.
    occurrence: Option<NonZeroU32>,
}

/// The fields that are supposed to identify an entry, as a structured tuple.
/// Mirrors the documented `EntryId` semantics: the registration triple plus the
/// occurrence — the adapter's own name for it when it gave one, and otherwise
/// the resolved (hostname, port) endpoint once instance data exists.
fn identity(entry: &Entry) -> (&str, &str, &str, Option<OccurrenceId>, Endpoint<'_>) {
    let endpoint = (entry.occurrence().is_none() && entry.has_instance_data())
        .then(|| (entry.hostname.as_deref().unwrap_or(""), entry.port));
    (
        &entry.name,
        &entry.service_type,
        &entry.domain,
        entry.occurrence(),
        endpoint,
    )
}

type Endpoint<'a> = Option<(&'a str, Option<u16>)>;

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
            entry.with_occurrence(r.occurrence.map(OccurrenceId))
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
                a.id() == b.id(),
                identity(a) == identity(b),
                "id/field equality diverged for {a:?} vs {b:?}"
            );
        }
    }

    for mode in [
        BrowseMode::LogicalService,
        BrowseMode::Host,
        BrowseMode::ServiceType,
    ] {
        let groups = browse_groups(&records, mode);

        let total: usize = groups.iter().map(|g| g.occurrence_count()).sum();
        assert_eq!(total, records.len(), "grouping lost or duplicated entries");

        let ids: HashSet<_> = groups.iter().map(|g| g.id()).collect();
        assert_eq!(ids.len(), groups.len(), "group ids collided");

        for group in &groups {
            // The label is derived from the facts, so it can never contradict
            // them, whatever a name contains.
            assert_eq!(group.label(), group.facts().label(), "label left its facts");

            for instance in group.instances() {
                match group.facts() {
                    GroupFacts::LogicalService(service) => {
                        assert_eq!(mode, BrowseMode::LogicalService, "facts/mode disagree");
                        assert_eq!(instance.base_display_name(), service.name);
                        assert_eq!(instance.service_type, service.service_type);
                        assert_eq!(instance.domain, service.domain);
                        assert_eq!(instance.hostname, service.hostname);
                        assert_eq!(instance.port, service.port);
                    }
                    GroupFacts::Host(host) => {
                        assert_eq!(mode, BrowseMode::Host, "facts/mode disagree");
                        assert_eq!(instance.hostname, host.hostname);
                    }
                    GroupFacts::ServiceType(aggregate) => {
                        assert_eq!(mode, BrowseMode::ServiceType, "facts/mode disagree");
                        assert_eq!(instance.service_type, aggregate.service_type);
                    }
                }
            }

            // A row may only claim a host or type that every occurrence shares.
            // `Varies` is always available as the honest answer, so a claim is
            // never forced — which is what stops a first child's value from
            // standing in for the aggregate.
            match group.facts().host() {
                RowHost::Resolved(host) => assert!(
                    group
                        .instances()
                        .iter()
                        .all(|record| record.hostname.as_deref() == Some(host)),
                    "row claimed a host its occurrences do not share"
                ),
                RowHost::Unresolved => assert!(
                    group
                        .instances()
                        .iter()
                        .all(|record| record.hostname.is_none()),
                    "row claimed no host while an occurrence resolved one"
                ),
                RowHost::Varies => {}
            }

            if let RowServiceType::Invariant(service_type) = group.facts().service_type() {
                assert!(
                    group
                        .instances()
                        .iter()
                        .all(|record| record.service_type == service_type),
                    "row claimed a service type its occurrences do not share"
                );
            }
        }
    }
});
