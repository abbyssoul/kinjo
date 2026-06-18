use std::collections::BTreeSet;

use crate::discovery::{Entry, GroupingMode};

#[derive(Debug, Clone)]
pub struct FilterState {
    pub text_query: String,
    pub enabled_service_types: BTreeSet<String>,
    pub disabled_service_types: BTreeSet<String>,
    pub host_filter: Option<String>,
    pub grouping: GroupingMode,
}

impl Default for FilterState {
    fn default() -> Self {
        Self {
            text_query: String::new(),
            enabled_service_types: BTreeSet::new(),
            disabled_service_types: BTreeSet::new(),
            host_filter: None,
            grouping: GroupingMode::LogicalService,
        }
    }
}

impl FilterState {
    pub fn sync_service_types(&mut self, records: &[Entry]) {
        for record in records {
            if !self.disabled_service_types.contains(&record.service_type) {
                self.enabled_service_types
                    .insert(record.service_type.clone());
            }
        }
    }

    pub fn discovered_types(records: &[Entry]) -> Vec<String> {
        records
            .iter()
            .map(|record| record.service_type.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn toggle_service_type(&mut self, service_type: &str) {
        if self.enabled_service_types.remove(service_type) {
            self.disabled_service_types.insert(service_type.to_string());
        } else {
            self.enabled_service_types.insert(service_type.to_string());
            self.disabled_service_types.remove(service_type);
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

    pub fn is_active(&self) -> bool {
        !self.text_query.trim().is_empty()
            || self.host_filter.is_some()
            || !self.disabled_service_types.is_empty()
    }

    pub fn apply(&self, records: &[Entry]) -> Vec<Entry> {
        records
            .iter()
            .filter(|record| self.enabled_service_types.contains(&record.service_type))
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
        filter.sync_service_types(&[ssh.clone(), http.clone()]);
        filter.toggle_service_type("_http._tcp");
        filter.text_query = "alp".to_string();

        let visible = filter.apply(&[ssh, http]);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].name, "alpha");
    }

    #[test]
    fn discovered_types_are_unique_and_sorted() {
        let records = vec![
            Entry::new("printer", "_ipp._tcp", "local"),
            Entry::new("shell", "_ssh._tcp", "local"),
            Entry::new("other printer", "_ipp._tcp", "local"),
        ];

        assert_eq!(
            FilterState::discovered_types(&records),
            vec!["_ipp._tcp".to_string(), "_ssh._tcp".to_string()]
        );
    }

    #[test]
    fn sync_service_types_does_not_reenable_disabled_types() {
        let mut filter = FilterState::default();
        filter.sync_service_types(&[Entry::new("alpha", "_ssh._tcp", "local")]);
        filter.toggle_service_type("_ssh._tcp");

        filter.sync_service_types(&[
            Entry::new("beta", "_ssh._tcp", "local"),
            Entry::new("site", "_http._tcp", "local"),
        ]);

        assert!(!filter.enabled_service_types.contains("_ssh._tcp"));
        assert!(filter.disabled_service_types.contains("_ssh._tcp"));
        assert!(filter.enabled_service_types.contains("_http._tcp"));
    }

    #[test]
    fn toggling_disabled_type_reenables_it() {
        let mut filter = FilterState::default();
        filter.sync_service_types(&[Entry::new("alpha", "_ssh._tcp", "local")]);

        filter.toggle_service_type("_ssh._tcp");
        filter.toggle_service_type("_ssh._tcp");

        assert!(filter.enabled_service_types.contains("_ssh._tcp"));
        assert!(!filter.disabled_service_types.contains("_ssh._tcp"));
    }

    #[test]
    fn host_filter_matches_exact_hostname_and_can_be_cleared() {
        let mut alpha = Entry::new("alpha", "_ssh._tcp", "local");
        alpha.hostname = Some("alpha.local".to_string());
        let mut beta = Entry::new("beta", "_ssh._tcp", "local");
        beta.hostname = Some("beta.local".to_string());
        let records = vec![alpha, beta];

        let mut filter = FilterState::default();
        filter.sync_service_types(&records);
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
        filter.sync_service_types(std::slice::from_ref(&ssh));
        filter.toggle_service_type("_ssh._tcp");

        assert!(filter.is_active());
        assert!(filter.apply(&[ssh]).is_empty());
    }

    #[test]
    fn whitespace_only_query_is_not_active_and_matches_all() {
        let ssh = Entry::new("alpha", "_ssh._tcp", "local");
        let mut filter = FilterState::default();
        filter.sync_service_types(std::slice::from_ref(&ssh));
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
        filter.sync_service_types(&[printer.clone()]);
        filter.text_query = "flr".to_string();

        assert_eq!(filter.apply(&[printer]).len(), 1);
    }
}
