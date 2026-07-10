use std::{
    collections::{BTreeMap, HashMap},
    net::IpAddr,
    time::Instant,
};

/// Identity of an [`Entry`]. The registration triple (name, service type,
/// domain) identifies a DNS-SD registration; `instance` carries the resolved
/// SRV host/port identity once known, keeping concurrent instances of the same
/// registration distinct. A structured key (rather than a joined string) so
/// that separator characters appearing in a service name cannot make two
/// different identities compare equal.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntryId {
    pub name: String,
    pub service_type: String,
    pub domain: String,
    pub instance: Option<(String, Option<u16>)>,
}

impl EntryId {
    /// The id of a (not yet resolved) registration.
    pub fn registration(
        name: impl Into<String>,
        service_type: impl Into<String>,
        domain: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            service_type: service_type.into(),
            domain: domain.into(),
            instance: None,
        }
    }

    /// The registration triple shared by every instance id of one registration.
    pub fn registration_key(&self) -> (&str, &str, &str) {
        (&self.name, &self.service_type, &self.domain)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntryGroupId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GroupingMode {
    LogicalService,
    Host,
    ServiceType,
    Command,
}

impl GroupingMode {
    /// The grouping modes surfaced as top-panel tabs, in display order. The
    /// first entry is the default view shown at startup.
    pub const TABS: [GroupingMode; 4] = [
        GroupingMode::LogicalService,
        GroupingMode::Host,
        GroupingMode::ServiceType,
        GroupingMode::Command,
    ];

    pub fn label(self) -> &'static str {
        match self {
            GroupingMode::LogicalService => "logical service",
            GroupingMode::Host => "host",
            GroupingMode::ServiceType => "service type",
            GroupingMode::Command => "command",
        }
    }

    /// Short label shown on the top-panel tab for this view.
    pub fn tab_title(self) -> &'static str {
        match self {
            GroupingMode::LogicalService => "services",
            GroupingMode::Host => "hosts",
            GroupingMode::ServiceType => "types",
            GroupingMode::Command => "commands",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub id: EntryId,
    pub name: String,
    pub service_type: String,
    pub domain: String,
    pub hostname: Option<String>,
    /// All IP addresses the service's host resolved to. A service may advertise
    /// several (e.g. IPv4 + IPv6, or DNS load-balanced A records); they are kept
    /// together on the single logical-service entry. The first is the primary.
    pub addresses: Vec<IpAddr>,
    pub port: Option<u16>,
    pub txt: BTreeMap<String, String>,
    pub last_seen: Instant,
}

impl Entry {
    pub fn new(
        name: impl Into<String>,
        service_type: impl Into<String>,
        domain: impl Into<String>,
    ) -> Self {
        let name = name.into();
        let service_type = service_type.into();
        let domain = domain.into();
        let id = EntryId::registration(name.clone(), service_type.clone(), domain.clone());
        Self {
            id,
            name,
            service_type,
            domain,
            hostname: None,
            addresses: Vec::new(),
            port: None,
            txt: BTreeMap::new(),
            last_seen: Instant::now(),
        }
    }

    /// Build a resolved entry from the fields a discovery backend reports.
    /// Blank hostnames and zero ports count as "not resolved yet"; the instance
    /// id is derived from whatever instance data is present. Shared by every
    /// backend so they agree on what an unresolved field looks like.
    pub fn resolved(
        name: &str,
        service_type: &str,
        domain: &str,
        hostname: Option<&str>,
        addresses: Vec<IpAddr>,
        port: Option<u16>,
        txt: BTreeMap<String, String>,
    ) -> Self {
        let mut record = Entry::new(name, service_type, domain);
        record.hostname = hostname
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);
        record.addresses = addresses;
        record.port = port.filter(|port| *port != 0);
        record.txt = txt;
        record.with_instance_id()
    }

    pub fn with_instance_id(mut self) -> Self {
        let mut id = self.pending_id();
        if self.has_instance_data() {
            // A logical service is identified by host and port, not by its
            // addresses: a service's address set can change (records added/removed)
            // without it becoming a different service, so addresses stay out of the id.
            id.instance = Some((self.hostname.clone().unwrap_or_default(), self.port));
        }
        self.id = id;
        self
    }

    pub fn pending_id(&self) -> EntryId {
        EntryId::registration(
            self.name.clone(),
            self.service_type.clone(),
            self.domain.clone(),
        )
    }

    pub fn has_instance_data(&self) -> bool {
        self.hostname.is_some() || !self.addresses.is_empty() || self.port.is_some()
    }

    /// The primary (first) address, used wherever a single IP is needed
    /// (command templating, sorting, compact display).
    pub fn primary_address(&self) -> Option<IpAddr> {
        self.addresses.first().copied()
    }

    pub fn field(&self, field: &str) -> Option<String> {
        match field {
            "name" => Some(self.name.clone()),
            "type" | "service_type" => Some(self.service_type.clone()),
            "domain" => Some(self.domain.clone()),
            "hostname" => self.hostname.clone(),
            "address" => self.primary_address().map(|value| value.to_string()),
            "port" => self.port.map(|value| value.to_string()),
            // `strip_prefix`, not `trim_start_matches`: the latter strips the
            // prefix repeatedly, so `txt.txt.path` would look up `path`
            // instead of the TXT key literally named `txt.path`.
            field => field
                .strip_prefix("txt.")
                .and_then(|key| self.txt.get(key).cloned()),
        }
    }

    pub fn searchable_text(&self) -> String {
        let mut text = format!(
            "{} {} {} {}",
            self.name,
            self.display_name(),
            self.service_type,
            self.domain
        );
        if let Some(hostname) = &self.hostname {
            text.push(' ');
            text.push_str(hostname);
        }
        for address in &self.addresses {
            text.push(' ');
            text.push_str(&address.to_string());
        }
        if let Some(port) = self.port {
            text.push(' ');
            text.push_str(&port.to_string());
        }
        for (key, value) in &self.txt {
            text.push(' ');
            text.push_str(key);
            text.push(' ');
            text.push_str(value);
        }
        text
    }

    pub fn display_name(&self) -> String {
        decode_dns_sd_escapes(&self.name)
    }

    /// The display name with Avahi's per-interface ` [aa:bb:cc:dd:ee:ff]` MAC
    /// decoration stripped. A multi-homed host publishing the same service on
    /// several interfaces (e.g. avahi-daemon's workstation publisher) uses one
    /// instance name per interface, differing only in this suffix; the base
    /// name is what identifies the service to a person. Bracket suffixes that
    /// are not a MAC address (e.g. a printer's `[9917FB]` serial) are kept.
    pub fn base_display_name(&self) -> String {
        let display = self.display_name();
        match display.rsplit_once(" [") {
            Some((base, rest)) if !base.is_empty() && is_mac_suffix(rest) => base.to_string(),
            _ => display,
        }
    }
}

/// Whether `rest` (the text following ` [`) is a MAC address plus the closing
/// bracket: six pairs of hex digits separated by colons.
fn is_mac_suffix(rest: &str) -> bool {
    let Some(mac) = rest.strip_suffix(']') else {
        return false;
    };
    let mut groups = 0;
    for group in mac.split(':') {
        if group.len() != 2 || !group.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return false;
        }
        groups += 1;
    }
    groups == 6
}

#[derive(Debug, Clone)]
pub struct EntryGroup {
    pub id: EntryGroupId,
    pub label: String,
    pub service_type: String,
    pub domain: String,
    pub hostname: Option<String>,
    pub port: Option<u16>,
    pub txt: BTreeMap<String, String>,
    pub instances: Vec<Entry>,
}

pub fn group_entries(records: &[Entry], mode: GroupingMode) -> Vec<EntryGroup> {
    let mut buckets: HashMap<String, Vec<Entry>> = HashMap::new();
    for record in records {
        buckets
            .entry(group_key(record, mode))
            .or_default()
            .push(record.clone());
    }

    let mut groups: Vec<EntryGroup> = buckets
        .into_values()
        .map(|mut instances| {
            instances.sort_by(|a, b| {
                a.name
                    .cmp(&b.name)
                    .then_with(|| a.service_type.cmp(&b.service_type))
                    .then_with(|| a.hostname.cmp(&b.hostname))
                    .then_with(|| a.addresses.cmp(&b.addresses))
                    .then_with(|| a.port.cmp(&b.port))
            });
            let first = instances[0].clone();
            let label = group_label(&first, mode);
            let id = EntryGroupId(format!("{}:{}", mode.label(), group_key(&first, mode)));
            EntryGroup {
                id,
                label,
                service_type: first.service_type,
                domain: first.domain,
                hostname: first.hostname,
                port: first.port,
                txt: first.txt,
                instances,
            }
        })
        .collect();

    groups.sort_by(|a, b| {
        a.label
            .cmp(&b.label)
            .then_with(|| a.service_type.cmp(&b.service_type))
    });
    groups
}

fn group_key(record: &Entry, mode: GroupingMode) -> String {
    match mode {
        // `Command` grouping is handled by a dedicated path in `App`; if it ever
        // reaches `group_entries` it behaves like logical-service grouping.
        //
        // The base display name (not the raw instance name) keys the group, so
        // a multi-homed host's per-interface instances — same service, names
        // differing only in Avahi's ` [MAC]` decoration — collapse into one
        // logical service whose instances carry the per-interface addresses.
        GroupingMode::LogicalService | GroupingMode::Command => join_key(&[
            &record.base_display_name(),
            &record.service_type,
            &record.domain,
            &hostname_key(record.hostname.as_deref()),
            &record
                .port
                .map(|p| p.to_string())
                .unwrap_or_else(|| "<unknown-port>".to_string()),
        ]),
        GroupingMode::Host => hostname_key(record.hostname.as_deref()),
        GroupingMode::ServiceType => record.service_type.clone(),
    }
}

/// Keys an optional hostname with a presence tag, so a record whose hostname
/// is literally a sentinel string (e.g. `"<unresolved host>"`) can never land
/// in the same group as records with no hostname at all. A port needs no such
/// tag: its rendered form is always digits, which no sentinel matches.
fn hostname_key(hostname: Option<&str>) -> String {
    match hostname {
        Some(host) => format!("host:{host}"),
        None => "unresolved".to_string(),
    }
}

/// Joins key parts into one string unambiguously: each part is prefixed with
/// its length, so a separator character occurring *inside* a part cannot shift
/// a boundary and make two different keys compare equal — the same reasoning
/// that gave [`EntryId`] a structured key instead of a joined string.
fn join_key(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| format!("{}:{part}", part.len()))
        .collect::<Vec<_>>()
        .join("|")
}

fn group_label(record: &Entry, mode: GroupingMode) -> String {
    match mode {
        GroupingMode::LogicalService | GroupingMode::Command => record.base_display_name(),
        GroupingMode::Host => record
            .hostname
            .clone()
            .unwrap_or_else(|| "<unresolved host>".to_string()),
        GroupingMode::ServiceType => record.service_type.clone(),
    }
}

pub fn decode_dns_sd_escapes(value: &str) -> String {
    let mut bytes = Vec::with_capacity(value.len());
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            let mut encoded = [0; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
            continue;
        }

        let mut digits = String::new();
        for _ in 0..3 {
            let Some(next) = chars.peek().copied() else {
                break;
            };
            if !next.is_ascii_digit() {
                break;
            }
            digits.push(next);
            chars.next();
        }

        if digits.len() == 3
            && let Ok(byte) = digits.parse::<u8>()
        {
            bytes.push(byte);
            continue;
        }

        bytes.push(b'\\');
        bytes.extend_from_slice(digits.as_bytes());
    }

    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_builds_record_from_browse_fields() {
        let mut txt = BTreeMap::new();
        txt.insert("path".to_string(), "/admin".to_string());

        let record = Entry::resolved(
            "nas",
            "_http._tcp",
            "local",
            Some("nas.local"),
            vec!["192.168.1.30".parse().unwrap()],
            Some(8080),
            txt,
        );

        assert_eq!(record.name, "nas");
        assert_eq!(record.hostname.as_deref(), Some("nas.local"));
        assert_eq!(
            record.primary_address(),
            Some("192.168.1.30".parse().unwrap())
        );
        assert_eq!(record.port, Some(8080));
        assert_eq!(record.txt.get("path").map(String::as_str), Some("/admin"));
        assert!(record.has_instance_data());
    }

    #[test]
    fn resolved_treats_blank_host_and_zero_port_as_unresolved() {
        let record = Entry::resolved(
            "pending",
            "_ipp._tcp",
            "local",
            Some(""),
            Vec::new(),
            Some(0),
            BTreeMap::new(),
        );

        assert_eq!(record.hostname, None);
        assert!(record.addresses.is_empty());
        assert_eq!(record.port, None);
        assert!(!record.has_instance_data());
    }

    #[test]
    fn logical_service_keeps_all_its_addresses_on_one_entry() {
        let mut entry = Entry::new("host", "_ssh._tcp", "local");
        entry.hostname = Some("host.local".to_string());
        entry.addresses = vec![
            "192.168.1.10".parse().unwrap(),
            "192.168.1.11".parse().unwrap(),
        ];
        entry.port = Some(22);

        let groups = group_entries(&[entry], GroupingMode::LogicalService);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].instances.len(), 1);
        assert_eq!(groups[0].instances[0].addresses.len(), 2);
    }

    #[test]
    fn display_name_decodes_avahi_decimal_escapes() {
        let record = Entry::new(
            r"HP\032OfficeJet\032Pro\0328020\032series\032\0919917FB\093",
            "_ipp._tcp",
            "local",
        );

        assert_eq!(
            record.display_name(),
            "HP OfficeJet Pro 8020 series [9917FB]"
        );
        assert_eq!(
            record.name,
            r"HP\032OfficeJet\032Pro\0328020\032series\032\0919917FB\093"
        );
    }

    #[test]
    fn display_name_decodes_utf8_byte_escapes() {
        let record = Entry::new(r"Caf\195\169", "_http._tcp", "local");

        assert_eq!(record.display_name(), "Café");
    }

    #[test]
    fn base_display_name_strips_interface_mac_decoration() {
        // Raw Avahi escapes for `rpi5-0 [d8:3a:dd:f4:b1:dc]`.
        let per_interface = Entry::new(
            r"rpi5-0\032\091d8\0583a\058dd\058f4\058b1\058dc\093",
            "_workstation._tcp",
            "local",
        );
        assert_eq!(per_interface.base_display_name(), "rpi5-0");

        // A non-MAC bracket suffix (printer serial) is not decoration.
        let printer = Entry::new(r"HP\032Printer\032\0919917FB\093", "_ipp._tcp", "local");
        assert_eq!(printer.base_display_name(), "HP Printer [9917FB]");

        // Plain names and near-misses are untouched.
        assert_eq!(
            Entry::new("plain", "_ssh._tcp", "local").base_display_name(),
            "plain"
        );
        assert_eq!(
            Entry::new("x [d8:3a:dd:f4:b1]", "_ssh._tcp", "local").base_display_name(),
            "x [d8:3a:dd:f4:b1]"
        );
        assert_eq!(
            Entry::new("[d8:3a:dd:f4:b1:dc]", "_ssh._tcp", "local").base_display_name(),
            "[d8:3a:dd:f4:b1:dc]"
        );
    }

    #[test]
    fn per_interface_instances_merge_into_one_logical_service() {
        let mut wired = Entry::new("rpi5-0 [d8:3a:dd:f4:b1:dc]", "_workstation._tcp", "local");
        wired.hostname = Some("rpi5-0.local".to_string());
        wired.addresses = vec!["192.168.50.244".parse().unwrap()];
        wired.port = Some(9);
        let mut wireless = Entry::new("rpi5-0 [d8:3a:dd:f4:b1:dd]", "_workstation._tcp", "local");
        wireless.hostname = Some("rpi5-0.local".to_string());
        wireless.addresses = vec!["192.168.50.245".parse().unwrap()];
        wireless.port = Some(9);

        let groups = group_entries(&[wired, wireless], GroupingMode::LogicalService);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].label, "rpi5-0");
        assert_eq!(groups[0].instances.len(), 2);
    }

    #[test]
    fn different_services_sharing_an_endpoint_stay_separate() {
        // Two genuinely different services (e.g. HTTP virtual hosts) on the
        // same host and port must not be merged by the base-name grouping.
        let mut site_a = Entry::new("Site A", "_http._tcp", "local");
        site_a.hostname = Some("nas.local".to_string());
        site_a.port = Some(80);
        let mut site_b = Entry::new("Site B", "_http._tcp", "local");
        site_b.hostname = Some("nas.local".to_string());
        site_b.port = Some(80);

        let groups = group_entries(&[site_a, site_b], GroupingMode::LogicalService);

        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn logical_group_label_uses_decoded_display_name() {
        let record = Entry::new(r"HP\032Printer", "_ipp._tcp", "local");

        let groups = group_entries(&[record], GroupingMode::LogicalService);

        assert_eq!(groups[0].label, "HP Printer");
    }

    fn resolved(name: &str, service_type: &str) -> Entry {
        let mut record = Entry::new(name, service_type, "local");
        record.hostname = Some(format!("{name}.local"));
        record.addresses = vec!["192.168.1.10".parse().unwrap()];
        record.port = Some(22);
        record
    }

    #[test]
    fn field_exposes_all_supported_keys_and_aliases() {
        let mut record = Entry::new("alpha", "_ssh._tcp", "local");
        record.hostname = Some("alpha.local".to_string());
        record.addresses = vec!["192.0.2.5".parse().unwrap()];
        record.port = Some(22);
        record.txt.insert("path".to_string(), "/admin".to_string());

        assert_eq!(record.field("name").as_deref(), Some("alpha"));
        assert_eq!(record.field("type").as_deref(), Some("_ssh._tcp"));
        assert_eq!(record.field("service_type").as_deref(), Some("_ssh._tcp"));
        assert_eq!(record.field("domain").as_deref(), Some("local"));
        assert_eq!(record.field("hostname").as_deref(), Some("alpha.local"));
        assert_eq!(record.field("address").as_deref(), Some("192.0.2.5"));
        assert_eq!(record.field("port").as_deref(), Some("22"));
        assert_eq!(record.field("txt.path").as_deref(), Some("/admin"));
        assert_eq!(record.field("txt.missing"), None);
        assert_eq!(record.field("unknown"), None);
    }

    #[test]
    fn pending_record_has_no_instance_identity_until_resolved() {
        let pending = Entry::new("alpha", "_ssh._tcp", "local").with_instance_id();
        assert_eq!(pending.id, pending.pending_id());
        assert_eq!(pending.id.instance, None);
        assert!(!pending.has_instance_data());

        let resolved = resolved("alpha", "_ssh._tcp").with_instance_id();
        assert_eq!(
            resolved.id.instance,
            Some(("alpha.local".to_string(), Some(22)))
        );
        assert!(resolved.has_instance_data());
    }

    #[test]
    fn registration_key_is_the_name_type_domain_triple() {
        let resolved = resolved("alpha", "_ssh._tcp").with_instance_id();
        assert_eq!(
            resolved.id.registration_key(),
            ("alpha", "_ssh._tcp", "local")
        );
        assert_eq!(
            resolved.id.registration_key(),
            resolved.pending_id().registration_key()
        );
    }

    #[test]
    fn separator_characters_in_names_cannot_collide_ids() {
        // With a joined-string id, `a|b` + `c` and `a` + `b|c` were equal.
        let first = Entry::new("a|b", "c", "local").with_instance_id();
        let second = Entry::new("a", "b|c", "local").with_instance_id();

        assert_ne!(first.id, second.id);
        assert_ne!(first.id.registration_key(), second.id.registration_key());
    }

    #[test]
    fn separator_characters_in_names_cannot_collide_group_keys() {
        // With a `|`-joined group key, `a|b` + `c` and `a` + `b|c` bucketed
        // into the same group and were shown as one service.
        let first = Entry::new("a|b", "c", "local");
        let second = Entry::new("a", "b|c", "local");

        let groups = group_entries(&[first, second], GroupingMode::LogicalService);

        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn txt_field_lookup_strips_the_prefix_exactly_once() {
        let mut record = Entry::new("nas", "_http._tcp", "local");
        record
            .txt
            .insert("txt.path".to_string(), "nested".to_string());
        record.txt.insert("path".to_string(), "plain".to_string());

        assert_eq!(record.field("txt.path").as_deref(), Some("plain"));
        // A TXT key literally named `txt.path` is reachable as `txt.txt.path`.
        assert_eq!(record.field("txt.txt.path").as_deref(), Some("nested"));
    }

    #[test]
    fn searchable_text_includes_every_instance_field() {
        let mut record = Entry::new("alpha", "_ssh._tcp", "local");
        record.hostname = Some("alpha.local".to_string());
        record.addresses = vec!["192.0.2.5".parse().unwrap()];
        record.port = Some(2222);
        record
            .txt
            .insert("note".to_string(), "third floor".to_string());

        let text = record.searchable_text();
        for needle in [
            "alpha",
            "_ssh._tcp",
            "local",
            "alpha.local",
            "192.0.2.5",
            "2222",
            "note",
            "third floor",
        ] {
            assert!(text.contains(needle), "missing `{needle}` in `{text}`");
        }
    }

    #[test]
    fn grouping_by_host_buckets_records_and_labels_unresolved() {
        let a = resolved("alpha", "_ssh._tcp");
        let b = resolved("beta", "_http._tcp");
        let pending = Entry::new("ghost", "_ipp._tcp", "local");

        let groups = group_entries(&[a, b, pending], GroupingMode::Host);

        let labels: Vec<&str> = groups.iter().map(|g| g.label.as_str()).collect();
        assert!(labels.contains(&"alpha.local"));
        assert!(labels.contains(&"beta.local"));
        assert!(labels.contains(&"<unresolved host>"));
    }

    #[test]
    fn hostname_equal_to_the_sentinel_does_not_join_the_unresolved_group() {
        let mut impostor = Entry::new("impostor", "_ssh._tcp", "local");
        impostor.hostname = Some("<unresolved host>".to_string());
        let pending = Entry::new("ghost", "_ipp._tcp", "local");

        let groups = group_entries(&[impostor, pending], GroupingMode::Host);

        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn grouping_by_service_type_merges_same_type() {
        let a = resolved("alpha", "_ssh._tcp");
        let b = resolved("beta", "_ssh._tcp");
        let c = resolved("gamma", "_http._tcp");

        let groups = group_entries(&[a, b, c], GroupingMode::ServiceType);

        assert_eq!(groups.len(), 2);
        let ssh = groups
            .iter()
            .find(|g| g.label == "_ssh._tcp")
            .expect("ssh group");
        assert_eq!(ssh.instances.len(), 2);
    }

    #[test]
    fn groups_are_sorted_by_label() {
        let groups = group_entries(
            &[
                resolved("charlie", "_ssh._tcp"),
                resolved("alpha", "_ssh._tcp"),
                resolved("bravo", "_ssh._tcp"),
            ],
            GroupingMode::LogicalService,
        );

        let labels: Vec<&str> = groups.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, vec!["alpha", "bravo", "charlie"]);
    }

    #[test]
    fn decode_handles_incomplete_and_out_of_range_escapes() {
        // Fewer than three digits: the backslash and digits are kept verbatim.
        assert_eq!(decode_dns_sd_escapes(r"a\09b"), r"a\09b");
        // A trailing lone backslash is preserved.
        assert_eq!(decode_dns_sd_escapes(r"a\"), r"a\");
        // `\999` is three digits but 999 does not fit in a byte, so it is kept.
        assert_eq!(decode_dns_sd_escapes(r"x\999y"), r"x\999y");
    }

    #[test]
    fn command_mode_group_key_behaves_like_logical_service() {
        let mut entry = resolved("alpha", "_ssh._tcp");
        entry.addresses = vec!["10.0.0.1".parse().unwrap(), "10.0.0.2".parse().unwrap()];

        let groups = group_entries(&[entry], GroupingMode::Command);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].instances.len(), 1);
        assert_eq!(groups[0].label, "alpha");
    }
}
