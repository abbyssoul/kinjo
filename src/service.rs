use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    net::IpAddr,
    time::Instant,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServiceId(pub String);

impl ServiceId {
    pub fn registration_key(&self) -> String {
        self.0.split('|').take(3).collect::<Vec<_>>().join("|")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServiceGroupId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GroupingMode {
    LogicalService,
    Host,
    ServiceType,
    Port,
    Address,
    Command,
}

impl GroupingMode {
    pub const ALL: [GroupingMode; 6] = [
        GroupingMode::LogicalService,
        GroupingMode::Host,
        GroupingMode::ServiceType,
        GroupingMode::Port,
        GroupingMode::Address,
        GroupingMode::Command,
    ];

    pub fn label(self) -> &'static str {
        match self {
            GroupingMode::LogicalService => "logical service",
            GroupingMode::Host => "host",
            GroupingMode::ServiceType => "service type",
            GroupingMode::Port => "port",
            GroupingMode::Address => "address",
            GroupingMode::Command => "command",
        }
    }
}

impl fmt::Display for GroupingMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone)]
pub struct ServiceRecord {
    pub id: ServiceId,
    pub name: String,
    pub service_type: String,
    pub domain: String,
    pub hostname: Option<String>,
    pub address: Option<IpAddr>,
    pub port: Option<u16>,
    pub txt: BTreeMap<String, String>,
    pub last_seen: Instant,
}

impl ServiceRecord {
    pub fn new(
        name: impl Into<String>,
        service_type: impl Into<String>,
        domain: impl Into<String>,
    ) -> Self {
        let name = name.into();
        let service_type = service_type.into();
        let domain = domain.into();
        let id = ServiceId(format!("{name}|{service_type}|{domain}"));
        Self {
            id,
            name,
            service_type,
            domain,
            hostname: None,
            address: None,
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

        self.id = ServiceId(format!(
            "{}|{}|{}|{}|{}|{}",
            self.name,
            self.service_type,
            self.domain,
            self.hostname.as_deref().unwrap_or(""),
            self.address.map(|a| a.to_string()).unwrap_or_default(),
            self.port.map(|p| p.to_string()).unwrap_or_default()
        ));
        self
    }

    pub fn pending_id(&self) -> ServiceId {
        ServiceId(format!(
            "{}|{}|{}",
            self.name, self.service_type, self.domain
        ))
    }

    pub fn has_instance_data(&self) -> bool {
        self.hostname.is_some() || self.address.is_some() || self.port.is_some()
    }

    pub fn field(&self, field: &str) -> Option<String> {
        match field {
            "name" => Some(self.name.clone()),
            "type" | "service_type" => Some(self.service_type.clone()),
            "domain" => Some(self.domain.clone()),
            "hostname" => self.hostname.clone(),
            "address" => self.address.map(|value| value.to_string()),
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
        if let Some(address) = self.address {
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
pub struct ServiceGroup {
    pub id: ServiceGroupId,
    pub label: String,
    pub service_type: String,
    pub domain: String,
    pub hostname: Option<String>,
    pub port: Option<u16>,
    pub txt: BTreeMap<String, String>,
    pub instances: Vec<ServiceRecord>,
}

pub fn group_records(records: &[ServiceRecord], mode: GroupingMode) -> Vec<ServiceGroup> {
    let mut buckets: HashMap<String, Vec<ServiceRecord>> = HashMap::new();
    for record in records {
        buckets
            .entry(group_key(record, mode))
            .or_default()
            .push(record.clone());
    }

    let mut groups: Vec<ServiceGroup> = buckets
        .into_values()
        .map(|mut instances| {
            instances.sort_by(|a, b| {
                a.name
                    .cmp(&b.name)
                    .then_with(|| a.service_type.cmp(&b.service_type))
                    .then_with(|| a.hostname.cmp(&b.hostname))
                    .then_with(|| a.address.cmp(&b.address))
                    .then_with(|| a.port.cmp(&b.port))
            });
            let first = instances[0].clone();
            let label = group_label(&first, mode);
            let id = ServiceGroupId(format!("{}:{}", mode.label(), group_key(&first, mode)));
            ServiceGroup {
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

fn group_key(record: &ServiceRecord, mode: GroupingMode) -> String {
    match mode {
        // `Command` grouping is handled by a dedicated path in `App`; if it ever
        // reaches `group_records` it behaves like logical-service grouping.
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
        GroupingMode::Port => record
            .port
            .map(|p| p.to_string())
            .unwrap_or_else(|| "<unknown port>".to_string()),
        GroupingMode::Address => record
            .address
            .map(|a| a.to_string())
            .unwrap_or_else(|| "<unknown address>".to_string()),
    }
}

fn group_label(record: &ServiceRecord, mode: GroupingMode) -> String {
    match mode {
        GroupingMode::LogicalService | GroupingMode::Command => record.display_name(),
        GroupingMode::Host => record
            .hostname
            .clone()
            .unwrap_or_else(|| "<unresolved host>".to_string()),
        GroupingMode::ServiceType => record.service_type.clone(),
        GroupingMode::Port => record
            .port
            .map(|p| p.to_string())
            .unwrap_or_else(|| "<unknown port>".to_string()),
        GroupingMode::Address => record
            .address
            .map(|a| a.to_string())
            .unwrap_or_else(|| "<unknown address>".to_string()),
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
    fn logical_grouping_preserves_distinct_addresses() {
        let mut a = ServiceRecord::new("host", "_ssh._tcp", "local");
        a.hostname = Some("host.local".to_string());
        a.address = Some("192.168.1.10".parse().unwrap());
        a.port = Some(22);
        let mut b = a.clone();
        b.address = Some("192.168.1.11".parse().unwrap());

        let groups = group_records(&[a, b], GroupingMode::LogicalService);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].instances.len(), 2);
    }

    #[test]
    fn display_name_decodes_avahi_decimal_escapes() {
        let record = ServiceRecord::new(
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
        let record = ServiceRecord::new(r"Caf\195\169", "_http._tcp", "local");

        assert_eq!(record.display_name(), "Café");
    }

    #[test]
    fn logical_group_label_uses_decoded_display_name() {
        let record = ServiceRecord::new(r"HP\032Printer", "_ipp._tcp", "local");

        let groups = group_records(&[record], GroupingMode::LogicalService);

        assert_eq!(groups[0].label, "HP Printer");
    }

    fn resolved(name: &str, service_type: &str) -> ServiceRecord {
        let mut record = ServiceRecord::new(name, service_type, "local");
        record.hostname = Some(format!("{name}.local"));
        record.address = Some("192.168.1.10".parse().unwrap());
        record.port = Some(22);
        record
    }

    #[test]
    fn field_exposes_all_supported_keys_and_aliases() {
        let mut record = ServiceRecord::new("alpha", "_ssh._tcp", "local");
        record.hostname = Some("alpha.local".to_string());
        record.address = Some("192.0.2.5".parse().unwrap());
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
        let pending = ServiceRecord::new("alpha", "_ssh._tcp", "local").with_instance_id();
        assert_eq!(pending.id.0, "alpha|_ssh._tcp|local");
        assert!(!pending.has_instance_data());

        let resolved = resolved("alpha", "_ssh._tcp").with_instance_id();
        assert_eq!(
            resolved.id.0,
            "alpha|_ssh._tcp|local|alpha.local|192.168.1.10|22"
        );
        assert!(resolved.has_instance_data());
    }

    #[test]
    fn registration_key_keeps_first_three_id_fields() {
        let resolved = resolved("alpha", "_ssh._tcp").with_instance_id();
        assert_eq!(resolved.id.registration_key(), "alpha|_ssh._tcp|local");
    }

    #[test]
    fn searchable_text_includes_every_instance_field() {
        let mut record = ServiceRecord::new("alpha", "_ssh._tcp", "local");
        record.hostname = Some("alpha.local".to_string());
        record.address = Some("192.0.2.5".parse().unwrap());
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
        let pending = ServiceRecord::new("ghost", "_ipp._tcp", "local");

        let groups = group_records(&[a, b, pending], GroupingMode::Host);

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

        let groups = group_records(&[a, b, c], GroupingMode::ServiceType);

        assert_eq!(groups.len(), 2);
        let ssh = groups
            .iter()
            .find(|g| g.label == "_ssh._tcp")
            .expect("ssh group");
        assert_eq!(ssh.instances.len(), 2);
    }

    #[test]
    fn grouping_by_port_and_address_uses_value_labels_and_fallbacks() {
        let mut a = resolved("alpha", "_ssh._tcp");
        a.port = Some(80);
        a.address = Some("10.0.0.1".parse().unwrap());
        let pending = ServiceRecord::new("ghost", "_ipp._tcp", "local");

        let by_port = group_records(&[a.clone(), pending.clone()], GroupingMode::Port);
        let port_labels: Vec<&str> = by_port.iter().map(|g| g.label.as_str()).collect();
        assert!(port_labels.contains(&"80"));
        assert!(port_labels.contains(&"<unknown port>"));

        let by_address = group_records(&[a, pending], GroupingMode::Address);
        let address_labels: Vec<&str> = by_address.iter().map(|g| g.label.as_str()).collect();
        assert!(address_labels.contains(&"10.0.0.1"));
        assert!(address_labels.contains(&"<unknown address>"));
    }

    #[test]
    fn groups_are_sorted_by_label() {
        let groups = group_records(
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
        let mut a = resolved("alpha", "_ssh._tcp");
        a.address = Some("10.0.0.1".parse().unwrap());
        let mut b = a.clone();
        b.address = Some("10.0.0.2".parse().unwrap());

        let groups = group_records(&[a, b], GroupingMode::Command);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].instances.len(), 2);
        assert_eq!(groups[0].label, "alpha");
    }
}
