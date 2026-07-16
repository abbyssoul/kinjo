//! What the user has narrowed the service list down to.
//!
//! The type filter is the part worth being careful about. There are two
//! different things a service type can be, and conflating them is what made the
//! `types n/m` chip lie: *discovered* is an observation of the records currently
//! in hand, and *disabled* is a preference the user expressed. Only the first
//! shrinks when a printer goes away.
//!
//! So only the preference is stored as a decision; "enabled" is derived as
//! `discovered − disabled` on demand. A type that vanishes leaves the discovered
//! set immediately and stops being counted, while its switched-off state waits
//! for it in case it comes back — which is why the count can never exceed the
//! number of types actually on screen.

use std::collections::BTreeSet;

use crate::discovery::{Entry, GroupingMode};

#[derive(Debug, Clone)]
pub struct FilterState {
    pub text_query: String,
    /// The service types present among the records discovery has right now,
    /// sorted. An observation, replaced wholesale on every recompute.
    discovered_types: Vec<String>,
    /// The types the user has switched off.
    ///
    /// A preference, not an observation, so it is deliberately *not* pruned when
    /// a type disappears: a device that drops off the network for a moment must
    /// not come back switched on after the user switched it off. It may
    /// therefore name types that are not currently discovered, which is exactly
    /// why nothing counts it directly — see [`FilterState::enabled_count`].
    disabled_types: BTreeSet<String>,
    pub host_filter: Option<String>,
    pub grouping: GroupingMode,
}

impl Default for FilterState {
    fn default() -> Self {
        Self {
            text_query: String::new(),
            discovered_types: Vec::new(),
            disabled_types: BTreeSet::new(),
            host_filter: None,
            grouping: GroupingMode::LogicalService,
        }
    }
}

impl FilterState {
    /// Replace the discovered-type observation from the current records.
    ///
    /// Wholesale rather than additive: a type with no records left is no longer
    /// discovered, and must leave the list and the count in the same recompute
    /// that its last record left the list.
    pub fn observe_types(&mut self, records: &[Entry]) {
        self.discovered_types = records
            .iter()
            .map(|record| record.service_type.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
    }

    /// The service types currently discovered, sorted. The rows of the type
    /// filter, and the only types any count here is over.
    pub fn discovered_types(&self) -> &[String] {
        &self.discovered_types
    }

    /// Whether records of `service_type` are shown. Types are shown unless the
    /// user switched them off, so an undiscovered type answers `true` — it has
    /// no records to show either way.
    pub fn is_enabled(&self, service_type: &str) -> bool {
        !self.disabled_types.contains(service_type)
    }

    /// How many of the currently discovered types are shown: the intersection,
    /// never a set length. A type the user switched off before it disappeared
    /// is not discovered, so it is counted on neither side of `enabled/total`.
    pub fn enabled_count(&self) -> usize {
        self.discovered_types
            .iter()
            .filter(|service_type| self.is_enabled(service_type))
            .count()
    }

    /// `(enabled, discovered)` for the type chip, from one consistent read so
    /// the two numbers cannot disagree.
    pub fn type_counts(&self) -> (usize, usize) {
        (self.enabled_count(), self.discovered_types.len())
    }

    pub fn toggle_service_type(&mut self, service_type: &str) {
        if !self.disabled_types.remove(service_type) {
            self.disabled_types.insert(service_type.to_string());
        }
    }

    pub fn clear_text(&mut self) {
        self.text_query.clear();
    }

    pub fn set_host_filter(&mut self, host: impl Into<String>) {
        self.host_filter = Some(host.into());
    }

    pub fn clear_host_filter(&mut self) {
        self.host_filter = None;
    }

    /// Whether the list the user is looking at is narrower than what was
    /// discovered. A preference against a type nothing is advertising hides
    /// nothing, so it does not make the filter active.
    pub fn is_active(&self) -> bool {
        !self.text_query.trim().is_empty()
            || self.host_filter.is_some()
            || self.enabled_count() < self.discovered_types.len()
    }

    pub fn apply(&self, records: &[Entry]) -> Vec<Entry> {
        records
            .iter()
            .filter(|record| self.is_enabled(&record.service_type))
            .filter(|record| match &self.host_filter {
                Some(host) => record.hostname.as_deref() == Some(host.as_str()),
                None => true,
            })
            .filter(|record| {
                self.text_query.trim().is_empty()
                    || fuzzy_match(&record.searchable_text(), self.text_query.trim())
            })
            .cloned()
            .collect()
    }
}

pub fn fuzzy_match(haystack: &str, needle: &str) -> bool {
    let mut chars = haystack.chars().flat_map(char::to_lowercase);
    for needle_char in needle.chars().flat_map(char::to_lowercase) {
        if !chars.any(|haystack_char| haystack_char == needle_char) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_match_accepts_subsequence() {
        assert!(fuzzy_match("Kitchen Printer _ipp._tcp", "kpr"));
        assert!(!fuzzy_match("Kitchen Printer", "zx"));
    }

    #[test]
    fn type_filter_and_text_filter_combine() {
        let ssh = Entry::new("alpha", "_ssh._tcp", "local");
        let http = Entry::new("beta", "_http._tcp", "local");
        let mut filter = FilterState::default();
        filter.observe_types(&[ssh.clone(), http.clone()]);
        filter.toggle_service_type("_http._tcp");
        filter.text_query = "alp".to_string();

        let visible = filter.apply(&[ssh, http]);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].name, "alpha");
    }

    #[test]
    fn discovered_types_are_unique_and_sorted() {
        let mut filter = FilterState::default();
        filter.observe_types(&[
            Entry::new("printer", "_ipp._tcp", "local"),
            Entry::new("shell", "_ssh._tcp", "local"),
            Entry::new("other printer", "_ipp._tcp", "local"),
        ]);

        assert_eq!(filter.discovered_types(), ["_ipp._tcp", "_ssh._tcp"]);
    }

    #[test]
    fn observing_records_does_not_reenable_a_disabled_type() {
        let mut filter = FilterState::default();
        filter.observe_types(&[Entry::new("alpha", "_ssh._tcp", "local")]);
        filter.toggle_service_type("_ssh._tcp");

        filter.observe_types(&[
            Entry::new("beta", "_ssh._tcp", "local"),
            Entry::new("site", "_http._tcp", "local"),
        ]);

        assert!(!filter.is_enabled("_ssh._tcp"));
        assert!(filter.is_enabled("_http._tcp"));
        assert_eq!(filter.type_counts(), (1, 2));
    }

    #[test]
    fn toggling_disabled_type_reenables_it() {
        let mut filter = FilterState::default();
        filter.observe_types(&[Entry::new("alpha", "_ssh._tcp", "local")]);

        filter.toggle_service_type("_ssh._tcp");
        filter.toggle_service_type("_ssh._tcp");

        assert!(filter.is_enabled("_ssh._tcp"));
        assert_eq!(filter.type_counts(), (1, 1));
    }

    /// The count the chip renders is over what is discovered *now*. An SSH
    /// service that has since gone away must not go on being counted as a shown
    /// type, which is what made a list showing nothing report `1/1`.
    #[test]
    fn a_type_that_disappeared_is_counted_on_neither_side() {
        let ssh = Entry::new("alpha", "_ssh._tcp", "local");
        let http = Entry::new("beta", "_http._tcp", "local");
        let mut filter = FilterState::default();
        filter.observe_types(&[ssh, http.clone()]);
        filter.toggle_service_type("_http._tcp");
        assert_eq!(filter.type_counts(), (1, 2));

        // SSH goes away; only the disabled HTTP type is left.
        filter.observe_types(std::slice::from_ref(&http));

        assert_eq!(filter.type_counts(), (0, 1));
        assert!(filter.is_active());
        assert!(filter.apply(&[http]).is_empty());
    }

    /// The preference outlives the disappearance, so a device that drops off the
    /// network and comes back does not come back switched on.
    #[test]
    fn a_disabled_type_stays_disabled_across_disappearing_and_reappearing() {
        let ssh = Entry::new("alpha", "_ssh._tcp", "local");
        let mut filter = FilterState::default();
        filter.observe_types(std::slice::from_ref(&ssh));
        filter.toggle_service_type("_ssh._tcp");

        // Gone: nothing is discovered, so nothing is narrowed either.
        filter.observe_types(&[]);
        assert_eq!(filter.type_counts(), (0, 0));
        assert!(!filter.is_active());

        // Back again, still switched off.
        filter.observe_types(std::slice::from_ref(&ssh));
        assert!(!filter.is_enabled("_ssh._tcp"));
        assert_eq!(filter.type_counts(), (0, 1));
        assert!(filter.apply(&[ssh]).is_empty());
    }

    #[test]
    fn host_filter_matches_exact_hostname_and_can_be_cleared() {
        let mut alpha = Entry::new("alpha", "_ssh._tcp", "local");
        alpha.hostname = Some("alpha.local".to_string());
        let mut beta = Entry::new("beta", "_ssh._tcp", "local");
        beta.hostname = Some("beta.local".to_string());
        let records = vec![alpha, beta];

        let mut filter = FilterState::default();
        filter.observe_types(&records);
        filter.set_host_filter("alpha.local");

        assert!(filter.is_active());
        assert_eq!(filter.apply(&records)[0].name, "alpha");

        filter.clear_host_filter();
        assert!(!filter.is_active());
        assert_eq!(filter.apply(&records).len(), 2);
    }

    #[test]
    fn fuzzy_match_accepts_empty_needle() {
        assert!(fuzzy_match("anything", ""));
    }

    #[test]
    fn clear_text_resets_query_and_active_state() {
        let mut filter = FilterState {
            text_query: "alpha".to_string(),
            ..Default::default()
        };
        assert!(filter.is_active());

        filter.clear_text();

        assert!(filter.text_query.is_empty());
        assert!(!filter.is_active());
    }

    #[test]
    fn disabling_every_type_hides_all_records() {
        let ssh = Entry::new("alpha", "_ssh._tcp", "local");
        let mut filter = FilterState::default();
        filter.observe_types(std::slice::from_ref(&ssh));
        filter.toggle_service_type("_ssh._tcp");

        assert!(filter.is_active());
        assert_eq!(filter.type_counts(), (0, 1));
        assert!(filter.apply(&[ssh]).is_empty());
    }

    #[test]
    fn whitespace_only_query_is_not_active_and_matches_all() {
        let ssh = Entry::new("alpha", "_ssh._tcp", "local");
        let mut filter = FilterState::default();
        filter.observe_types(std::slice::from_ref(&ssh));
        filter.text_query = "   ".to_string();

        assert!(!filter.is_active());
        assert_eq!(filter.apply(&[ssh]).len(), 1);
    }

    #[test]
    fn text_query_matches_searchable_txt_and_instance_fields() {
        let mut printer = Entry::new("printer", "_ipp._tcp", "local");
        printer.hostname = Some("print.local".to_string());
        printer.port = Some(631);
        printer
            .txt
            .insert("note".to_string(), "third floor".to_string());
        let mut filter = FilterState::default();
        filter.observe_types(&[printer.clone()]);
        filter.text_query = "flr".to_string();

        assert_eq!(filter.apply(&[printer]).len(), 1);
    }
}
