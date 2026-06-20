use std::{
    collections::{BTreeMap, HashMap},
    net::IpAddr,
    time::Instant,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntryId(pub String);

impl EntryId {
    pub fn registration_key(&self) -> String {
        self.0.split('|').take(3).collect::<Vec<_>>().join("|")
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
        let id = EntryId(format!("{name}|{service_type}|{domain}"));
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

    pub fn with_instance_id(mut self) -> Self {
        if !self.has_instance_data() {
            self.id = self.pending_id();
            return self;
        }

        // A logical service is identified by host and port, not by its
        // addresses: a service's address set can change (records added/removed)
        // without it becoming a different service, so addresses stay out of the id.
        self.id = EntryId(format!(
            "{}|{}|{}|{}|{}",
            self.name,
            self.service_type,
            self.domain,
            self.hostname.as_deref().unwrap_or(""),
            self.port.map(|p| p.to_string()).unwrap_or_default()
        ));
        self
    }

    pub fn pending_id(&self) -> EntryId {
        EntryId(format!(
            "{}|{}|{}",
            self.name, self.service_type, self.domain
        ))
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
            field if field.starts_with("txt.") => {
                let key = field.trim_start_matches("txt.");
                self.txt.get(key).cloned()
            }
            _ => None,
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
        GroupingMode::LogicalService | GroupingMode::Command => format!(
            "{}|{}|{}|{}|{}",
            record.name,
            record.service_type,
            record.domain,
            record.hostname.as_deref().unwrap_or("<unresolved-host>"),
            record
                .port
                .map(|p| p.to_string())
                .unwrap_or_else(|| "<unknown-port>".to_string())
        ),
        GroupingMode::Host => record
            .hostname
            .clone()
            .unwrap_or_else(|| "<unresolved host>".to_string()),
        GroupingMode::ServiceType => record.service_type.clone(),
    }
}

fn group_label(record: &Entry, mode: GroupingMode) -> String {
    match mode {
        GroupingMode::LogicalService | GroupingMode::Command => record.display_name(),
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
    fn pending_record_keeps_three_field_id_until_resolved() {
        let pending = Entry::new("alpha", "_ssh._tcp", "local").with_instance_id();
        assert_eq!(pending.id.0, "alpha|_ssh._tcp|local");
        assert!(!pending.has_instance_data());

        let resolved = resolved("alpha", "_ssh._tcp").with_instance_id();
        assert_eq!(resolved.id.0, "alpha|_ssh._tcp|local|alpha.local|22");
        assert!(resolved.has_instance_data());
    }

    #[test]
    fn registration_key_keeps_first_three_id_fields() {
        let resolved = resolved("alpha", "_ssh._tcp").with_instance_id();
        assert_eq!(resolved.id.registration_key(), "alpha|_ssh._tcp|local");
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
